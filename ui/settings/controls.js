// Row controls for the settings window. Each factory returns
//   { el, text, update(cfg) }
// `el` is the row, `text` is what the search box matches (label + description),
// and `update` syncs the control to the current settings without rebuilding it,
// so CSS transitions (sliding thumb, toggle knob) keep running.
//
// `ctx` is provided by settings.js:
//   ctx.t(key, args)        translated text
//   ctx.get(cfg, path)      value at a dotted settings path
//   ctx.set(path, value, flash)   ask Rust to change a setting

const h = (tag, cls, text) => {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (text != null) node.textContent = text;
  return node;
};

/** The common row shell: label + description on the left, controls on the right. */
function row(ctx, { label, desc, sub, dep }) {
  const el = h('div', 'row' + (sub ? ' sub' : ''));
  const text = h('div', 'row-text'), labelEl = h('div', 'row-label', label), ctl = h('div', 'row-ctl');
  text.append(labelEl);
  if (desc) text.append(h('div', 'row-desc', desc));
  const error = h('div', 'row-error');
  error.hidden = true;
  text.append(error);
  el.append(text, ctl);
  return {
    el, ctl, labelEl, error,
    text: `${label} ${desc || ''}`.toLowerCase(),
    showError(message) { error.textContent = message || ''; error.hidden = !message; },
    syncDep(cfg) { if (dep) el.classList.toggle('off', !ctx.get(cfg, dep)); },
  };
}

const labelOf = (ctx, path) => ({ label: ctx.t(`s.${path}.label`), desc: ctx.t(`s.${path}.desc`, null, '') });

function makeSwitch(ctx, label, onToggle) {
  const state = h('span', 'switch-text'), button = h('button', 'switch');
  button.type = 'button';
  button.setAttribute('role', 'switch');
  button.setAttribute('aria-label', label);
  button.addEventListener('click', onToggle);
  return {
    nodes: [state, button],
    set(on) { button.setAttribute('aria-checked', String(!!on)); state.textContent = ctx.t(on ? 'win.on' : 'win.off'); },
  };
}

export function toggle(ctx, path, opts = {}) {
  const meta = { ...labelOf(ctx, path), ...opts };
  const r = row(ctx, meta);
  let value = false;
  const sw = makeSwitch(ctx, meta.label, () => ctx.set(path, !value));
  r.ctl.append(...sw.nodes);
  return {
    ...r,
    update(cfg) {
      value = !!ctx.get(cfg, path);
      sw.set(value);
      if (opts.labelArgs) r.labelEl.textContent = ctx.t(`s.${path}.label`, opts.labelArgs(cfg));
      r.syncDep(cfg);
    },
  };
}

/** Segmented control. `values` are the stored values; labels come from `s.<path>.opt.<value>`. */
export function seg(ctx, path, values, opts = {}) {
  const meta = { ...labelOf(ctx, path), ...opts };
  const r = row(ctx, meta);
  const box = h('div', 'seg'), thumb = h('div', 'seg-thumb');
  box.setAttribute('role', 'radiogroup');
  box.setAttribute('aria-label', meta.label);
  box.style.gridTemplateColumns = `repeat(${values.length}, minmax(0, 1fr))`;
  thumb.style.width = `calc((100% - 4px) / ${values.length})`;
  box.append(thumb);
  const buttons = values.map((value) => {
    const b = h('button', '', ctx.t(`s.${path}.opt.${value}`));
    b.type = 'button';
    b.setAttribute('role', 'radio');
    b.addEventListener('click', () => ctx.set(path, value));
    box.append(b);
    return b;
  });
  r.ctl.append(box);
  if (opts.buttons) r.ctl.append(...opts.buttons.map((b) => button(ctx, b)));
  return {
    ...r,
    update(cfg) {
      const i = Math.max(0, values.indexOf(ctx.get(cfg, path)));
      thumb.style.transform = `translateX(${i * 100}%)`;
      buttons.forEach((b, j) => b.setAttribute('aria-checked', String(j === i)));
      r.syncDep(cfg);
    },
  };
}

function selectBox(small) {
  const wrap = h('span', 'select-wrap'), sel = h('select', 'select' + (small ? ' small' : ''));
  const chevron = h('img');
  chevron.src = '../icons/chevron_down.svg';
  chevron.alt = '';
  wrap.append(sel, chevron);
  return { wrap, sel };
}

