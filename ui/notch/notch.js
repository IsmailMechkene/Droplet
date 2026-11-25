// The notch: rings docked to a screen edge, the morphing usage card, peeks,
// jiggle mode and drag.
//
// Coordinates. Everything is laid out in "edge space":
//   d = distance into the screen from the docked edge,
//   s = distance along the edge from the dock point (the centre of the body).
// `toXY(d, s)` maps that to window pixels for whichever edge we are on, so the
// same code draws all four edges.
//
// Window size. The native window is a stable "envelope" big enough for
// everything the notch can show in its current layout (body, pill, the card
// next to any ring). It only changes when the layout does (edge, size, number
// of rings), because moving or resizing a window makes WebView2 show the old
// frame at the new position for a moment, which reads as a twitch. What is
// visible and clickable is a window *region* inside the envelope: before an
// animation it grows (plus a margin for overshoot), afterwards it shrinks back.
// Changing the region never moves the content, and outside it clicks go to the
// windows below. Rust applies both.
//
// CPU. requestAnimationFrame only runs while a spring is moving (or in jiggle
// mode). At rest there are no timers and no frames.

import { invoke, listen } from '../shared/ipc.js';
import * as spring from '../shared/spring.js';
import { setAppearance, reducedMotion, spring as springFor } from '../shared/theme.js';
import { createRing, SIZES, usageColor } from '../shared/ring.js';
import { renderCard } from './card.js';

const SHADOW = 20;      // room around the notch for its drop shadow
const CARD_SHADOW = 34; // room around the card for its larger shadow
const OVERSHOOT = 24;   // extra room while animating
const COLLAPSED = { depth: 6, len: 56 };
const GEAR = 30, GRIP = 24;
const TAIL = 8 + GEAR + 4 + GRIP; // gear + grip after the body
const CARD_W = 260, CARD_GAP = 12;
const CARD_H_GUESS = 360; // card height assumed for the envelope before one has been measured
const CARD_SWITCH = 90;   // ms on another ring before an open card moves to it
const BODY_R = 16, FILLET = 8; // body corner radius; pill fillet size
const K = 0.5523;       // circle approximation with cubic Béziers

const $ = (id) => document.getElementById(id);
const el = {
  notch: $('notch'), g: $('shape-g'), body: $('body'), stroke: $('stroke'), layer: $('layer'), hit: $('hit'),
  pill: $('pill'), pilltext: $('pilltext'), f1: $('fillet1'), f2: $('fillet2'), cells: $('cells'),
  gear: $('gear'), grip: $('grip'), cardbg: $('cardbg'), arrow: $('arrow'), card: $('card'),
};

const st = {
  cfg: null, strings: {}, snap: { providers: [], suggestion: null },
  place: { edge: 'right', before: 400, after: 400, depth: 800 },
  order: [],                 // provider ids in display order (live while reordering)
  tabOpen: false, keepOpen: false, idle: false, jiggle: false, dragging: false, peeking: false,
  hover: false, focusIn: false, // pointer over the notch; keyboard focus inside it (they reveal the gear and grip)
  card: { id: null, open: false, vis: false, in: false },
  pill: null,                // { id, a, b, dot, kind: 'chip' | 'peek' }
  dismissedChip: null, ringDrag: null,
};
const geom = { W: 1, H: 1, anchor: 0 };
const timers = {};

const sp = {
  depth: spring.make(COLLAPSED.depth), len: spring.make(COLLAPSED.len),
  lift: spring.make(1, 0.0005), squash: spring.make(0, 0.0005), shake: spring.make(0), pill: spring.make(0, 0.002),
  tail: spring.make(0, 0.002), // gear and grip: 0 hidden, 1 shown
  cd: spring.make(0), cs: spring.make(0), cw: spring.make(0), ch: spring.make(0), cr: spring.make(0),
};
const cells = new Map(); // id -> { el, ring, x, y, s, phase, freq, placed }

// ------------------------------------------------------------ layout

const edge = () => st.place.edge;
const isVert = () => edge() === 'left' || edge() === 'right';
const look = () => st.cfg.appearance;
const view = (id) => st.snap.providers.find((p) => p.id === id);

/** Ids shown in the notch: enabled providers, or every installed one in jiggle mode. */
function visibleIds() {
  return st.order.filter((id) => { const v = view(id); return v && (st.jiggle || v.enabled); });
}

function layout() {
  const z = SIZES[look().size] || SIZES.m, ids = visibleIds(), n = Math.max(1, ids.length), cw = z.d + 12;
  if (isVert()) {
    return { z, ids, depth: z.W, len: 2 * z.padY + n * z.cellH + (n - 1) * z.gap, boxW: z.W, w: z.W,
      slot: (i) => [0, z.padY + i * (z.cellH + z.gap)] };
  }
  const len = 24 + n * cw + (n - 1) * 4;
  return { z, ids, depth: z.cellH + 14, len, boxW: cw, w: len, slot: (i) => [12 + i * (cw + 4), 7] };
}

/** Collapsed to the slim tab: "on hover" mode (or idle) and nothing is holding it open. */
function collapsedNow() {
  const hover = st.cfg.notch.show === 'hover' || st.idle;
  return hover && !st.tabOpen && !st.keepOpen && !st.dragging && !st.jiggle && !st.peeking && !st.card.open;
}

