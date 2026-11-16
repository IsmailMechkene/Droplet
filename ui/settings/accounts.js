// Settings › Accounts: one card per provider, with drag-to-reorder.
// The order is the same one the notch and jiggle mode use.

import { fillSelect, selectBox } from './controls.js';

const h = (tag, cls, text) => {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (text != null) node.textContent = text;
  return node;
};
const icon = (name) => { const img = h('img'); img.src = `../icons/${name}.svg`; img.alt = ''; return img; };

/**
 * Draws the accounts page into `host`.
 * `ctx` is the settings context; `act` has refresh / showFile / browseAgy; `accounts` comes from Rust.
 */
export function renderAccounts(host, ctx, act, accounts, cfg) {
  const t = ctx.t;
  host.replaceChildren();

  const banner = h('div', 'info-banner');
  banner.append(icon('info'), h('div', '', t('acc.info')));
  host.append(banner);

  if (!accounts.some((a) => a.installed)) {
    const empty = h('div', 'empty');
    const chips = h('div', 'chips');
    for (const name of ['Claude Code', 'Codex CLI', 'Cursor', 'OpenCode', 'Kimi Code', 'Antigravity']) chips.append(h('span', 'chip', name));
    const refresh = h('button', 'primary', t('acc.refresh'));
    refresh.type = 'button';
    refresh.addEventListener('click', () => act.refresh(null));
    empty.append(h('span', 'ghost'), h('div', 't', t('acc.empty.title')), h('div', 'b', t('acc.empty.body')), chips, refresh);
    host.append(empty);
  }

  const list = h('div');
  list.id = 'accounts';
  for (const a of accounts) list.append(card(a));
  host.append(list);

  function card(a) {
    const p = cfg.providers[a.id];
    const el = h('div', 'acc');
    el.dataset.id = a.id;

    const grip = h('button', 'acc-grip');
    grip.type = 'button';
    grip.setAttribute('aria-label', t('acc.drag', { name: a.name }));
    grip.append(icon('re_order_dots_vertical'));
    grip.addEventListener('pointerdown', (e) => startDrag(e, el, a.id));
    grip.addEventListener('keydown', (e) => {
      if (!e.altKey || (e.key !== 'ArrowUp' && e.key !== 'ArrowDown')) return;
      e.preventDefault();
      const order = cfg.providers.order.slice(), i = order.indexOf(a.id), j = i + (e.key === 'ArrowUp' ? -1 : 1);
      if (j < 0 || j >= order.length) return;
      order.splice(i, 1);
      order.splice(j, 0, a.id);
      act.refocus = a.id;
      ctx.set('providers.order', order, t('acc.order_saved'));
    });

    const logo = h('div', 'acc-logo');
    logo.append(icon(`p-${a.id}`));

    const head = h('div', 'acc-head');
    const sw = h('button', 'switch');
    sw.type = 'button';
    sw.setAttribute('role', 'switch');
    sw.setAttribute('aria-checked', String(p.enabled));
    sw.setAttribute('aria-label', t('acc.enabled_label', { name: a.name }));
    sw.addEventListener('click', () => ctx.set(`providers.${a.id}.enabled`, !p.enabled));
    head.append(h('span', 'acc-name', a.name), h('span', `pill ${a.kind}`, a.status), h('span', 'grow'), h('span', 'acc-en', t(p.enabled ? 'acc.enabled' : 'acc.disabled')), sw);

    const meta = h('div', 'acc-meta');
    meta.append(h('span', 'acc-path', t('acc.reads', { path: a.path })), h('span', 'acc-updated', a.updated));

    const ctl = h('div', 'acc-ctl' + (p.enabled ? '' : ' off'));
    const limit = h('div', 'acc-limit');
    const { wrap, sel } = selectBox(true);
    sel.setAttribute('aria-label', t('acc.main_limit'));
    const options = a.limits.map((l) => ({ value: l.id, label: l.label }));
    if (!options.some((o) => o.value === p.main_limit)) options.unshift({ value: p.main_limit, label: options.length ? p.main_limit : '—' });
    fillSelect(sel, options, p.main_limit);
    sel.disabled = options.length < 2;
    sel.addEventListener('change', () => ctx.set(`providers.${a.id}.main_limit`, sel.value));
    limit.append(h('span', '', t('acc.main_limit')), wrap);

    const include = h('button', 'check');
    include.type = 'button';
    include.setAttribute('role', 'checkbox');
    include.setAttribute('aria-checked', String(p.suggest));
    include.append(h('span', 'box', p.suggest ? '✓' : ''), h('span', '', t('acc.include')));
    include.addEventListener('click', () => ctx.set(`providers.${a.id}.suggest`, !p.suggest));

    const actions = h('div', 'acc-actions');
    const refresh = h('button', 'btn small');
    refresh.type = 'button';
    refresh.append(icon('arrow_clockwise'), h('span', '', t('acc.refresh')));
    refresh.addEventListener('click', () => act.refresh(a));
    const show = h('button', 'btn small');
    show.type = 'button';
    show.append(icon('folder_open'), h('span', '', t('acc.show_file')));
    show.addEventListener('click', () => act.showFile(a));
    actions.append(refresh, show);
    ctl.append(limit, include, h('span', 'grow'), actions);

    const body = h('div', 'acc-body');
    body.append(head, meta, ctl);

    if (a.id === 'antigravity') {
      const agy = h('div', 'acc-agy');
      const txt = h('div', 'txt');
      txt.append(h('span', '', t('acc.agy.label')), h('span', '', t('acc.agy.desc')));
      const input = h('input');
      input.type = 'text';
      input.spellcheck = false;
      input.value = p.agy_path || '';
      input.placeholder = t('acc.agy.auto');
      input.setAttribute('aria-label', t('acc.agy.label'));
      input.addEventListener('change', () => ctx.set('providers.antigravity.agy_path', input.value.trim()));
      const browse = h('button', 'btn', t('acc.agy.browse'));
      browse.type = 'button';
      browse.addEventListener('click', act.browseAgy);
      agy.append(txt, input, browse);
      body.append(agy);
    }
    el.append(grip, logo, body);
    return el;
  }

  // Drag a card by its grip: the others slide out of the way, the order is saved on drop.
  function startDrag(e, el, id) {
    e.preventDefault();
    const cards = [...list.children];
    const rects = cards.map((c) => c.getBoundingClientRect());
    const from = cards.indexOf(el), startY = e.clientY;
    const stepOf = (i) => (rects[i + 1] ? rects[i + 1].top - rects[i].top : rects[i].height + 6);
    let to = from;
    el.classList.add('dragging');
    act.dragging = true;
    const moved = (ev) => {
      const dy = ev.clientY - startY, centre = rects[from].top + rects[from].height / 2 + dy;
      to = rects.findIndex((r) => centre < r.top + r.height / 2);
      if (to < 0) to = rects.length - 1;
      cards.forEach((c, i) => {
        let shift = 0;
        if (i === from) shift = dy;
        else if (from < to && i > from && i <= to) shift = -stepOf(from);
        else if (from > to && i < from && i >= to) shift = stepOf(from);
        c.style.transform = shift ? `translateY(${shift}px)` : '';
      });
    };
    const up = () => {
      window.removeEventListener('pointermove', moved);
      window.removeEventListener('pointerup', up);
      act.dragging = false;
      el.classList.remove('dragging');
      if (to === from) { cards.forEach((c) => { c.style.transform = ''; }); return; }
      const order = cards.map((c) => c.dataset.id);
      order.splice(from, 1);
      order.splice(to, 0, id);
      ctx.set('providers.order', order, t('acc.order_saved'));
    };
    window.addEventListener('pointermove', moved);
    window.addEventListener('pointerup', up);
  }
}
