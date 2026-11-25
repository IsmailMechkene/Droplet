// The settings window. Rust owns the settings: every control asks Rust to
// change one value, Rust validates and saves it, and "settings-changed" comes
// back to every window. There is no Save button and no local copy to drift.

import { invoke, listen } from '../shared/ipc.js';
import { setAppearance, spring } from '../shared/theme.js';
import { SECTIONS, buildSections } from './sections.js';
import { renderAccounts } from './accounts.js';
import { renderPreview, previewPlacement, bounce } from './preview.js';

const $ = (id) => document.getElementById(id);
const h = (tag, cls, text) => {
  const node = document.createElement(tag);
  if (cls) node.className = cls;
  if (text != null) node.textContent = text;
  return node;
};

const st = {
  cfg: null, strings: {}, data: { accounts: [], files: [], monitors: [], version: '', config_path: '' },
  section: window.__DROPLET_SECTION__ || 'general', query: '', rows: null, live: [], previewHover: false,
};
const act = { dragging: false, refocus: null };
let flashTimer = 0;

// ------------------------------------------------------------ context

/** Translated text. `{name}` placeholders are filled from `args`. */
function t(key, args, fallback) {
  let text = st.strings[key] ?? fallback ?? key;
  if (args) for (const [name, value] of Object.entries(args)) text = text.replaceAll(`{${name}}`, value);
  return text;
}
const get = (cfg, path) => path.split('.').reduce((node, key) => node?.[key], cfg);

function flash(text, error = false) {
  clearTimeout(flashTimer);
  $('saved-text').textContent = text;
  $('saved').classList.toggle('error', error);
  $('saved').firstElementChild.textContent = error ? '!' : '✓';
  $('saved').classList.add('on');
  flashTimer = setTimeout(() => $('saved').classList.remove('on'), error ? 4000 : 1300);
}

/** Asks Rust to change one setting. Resolves to null, or to the error text if Rust refused. */
async function set(path, value, message) {
  try {
    await invoke('set_setting', { path, value });
    flash(message || t('win.saved'));
    return null;
  } catch (error) {
    // Rust refused: redraw from the unchanged settings so the control snaps back.
    sync();
    const text = String(error);
    if (!text.startsWith('s.')) flash(t('win.error', { error: text }), true);
    return text;
  }
}
const ctx = { t, get, set };

// ------------------------------------------------------------ actions

Object.assign(act, {
  monitors: () => st.data.monitors,
  version: () => st.data.version,
  configPath: () => st.data.config_path,
  async resetColors() {
    // Order matters: critical must stay above warning at every step.
    const a = st.cfg.appearance;
    if (a.warn_at > 70) { await set('appearance.warn_at', 70); await set('appearance.crit_at', 90); }
    else { await set('appearance.crit_at', 90); await set('appearance.warn_at', 70); }
    set('appearance.color_mode', 'hard', t('s.appearance.colors.done'));
  },
  previewSpring() {
    bounce($('preview-bounce'), spring('bouncy'));
    invoke('notch_preview').catch(() => {});
  },
  async setScreen(value) {
    if (value.startsWith('monitor:')) {
      await set('notch.monitor_id', value.slice('monitor:'.length));
      set('notch.screen', 'monitor');
    } else set('notch.screen', value);
  },
  recentre: () => invoke('notch_recentre').then(() => flash(t('s.notch.position.done'))),
  refresh(account) {
    invoke('refresh', { id: account ? account.id : null });
    flash(account ? t('acc.refreshed', { name: account.name }) : t('win.done'));
  },
  async showFile(account) {
    const ok = await invoke('show_file_location', { id: account.id });
    flash(t(ok ? 'acc.opened' : 'acc.no_file'), !ok);
  },
  browseAgy: () => invoke('pick_agy_path').catch(() => {}),
  openPage: (page) => invoke('open_project_page', { page }),
  openConfig: () => invoke('open_config_file'),
  async exportCsv() {
    const ok = await invoke('export_history').catch(() => false);
    flash(t(ok ? 's.history.data.exported' : 's.history.data.empty'), !ok);
  },
  clearHistory: () => confirmDialog(t('confirm.clear.title'), t('confirm.clear.body'), t('confirm.clear.yes'), async () => {
    await invoke('clear_history').catch(() => {});
    flash(t('s.history.data.cleared'));
  }),
  openNetworkLog: () => invoke('open_network_log').catch(() => {}),
  async copyDiagnostics() {
    await invoke('copy_diagnostics').catch(() => {});
    flash(t('s.about.diagnostics.copied'));
  },
});