function toXY(d, s) {
  switch (edge()) {
    case 'right': return [geom.W - d, geom.anchor + s];
    case 'left': return [d, geom.anchor + s];
    case 'top': return [geom.anchor + s, d];
    default: return [geom.anchor + s, geom.H - d];
  }
}
function rectOf(d0, d1, s0, s1) {
  const a = toXY(d0, s0), b = toXY(d1, s1);
  return { x: Math.min(a[0], b[0]), y: Math.min(a[1], b[1]), w: Math.abs(a[0] - b[0]), h: Math.abs(a[1] - b[1]) };
}
const clamp = (v, lo, hi) => Math.max(lo, Math.min(hi, v));

/** Centre of a ring in edge space, for its slot in the settled layout. */
function ringCentre(L, index) {
  const [x, y] = L.slot(index);
  return isVert() ? { d: L.depth / 2, s: -L.len / 2 + y + L.z.d / 2 } : { d: y + L.z.d / 2, s: -L.len / 2 + x + L.boxW / 2 };
}

/** Where the open card goes, in edge space. Kept on screen using the room Rust reported. */
function cardBox() {
  const L = layout();
  return cardBoxAt(L, Math.max(0, L.ids.indexOf(st.card.id)), el.card.offsetHeight || 220);
}

/** The card next to ring `i`, for a card `h` pixels tall. */
function cardBoxAt(L, i, h) {
  const c = ringCentre(L, i);
  const along = isVert() ? h : CARD_W, across = isVert() ? CARD_W : h;
  const lo = -st.place.before + 8, hi = st.place.after - 8;
  const s0 = clamp(c.s - along / 2, lo, Math.max(lo, hi - along));
  const d0 = L.depth + CARD_GAP;
  return { d0, d1: d0 + across, s0, s1: s0 + along, ring: c, ringD: L.z.d };
}

/** Pill centre along the edge: level with its ring. A horizontal pill is long, so it
 *  moves along until its start clears the body's rounded corner with room for a fillet. */
function pillCentre(centre, half, s0) {
  return isVert() ? centre : Math.max(centre, s0 + BODY_R + FILLET + half);
}

function pillSize() {
  const text = el.pilltext.scrollWidth || 160;
  return isVert() ? { depth: text, half: 15 } : { depth: 30, half: text / 2 };
}

/** Bounding box of everything visible now or at the end of the current animation, in edge space. */
function visibleBox(animating) {
  const L = layout(), col = collapsedNow();
  const depth = Math.max(col ? COLLAPSED.depth : L.depth, sp.depth.x);
  const len = Math.max(col ? COLLAPSED.len : L.len, sp.len.x);
  // The gear and grip only take room while shown, so a hidden gear does not catch clicks.
  const tailRoom = (!col || sp.depth.x > 14) && (sp.tail.t > 0 || sp.tail.x > 0.01);
  let dMax = depth + SHADOW, sMin = -len / 2 - 12 - SHADOW, sMax = len / 2 + (tailRoom ? TAIL : 12) + SHADOW;
  if (st.pill && (sp.pill.t > 0 || sp.pill.x > 0.01)) {
    const p = pillSize(), centre = ringCentre(L, Math.max(0, L.ids.indexOf(st.pill.id))).s;
    const c = pillCentre(centre, p.half, -L.len / 2);
    dMax = Math.max(dMax, depth + p.depth + SHADOW);
    sMin = Math.min(sMin, c - p.half - 8 - SHADOW);
    sMax = Math.max(sMax, c + p.half + 8 + SHADOW);
  }
  if (st.card.vis) {
    const c = cardBox();
    dMax = Math.max(dMax, c.d1 + CARD_SHADOW);
    sMin = Math.min(sMin, c.s0 - CARD_SHADOW);
    sMax = Math.max(sMax, c.s1 + CARD_SHADOW);
  }
  const m = animating ? OVERSHOOT : 0;
  return { sMin: sMin - m, sMax: sMax + m, dMax: dMax + m };
}

// The envelope only grows while the layout stays the same, so the window keeps still.
let env = null, cardH = CARD_H_GUESS, pillW = 0;

/** Room for everything the notch can show in the current layout, in edge space. */
function envelope() {
  const L = layout(), key = [edge(), look().size, L.ids.length].join('|');
  let dMax = L.depth + SHADOW, sMin = -L.len / 2 - 12 - SHADOW, sMax = L.len / 2 + TAIL + SHADOW;
  if (st.pill) pillW = Math.max(pillW, el.pilltext.scrollWidth || 0);
  if (st.card.vis) cardH = Math.max(cardH, el.card.offsetHeight || 0);
  const p = isVert() ? { depth: Math.max(pillW, 160), half: 15 } : { depth: 30, half: Math.max(pillW, 160) / 2 };
  L.ids.forEach((_, i) => {
    const c = pillCentre(ringCentre(L, i).s, p.half, -L.len / 2);
    dMax = Math.max(dMax, L.depth + p.depth + SHADOW);
    sMin = Math.min(sMin, c - p.half - 8 - SHADOW);
    sMax = Math.max(sMax, c + p.half + 8 + SHADOW);
    const card = cardBoxAt(L, i, cardH);
    dMax = Math.max(dMax, card.d1 + CARD_SHADOW);
    sMin = Math.min(sMin, card.s0 - CARD_SHADOW);
    sMax = Math.max(sMax, card.s1 + CARD_SHADOW);
  });
  const now = visibleBox(true); // anything unusual on screen right now (e.g. a taller card)
  dMax = Math.max(dMax + OVERSHOOT, now.dMax);
  sMin = Math.min(sMin - OVERSHOOT, now.sMin);
  sMax = Math.max(sMax + OVERSHOOT, now.sMax);
  if (!env || env.key !== key) env = { key, sMin, sMax, dMax };
  else env = { key, sMin: Math.min(env.sMin, sMin), sMax: Math.max(env.sMax, sMax), dMax: Math.max(env.dMax, dMax) };
  return env;
}

