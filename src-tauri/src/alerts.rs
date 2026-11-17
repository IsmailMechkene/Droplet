//! Which alert (if any) to raise for a provider, per the Alerts settings.
//!
//! - warning / critical: when the main limit *crosses* the level,
//! - run out: when the forecast says the limit runs out at least the chosen
//!   lead time before its reset,
//! - reset: when a limit starts a new period,
//! - resets soon: 10 minutes before a reset.
//!
//! Each kind fires at most once per period of a limit, a provider gets at most
//! one alert per 15 minutes, and nothing is raised during quiet hours (an alert
//! skipped for quiet hours or the 15-minute cap is not delivered later).

use std::collections::{HashMap, HashSet};

use crate::config::validate::parse_hhmm;
use crate::config::Config;
use crate::forecast::Outlook;
use crate::providers::ProviderId;

pub const PER_PROVIDER_GAP_SECS: i64 = 15 * 60;
pub const RESET_SOON_SECS: i64 = 10 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Kind {
    // Most important first: when several are due at once, the first one is sent.
    Critical,
    RunOut,
    Warning,
    Reset,
    ResetSoon,
}

/// What is known about a provider's main limit at one moment.
#[derive(Debug, Clone, PartialEq)]
pub struct Observation {
    pub id: ProviderId,
    pub pct: f64,
    /// The previous reading in the same period, if any. Crossings need one,
    /// so a level that was already reached before Droplet started does not alert.
    pub prev_pct: Option<f64>,
    /// This reading started a new period, after an earlier one.
    pub new_period: bool,
    pub resets_at: Option<i64>,
    pub outlook: Outlook,
}

/// Alerts already sent, kept in memory.
#[derive(Debug, Default)]
pub struct Memory {
    fired: HashSet<(ProviderId, Kind, i64)>,
    last_sent: HashMap<ProviderId, i64>,
}

/// Identifies a period by its reset time, in 10-minute buckets so that small
/// jitter in "resets in N seconds" answers does not look like a new period.
fn period_key(resets_at: Option<i64>) -> i64 {
    resets_at.map_or(0, |r| r.div_euclid(600))
}

/// True when `minutes` (since local midnight) is inside quiet hours. Spans midnight when from > to.
pub fn in_quiet_hours(config: &Config, minutes: u32) -> bool {
    let q = &config.alerts.quiet;
    let (Some(from), Some(to)) = (parse_hhmm(&q.from), parse_hhmm(&q.to)) else { return false };
    q.enabled
        && match from.cmp(&to) {
            std::cmp::Ordering::Less => minutes >= from && minutes < to,
            std::cmp::Ordering::Greater => minutes >= from || minutes < to,
            std::cmp::Ordering::Equal => false,
        }
}

/// Kinds that are due for this observation, before de-duplication.
fn due(o: &Observation, config: &Config, now: i64) -> Vec<Kind> {
    let a = &config.alerts;
    let (warn, crit) = (config.appearance.warn_at as f64, config.appearance.crit_at as f64);
    let crossed = |level: f64| o.prev_pct.is_some_and(|prev| prev < level) && o.pct >= level;
    let mut kinds = Vec::new();
    if a.critical && crossed(crit) {
        kinds.push(Kind::Critical);
    }
    if a.run_out {
        if let Outlook::RunsOut { before_reset_secs: Some(before), .. } = o.outlook {
            if before >= a.run_out_lead_min as i64 * 60 {
                kinds.push(Kind::RunOut);
            }
        }
    }
    if a.warning && crossed(warn) && !crossed(crit) {
        kinds.push(Kind::Warning);
    }
    if a.reset && o.new_period {
        kinds.push(Kind::Reset);
    }
    if a.reset_soon {
        if let Some(reset) = o.resets_at {
            let left = reset - now;
            if left > 0 && left <= RESET_SOON_SECS && o.pct > 0.0 {
                kinds.push(Kind::ResetSoon);
            }
        }
    }
    kinds
}

/// Decides what to send now. Everything that was due is remembered as handled,
/// whether it was sent, merged into a more important alert, or held back.
pub fn decide(o: &Observation, config: &Config, memory: &mut Memory, now: i64, local_minutes: u32) -> Option<Kind> {
    let period = period_key(o.resets_at);
    let fresh: Vec<Kind> = due(o, config, now).into_iter().filter(|k| !memory.fired.contains(&(o.id, *k, period))).collect();
    let first = fresh.iter().min().copied()?;
    for kind in &fresh {
        memory.fired.insert((o.id, *kind, period));
    }
    if in_quiet_hours(config, local_minutes) {
        return None;
    }
    if memory.last_sent.get(&o.id).is_some_and(|at| now - at < PER_PROVIDER_GAP_SECS) {
        return None;
    }
    memory.last_sent.insert(o.id, now);
    Some(first)
}

