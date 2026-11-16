//! "Use this one now": which tool has the most room left.
//!
//! Score = headroom (100 − main %). A tool that will run out before its reset
//! is heavily penalised, and a full one scores zero. The pick only changes
//! when another tool beats the current pick by the hysteresis margin, so the
//! chip does not flip-flop between tools that are close.

use crate::providers::ProviderId;

/// A provider that may take part: enabled, fresh data, and marked "include in suggestions".
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Candidate {
    pub id: ProviderId,
    pub used_pct: f64,
    pub runs_out_before_reset: bool,
}

const RUNS_OUT_PENALTY: f64 = 0.25;

pub fn score(c: &Candidate) -> f64 {
    let headroom = (100.0 - c.used_pct).max(0.0);
    if c.used_pct >= 100.0 {
        0.0
    } else if c.runs_out_before_reset {
        headroom * RUNS_OUT_PENALTY
    } else {
        headroom
    }
}

/// The tool to recommend. `current` is the previous pick: it is kept unless
/// the best candidate beats it by at least `hysteresis` points.
pub fn pick(candidates: &[Candidate], current: Option<ProviderId>, hysteresis: f64) -> Option<ProviderId> {
    let best = candidates.iter().max_by(|a, b| score(a).total_cmp(&score(b)))?;
    if score(best) <= 0.0 {
        return None;
    }
    if let Some(cur) = current.and_then(|id| candidates.iter().find(|c| c.id == id)) {
        if score(cur) > 0.0 && score(best) - score(cur) < hysteresis {
            return Some(cur.id);
        }
    }
    Some(best.id)
}

/// A switch worth showing: `from` is in trouble and `to` is clearly better.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Switch {
    pub from: ProviderId,
    pub to: ProviderId,
}

/// `from` is the tool closest to its limit. A switch is suggested when it has
/// reached the warning level (or will run out before its reset) and the pick
/// beats it by at least `hysteresis` points.
pub fn switch(candidates: &[Candidate], pick: Option<ProviderId>, warn_at: f64, hysteresis: f64) -> Option<Switch> {
    let to = candidates.iter().find(|c| Some(c.id) == pick)?;
    let from = candidates.iter().filter(|c| c.id != to.id).min_by(|a, b| score(a).total_cmp(&score(b)))?;
    let in_trouble = from.used_pct >= warn_at || from.runs_out_before_reset;
    (in_trouble && score(to) - score(from) >= hysteresis).then_some(Switch { from: from.id, to: to.id })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ProviderId::*;

    fn c(id: ProviderId, used: f64) -> Candidate {
        Candidate { id, used_pct: used, runs_out_before_reset: false }
    }

    #[test]
    fn scoring() {
        assert_eq!(score(&c(Claude, 12.0)), 88.0);
        assert_eq!(score(&c(Claude, 100.0)), 0.0);
        assert_eq!(score(&Candidate { id: Claude, used_pct: 20.0, runs_out_before_reset: true }), 20.0);
    }

    #[test]
    fn picks_most_headroom() {
        assert_eq!(pick(&[c(Claude, 92.0), c(Codex, 12.0), c(Cursor, 50.0)], None, 15.0), Some(Codex));
        assert_eq!(pick(&[], None, 15.0), None);
        assert_eq!(pick(&[c(Claude, 100.0)], None, 15.0), None, "nothing to recommend when everything is full");
    }

    #[test]
    fn running_out_is_penalised() {
        let hot = Candidate { id: Codex, used_pct: 30.0, runs_out_before_reset: true }; // 70 × 0.25 = 17.5
        assert_eq!(pick(&[hot, c(Cursor, 60.0)], None, 15.0), Some(Cursor));
    }

    #[test]
    fn hysteresis_keeps_the_current_pick() {
        let list = [c(Codex, 30.0), c(Cursor, 20.0)]; // scores 70 and 80
        assert_eq!(pick(&list, Some(Codex), 15.0), Some(Codex), "10 points better is not enough to switch");
        assert_eq!(pick(&list, Some(Codex), 10.0), Some(Cursor), "exactly the margin switches");
        assert_eq!(pick(&list, None, 15.0), Some(Cursor), "no current pick: take the best");
        let list = [c(Codex, 45.0), c(Cursor, 20.0)]; // 55 and 80
        assert_eq!(pick(&list, Some(Codex), 15.0), Some(Cursor));
    }

    #[test]
    fn current_pick_is_dropped_when_gone_or_full() {
        let list = [c(Cursor, 20.0)];
        assert_eq!(pick(&list, Some(Codex), 15.0), Some(Cursor), "current is no longer a candidate");
        let list = [c(Codex, 100.0), c(Cursor, 95.0)];
        assert_eq!(pick(&list, Some(Codex), 15.0), Some(Cursor), "a full tool is never kept");
    }

    #[test]
    fn switch_needs_trouble_and_margin() {
        let list = [c(Claude, 92.0), c(Codex, 12.0)];
        assert_eq!(switch(&list, Some(Codex), 70.0, 15.0), Some(Switch { from: Claude, to: Codex }));
        let calm = [c(Claude, 40.0), c(Codex, 12.0)];
        assert_eq!(switch(&calm, Some(Codex), 70.0, 15.0), None, "nobody is near a limit");
        let close = [c(Claude, 80.0), c(Codex, 70.0)];
        assert_eq!(switch(&close, Some(Codex), 70.0, 15.0), None, "the other tool is not clearly better");
        let burning = [Candidate { id: Claude, used_pct: 50.0, runs_out_before_reset: true }, c(Codex, 12.0)];
        assert_eq!(switch(&burning, Some(Codex), 70.0, 15.0), Some(Switch { from: Claude, to: Codex }));
        assert_eq!(switch(&[c(Codex, 12.0)], Some(Codex), 70.0, 15.0), None, "one tool: nothing to switch from");
    }
}