/** The window shape for Rust: the envelope, and the visible part of it as the region. */
function extent(animating) {
  const L = layout(), e = envelope(), v = visibleBox(animating);
  // Along the edge the envelope is not clamped to the screen: that would change it
  // after every drag. Any overhang lies outside the region (not drawn, not
  // clickable) and is always under half the window, so Windows keeps the window
  // on this monitor's DPI. The visible region never leaves the screen.
  const anchor = Math.round(-e.sMin), along = Math.ceil(e.sMax - e.sMin), across = Math.ceil(Math.min(e.dMax, st.place.depth));
  const lo = Math.max(e.sMin, -st.place.before), hi = Math.min(e.sMax, st.place.after);
  const a0 = Math.floor(anchor + Math.max(v.sMin, lo)), a1 = Math.ceil(anchor + Math.min(v.sMax, hi));
  const dep = Math.ceil(Math.min(v.dMax, across));
  const far = edge() === 'right' || edge() === 'bottom';
  const c0 = far ? across - dep : 0;
  const region = isVert() ? { x: c0, y: a0, w: dep, h: a1 - a0 } : { x: a0, y: c0, w: a1 - a0, h: dep };
  return {
    edge: edge(), w: isVert() ? across : along, h: isVert() ? along : across, anchor,
    min_before: L.len / 2 + 14, min_after: L.len / 2 + TAIL + 14, region,
  };
}

// ------------------------------------------------------------ native window shape

let shapeSeq = 0, lastShape = '', pendingAnchor = null, pendingSize = null;

function applyPending() {
  if (pendingAnchor == null) return;
  geom.anchor = pendingAnchor;
  pendingAnchor = pendingSize = null;
  geom.W = window.innerWidth;
  geom.H = window.innerHeight;
  render();
}

/** Asks Rust for a window of this shape. Resolves once the new size is in effect. */
async function setShape(shape) {
  const key = JSON.stringify(shape);
  if (key === lastShape) return;
  lastShape = key;
  const seq = ++shapeSeq;
  pendingAnchor = shape.anchor;
  pendingSize = [shape.w, shape.h];
  let place = null;
  try { place = await invoke('notch_shape', { shape }); } catch (_) { /* window is closing */ }
  if (seq !== shapeSeq) return;
  if (place) adoptPlacement(place);
  // The resize event normally applies the geometry in the same frame as the new
  // size. If the size did not change (or the event was missed), apply it now.
  const sized = pendingSize && Math.abs(window.innerWidth - pendingSize[0]) < 2 && Math.abs(window.innerHeight - pendingSize[1]) < 2;
  if (sized) applyPending();
  else await new Promise((done) => { timers.shape = setTimeout(() => { applyPending(); done(); }, 80); onResized = () => { clearTimeout(timers.shape); done(); }; });
}
let onResized = null;
window.addEventListener('resize', () => {
  applyPending();
  geom.W = window.innerWidth;
  geom.H = window.innerHeight;
  if (onResized) { const done = onResized; onResized = null; done(); }
});

function adoptPlacement(place) {
  const hopped = place.edge !== st.place.edge;
  st.place = place;
  if (hopped) {
    // Edge hop: start as a zero-depth tab on the new edge and grow out of it.
    spring.set(sp.depth, 0);
    spring.set(sp.len, COLLAPSED.len);
    for (const c of cells.values()) c.placed = false;
    el.grip.classList.toggle('flat', !isVert());
    lastShape = '';
    update();
  }
}

// ------------------------------------------------------------ animation loop

let raf = 0, lastT = 0, clock = 0;

function allSprings() {
  const list = Object.values(sp);
  for (const c of cells.values()) list.push(c.x, c.y, c.s);
  return list;
}

function frame(t) {
  raf = 0;
  const dt = Math.min(1 / 30, Math.max(0, (t - lastT) / 1000));
  lastT = t;
  clock += dt;
  let moving = false;
  for (const s of allSprings()) {
    if (s.x === s.t && s.v === 0) continue;
    spring.step(s, dt);
    if (!spring.settle(s)) moving = true;
  }
  render();
  if (moving || (st.jiggle && !reducedMotion())) raf = requestAnimationFrame(frame);
  else setShape(extent(false)); // at rest: shrink the visible region back to the content
}

/** True when some spring is away from its target (or jiggle mode keeps frames running). */
function anyMoving() {
  if (st.jiggle && !reducedMotion()) return true;
  return allSprings().some((s) => s.x !== s.t || s.v !== 0);
}

function wake() {
  if (raf) return;
  lastT = performance.now();
  raf = requestAnimationFrame(frame);
}

/** Sets spring targets from the current state. */
function retarget() {
  const L = layout(), col = collapsedNow();
  const size = springFor(col ? 'snappy' : 'bouncy');
  sp.depth.c = sp.len.c = size;
  sp.depth.t = col ? COLLAPSED.depth : L.depth;
  sp.len.t = col ? COLLAPSED.len : L.len;
  sp.pill.c = springFor('bouncy');
  sp.pill.t = st.pill && !st.dragging && !col ? 1 : 0;
  sp.tail.c = springFor('snappy');
  sp.tail.t = !col && (st.hover || st.focusIn || st.card.open || st.dragging || st.jiggle) ? 1 : 0;
  const snappy = springFor('snappy');
  for (const [id, c] of cells) {
    const i = L.ids.indexOf(id);
    c.x.c = c.y.c = c.s.c = snappy;
    if (i < 0) { c.s.t = 0; continue; }
    const [x, y] = L.slot(i);
    if (!c.placed) { spring.set(c.x, x); spring.set(c.y, y); c.placed = true; }
    c.x.t = x; c.y.t = y; c.s.t = 1;
  }
}