/** Fills a <select> with [{ value, label }] and selects `value`. */
export function fillSelect(sel, options, value) {
  const key = JSON.stringify(options);
  if (sel.dataset.key !== key) {
    sel.dataset.key = key;
    sel.replaceChildren(...options.map((o) => { const opt = h('option', '', o.label); opt.value = o.value; return opt; }));
  }
  sel.value = value;
}

/**
 * Drop-down. `options(cfg)` returns [{ value, label }]; `read(cfg)` the selected
 * value and `write(value)` applies a choice (defaults to the plain path).
 */
export function select(ctx, path, { options, read, write, small, ...opts }) {
  const meta = { ...labelOf(ctx, path), ...opts };
  const r = row(ctx, meta);
  const { wrap, sel } = selectBox(small);
  sel.setAttribute('aria-label', meta.label);
  sel.addEventListener('change', () => (write ? write(sel.value) : ctx.set(path, sel.value)));
  r.ctl.append(wrap);
  return {
    ...r,
    update(cfg) { fillSelect(sel, options(cfg), read ? read(cfg) : ctx.get(cfg, path)); r.syncDep(cfg); },
  };
}
export { selectBox };

export function slider(ctx, path, { min, max, step, unit, limit, ...opts }) {
  const meta = { ...labelOf(ctx, path), ...opts };
  const r = row(ctx, meta);
  const valueEl = h('span', 'slider-value'), track = h('div', 'slider');
  const fill = h('div', 'slider-fill'), thumb = h('div', 'slider-thumb');
  track.append(h('div', 'slider-track'), fill, thumb);
  track.tabIndex = 0;
  track.setAttribute('role', 'slider');
  track.setAttribute('aria-label', meta.label);
  track.setAttribute('aria-valuemin', min);
  track.setAttribute('aria-valuemax', max);
  let value = min, bounds = [min, max];
  // Shows the value at once; Rust confirms (or corrects) it through settings-changed.
  const show = (v) => {
    const pct = ((v - min) / (max - min)) * 100;
    fill.style.width = thumb.style.left = pct.toFixed(1) + '%';
    valueEl.textContent = unit === '%' ? `${v}%` : `${v} ${unit}`;
    track.setAttribute('aria-valuenow', v);
  };
  const commit = (v) => {
    v = Math.max(bounds[0], Math.min(bounds[1], Math.round(v / step) * step));
    if (v === value) return;
    value = v;
    show(v);
    ctx.set(path, v);
  };
  track.addEventListener('pointerdown', (e) => {
    const box = track.getBoundingClientRect();
    const at = (x) => commit(min + (max - min) * Math.max(0, Math.min(1, (x - box.left) / box.width)));
    at(e.clientX);
    track.setPointerCapture(e.pointerId);
    const moved = (ev) => at(ev.clientX);
    const up = () => { track.removeEventListener('pointermove', moved); track.removeEventListener('pointerup', up); track.removeEventListener('pointercancel', up); };
    track.addEventListener('pointermove', moved);
    track.addEventListener('pointerup', up);
    track.addEventListener('pointercancel', up);
  });
  track.addEventListener('keydown', (e) => {
    const delta = e.key === 'ArrowRight' || e.key === 'ArrowUp' ? step : e.key === 'ArrowLeft' || e.key === 'ArrowDown' ? -step : 0;
    if (delta) { e.preventDefault(); commit(value + delta); }
  });
  r.ctl.append(valueEl, track);
  return {
    ...r,
    update(cfg) {
      value = ctx.get(cfg, path);
      // `limit(cfg)` narrows the range (critical must stay above warning).
      bounds = limit ? limit(cfg) : [min, max];
      show(value);
      r.syncDep(cfg);
    },
  };
}

/** A button: { label, onClick, danger, external }. */
export function button(ctx, { label, onClick, danger, external, small }) {
  const b = h('button', 'btn' + (danger ? ' danger' : '') + (small ? ' small' : ''));
  b.type = 'button';
  b.append(h('span', '', label));
  if (external) { const icon = h('img'); icon.src = '../icons/open.svg'; icon.alt = ''; b.append(icon); }
  b.addEventListener('click', onClick);
  return b;
}