/// The next moment a "resets in 10 minutes" alert could become due, so the
/// worker can wake for it instead of polling.
pub fn next_reset_soon(resets: impl Iterator<Item = i64>, now: i64) -> Option<i64> {
    resets.map(|r| r - RESET_SOON_SECS).filter(|at| *at > now).min()
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_794_800_400; // a multiple of 600
    const NOON: u32 = 12 * 60;

    fn obs(prev: Option<f64>, pct: f64) -> Observation {
        Observation { id: ProviderId::Claude, pct, prev_pct: prev, new_period: false, resets_at: Some(T0 + 3 * 3600), outlook: Outlook::Learning }
    }

    #[test]
    fn crossing_levels() {
        let config = Config::default();
        let mut m = Memory::default();
        assert_eq!(decide(&obs(Some(60.0), 72.0), &config, &mut m, T0, NOON), Some(Kind::Warning));
        let mut m = Memory::default();
        assert_eq!(decide(&obs(Some(80.0), 91.0), &config, &mut m, T0, NOON), Some(Kind::Critical));
        let mut m = Memory::default();
        assert_eq!(decide(&obs(Some(60.0), 95.0), &config, &mut m, T0, NOON), Some(Kind::Critical), "jumping both levels: critical only");
        let mut m = Memory::default();
        assert_eq!(decide(&obs(None, 95.0), &config, &mut m, T0, NOON), None, "no previous reading, no crossing");
        assert_eq!(decide(&obs(Some(75.0), 80.0), &config, &mut m, T0, NOON), None, "already past the level");
    }

    #[test]
    fn once_per_period_and_per_provider_gap() {
        let config = Config::default();
        let mut m = Memory::default();
        assert_eq!(decide(&obs(Some(60.0), 72.0), &config, &mut m, T0, NOON), Some(Kind::Warning));
        // Dropped back and crossed again in the same period: not repeated.
        assert_eq!(decide(&obs(Some(65.0), 71.0), &config, &mut m, T0 + 3600, NOON), None);
        // Critical 5 minutes later: held back by the 15-minute gap, and not sent later either.
        assert_eq!(decide(&obs(Some(85.0), 92.0), &config, &mut m, T0 + 300, NOON), None);
        assert_eq!(decide(&obs(Some(85.0), 93.0), &config, &mut m, T0 + 1200, NOON), None);
        // The next period alerts again.
        let mut next = obs(Some(60.0), 72.0);
        next.resets_at = Some(T0 + 8 * 3600);
        assert_eq!(decide(&next, &config, &mut m, T0 + 5 * 3600, NOON), Some(Kind::Warning));
    }

    #[test]
    fn other_providers_are_independent() {
        let config = Config::default();
        let mut m = Memory::default();
        assert!(decide(&obs(Some(60.0), 72.0), &config, &mut m, T0, NOON).is_some());
        let codex = Observation { id: ProviderId::Codex, ..obs(Some(60.0), 72.0) };
        assert_eq!(decide(&codex, &config, &mut m, T0 + 60, NOON), Some(Kind::Warning));
    }

    #[test]
    fn run_out_respects_the_lead_time() {
        let config = Config::default(); // 30 minutes
        let out = |before| Observation { outlook: Outlook::RunsOut { at: T0 + 600, before_reset_secs: Some(before) }, ..obs(Some(50.0), 52.0) };
        assert_eq!(decide(&out(45 * 60), &config, &mut Memory::default(), T0, NOON), Some(Kind::RunOut));
        assert_eq!(decide(&out(20 * 60), &config, &mut Memory::default(), T0, NOON), None, "a close call is ignored");
        let mut off = Config::default();
        off.alerts.run_out = false;
        assert_eq!(decide(&out(45 * 60), &off, &mut Memory::default(), T0, NOON), None);
    }

    #[test]
    fn reset_and_reset_soon() {
        let config = Config::default();
        let reset = Observation { new_period: true, ..obs(None, 0.0) };
        assert_eq!(decide(&reset, &config, &mut Memory::default(), T0, NOON), Some(Kind::Reset));

        let soon = Observation { resets_at: Some(T0 + 300), ..obs(Some(40.0), 40.0) };
        assert_eq!(decide(&soon, &config, &mut Memory::default(), T0, NOON), None, "off by default");
        let mut on = Config::default();
        on.alerts.reset_soon = true;
        assert_eq!(decide(&soon, &on, &mut Memory::default(), T0, NOON), Some(Kind::ResetSoon));
        let later = Observation { resets_at: Some(T0 + 3600), ..soon.clone() };
        assert_eq!(decide(&later, &on, &mut Memory::default(), T0, NOON), None);
    }

    #[test]
    fn toggles_turn_kinds_off() {
        let mut config = Config::default();
        config.alerts.warning = false;
        assert_eq!(decide(&obs(Some(60.0), 72.0), &config, &mut Memory::default(), T0, NOON), None);
        config.alerts.critical = false;
        assert_eq!(decide(&obs(Some(80.0), 95.0), &config, &mut Memory::default(), T0, NOON), None);
    }

    #[test]
    fn quiet_hours() {
        let mut config = Config::default();
        assert!(!in_quiet_hours(&config, 23 * 60), "off by default");
        config.alerts.quiet.enabled = true; // 22:00 to 08:00
        assert!(in_quiet_hours(&config, 23 * 60));
        assert!(in_quiet_hours(&config, 7 * 60 + 59));
        assert!(!in_quiet_hours(&config, 8 * 60));
        assert!(!in_quiet_hours(&config, NOON));
        config.alerts.quiet.from = "13:00".into();
        config.alerts.quiet.to = "14:00".into();
        assert!(in_quiet_hours(&config, 13 * 60 + 30));
        assert!(!in_quiet_hours(&config, 14 * 60));

        let mut m = Memory::default();
        assert_eq!(decide(&obs(Some(60.0), 72.0), &config, &mut m, T0, 13 * 60 + 30), None);
        assert_eq!(decide(&obs(Some(60.0), 72.0), &config, &mut m, T0 + 3600, NOON), None, "not delivered later");
    }

    #[test]
    fn wakes_for_the_next_reset_soon() {
        assert_eq!(next_reset_soon([T0 + 3600, T0 + 900, T0 + 300].into_iter(), T0), Some(T0 + 300));
        assert_eq!(next_reset_soon([T0 + 300].into_iter(), T0), None, "already inside the window");
    }
}