/** State changed: grow the window first, then animate. */
async function update() {
  retarget();
  syncClasses();
  if (!anyMoving() && !raf) { await setShape(extent(false)); render(); return; } // nothing to animate
  await setShape(extent(true));
  retarget();
  wake();
}

function syncClasses() {
  const n = st.cfg.notch;
  el.notch.classList.toggle('contrast', n.high_contrast);
  el.notch.classList.toggle('locked', n.locked);
  el.notch.classList.toggle('jiggle', st.jiggle);
  el.notch.classList.toggle('dragging', st.dragging);
  el.grip.classList.toggle('flat', !isVert());
}

// ------------------------------------------------------------ drawing

const mul = (A, B) => [A[0] * B[0] + A[2] * B[1], A[1] * B[0] + A[3] * B[1], A[0] * B[2] + A[2] * B[3], A[1] * B[2] + A[3] * B[3], A[0] * B[4] + A[2] * B[5] + A[4], A[1] * B[4] + A[3] * B[5] + A[5]];
const move = (x, y) => [1, 0, 0, 1, x, y];
const scale = (x, y) => [x, 0, 0, y, 0, 0];
const about = (x, y, M) => mul(move(x, y), mul(M, move(-x, -y)));
const put = (node, r) => Object.assign(node.style, { left: r.x + 'px', top: r.y + 'px', width: r.w + 'px', height: r.h + 'px' });

function render() {
  if (!st.cfg) return;
  const L = layout(), vert = isVert(), reduced = reducedMotion();
  const depth = Math.max(0, sp.depth.x), len = Math.max(0, sp.len.x);
  const d1 = Math.max(4, depth), s0 = -len / 2, s1 = len / 2;
  const P = (d, s) => { const q = toXY(d, s); return q[0].toFixed(2) + ' ' + q[1].toFixed(2); };

  // Body: a rounded tab whose ends flare into the edge with concave fillets.
  const r = Math.min(16, d1 / 2, len / 2), f = Math.min(12, d1 * 0.9);
  const path = `M${P(0, s0 - f)}C${P(0, s0 - f + K * f)} ${P(f - K * f, s0)} ${P(f, s0)}` +
    `L${P(d1 - r, s0)}C${P(d1 - r + K * r, s0)} ${P(d1, s0 + r - K * r)} ${P(d1, s0 + r)}` +
    `L${P(d1, s1 - r)}C${P(d1, s1 - r + K * r)} ${P(d1 - r + K * r, s1)} ${P(d1 - r, s1)}` +
    `L${P(f, s1)}C${P(f - K * f, s1)} ${P(0, s1 + f - K * f)} ${P(0, s1 + f)}`;
  el.body.setAttribute('d', path + 'Z');
  el.stroke.setAttribute('d', path); // open along the screen edge

  // Squash against the edge (landing wobble), lift about the centre (drag), shake (locked).
  const R = rectOf(0, d1, s0, s1), cx = R.x + R.w / 2, cy = R.y + R.h / 2;
  const sq = reduced ? 0 : sp.squash.x, near = toXY(0, 0);
  const squash = vert ? scale(1 - sq * 1.2, 1 + sq) : scale(1 + sq, 1 - sq * 1.2);
  let M = mul(about(near[0], near[1], squash), about(cx, cy, scale(sp.lift.x, sp.lift.x)));
  if (Math.abs(sp.shake.x) > 0.01) M = mul(vert ? move(0, sp.shake.x) : move(sp.shake.x, 0), M);
  const matrix = `matrix(${M.map((v) => v.toFixed(4)).join(',')})`;
  el.g.setAttribute('transform', matrix);
  el.layer.style.transform = matrix;

  const collapsed = depth < 14;
  el.notch.classList.toggle('collapsed', collapsed);
  const progress = clamp((depth - COLLAPSED.depth) / Math.max(1, L.depth - COLLAPSED.depth), 0, 1);
  const content = st.dragging ? 1 : collapsed ? 0 : clamp((progress - 0.6) / 0.4, 0, 1);
  const pad = collapsed ? 12 : 0;
  put(el.hit, { x: R.x - pad, y: R.y - pad, w: R.w + pad * 2, h: R.h + pad * 2 });

  // Rings.
  const offX = (R.w - (vert ? L.depth : L.len)) / 2, offY = (R.h - (vert ? L.len : L.depth)) / 2;
  const drag = st.ringDrag;
  for (const [id, c] of cells) {
    const shown = L.ids.includes(id), held = drag && drag.id === id;
    if (held) { c.x.x = drag.x; c.y.x = drag.y; c.x.v = c.y.v = 0; }
    const wiggle = st.jiggle && !reduced ? 1.5 * Math.sin((clock * 2 * Math.PI / 0.26) * c.freq + c.phase) : 0;
    const size = Math.max(0, c.s.x) * (held ? 1.12 : 1);
    Object.assign(c.el.style, {
      width: L.boxW + 'px', height: L.z.cellH + 'px',
      transform: `translate(${(R.x + offX + c.x.x).toFixed(2)}px,${(R.y + offY + c.y.x).toFixed(2)}px) rotate(${wiggle.toFixed(2)}deg) scale(${size.toFixed(3)})`,
      opacity: (content * clamp(c.s.x, 0, 1)).toFixed(3),
      pointerEvents: content > 0.5 && shown ? 'auto' : 'none', zIndex: held ? 5 : 1,
    });
  }

  // Gear and grip follow the end of the body, and only show while the notch is in use.
  const gx = R.x + (vert ? R.w / 2 : R.w + 8 + GEAR / 2), gy = R.y + (vert ? R.h + 8 + GEAR / 2 : R.h / 2);
  const gripX = vert ? gx - GRIP / 2 : gx + GEAR / 2 + 4, gripY = vert ? gy + GEAR / 2 + 4 : gy - GRIP / 2;
  const tail = clamp(sp.tail.x, 0, 1), shown = content * tail, grow = reduced ? 1 : 0.6 + 0.4 * Math.max(0, sp.tail.x);
  for (const [node, x, y] of [[el.gear, gx - GEAR / 2, gy - GEAR / 2], [el.grip, gripX, gripY]]) {
    node.style.transform = `translate(${x.toFixed(2)}px,${y.toFixed(2)}px) scale(${grow.toFixed(3)})`;
    node.style.opacity = shown.toFixed(3);
    node.style.pointerEvents = shown > 0.5 ? 'auto' : 'none';
  }

  renderPill(L, d1, s0, s1, r, collapsed);
  renderCardFrame();
}