/** A row that only has buttons (and optionally a value on the right). */
export function buttons(ctx, { label, desc, info, list, ...opts }) {
  const r = row(ctx, { label, desc, ...opts });
  if (info) r.ctl.append(h('span', 'row-info', info));
  r.ctl.append(...list.map((b) => button(ctx, b)));
  return { ...r, update(cfg) { r.syncDep(cfg); } };
}

/** Global shortcut capture. A taken shortcut is refused by Rust and shown as an error. */
export function hotkey(ctx, path) {
  const r = row(ctx, labelOf(ctx, path));
  const keys = h('div', 'keys');
  keys.tabIndex = 0;
  keys.setAttribute('aria-label', ctx.t(`s.${path}.label`));
  let capturing = false, value = '';
  const change = button(ctx, { label: '', onClick: () => { capturing = !capturing; r.showError(''); draw(); if (capturing) keys.focus(); } });
  const clear = button(ctx, { label: ctx.t(`s.${path}.clear`), onClick: () => { capturing = false; save(''); } });

  function draw() {
    keys.classList.toggle('capture', capturing);
    keys.replaceChildren();
    if (capturing) keys.append(h('span', 'hint', ctx.t(`s.${path}.press`)));
    else if (!value) keys.append(h('span', 'hint', ctx.t(`s.${path}.none`)));
    else for (const part of value.split('+')) keys.append(h('kbd', '', part));
    change.firstChild.textContent = ctx.t(`s.${path}.${capturing ? 'cancel' : 'change'}`);
  }
  async function save(next) {
    r.showError('');
    const error = await ctx.set(path, next, ctx.t(`s.${path}.saved`));
    if (error) r.showError(ctx.t(error, null, error));
    draw();
  }
  keys.addEventListener('keydown', (e) => {
    if (!capturing) return;
    e.preventDefault();
    if (e.key === 'Escape') { capturing = false; draw(); return; }
    if (['Shift', 'Control', 'Alt', 'Meta', 'OS'].includes(e.key)) return;
    const parts = [];
    if (e.metaKey) parts.push('Win');
    if (e.ctrlKey) parts.push('Ctrl');
    if (e.altKey) parts.push('Alt');
    if (e.shiftKey) parts.push('Shift');
    // Physical key: "KeyU" → "U", "Digit7" → "7", otherwise the name ("F5", "Space").
    const key = e.code.replace(/^Key|^Digit/, '');
    if (!/^[A-Za-z0-9]{1,12}$/.test(key)) return;
    capturing = false;
    if (!parts.length) { r.showError(ctx.t(`s.${path}.need_modifier`)); draw(); return; }
    save([...parts, key].join('+'));
  });
  keys.addEventListener('blur', () => { if (capturing) { capturing = false; draw(); } });
  r.ctl.append(keys, change, clear);
  return { ...r, update(cfg) { value = ctx.get(cfg, path); draw(); } };
}

/** Quiet hours: a toggle plus the two times. */
export function quietHours(ctx) {
  const base = 'alerts.quiet';
  const r = row(ctx, labelOf(ctx, `${base}.enabled`));
  const times = h('div', 'times');
  const input = (which) => {
    const el = h('input');
    el.type = 'time';
    el.setAttribute('aria-label', ctx.t(`s.${base}.${which}_label`));
    el.addEventListener('change', () => { if (el.value) ctx.set(`${base}.${which}`, el.value); });
    return el;
  };
  const from = input('from'), to = input('to');
  times.append(h('span', '', ctx.t(`s.${base}.from`)), from, h('span', '', ctx.t(`s.${base}.to`)), to);
  let on = false;
  const sw = makeSwitch(ctx, ctx.t(`s.${base}.enabled.label`), () => ctx.set(`${base}.enabled`, !on));
  r.ctl.append(times, ...sw.nodes);
  return {
    ...r,
    update(cfg) {
      const q = cfg.alerts.quiet;
      on = q.enabled;
      sw.set(on);
      times.classList.toggle('off', !on);
      if (document.activeElement !== from) from.value = q.from;
      if (document.activeElement !== to) to.value = q.to;
    },
  };
}