function confirmDialog(title, text, yes, onYes) {
  $('dialog-title').textContent = title;
  $('dialog-text').textContent = text;
  $('dialog-yes').textContent = yes;
  $('dialog-no').textContent = t('confirm.cancel');
  $('dialog').setAttribute('aria-label', title);
  $('scrim').hidden = false;
  $('dialog-no').focus();
  const close = () => { $('scrim').hidden = true; };
  $('dialog-yes').onclick = () => { close(); onYes(); };
  $('dialog-no').onclick = close;
}

// ------------------------------------------------------------ drawing

function group(rows, title) {
  const box = h('div', 'group');
  if (title) box.append(h('div', 'group-title', title));
  const card = h('div', 'card');
  rows.forEach((r) => card.append(r.el));
  box.append(card);
  return box;
}

function privacyExtras(page) {
  page.append(h('div', 'group-title', t('privacy.files')));
  const files = h('div', 'files');
  for (const f of st.data.files) {
    const row = h('div', 'file' + (f.found ? ' found' : ''));
    row.append(h('span', 'mark', f.found ? '✓' : '–'), h('span', 'path', f.path), h('span', 'state', t(f.found ? 'privacy.found' : 'privacy.not_found')));
    files.append(row);
  }
  page.append(files);
  const promises = h('div', 'promises');
  for (const key of ['no_telemetry', 'no_update', 'no_account', 'read_only']) {
    const item = h('span');
    item.append(h('b', '', '✓'), document.createTextNode(t(`privacy.${key}`)));
    promises.append(item);
  }
  page.append(promises);
}

/** Centres the accent bar on the current nav item (items grow when a translation wraps). */
function placeNavMark() {
  const item = $('nav').querySelector(`.nav-item[data-id="${st.section}"]`);
  if (!item) return;
  const mark = $('nav-mark');
  mark.style.transform = `translateY(${item.offsetTop + (item.offsetHeight - mark.offsetHeight) / 2 - mark.offsetTop}px)`;
}

/** Rebuilds the page for the current section or search. */
function draw(animate) {
  const page = $('page'), q = st.query.trim().toLowerCase();
  page.replaceChildren();
  st.live = [];

  $('title').textContent = q ? t('win.search_results') : t(`sec.${st.section}`);
  for (const b of $('nav').querySelectorAll('.nav-item')) {
    const current = !q && b.dataset.id === st.section;
    b.classList.toggle('current', current);
    b.setAttribute('aria-current', current ? 'page' : 'false');
  }
  placeNavMark();
  $('nav-mark').style.opacity = q ? 0 : 1;
  $('preview').hidden = !!q || (st.section !== 'appearance' && st.section !== 'notch');

  if (q) {
    // Search matches a row's label and description, across every section.
    for (const s of SECTIONS) {
      const rows = (st.rows[s.id] || []).flat().filter((r) => r.text.includes(q));
      if (rows.length) { page.append(group(rows, t(`sec.${s.id}`))); st.live.push(...rows); }
    }
    if (!st.live.length) page.append(h('div', 'no-results', t('win.no_results', { q: st.query.trim() })));
  } else if (st.section === 'accounts') {
    renderAccounts(page, ctx, act, st.data.accounts, st.cfg);
    if (act.refocus) { page.querySelector(`.acc[data-id="${act.refocus}"] .acc-grip`)?.focus(); act.refocus = null; }
  } else {
    for (const rows of st.rows[st.section] || []) { page.append(group(rows)); st.live.push(...rows); }
    if (st.section === 'privacy') privacyExtras(page);
  }
  sync();
  if (animate) {
    $('content').scrollTop = 0;
    page.classList.remove('enter');
    void page.offsetWidth; // restart the animation
    page.classList.add('enter');
  }
}