/** The peek / suggestion pill: grows out of the body next to a ring, with concave fillets. */
// Pill corners as [side, end] for [top-left, top-right, bottom-right, bottom-left]:
// side 'i' touches the body, 'o' faces away; end 's' is the start along the edge, 'e' the end.
const PILL_CORNERS = {
  right: [['o', 's'], ['i', 's'], ['i', 'e'], ['o', 'e']],
  left: [['i', 's'], ['o', 's'], ['o', 'e'], ['i', 'e']],
  top: [['i', 's'], ['i', 'e'], ['o', 'e'], ['o', 's']],
  bottom: [['o', 's'], ['o', 'e'], ['i', 'e'], ['i', 's']],
};

function renderPill(L, d1, s0, s1, r, collapsed) {
  const amount = Math.max(0, sp.pill.x), pl = st.pill;
  if (amount <= 0.01 || !pl || collapsed) {
    el.pill.style.display = el.f1.style.display = el.f2.style.display = 'none';
    el.pilltext.style.opacity = 0;
    return;
  }
  const size = pillSize(), i = Math.max(0, L.ids.indexOf(pl.id)), half = size.half;
  const c = pillCentre(ringCentre(L, i).s, half, s0);
  const grown = size.depth * amount;
  put(el.pill, rectOf(d1 - 2, d1 - 2 + grown, c - half, c + half));

  // Each end of the pill's inner side either joins the body's straight part (square
  // corner plus a concave fillet), sits a few pixels into its rounded corner (square:
  // the 2 px overlap covers the seam), or hangs past the body (rounded, no fillet).
  const grow = Math.min(1, amount * 1.6);
  const ends = { s: c - half - (s0 + r), e: s1 - r - (c + half) }; // room left on the straight part
  const rad = Math.min(15, grown / 2 + 2);
  const radius = ([side, end]) => (side === 'o' || ends[end] < -7 ? rad : 0) + 'px';
  el.pill.style.borderRadius = PILL_CORNERS[edge()].map(radius).join(' ');
  el.pill.style.display = 'flex';
  el.pilltext.style.opacity = amount > 0.7 ? 1 : 0;

  for (const [node, end, dir] of [[el.f1, 's', -1], [el.f2, 'e', 1]]) {
    const fs = Math.min(FILLET, ends[end]) * grow;
    if (fs < 0.5) { node.style.display = 'none'; continue; }
    const at = c + dir * half, far = at + dir * fs;
    const q = rectOf(d1 - 1, d1 + fs, Math.min(at, far), Math.max(at, far)), cp = toXY(d1 + fs, far);
    const gx = Math.abs(cp[0] - q.x) < 0.5 ? '0' : '100%', gy = Math.abs(cp[1] - q.y) < 0.5 ? '0' : '100%';
    put(node, q);
    node.style.background = `radial-gradient(circle at ${gx} ${gy}, transparent ${Math.max(0, fs - 0.6)}px, var(--dp-bg) ${fs}px)`;
    node.style.display = 'block';
  }
}

/** The card background morphs between a ring-sized circle and the card rectangle. */
function renderCardFrame() {
  if (!st.card.vis) return;
  const box = cardBox(), vert = isVert();
  put(el.card, rectOf(box.d0, box.d1, box.s0, box.s1));
  el.card.style.height = '';
  const bg = rectOf(sp.cd.x - sp.cw.x / 2, sp.cd.x + sp.cw.x / 2, sp.cs.x - sp.ch.x / 2, sp.cs.x + sp.ch.x / 2);
  put(el.cardbg, bg);
  el.cardbg.style.borderRadius = Math.max(0, sp.cr.x) + 'px';
  // Arrow on the side facing the notch, level with the ring.
  const as = clamp(box.ring.s, sp.cs.x - sp.ch.x / 2 + 22, sp.cs.x + sp.ch.x / 2 - 22);
  const [ax, ay] = toXY(sp.cd.x - sp.cw.x / 2, as);
  el.arrow.style.transform = `translate(${(ax - 6).toFixed(2)}px,${(ay - 6).toFixed(2)}px) rotate(45deg)`;

  const full = vert ? CARD_W : (box.d1 - box.d0), progress = (sp.cw.x - box.ringD) / Math.max(1, full - box.ringD);
  if (st.card.open && !st.card.in && progress > 0.7) { st.card.in = true; document.body.classList.add('card-in'); }
  if (!st.card.open && st.card.closing && progress < 0.06) {
    st.card.vis = st.card.closing = false;
    st.card.id = null;
    document.body.classList.remove('card-vis');
    retarget(); // the notch may collapse now that the card is gone
  }
}

