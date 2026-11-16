// The usage ring, shared by the notch and the settings preview.

/** Ring and notch dimensions per size setting. */
export const SIZES = {
  s: { d: 28, sw: 3, fs: 10, gf: 9, W: 46, cellH: 44, gap: 8, padY: 10 },
  m: { d: 34, sw: 3.5, fs: 11, gf: 10.5, W: 56, cellH: 51, gap: 10, padY: 12 },
  l: { d: 40, sw: 4, fs: 12, gf: 12, W: 66, cellH: 58, gap: 12, padY: 14 },
};

// Providers that have a colour logo; the others reuse the monochrome one.
const COLOR_LOGOS = new Set(['claude', 'codex', 'kimi', 'antigravity']);

const GREEN = [0.70, 0.15, 152], AMBER = [0.78, 0.15, 75], RED = [0.64, 0.20, 27];
const mix = (a, b, t) => a.map((x, i) => x + (b[i] - x) * Math.max(0, Math.min(1, t)));

/** Colour for a usage percentage: hard steps at the two levels, or a smooth ramp between them. */
export function usageColor(pct, look) {
  const warn = look.warn_at, crit = look.crit_at;
  let c;
  if (look.color_mode === 'smooth') {
    c = pct <= warn - 20 ? GREEN
      : pct < warn ? mix(GREEN, AMBER, (pct - (warn - 20)) / 20)
      : pct < crit ? mix(AMBER, RED, (pct - warn) / Math.max(1, crit - warn))
      : RED;
  } else {
    c = pct >= crit ? RED : pct >= warn ? AMBER : GREEN;
  }
  return `oklch(${c[0].toFixed(3)} ${c[1].toFixed(3)} ${c[2].toFixed(1)})`;
}

const TEMPLATE = `
  <div class="ring-disc">
    <svg>
      <circle class="track"></circle>
      <circle class="wtrack"></circle>
      <circle class="warc"></circle>
      <g class="spin"><circle class="arc"></circle></g>
      <circle class="sweep"></circle>
      <line class="tick-bg"></line>
      <line class="tick"></line>
    </svg>
    <div class="ring-glyph"><img alt=""></div>
    <div class="ring-danger" hidden>!</div>
    <button class="ring-badge" hidden></button>
  </div>
  <div class="ring-label">
    <span class="digits"><span class="digit"><span>${'0123456789'.split('').map((n) => `<span>${n}</span>`).join('')}</span></span></span>
    <span class="unit">%</span>
    <span class="text" hidden></span>
  </div>`;

/**
 * Creates a ring. Call `update(props)` with:
 *   id, pct, state ('ok' | 'stale' | 'loading' | 'na'), weekly, pace,
 *   look (the `appearance` settings), badge (null | 'plus' | 'minus'), dimmed.
 */
