//! Pace and forecast maths. Pure functions over readings, so everything here is unit-tested.
//!
//! - A *period* is one stretch between two resets of a limit. A new period
//!   starts when `resets_at` changes or the percentage drops.
//! - *Even pace* is how far through the window we are: 1 − (resets_at − now) / window.
//! - *Burn rate* is the slope of % over the configured pace window. It needs at
//!   least two readings 10 minutes apart; one point is never extrapolated.

/// One reading of a limit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    pub t: i64,
    pub pct: f64,
}

/// Readings of one limit within its current period.
#[derive(Debug, Clone, Default)]
pub struct Series {
    pub samples: Vec<Sample>,
    pub resets_at: Option<i64>,
    /// Counts periods seen since the app started. The UI plays the reset sweep when it changes.
    pub period: u32,
}

/// Reset times from rolling "resets in N seconds" answers jitter by a few seconds.
const RESET_JITTER_SECS: i64 = 120;
/// Ignore float noise when deciding that usage dropped.
const DROP_EPSILON: f64 = 0.5;
const MIN_SPAN_SECS: i64 = 10 * 60;
const MAX_SAMPLES: usize = 2000;

/// True when a new reading belongs to a new period.
pub fn is_new_period(prev_pct: f64, prev_resets_at: Option<i64>, pct: f64, resets_at: Option<i64>) -> bool {
    if pct < prev_pct - DROP_EPSILON {
        return true;
    }
    match (prev_resets_at, resets_at) {
        (Some(a), Some(b)) => (a - b).abs() > RESET_JITTER_SECS,
        _ => false,
    }
}

impl Series {
    /// Adds a reading. Returns true when it started a new period (earlier readings are dropped).
    pub fn push(&mut self, t: i64, pct: f64, resets_at: Option<i64>) -> bool {
        let new_period = match self.samples.last() {
            Some(last) => is_new_period(last.pct, self.resets_at, pct, resets_at),
            None => false,
        };
        if new_period {
            self.samples.clear();
            self.period += 1;
        }
        if resets_at.is_some() || new_period {
            self.resets_at = resets_at;
        }
        if self.samples.last().map_or(true, |last| t > last.t) {
            self.samples.push(Sample { t, pct });
        }
        if self.samples.len() > MAX_SAMPLES {
            self.samples.remove(0);
        }
        new_period
    }
}

/// Share of the window that has elapsed, 0..=1. `None` when the reset time or window length is unknown.
pub fn even_pace(resets_at: Option<i64>, window_secs: Option<i64>, now: i64) -> Option<f64> {
    let (resets_at, window) = (resets_at?, window_secs?);
    if window <= 0 {
        return None;
    }
    Some((1.0 - (resets_at - now) as f64 / window as f64).clamp(0.0, 1.0))
}