// ------------------------------------------------------------ card

function fillCard() {
  const v = view(st.card.id);
  if (!v) return;
  const s = st.snap.suggestion;
  renderCard(el.card, v, {
    strings: st.strings,
    colorFor: (pct) => usageColor(pct, { ...look(), color_mode: 'hard' }),
    suggestion: s && (s.from === v.id || s.to === v.id) ? s : null,
  });
}

function cardTargets(open) {
  const box = cardBox(), c = springFor('bouncy');
  for (const k of ['cd', 'cs', 'cw', 'ch', 'cr']) sp[k].c = c;
  if (open) {
    sp.cd.t = (box.d0 + box.d1) / 2; sp.cs.t = (box.s0 + box.s1) / 2;
    sp.cw.t = box.d1 - box.d0; sp.ch.t = box.s1 - box.s0; sp.cr.t = 14;
  } else {
    sp.cd.t = box.ring.d; sp.cs.t = box.ring.s;
    sp.cw.t = sp.ch.t = box.ringD * 0.9; sp.cr.t = box.ringD * 0.45;
  }
}

async function openCard(id) {
  if (st.jiggle || st.dragging || !view(id)) return;
  clearTimeout(timers.close);
  clearTimeout(timers.closing);
  const fresh = !st.card.vis;
  st.card.id = id;
  st.card.open = true;
  st.card.closing = false;
  st.card.vis = true;
  document.body.classList.add('card-vis');
  el.card.classList.toggle('instant', !fresh);
  fillCard();
  if (fresh) {
    // Start as a circle on the ring, then spring out to the card.
    cardTargets(false);
    for (const k of ['cd', 'cs', 'cw', 'ch', 'cr']) spring.set(sp[k], sp[k].t);
  }
  retarget();
  await setShape(extent(true));
  cardTargets(true);
  wake();
  // Ages such as "2 min ago" are worded by Rust: fetch a current snapshot.
  invoke('card_opened', { id }).catch(() => {});
}

function closeCard() {
  if (!st.card.open) return;
  st.card.open = false;
  st.card.in = false;
  document.body.classList.remove('card-in');
  // Content fades first, then the background shrinks back into the ring.
  timers.closing = setTimeout(() => { st.card.closing = true; cardTargets(false); wake(); }, 100);
}

// ------------------------------------------------------------ data

function ringProps(v) {
  const hidden = !v.enabled;
  return {
    id: v.id, name: v.name, pct: v.pct, state: v.ring, weekly: v.weekly, pace: v.pace, look: look(),
    badge: st.jiggle ? (hidden ? 'plus' : 'minus') : null, dimmed: st.jiggle && hidden,
    badgeLabel: (st.strings[hidden ? 'notch.badge_show' : 'notch.badge_hide'] || '').replace('{name}', v.name),
  };
}

function makeCell(id) {
  const node = document.createElement('div');
  node.className = 'cell';
  node.tabIndex = 0;
  node.setAttribute('role', 'button');
  const press = document.createElement('div');
  press.className = 'press';
  const ring = createRing();
  press.append(ring.el);
  node.append(press);
  el.cells.append(node);
  const c = { el: node, ring, x: spring.make(0), y: spring.make(0), s: spring.make(0, 0.002), phase: Math.random() * 6.28, freq: 0.92 + Math.random() * 0.16, placed: false, seen: null };
  bindCell(id, c);
  cells.set(id, c);
  return c;
}

/** Applies a snapshot: updates rings, plays the reset sweep / critical shake, refreshes the card and chip. */
function applySnapshot(snap) {
  st.snap = snap;
  const ids = snap.providers.map((p) => p.id);
  if (!st.ringDrag) st.order = st.cfg.providers.order.filter((id) => ids.includes(id));
  for (const [id, c] of cells) if (!ids.includes(id)) { c.el.remove(); cells.delete(id); }
  const crit = look().crit_at, reduced = reducedMotion();
  for (const v of snap.providers) {
    const c = cells.get(v.id) || makeCell(v.id);
    c.ring.update(ringProps(v));
    c.el.setAttribute('aria-label', v.label);
    const before = c.seen;
    if (before && v.ring === 'ok') {
      if (v.period !== before.period) c.ring.pulse(reduced);
      else if (before.pct < crit && v.pct >= crit) c.ring.shake(reduced);
    }
    if (v.ring === 'ok') c.seen = { pct: v.pct, period: v.period };
  }
  if (st.card.vis) {
    if (view(st.card.id)) { el.card.classList.add('instant'); fillCard(); } else closeCard();
  }
  syncChip();
  update();
}

/** The "use this one now" chip follows the suggestion, unless the user dismissed this exact one. */
function syncChip() {
  const s = st.snap.suggestion;
  if (st.pill && st.pill.kind === 'peek') return;
  if (s && s.line !== st.dismissedChip) showPill({ id: s.from, a: s.a, b: s.b, dot: 'var(--dp-accent)', kind: 'chip' });
  else st.pill = null;
  if (!s) st.dismissedChip = null;
}

function showPill(pill) {
  st.pill = pill;
  el.pilltext.querySelector('.dot').style.background = pill.dot;
  el.pilltext.querySelector('.a').textContent = pill.a;
  el.pilltext.querySelector('.b').textContent = pill.b;
}

/** A peek extends the notch for a few seconds, then gives the chip back. */
function peek(p) {
  clearTimeout(timers.peek);
  showPill({ ...p, kind: 'peek' });
  st.peeking = true;
  update();
  timers.peek = setTimeout(() => { st.peeking = false; st.pill = null; syncChip(); update(); }, 3400);
}

