//! The one worker thread. It polls providers one request at a time and sleeps
//! in between, so the app uses no CPU at rest. Besides the poll interval it
//! only wakes for things that are due at a known moment: a "resets in 10
//! minutes" alert and the Monday summary.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter, Manager};

use crate::alerts::{self, Observation};
use crate::history::{self, History};
use crate::providers::{self, FetchError, ProviderId, Usage, DAY, HOUR};
use crate::state::{main_limit, outlook_for, Core, Status};
use crate::{clock, net, notify, win};

/// Why a refresh happens. Antigravity is expensive, so it only reacts to some of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    Startup,
    Timer,
    Manual,
    CardOpen,
}

pub enum Msg {
    RefreshAll(Reason),
    RefreshOne(ProviderId, Reason),
    /// Settings changed: re-evaluate timers, apply retention and redraw.
    ConfigChanged,
    /// History was cleared from Settings.
    HistoryCleared,
}

/// How long history is replayed into the forecast at startup.
const REPLAY_SECS: i64 = 8 * DAY;

pub fn start(app: AppHandle) {
    let (tx, rx) = mpsc::channel();
    let core = app.state::<Arc<Core>>().inner().clone();
    *core.scheduler.lock().unwrap() = Some(tx);
    std::thread::Builder::new()
        .name("droplet-worker".into())
        .spawn(move || run(app, core, rx))
        .expect("start worker thread");
}

fn interval(core: &Core) -> Duration {
    // The config validator only allows 3, 5, 10 or 15; never poll faster than 3 minutes.
    Duration::from_secs(core.config.read().unwrap().general.refresh_minutes.max(3) as u64 * 60)
}