/// Burn rate in percentage points per second: the least-squares slope over the
/// readings of the last `pace_window_secs`. `None` until there are two readings
/// at least 10 minutes apart.
pub fn burn_rate(samples: &[Sample], now: i64, pace_window_secs: i64) -> Option<f64> {
    let recent: Vec<&Sample> = samples.iter().filter(|s| s.t >= now - pace_window_secs).collect();
    let (first, last) = (recent.first()?, recent.last()?);
    if last.t - first.t < MIN_SPAN_SECS {
        return None;
    }
    let n = recent.len() as f64;
    let mean_t = recent.iter().map(|s| (s.t - first.t) as f64).sum::<f64>() / n;
    let mean_p = recent.iter().map(|s| s.pct).sum::<f64>() / n;
    let (mut num, mut den) = (0.0, 0.0);
    for s in &recent {
        let dt = (s.t - first.t) as f64 - mean_t;
        num += dt * (s.pct - mean_p);
        den += dt * dt;
    }
    (den > 0.0).then(|| num / den)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Outlook {
    /// Not enough readings yet.
    Learning,
    /// The limit is already used up.
    Exhausted,
    /// At the current burn rate the limit runs out at `at`. `before_reset_secs`
    /// is how long before the reset that is (`None` when the reset time is unknown).
    RunsOut { at: i64, before_reset_secs: Option<i64> },
    /// Will last until the reset, but usage is ahead of an even pace by this many points.
    AheadOfPace { by_pct: f64 },
    /// Will last until the reset, and usage is close to an even pace.
    OnPace,
    /// Will last until the reset with room to spare.
    WontRunOut,
}

impl Outlook {
    /// True when the limit is gone, or will be before it resets.
    pub fn runs_out_before_reset(self) -> bool {
        matches!(self, Outlook::Exhausted | Outlook::RunsOut { .. })
    }
}

/// Within this many points of even pace counts as "on pace".
const ON_PACE_BAND: f64 = 5.0;

pub fn outlook(samples: &[Sample], resets_at: Option<i64>, window_secs: Option<i64>, now: i64, pace_window_secs: i64) -> Outlook {
    let Some(used) = samples.last().map(|s| s.pct) else {
        return Outlook::Learning;
    };
    if used >= 100.0 {
        return Outlook::Exhausted;
    }
    let Some(rate) = burn_rate(samples, now, pace_window_secs) else {
        return Outlook::Learning;
    };
    if rate > 0.0 {
        let eta = now + ((100.0 - used) / rate).round() as i64;
        match resets_at {
            Some(reset) if eta < reset => return Outlook::RunsOut { at: eta, before_reset_secs: Some(reset - eta) },
            None => return Outlook::RunsOut { at: eta, before_reset_secs: None },
            _ => {}
        }
    }
    match even_pace(resets_at, window_secs, now) {
        Some(pace) => {
            let ahead = used - pace * 100.0;
            if ahead >= ON_PACE_BAND {
                Outlook::AheadOfPace { by_pct: ahead }
            } else if ahead > -ON_PACE_BAND {
                Outlook::OnPace
            } else {
                Outlook::WontRunOut
            }
        }
        None => Outlook::WontRunOut,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_794_800_000;
    const MIN: i64 = 60;
    const HOUR: i64 = 3600;

    fn series(points: &[(i64, f64)]) -> Vec<Sample> {
        points.iter().map(|(m, p)| Sample { t: T0 + m * MIN, pct: *p }).collect()
    }

    #[test]
    fn period_reset_detection() {
        let reset = Some(T0 + 5 * HOUR);
        assert!(!is_new_period(40.0, reset, 41.0, reset), "normal growth");
        assert!(!is_new_period(40.0, reset, 40.0, Some(T0 + 5 * HOUR + 30)), "jitter in the reset time");
        assert!(!is_new_period(40.0, reset, 39.8, reset), "float noise");
        assert!(is_new_period(40.0, reset, 2.0, reset), "percentage dropped");
        assert!(is_new_period(40.0, reset, 40.0, Some(T0 + 10 * HOUR)), "reset time moved");
        assert!(!is_new_period(40.0, None, 41.0, reset), "reset time appearing is not a reset");
    }

    #[test]
    fn series_restarts_on_new_period() {
        let mut s = Series::default();
        let r1 = Some(T0 + 5 * HOUR);
        assert!(!s.push(T0, 10.0, r1));
        assert!(!s.push(T0 + 300, 20.0, r1));
        assert_eq!((s.samples.len(), s.period), (2, 0));
        let r2 = Some(T0 + 10 * HOUR);
        assert!(s.push(T0 + 600, 1.0, r2));
        assert_eq!((s.samples.len(), s.period, s.resets_at), (1, 1, r2));
        assert!(!s.push(T0 + 600, 1.0, r2), "a duplicate timestamp is ignored");
        assert_eq!(s.samples.len(), 1);
    }

    #[test]
    fn even_pace_is_elapsed_share() {
        let window = Some(5 * HOUR);
        assert_eq!(even_pace(Some(T0 + 5 * HOUR), window, T0), Some(0.0));
        assert_eq!(even_pace(Some(T0 + 150 * MIN), window, T0), Some(0.5));
        assert_eq!(even_pace(Some(T0), window, T0), Some(1.0));
        assert_eq!(even_pace(Some(T0 - 60), window, T0), Some(1.0), "clamped after the reset");
        assert_eq!(even_pace(None, window, T0), None);
        assert_eq!(even_pace(Some(T0), None, T0), None);
    }

    #[test]
    fn burn_rate_needs_two_points_ten_minutes_apart() {
        let now = T0 + 60 * MIN;
        assert_eq!(burn_rate(&[], now, HOUR), None);
        assert_eq!(burn_rate(&series(&[(60, 50.0)]), now, HOUR), None, "never from one point");
        assert_eq!(burn_rate(&series(&[(52, 40.0), (60, 50.0)]), now, HOUR), None, "8 minutes is too short");
        let rate = burn_rate(&series(&[(50, 40.0), (60, 50.0)]), now, HOUR).unwrap();
        assert!((rate - 10.0 / 600.0).abs() < 1e-9);
    }

    #[test]
    fn burn_rate_only_uses_the_pace_window() {
        let now = T0 + 120 * MIN;
        // Steep early on, flat for the last hour.
        let s = series(&[(0, 0.0), (30, 40.0), (60, 50.0), (90, 50.0), (120, 50.0)]);
        assert_eq!(burn_rate(&s, now, HOUR), Some(0.0));
        assert!(burn_rate(&s, now, 2 * HOUR).unwrap() > 0.0);
    }

    #[test]
    fn learning_until_enough_data() {
        let now = T0 + 5 * MIN;
        let o = outlook(&series(&[(0, 10.0), (5, 12.0)]), Some(T0 + 5 * HOUR), Some(5 * HOUR), now, HOUR);
        assert_eq!(o, Outlook::Learning);
        assert_eq!(outlook(&[], None, None, now, HOUR), Outlook::Learning);
    }

    #[test]
    fn runs_out_before_reset() {
        // 60% → 80% in 20 minutes: 1 point per minute, 20 more minutes to 100%.
        let now = T0 + 20 * MIN;
        let reset = T0 + 80 * MIN;
        let o = outlook(&series(&[(0, 60.0), (10, 70.0), (20, 80.0)]), Some(reset), Some(5 * HOUR), now, HOUR);
        assert_eq!(o, Outlook::RunsOut { at: now + 20 * MIN, before_reset_secs: Some(40 * MIN) });
        assert!(o.runs_out_before_reset());
    }

    #[test]
    fn eta_after_reset_is_not_running_out() {
        // 1 point per minute but the reset comes in 5 minutes.
        let now = T0 + 20 * MIN;
        let o = outlook(&series(&[(0, 10.0), (20, 30.0)]), Some(now + 5 * MIN), Some(5 * HOUR), now, HOUR);
        assert!(!o.runs_out_before_reset());
    }

    #[test]
    fn pace_states_when_it_will_last() {
        let window = 5 * HOUR;
        let now = T0 + 150 * MIN; // half way
        let reset = Some(T0 + 300 * MIN);
        let slow = |end: f64| series(&[(120, end - 0.3), (150, end)]); // 0.01 pt/min: never runs out in time
        assert_eq!(outlook(&slow(50.0), reset, Some(window), now, HOUR), Outlook::OnPace);
        assert_eq!(outlook(&slow(53.0), reset, Some(window), now, HOUR), Outlook::OnPace);
        assert_eq!(outlook(&slow(20.0), reset, Some(window), now, HOUR), Outlook::WontRunOut);
        match outlook(&slow(62.0), reset, Some(window), now, HOUR) {
            Outlook::AheadOfPace { by_pct } => assert!((by_pct - 12.0).abs() < 1e-6),
            other => panic!("expected AheadOfPace, got {other:?}"),
        }
    }

    #[test]
    fn flat_or_falling_usage_wont_run_out() {
        let now = T0 + 30 * MIN;
        let o = outlook(&series(&[(0, 40.0), (30, 40.0)]), None, None, now, HOUR);
        assert_eq!(o, Outlook::WontRunOut);
    }

    #[test]
    fn unknown_reset_still_gives_an_eta() {
        let now = T0 + 20 * MIN;
        let o = outlook(&series(&[(0, 60.0), (20, 80.0)]), None, None, now, HOUR);
        assert_eq!(o, Outlook::RunsOut { at: now + 20 * MIN, before_reset_secs: None });
    }

    #[test]
    fn full_limit_is_exhausted() {
        let now = T0 + 20 * MIN;
        let o = outlook(&series(&[(20, 100.0)]), Some(now + HOUR), Some(5 * HOUR), now, HOUR);
        assert_eq!(o, Outlook::Exhausted);
    }
}