function applyConfig(cfg) {
  st.cfg = cfg;
  setAppearance(cfg.appearance, () => { if (st.cfg) update(); });
  for (const v of st.snap.providers) cells.get(v.id)?.ring.update(ringProps(v));
  if (!st.ringDrag) st.order = cfg.providers.order.filter((id) => view(id));
  armIdle();
}

// ------------------------------------------------------------ interaction

function armIdle() {
  clearTimeout(timers.idle);
  const minutes = st.cfg.notch.collapse_after_min;
  if (minutes > 0) timers.idle = setTimeout(() => { st.idle = true; closeCard(); update(); }, minutes * 60000);
}
function touch() {
  if (st.idle) st.idle = false;
  armIdle();
}

function enterNotch() {
  clearTimeout(timers.close);
  clearTimeout(timers.tab);
  if (!st.hover) { st.hover = true; update(); }
  if (collapsedNow()) timers.tab = setTimeout(() => { st.tabOpen = true; update(); }, Math.min(150, st.cfg.notch.hover_delay_ms));
}
function leaveNotch() {
  clearTimeout(timers.open);
  clearTimeout(timers.tab);
  clearTimeout(timers.close);
  timers.close = setTimeout(() => {
    closeCard();
    st.hover = false;
    if (!st.dragging && !st.jiggle) st.tabOpen = false;
    touch();
    update();
  }, 400);
}

function setJiggle(on) {
  if (st.jiggle === on) return;
  st.jiggle = on;
  if (on) closeCard();
  for (const v of st.snap.providers) cells.get(v.id)?.ring.update(ringProps(v));
  update();
}

function bindCell(id, c) {
  c.el.addEventListener('pointerenter', () => {
    if (st.jiggle || st.dragging) return;
    clearTimeout(timers.close);
    clearTimeout(timers.open);
    if (st.card.vis && st.card.open) { if (st.card.id !== id) timers.open = setTimeout(() => openCard(id), CARD_SWITCH); }
    else timers.open = setTimeout(() => openCard(id), st.cfg.notch.hover_delay_ms);
  });
  c.el.addEventListener('pointerleave', () => clearTimeout(timers.open));
  c.el.addEventListener('pointerdown', (e) => {
    if (e.button !== 0) return;
    if (e.altKey) { e.preventDefault(); startDrag(); return; }
    if (st.jiggle) { e.preventDefault(); dragRing(e, id, c); return; }
    // Long press enters jiggle mode.
    const sx = e.clientX, sy = e.clientY;
    c.longPressed = false;
    clearTimeout(timers.long);
    timers.long = setTimeout(() => { c.longPressed = true; setJiggle(true); }, 600);
    const moved = (ev) => { if (Math.hypot(ev.clientX - sx, ev.clientY - sy) > 6) clearTimeout(timers.long); };
    const up = () => { clearTimeout(timers.long); window.removeEventListener('pointermove', moved); window.removeEventListener('pointerup', up); };
    window.addEventListener('pointermove', moved);
    window.addEventListener('pointerup', up);
  });
  c.el.addEventListener('click', () => {
    if (c.longPressed || st.jiggle) { c.longPressed = false; return; }
    touch();
    invoke('refresh', { id });
  });
  c.el.addEventListener('keydown', (e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); openCard(id); } });
  c.el.addEventListener('focus', () => { if (document.hasFocus()) openCard(id); });
  c.ring.badge.addEventListener('pointerdown', (e) => e.stopPropagation());
  c.ring.badge.addEventListener('click', (e) => {
    e.stopPropagation();
    invoke('set_setting', { path: `providers.${id}.enabled`, value: !view(id).enabled });
  });
}

/** Jiggle mode: drag a ring to a new slot. The order is saved when it is dropped. */
function dragRing(e, id, c) {
  const sx = e.clientX, sy = e.clientY, bx = c.x.x, by = c.y.x, vert = isVert();
  c.el.setPointerCapture(e.pointerId);
  st.ringDrag = { id, x: bx, y: by };
  const moved = (ev) => {
    const x = bx + ev.clientX - sx, y = by + ev.clientY - sy;
    st.ringDrag = { id, x, y };
    const L = layout(), ids = L.ids.slice(), from = ids.indexOf(id), pos = vert ? y : x;
    let to = 0;
    ids.forEach((_, i) => { const slot = L.slot(i); if (pos > (vert ? slot[1] : slot[0]) - (vert ? L.z.cellH : L.boxW) / 2) to = i; });
    if (to !== from) {
      const order = st.order.slice();
      order.splice(order.indexOf(id), 1);
      order.splice(order.indexOf(ids[to]) + (to > from ? 1 : 0), 0, id);
      st.order = order;
      retarget();
    }
    wake();
  };
  const up = () => {
    c.el.removeEventListener('pointermove', moved);
    c.el.removeEventListener('pointerup', up);
    c.el.removeEventListener('pointercancel', up);
    st.ringDrag = null;
    const rest = st.cfg.providers.order.filter((x) => !st.order.includes(x));
    invoke('set_setting', { path: 'providers.order', value: st.order.concat(rest) });
    update();
  };
  c.el.addEventListener('pointermove', moved);
  c.el.addEventListener('pointerup', up);
  c.el.addEventListener('pointercancel', up);
}

/** Starts moving the notch. Rust follows the cursor; the UI only does lift and wobble. */
async function startDrag() {
  if (st.dragging) return;
  if (st.cfg.notch.locked) {
    sp.shake.c = { r: 0.12, d: 0.25 };
    sp.shake.v = 260;
    peek({ id: visibleIds()[0], a: st.strings['notch.locked.title'], b: '', dot: 'var(--dp-ink3)' });
    return;
  }
  closeCard();
  st.dragging = true;
  st.tabOpen = true;
  sp.lift.c = springFor('snappy');
  sp.lift.t = reducedMotion() ? 1 : 1.02;
  await update();
  invoke('notch_drag_start').catch(() => endDrag());
}