export function createRing() {
  const el = document.createElement('div');
  el.className = 'ring';
  el.innerHTML = TEMPLATE;
  const q = (s) => el.querySelector(s);
  const disc = q('.ring-disc'), svg = q('svg'), track = q('.track'), wtrack = q('.wtrack'), warc = q('.warc');
  const spinG = q('.spin'), arc = q('.arc'), sweep = q('.sweep'), tickBg = q('.tick-bg'), tick = q('.tick');
  const glyph = q('.ring-glyph'), img = q('.ring-glyph img'), danger = q('.ring-danger'), badge = q('.ring-badge');
  const label = q('.ring-label'), digitsBox = q('.digits'), unit = q('.unit'), text = q('.text');

  // Three odometer columns: hundreds, tens, ones.
  const column = digitsBox.firstElementChild;
  digitsBox.append(column.cloneNode(true), column.cloneNode(true));
  const digits = [...digitsBox.children];

  let spin = null, circumference = 0, lastProps = null;

  function update(p) {
    lastProps = p;
    const z = SIZES[p.look.size] || SIZES.m;
    const state = p.state || 'ok';
    const pct = Math.max(0, Math.min(100, Math.round(p.pct ?? 0)));
    const c = z.d / 2;
    const mode = p.look.weekly_ring;
    const hasWeekly = mode !== 'off' && p.weekly != null && (state === 'ok' || state === 'stale');
    let rMain, rWeekly;
    if (mode === 'outside') { rWeekly = c + 1.2; rMain = c - z.sw / 2 - 1.6; }
    else { rMain = c - z.sw / 2; rWeekly = rMain - z.sw / 2 - 2.4; }
    const C = 2 * Math.PI * rMain, Cw = 2 * Math.PI * rWeekly;
    circumference = C;
    const progress = state === 'loading' ? 0.22 : state === 'na' ? 0 : pct / 100;
    const stale = state === 'stale';
    const turn = `rotate(-90 ${c} ${c})`;

    el.classList.toggle('stale', stale);
    el.classList.toggle('dimmed', !!p.dimmed);
    el.classList.toggle('muted', state !== 'ok');
    disc.style.width = disc.style.height = z.d + 'px';
    svg.setAttribute('width', z.d);
    svg.setAttribute('height', z.d);

    for (const circle of [track, arc, sweep]) {
      circle.setAttribute('cx', c); circle.setAttribute('cy', c); circle.setAttribute('r', rMain.toFixed(2));
      circle.style.strokeWidth = z.sw + 'px';
    }
    for (const circle of [wtrack, warc]) {
      circle.setAttribute('cx', c); circle.setAttribute('cy', c); circle.setAttribute('r', rWeekly.toFixed(2));
    }
    arc.setAttribute('transform', turn);
    arc.style.stroke = stale ? 'var(--dp-stale)' : state === 'loading' ? 'var(--dp-ink3)' : usageColor(pct, p.look);
    arc.style.strokeDasharray = C.toFixed(2);
    arc.style.strokeDashoffset = (C * (1 - progress)).toFixed(2);
    arc.style.opacity = progress > 0.004 ? 1 : 0;
    sweep.setAttribute('transform', turn);
    sweep.style.strokeDasharray = C.toFixed(2);
    sweep.style.strokeDashoffset = C.toFixed(2);

    wtrack.style.opacity = hasWeekly ? 1 : 0;
    warc.setAttribute('transform', turn);
    warc.style.stroke = stale ? 'var(--dp-stale)' : usageColor(p.weekly || 0, p.look);
    warc.style.strokeDasharray = Cw.toFixed(2);
    warc.style.strokeDashoffset = (Cw * (1 - (p.weekly || 0) / 100)).toFixed(2);
    warc.style.opacity = hasWeekly && (p.weekly || 0) > 0 ? 0.9 : 0;

    // Pace tick: where an even pace would be right now.
    const showPace = p.look.pace_tick && p.pace != null && state === 'ok';
    const reach = z.sw / 2 + 1.6;
    for (const line of [tickBg, tick]) {
      line.setAttribute('x1', c); line.setAttribute('x2', c);
      line.setAttribute('y1', (c - rMain - reach).toFixed(2)); line.setAttribute('y2', (c - rMain + reach).toFixed(2));
      line.setAttribute('transform', `rotate(${((p.pace || 0) * 360).toFixed(1)} ${c} ${c})`);
      line.style.opacity = showPace ? 1 : 0;
    }

    // Glyph: the provider logo, tinted or monochrome.
    const color = p.look.glyphs === 'color' && COLOR_LOGOS.has(p.id) && !stale;
    const src = `../icons/p-${p.id}${color ? '-color' : ''}.svg`;
    if (img.getAttribute('src') !== src) img.setAttribute('src', src);
    img.classList.toggle('color', color);
    img.alt = p.name || '';
    img.style.width = img.style.height = Math.round(z.d * 0.44) + 'px';
    glyph.style.fontSize = z.gf + 'px';

    danger.hidden = !(state === 'ok' && pct >= p.look.crit_at);
    badge.hidden = !p.badge;
    if (p.badge) {
      badge.textContent = p.badge === 'plus' ? '+' : '−';
      badge.setAttribute('aria-label', p.badgeLabel || '');
    }

    // Label: odometer digits, or a placeholder when there is no number.
    const lineHeight = Math.round(z.fs * 1.25);
    label.style.fontSize = z.fs + 'px';
    label.style.lineHeight = label.style.height = lineHeight + 'px';
    const numeric = state === 'ok' || state === 'stale';
    digitsBox.hidden = unit.hidden = !numeric;
    text.hidden = numeric;
    text.textContent = state === 'loading' ? '…' : '—';
    const shown = p.look.show_remaining ? 100 - pct : pct;
    const values = [Math.floor(shown / 100) % 10, Math.floor(shown / 10) % 10, shown % 10];
    digits.forEach((digit, i) => {
      const visible = i === 2 || (i === 1 && shown >= 10) || (i === 0 && shown >= 100);
      digit.classList.toggle('off', !visible);
      digit.style.height = lineHeight + 'px';
      digit.firstElementChild.style.transform = `translateY(${-values[i] * lineHeight}px)`;
    });

    // Loading spinner: one compositor animation, only while loading.
    const loading = state === 'loading';
    if (loading && !spin) spin = spinG.animate([{ transform: 'rotate(0deg)' }, { transform: 'rotate(360deg)' }], { duration: 1000, iterations: Infinity });
    if (!loading && spin) { spin.cancel(); spin = null; }
  }

  /** Reset sweep: a green arc runs once round the ring, with a soft glow. */
  function pulse(reduced) {
    const C = circumference;
    sweep.animate(
      [{ strokeDashoffset: C, opacity: 1 }, { strokeDashoffset: 0, opacity: 1, offset: 0.65 }, { strokeDashoffset: 0, opacity: 0 }],
      { duration: reduced ? 320 : 950, easing: 'cubic-bezier(.25,.9,.3,1)' });
    if (reduced) return;
    disc.animate(
      [{ boxShadow: '0 0 0 0 oklch(0.70 0.15 152 / 0)' }, { boxShadow: '0 0 0 6px oklch(0.70 0.15 152 / 0.28)', offset: 0.35 }, { boxShadow: '0 0 0 11px oklch(0.70 0.15 152 / 0)' }],
      { duration: 950, easing: 'ease-out' });
  }

  /** Critical shake: a short side-to-side shake when usage crosses the critical level. */
  function shake(reduced) {
    if (reduced) return;
    el.animate([0, 5, -4.5, 3.2, -2.2, 1.1, 0].map((x) => ({ transform: `translateX(${x}px)` })), { duration: 460, easing: 'linear' });
  }

  return { el, badge, update, pulse, shake, get props() { return lastProps; } };
}
