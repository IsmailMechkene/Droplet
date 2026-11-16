//! The same spring as `ui/shared/spring.js`, used to move the native window.
//! `r` = response (seconds per undamped oscillation), `d` = damping ratio.

use crate::config::SpringFeel;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tune {
    pub r: f64,
    pub d: f64,
}

pub const SNAPPY: Tune = Tune { r: 0.28, d: 0.92 };
pub const BOUNCY: Tune = Tune { r: 0.36, d: 0.84 };
pub const DRAG: Tune = Tune { r: 0.16, d: 0.86 };
pub const REDUCED: Tune = Tune { r: 0.22, d: 1.0 };

/// Applies the "Spring feel" setting: it scales response and damping.
pub fn with_feel(base: Tune, feel: SpringFeel, reduced: bool) -> Tune {
    if reduced {
        return REDUCED;
    }
    let (r, d) = match feel {
        SpringFeel::Calm => (1.1, 1.3),
        SpringFeel::Default => (1.0, 1.0),
        SpringFeel::Playful => (0.95, 0.72),
    };
    Tune { r: base.r * r, d: (base.d * d).min(1.0) }
}

#[derive(Debug, Clone, Copy)]
pub struct Spring {
    pub x: f64,
    pub v: f64,
    pub target: f64,
    pub tune: Tune,
}

impl Spring {
    pub fn at(x: f64, tune: Tune) -> Spring {
        Spring { x, v: 0.0, target: x, tune }
    }

    /// Advances by `dt` seconds (semi-implicit Euler in sub-steps of at most 4 ms).
    pub fn step(&mut self, dt: f64) {
        let w = 2.0 * std::f64::consts::PI / self.tune.r;
        let (k, b) = (w * w, 2.0 * self.tune.d * w);
        let n = (dt / 0.004).ceil().max(1.0);
        let h = dt / n;
        for _ in 0..n as usize {
            let a = -k * (self.x - self.target) - b * self.v;
            self.v += a * h;
            self.x += self.v * h;
        }
    }

    /// True once the spring is within `eps` of its target and nearly still.
    pub fn settled(&self, eps: f64) -> bool {
        (self.x - self.target).abs() < eps && self.v.abs() < eps * 20.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converges_to_target() {
        let mut s = Spring::at(0.0, BOUNCY);
        s.target = 100.0;
        for _ in 0..240 {
            s.step(1.0 / 120.0);
        }
        assert!(s.settled(0.5), "x = {}", s.x);
    }

    #[test]
    fn underdamped_overshoots_and_critical_does_not() {
        let peak = |tune| {
            let mut s = Spring::at(0.0, tune);
            s.target = 100.0;
            (0..600).map(|_| { s.step(1.0 / 240.0); s.x }).fold(0.0, f64::max)
        };
        assert!(peak(Tune { r: 0.42, d: 0.6 }) > 101.0);
        assert!(peak(REDUCED) <= 100.01);
    }

    #[test]
    fn feel_scales_damping() {
        assert_eq!(with_feel(BOUNCY, SpringFeel::Default, false), BOUNCY);
        assert!(with_feel(BOUNCY, SpringFeel::Calm, false).d > BOUNCY.d);
        assert!(with_feel(BOUNCY, SpringFeel::Playful, false).d < BOUNCY.d);
        assert!(with_feel(SNAPPY, SpringFeel::Calm, false).d <= 1.0);
        assert_eq!(with_feel(BOUNCY, SpringFeel::Playful, true), REDUCED);
    }
}