function endDrag() {
  if (!st.dragging) return;
  st.dragging = false;
  st.tabOpen = false;
  sp.lift.c = springFor('snappy');
  sp.lift.t = 1;
  if (!reducedMotion()) { sp.squash.c = springFor('wobbly'); sp.squash.v += 2.6; } // landing wobble
  touch();
  update();
}

function bind() {
  el.layer.addEventListener('pointerenter', enterNotch);
  el.layer.addEventListener('pointerleave', leaveNotch);
  // Tabbing in reveals the gear and grip too (they stay focusable while hidden).
  el.layer.addEventListener('focusin', () => { if (!st.focusIn) { st.focusIn = true; update(); } });
  el.layer.addEventListener('focusout', (e) => { if (!el.layer.contains(e.relatedTarget)) { st.focusIn = false; update(); } });
  el.card.addEventListener('pointerenter', () => clearTimeout(timers.close));
  el.card.addEventListener('pointerleave', leaveNotch);

  el.hit.addEventListener('pointerdown', (e) => { if (e.button === 0 && e.altKey) { e.preventDefault(); startDrag(); } });
  el.hit.addEventListener('click', (e) => {
    if (e.altKey) return;
    touch();
    if (st.jiggle) { setJiggle(false); return; }
    if (st.cfg.notch.show === 'always' && !st.idle) return;
    st.keepOpen = !st.keepOpen;
    update();
  });

  el.gear.addEventListener('click', () => invoke('open_settings', { section: null }).catch(() => {}));
  el.gear.setAttribute('aria-label', st.strings['notch.settings'] || 'Settings');
  el.grip.title = st.strings['notch.grip'] || '';

  // Grip: hold 150 ms, or move 5 px, to start dragging. Double-click recentres.
  el.grip.addEventListener('pointerdown', (e) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const sx = e.clientX, sy = e.clientY;
    const cleanup = () => { clearTimeout(timers.grip); window.removeEventListener('pointermove', moved); window.removeEventListener('pointerup', cleanup); };
    const go = () => { cleanup(); startDrag(); };
    const moved = (ev) => { if (Math.hypot(ev.clientX - sx, ev.clientY - sy) > 5) go(); };
    timers.grip = setTimeout(go, 150);
    window.addEventListener('pointermove', moved);
    window.addEventListener('pointerup', cleanup);
  });
  el.grip.addEventListener('dblclick', () => invoke('notch_recentre').catch(() => {}));

  el.pill.addEventListener('click', () => {
    clearTimeout(timers.peek);
    if (st.pill && st.pill.kind === 'chip') st.dismissedChip = st.snap.suggestion?.line ?? null;
    st.peeking = false;
    st.pill = null;
    syncChip();
    update();
  });

  window.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    closeCard();
    invoke('notch_menu', { keepOpen: st.keepOpen, jiggle: st.jiggle }).catch(() => {});
  });
  window.addEventListener('keydown', (e) => {
    if (e.key !== 'Escape') return;
    closeCard();
    setJiggle(false);
    invoke('notch_blur').catch(() => {});
  });
}

// ------------------------------------------------------------ start

async function start() {
  const boot = await invoke('bootstrap');
  st.strings = boot.strings;
  document.documentElement.lang = boot.lang;
  if (boot.placement) st.place = boot.placement;
  applyConfig(boot.config);
  bind();

  listen('usage', applySnapshot);
  listen('settings-changed', (cfg) => { applyConfig(cfg); applySnapshot(st.snap); });
  listen('strings-changed', (s) => { st.strings = s.strings; document.documentElement.lang = s.lang; });
  listen('notch-dock', (place) => { const same = place.edge === st.place.edge; adoptPlacement(place); if (same) { lastShape = ''; update(); } });
  listen('notch-drop', endDrag);
  // Opened from the keyboard shortcut or the tray, the notch has focus; clicking
  // elsewhere closes the card and hands focus back to the no-activate mode.
  window.addEventListener('blur', () => {
    // Drop focus too: WebView2 restores it (with a focus ring, and the card) when the window is next focused.
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
    if (!st.card.open && !st.tabOpen) return;
    closeCard();
    st.tabOpen = false;
    update();
    invoke('notch_blur').catch(() => {});
  });
  listen('notch-peek', peek);
  listen('notch-command', (cmd) => {
    if (cmd === 'rearrange') setJiggle(!st.jiggle);
    if (cmd === 'keep_open') { st.keepOpen = !st.keepOpen; update(); }
    if (cmd === 'kick' && !reducedMotion()) { sp.lift.c = springFor('bouncy'); sp.lift.v += 0.5; update(); }
    if (cmd.open_card) { st.tabOpen = true; touch(); openCard(cmd.open_card).then(() => cells.get(cmd.open_card)?.el.focus()); }
  });

  // First frame: no animation, springs start at their targets.
  st.snap = boot.snapshot;
  st.order = st.cfg.providers.order.filter((id) => view(id));
  for (const v of st.snap.providers) { const c = makeCell(v.id); c.ring.update(ringProps(v)); c.el.setAttribute('aria-label', v.label); if (v.ring === 'ok') c.seen = { pct: v.pct, period: v.period }; }
  syncChip();
  retarget();
  for (const s of allSprings()) spring.set(s, s.t);
  syncClasses();
  await setShape(extent(false));
  render();
}

start();