/** Syncs every visible control and the preview to the current settings. */
function sync() {
  if (!st.cfg) return;
  for (const row of st.live) row.update(st.cfg);
  if ($('preview').hidden) return;
  const n = st.cfg.notch, hidden = n.show === 'hidden', collapsed = n.show === 'hover' && !st.previewHover;
  $('preview-caption').textContent = t(hidden ? 'pv.hidden' : collapsed ? 'pv.hover' : 'pv.live');
  const stage = $('preview-stage'), place = previewPlacement(n.edge);
  stage.removeAttribute('style');
  Object.assign(stage.style, place, { opacity: hidden ? 0.25 : 1 });
  $('preview-bounce').style.transformOrigin = place.transformOrigin;
  renderPreview($('preview-bounce'), { edge: n.edge, look: st.cfg.appearance, collapsed, highContrast: n.high_contrast });
}

function applyConfig(cfg) {
  const languageChanged = st.cfg && st.cfg.general.language !== cfg.general.language;
  st.cfg = cfg;
  setAppearance(cfg.appearance);
  if (languageChanged) return; // "strings-changed" follows and rebuilds everything
  if (st.section === 'accounts' && !st.query) { if (!act.dragging) draw(false); } else sync();
}

/** (Re)builds all static text and rows. Runs at start and when the language changes. */
function build() {
  document.documentElement.lang = st.lang || 'en';
  document.title = t('win.title');
  $('win-title').textContent = t('win.title');
  $('tagline').textContent = t('app.tagline');
  $('search').placeholder = t('win.search');
  $('search').setAttribute('aria-label', t('win.search'));
  for (const [id, key] of [['win-min', 'win.minimize'], ['win-max', 'win.maximize'], ['win-close', 'win.close']]) $(id).setAttribute('aria-label', t(key));
  const nav = $('nav');
  nav.querySelectorAll('.nav-item').forEach((b) => b.remove());
  for (const s of SECTIONS) {
    const b = h('button', 'nav-item');
    b.type = 'button';
    b.dataset.id = s.id;
    const img = h('img');
    img.src = `../icons/${s.icon}.svg`;
    img.alt = '';
    b.append(img, h('span', '', t(`sec.${s.id}`)));
    b.addEventListener('click', () => go(s.id));
    nav.append(b);
  }
  st.rows = buildSections(ctx, act);
  draw(false);
}

function go(section) {
  st.section = section;
  st.query = '';
  $('search').value = '';
  draw(true);
}

async function refreshData() {
  st.data = await invoke('settings_data');
  if (st.section === 'accounts' && !st.query && !act.dragging) draw(false);
  else if (st.section === 'privacy' && !st.query) draw(false);
}

// ------------------------------------------------------------ start

async function start() {
  const [boot, data] = await Promise.all([invoke('bootstrap'), invoke('settings_data')]);
  st.strings = boot.strings;
  st.lang = boot.lang;
  st.data = data;
  st.cfg = boot.config;
  setAppearance(boot.config.appearance, () => { if (st.cfg) sync(); });
  build();

  $('search').addEventListener('input', (e) => { st.query = e.target.value; draw(false); });
  $('win-min').addEventListener('click', () => invoke('settings_window', { action: 'minimize' }));
  $('win-max').addEventListener('click', () => invoke('settings_window', { action: 'maximize' }));
  $('win-close').addEventListener('click', () => invoke('settings_window', { action: 'close' }));
  $('titlebar').addEventListener('dblclick', (e) => { if (!e.target.closest('button')) invoke('settings_window', { action: 'maximize' }); });
  $('preview').addEventListener('pointerenter', () => { st.previewHover = true; sync(); });
  $('preview').addEventListener('pointerleave', () => { st.previewHover = false; sync(); });
  window.addEventListener('keydown', (e) => {
    if (e.key === 'Escape' && !$('scrim').hidden) $('scrim').hidden = true;
    if (e.key === 'f' && e.ctrlKey) { e.preventDefault(); $('search').focus(); }
  });
  window.addEventListener('contextmenu', (e) => { if (!e.target.closest('input')) e.preventDefault(); });

  listen('settings-changed', applyConfig);
  listen('strings-changed', (s) => { st.strings = s.strings; st.lang = s.lang; build(); });
  listen('settings-section', go);
  listen('usage', refreshData);       // statuses and "updated … ago" follow each refresh
  listen('monitors-changed', refreshData);

  // Show the window only now that the first frame is ready.
  requestAnimationFrame(() => invoke('settings_window', { action: 'ready' }));
}

start();
