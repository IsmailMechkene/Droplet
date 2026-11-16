// The live preview at the top of Appearance and Notch: a small static notch
// drawn with the same shape maths and the same ring component as the real one.
// It uses sample numbers so every colour level is visible.

import { createRing, SIZES } from '../shared/ring.js';

const SAMPLE = {
  claude: { pct: 64, weekly: 41, pace: 0.52, state: 'ok' },
  codex: { pct: 12, weekly: 30, pace: 0.35, state: 'ok' },
  cursor: { pct: 72, weekly: 58, pace: 0.62, state: 'ok' },
  opencode: { pct: 38, weekly: 22, pace: 0.4, state: 'stale' },
  kimi: { pct: 93, weekly: 60, pace: 0.7, state: 'ok' },
};
const K = 0.5523, GEAR = 30;

/** Draws the preview into `host`. `collapsed` shows the slim tab. */
export function renderPreview(host, { edge, look, collapsed, highContrast }) {
  const vert = edge === 'left' || edge === 'right';
  const ids = vert ? ['claude', 'codex', 'cursor'] : ['claude', 'codex', 'cursor', 'opencode', 'kimi'];
  const z = SIZES[look.size] || SIZES.m, n = ids.length, cellW = z.d + 12;
  let depth = vert ? z.W : z.cellH + 14;
  let length = vert ? 2 * z.padY + n * z.cellH + (n - 1) * z.gap : 24 + n * cellW + (n - 1) * 4;
  if (collapsed) { depth = 6; length = 56; }
  const tail = collapsed ? 0 : 8 + GEAR + 6 + 16;
  const lead = 18, along = lead + length + tail + 18, across = depth + 10;
  const W = vert ? across : along, H = vert ? along : across;
  const toXY = (d, s) => (edge === 'right' ? [W - d, s] : edge === 'left' ? [d, s] : edge === 'top' ? [s, d] : [s, H - d]);
  const P = (d, s) => { const [x, y] = toXY(d, s); return x.toFixed(2) + ' ' + y.toFixed(2); };
  const rect = (d0, d1, s0, s1) => { const a = toXY(d0, s0), b = toXY(d1, s1); return { x: Math.min(a[0], b[0]), y: Math.min(a[1], b[1]), w: Math.abs(a[0] - b[0]), h: Math.abs(a[1] - b[1]) }; };

  const s0 = lead, s1 = lead + length, r = Math.min(collapsed ? 3 : 16, depth / 2, length / 2), f = collapsed ? 5 : 12;
  const path = `M${P(0, s0 - f)}C${P(0, s0 - f + K * f)} ${P(f - K * f, s0)} ${P(f, s0)}` +
    `L${P(depth - r, s0)}C${P(depth - r + K * r, s0)} ${P(depth, s0 + r - K * r)} ${P(depth, s0 + r)}` +
    `L${P(depth, s1 - r)}C${P(depth, s1 - r + K * r)} ${P(depth - r + K * r, s1)} ${P(depth - r, s1)}` +
    `L${P(f, s1)}C${P(f - K * f, s1)} ${P(0, s1 + f - K * f)} ${P(0, s1 + f)}`;

  const root = document.createElement('div');
  root.className = 'pv' + (collapsed && highContrast ? ' contrast' : '');
  root.style.width = W + 'px';
  root.style.height = H + 'px';
  root.innerHTML = `<svg width="${W}" height="${H}"><path class="fill" d="${path}Z"></path><path class="line" d="${path}"></path></svg>`;

  if (!collapsed) {
    const body = rect(0, depth, s0, s1);
    ids.forEach((id, i) => {
      const cell = document.createElement('div');
      cell.className = 'pv-cell';
      const box = vert
        ? { x: body.x, y: body.y + z.padY + i * (z.cellH + z.gap), w: body.w, h: z.cellH }
        : { x: body.x + 12 + i * (cellW + 4), y: body.y + (body.h - z.cellH) / 2, w: cellW, h: z.cellH };
      Object.assign(cell.style, { left: box.x + 'px', top: box.y + 'px', width: box.w + 'px', height: box.h + 'px' });
      const ring = createRing();
      ring.update({ id, look, ...SAMPLE[id] });
      cell.append(ring.el);
      root.append(cell);
    });
    const gear = toXY(depth / 2, s1 + 8 + GEAR / 2), grip = toXY(depth / 2, s1 + 8 + GEAR + 6 + 8);
    const gearEl = document.createElement('div');
    gearEl.className = 'pv-gear';
    gearEl.innerHTML = '<img src="../icons/settings.svg" alt="">';
    Object.assign(gearEl.style, { left: gear[0] - GEAR / 2 + 'px', top: gear[1] - GEAR / 2 + 'px' });
    const gw = vert ? 9 : 15, gh = vert ? 15 : 9;
    const gripEl = document.createElement('div');
    gripEl.className = 'pv-grip';
    gripEl.innerHTML = '<span></span>'.repeat(6);
    Object.assign(gripEl.style, { left: grip[0] - gw / 2 + 'px', top: grip[1] - gh / 2 + 'px', width: gw + 'px', height: gh + 'px' });
    root.append(gearEl, gripEl);
  }
  host.replaceChildren(root);
}

/** Where the preview sits in its box, and how much it is scaled down. */
export function previewPlacement(edge) {
  const vert = edge === 'left' || edge === 'right', k = vert ? 0.56 : 0.86;
  return {
    right: { right: '0', top: '50%', transform: `translateY(-50%) scale(${k})`, transformOrigin: 'right center' },
    left: { left: '0', top: '50%', transform: `translateY(-50%) scale(${k})`, transformOrigin: 'left center' },
    top: { top: '0', left: '50%', transform: `translateX(-50%) scale(${k})`, transformOrigin: 'center top' },
    bottom: { bottom: '0', left: '50%', transform: `translateX(-50%) scale(${k})`, transformOrigin: 'center bottom' },
  }[edge];
}

/** Plays the spring as a scale bounce on `node` (the "Preview" button). */
export function bounce(node, { r, d }) {
  const w = (2 * Math.PI) / r, z = Math.min(d, 0.999), wd = w * Math.sqrt(1 - z * z);
  const T = Math.min(2.5, Math.log(800) / (z * w)), N = 48, frames = [];
  for (let i = 0; i <= N; i++) {
    const t = (T * i) / N;
    const x = 1 - Math.exp(-z * w * t) * (Math.cos(wd * t) + ((z * w) / wd) * Math.sin(wd * t));
    frames.push({ transform: `scale(${(0.82 + 0.18 * x).toFixed(4)})` });
  }
  node.animate(frames, { duration: T * 1000, easing: 'linear' });
}
