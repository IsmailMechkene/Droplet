// A tiny spring solver. The same maths runs in Rust (src-tauri/src/spring.rs)
// to move the native window.
//
// A spring is described by `r` (response: seconds per undamped oscillation)
// and `d` (damping ratio: 1 = no overshoot). With mass = 1:
// stiffness = (2π / r)², damping = 4π·d / r.

export const TUNE = {
  snappy: { r: 0.28, d: 0.92 }, // hover, small changes
  bouncy: { r: 0.36, d: 0.84 }, // open / close, snap
  wobbly: { r: 0.42, d: 0.78 }, // landing after a drag
  drag: { r: 0.16, d: 0.86 },   // following the pointer
  reduced: { r: 0.22, d: 1 },   // reduced motion: quick, no overshoot
};

// The "Spring feel" setting scales response and damping.
const FEEL = {
  calm: { r: 1.1, d: 1.3 },
  default: { r: 1, d: 1 },
  playful: { r: 0.95, d: 0.72 },
};

/** Spring parameters for a preset under a feel / reduced-motion setting. */
export function config(name, feel = 'default', reduced = false) {
  if (reduced) return TUNE.reduced;
  if (name === 'drag') return TUNE.drag;
  const base = TUNE[name] || TUNE.bouncy;
  const f = FEEL[feel] || FEEL.default;
  return { r: base.r * f.r, d: Math.min(1, base.d * f.d) };
}

/** A spring value at rest at `x`. `eps` is the distance that counts as "arrived". */
export function make(x, eps = 0.05) {
  return { x, v: 0, t: x, c: TUNE.bouncy, eps };
}

/** Advances a spring by `dt` seconds (semi-implicit Euler in sub-steps of at most 4 ms). */
export function step(s, dt) {
  const w = (2 * Math.PI) / s.c.r, k = w * w, b = 2 * s.c.d * w;
  const n = Math.max(1, Math.ceil(dt / 0.004)), h = dt / n;
  for (let i = 0; i < n; i++) {
    const a = -k * (s.x - s.t) - b * s.v;
    s.v += a * h;
    s.x += s.v * h;
  }
}

/** True when the spring has arrived; it is then snapped exactly onto its target. */
export function settle(s) {
  if (Math.abs(s.x - s.t) < s.eps && Math.abs(s.v) < s.eps * 20) {
    s.x = s.t;
    s.v = 0;
    return true;
  }
  return false;
}

/** Jumps a spring to a value with no motion. */
export function set(s, x) {
  s.x = s.t = x;
  s.v = 0;
}

/** The spring's step response as a CSS `<duration> linear(...)` easing. */
export function cssEasing(c) {
  const w = (2 * Math.PI) / c.r, z = c.d;
  const T = Math.min(3, z < 1 ? Math.log(1000) / (z * w) : 7 / w);
  const N = 36, pts = [];
  for (let i = 0; i <= N; i++) {
    const t = (T * i) / N;
    let x;
    if (z < 0.999) {
      const wd = w * Math.sqrt(1 - z * z);
      x = 1 - Math.exp(-z * w * t) * (Math.cos(wd * t) + ((z * w) / wd) * Math.sin(wd * t));
    } else {
      x = 1 - Math.exp(-w * t) * (1 + w * t);
    }
    pts.push(+x.toFixed(3));
  }
  pts[N] = 1;
  return `${Math.round(T * 1000)}ms linear(${pts.join(', ')})`;
}