fn run(app: AppHandle, core: Arc<Core>, rx: Receiver<Msg>) {
    replay_history(&core);
    refresh_all(&app, &core, Reason::Startup);
    let mut next_poll = Instant::now() + interval(&core);
    loop {
        let poll_in = next_poll.saturating_duration_since(Instant::now());
        let wait = match next_deadline(&core) {
            Some(at) => poll_in.min(Duration::from_secs((at - clock::now()).max(1) as u64)),
            None => poll_in,
        };
        match rx.recv_timeout(wait) {
            Ok(Msg::RefreshAll(reason)) => {
                refresh_all(&app, &core, reason);
                next_poll = Instant::now() + interval(&core);
            }
            Ok(Msg::RefreshOne(id, reason)) => refresh_one(&app, &core, id, reason),
            Ok(Msg::ConfigChanged) => {
                apply_retention(&core);
                publish(&app);
            }
            Ok(Msg::HistoryCleared) => {
                for state in core.providers.lock().unwrap().values_mut() {
                    state.series.clear();
                    state.spark.clear();
                }
                publish(&app);
            }
            Err(RecvTimeoutError::Timeout) => {
                if Instant::now() >= next_poll {
                    refresh_all(&app, &core, Reason::Timer);
                    next_poll = Instant::now() + interval(&core);
                }
                timed_checks(&app, &core);
            }
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn refresh_all(app: &AppHandle, core: &Core, reason: Reason) {
    let order = core.config.read().unwrap().providers.order.clone();
    for id in order {
        refresh_one(app, core, id, reason);
    }
    timed_checks(app, core);
}

fn refresh_one(app: &AppHandle, core: &Core, id: ProviderId, reason: Reason) {
    let config = core.config();
    if !config.providers.get(id).enabled {
        return;
    }
    // Opening a card only re-reads Antigravity; for the others it just refreshes the wording.
    if reason == Reason::CardOpen && id != ProviderId::Antigravity {
        publish(app);
        return;
    }
    let now = clock::now();
    {
        let mut all = core.providers.lock().unwrap();
        let state = all.entry(id).or_default();
        // agy.exe is heavy: never on the timer, and at most once per 10 minutes.
        if id == ProviderId::Antigravity {
            let recent = state.checked_at.is_some_and(|at| now - at < providers::antigravity::MIN_INTERVAL_SECS);
            if reason == Reason::Timer || recent {
                if reason == Reason::CardOpen {
                    drop(all);
                    publish(app);
                }
                return;
            }
        }
        // Honour a 429 back-off even for manual refreshes.
        if now < state.retry_at {
            return;
        }
        state.loading = true;
    }
    publish(app);

    crate::trace!("fetch {} start", id.key());
    let result = providers::fetch(id, &core.net, &config);
    crate::trace!("fetch {} done: {:?}", id.key(), result.as_ref().map(|u| u.limits.len()));
    let now = clock::now();
    if let Ok(usage) = &result {
        record_history(core, id, usage, now);
    }
    let change = apply(core, id, result, now);
    publish(app);
    if let Some(change) = change {
        check_alert(app, core, id, change.prev_main, change.new_period, now);
    }
}

/// How a successful reading moved the main limit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Change {
    /// The previous main reading in the same period.
    pub prev_main: Option<f64>,
    /// The main limit started a new period with this reading.
    pub new_period: bool,
}

/// Records the outcome of one fetch. On failure the last reading is kept and
/// shown as stale; a number is never made up. Returns how the main limit moved.
pub fn apply(core: &Core, id: ProviderId, result: Result<Usage, FetchError>, now: i64) -> Option<Change> {
    let main_id = core.config.read().unwrap().providers.get(id).main_limit.clone();
    let mut all = core.providers.lock().unwrap();
    let state = all.entry(id).or_default();
    state.loading = false;
    state.checked_at = Some(now);
    match result {
        Ok(usage) => {
            let main = main_limit(&usage, &main_id).map(|l| l.id.clone());
            let mut change = Change { prev_main: None, new_period: false };
            for limit in &usage.limits {
                let series = state.series.entry(limit.id.clone()).or_default();
                let prev = series.samples.last().map(|s| s.pct);
                let new_period = series.push(now, limit.used_pct, limit.resets_at);
                if Some(&limit.id) == main.as_ref() {
                    change = Change { prev_main: if new_period { None } else { prev }, new_period: new_period && prev.is_some() };
                }
            }
            state.usage = Some(usage);
            state.fetched_at = Some(now);
            state.status = Status::Ok;
            state.rate_limited = 0;
            state.retry_at = 0;
            Some(change)
        }
        Err(error) => {
            match error {
                FetchError::NotInstalled => {
                    state.status = Status::NotInstalled;
                    state.usage = None;
                }
                FetchError::SignIn => {
                    state.status = Status::SignIn;
                    state.usage = None;
                }
                FetchError::NoSubscription => {
                    state.status = Status::NoSubscription;
                    state.usage = None;
                }
                FetchError::RateLimited { retry_after } => {
                    state.rate_limited += 1;
                    let wait = net::backoff_secs(state.rate_limited, retry_after);
                    state.retry_at = now + wait as i64;
                    state.status = Status::Error("HTTP 429 (rate limited)".into());
                }
                FetchError::Failed(error) => state.status = Status::Error(error),
            }
            None
        }
    }
}

// ---------------------------------------------------------------- history

fn record_history(core: &Core, id: ProviderId, usage: &Usage, now: i64) {
    let config = core.config();
    let guard = core.history.lock().unwrap();
    let Some(history) = guard.as_ref() else { return };
    if config.history.retention_days > 0 {
        let _ = history.record(id, usage, now);
    }
    let main = main_limit(usage, &config.providers.get(id).main_limit).map(|l| l.id.clone());
    let spark = match main {
        Some(window) if config.history.retention_days > 0 => history::sparkline(&history.series(id.key(), &window, now - 25 * HOUR), now),
        _ => Vec::new(),
    };
    drop(guard);
    if let Some(state) = core.providers.lock().unwrap().get_mut(&id) {
        state.spark = spark;
    }
}

/// Feeds stored readings back into the forecast, so pace survives a restart.
fn replay_history(core: &Core) {
    apply_retention(core);
    let now = clock::now();
    let guard = core.history.lock().unwrap();
    let Some(history) = guard.as_ref() else { return };
    let mut all = core.providers.lock().unwrap();
    for id in ProviderId::ALL {
        for window in history.windows(id.key(), now - REPLAY_SECS) {
            let state = all.entry(id).or_default();
            let series = state.series.entry(window.clone()).or_default();
            for (t, pct, resets_at) in history.series(id.key(), &window, now - REPLAY_SECS) {
                series.push(t, pct, resets_at);
            }
        }
    }
}

fn apply_retention(core: &Core) {
    let days = core.config.read().unwrap().history.retention_days;
    if let Some(history) = core.history.lock().unwrap().as_ref() {
        let _ = history.prune(days, clock::now());
    }
}

// ---------------------------------------------------------------- alerts

fn check_alert(app: &AppHandle, core: &Core, id: ProviderId, prev_main: Option<f64>, new_period: bool, now: i64) {
    let config = core.config();
    let observation = {
        let all = core.providers.lock().unwrap();
        let Some(state) = all.get(&id).filter(|s| s.has_fresh_reading()) else { return };
        let Some(main) = state.usage.as_ref().and_then(|u| main_limit(u, &config.providers.get(id).main_limit)) else { return };
        let pace_window = config.forecast.pace_window_min as i64 * 60;
        Observation { id, pct: main.used_pct, prev_pct: prev_main, new_period, resets_at: main.resets_at, outlook: outlook_for(state, main, now, pace_window) }
    };
    let (minutes, _) = clock::local_minutes_and_weekday(now, win::utc_offset());
    let kind = alerts::decide(&observation, &config, &mut core.alerts.lock().unwrap(), now, minutes);
    if let Some(kind) = kind {
        let snapshot = core.snapshot(now);
        if let Some(view) = snapshot.providers.iter().find(|v| v.id == id.key()) {
            notify::send(app, kind, view);
        }
    }
}

/// The next moment the worker must wake for something other than polling.
fn next_deadline(core: &Core) -> Option<i64> {
    let config = core.config();
    let now = clock::now();
    let mut soonest: Option<i64> = None;
    if config.alerts.reset_soon {
        let all = core.providers.lock().unwrap();
        let resets = ProviderId::ALL.into_iter().filter_map(|id| {
            let usage = all.get(&id)?.usage.as_ref()?;
            main_limit(usage, &config.providers.get(id).main_limit)?.resets_at
        });
        soonest = alerts::next_reset_soon(resets, now);
    }
    if config.history.weekly_summary && config.history.retention_days > 0 {
        let at = next_monday_9(now, win::utc_offset());
        soonest = Some(soonest.map_or(at, |s| s.min(at)));
    }
    soonest
}

/// Checks that depend on the clock rather than on a new reading.
fn timed_checks(app: &AppHandle, core: &Core) {
    let now = clock::now();
    let config = core.config();
    if config.alerts.reset_soon {
        for id in ProviderId::ALL {
            let current = core.providers.lock().unwrap().get(&id).and_then(|s| {
                let usage = s.usage.as_ref()?;
                main_limit(usage, &config.providers.get(id).main_limit).map(|l| l.used_pct)
            });
            if current.is_some() {
                // Same reading as before: no crossing, only time-based kinds can fire.
                check_alert(app, core, id, current, false, now);
            }
        }
    }
    weekly_summary(app, core, now);
}

// ---------------------------------------------------------------- weekly summary

/// The next Monday 09:00 local time after `now` (Unix seconds).
pub fn next_monday_9(now: i64, utc_offset: i64) -> i64 {
    let (minutes, weekday) = clock::local_minutes_and_weekday(now, utc_offset);
    let today_start = now - minutes as i64 * 60 - (now + utc_offset).rem_euclid(60);
    let monday_9 = today_start - weekday as i64 * DAY + 9 * HOUR;
    if monday_9 > now { monday_9 } else { monday_9 + 7 * DAY }
}

/// On Monday from 09:00, once per week: a notification with last week's average and peak per tool.
fn weekly_summary(app: &AppHandle, core: &Core, now: i64) {
    let config = core.config();
    if !config.history.weekly_summary || config.history.retention_days == 0 {
        return;
    }
    let offset = win::utc_offset();
    let (minutes, weekday) = clock::local_minutes_and_weekday(now, offset);
    if weekday != 0 || minutes < 9 * 60 {
        return;
    }
    let this_monday = next_monday_9(now, offset) - 7 * DAY - 9 * HOUR;
    let week_key = this_monday.to_string();
    let lines = {
        let guard = core.history.lock().unwrap();
        let Some(history) = guard.as_ref() else { return };
        if history.meta("summary_week").as_deref() == Some(week_key.as_str()) {
            return;
        }
        history.set_meta("summary_week", &week_key);
        let ids: Vec<(ProviderId, String)> =
            config.providers.order.iter().map(|id| (*id, config.providers.get(*id).main_limit.clone())).collect();
        history::weekly_summary(history, &ids, this_monday - 7 * DAY)
    };
    if lines.is_empty() {
        return;
    }
    let i18n = core.i18n.read().unwrap();
    let body = lines
        .iter()
        .map(|(name, avg, peak)| i18n.f("summary.line", &[("name", name), ("avg", &format!("{avg:.0}")), ("peak", &format!("{peak:.0}"))]))
        .collect::<Vec<_>>()
        .join("\n");
    let title = i18n.t("summary.title");
    drop(i18n);
    notify::toast(app, &title, &body);
}

/// Opens the history database at startup. Failing to open it only disables history.
pub fn open_history(dir: &std::path::Path) -> Option<History> {
    History::open(dir).ok()
}

/// Sends the current snapshot to every open window and updates the tray.
pub fn publish(app: &AppHandle) {
    let core = app.state::<Arc<Core>>();
    let snapshot = core.snapshot(clock::now());
    let _ = app.emit("usage", &snapshot);
    crate::tray::update(app, &snapshot);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_monday_nine() {
        // 2026-11-16 is a Monday. 15:04:05 UTC.
        let monday_afternoon = 1_794_841_445;
        let next = next_monday_9(monday_afternoon, 0);
        assert_eq!(clock::datetime(next, 0), "2026-11-23 09:00:00");
        // Monday 08:00 UTC: still this Monday.
        let monday_morning = clock::parse_rfc3339("2026-11-16T08:00:00Z").unwrap();
        assert_eq!(clock::datetime(next_monday_9(monday_morning, 0), 0), "2026-11-16 09:00:00");
        // In UTC+1, Sunday 23:30 UTC is already Monday 00:30 local.
        let sunday_late = clock::parse_rfc3339("2026-11-15T23:30:00Z").unwrap();
        assert_eq!(clock::datetime(next_monday_9(sunday_late, 3600), 3600), "2026-11-16 09:00:00");
    }
}
