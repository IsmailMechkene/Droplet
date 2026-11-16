// What each settings section contains. A section is a list of groups (cards);
// a group is a list of rows built with the factories in controls.js.
// Accounts is drawn by accounts.js, and the privacy file list by settings.js.

import { toggle, seg, select, slider, buttons, hotkey, quietHours } from './controls.js';

export const SECTIONS = [
  { id: 'general', icon: 'settings' },
  { id: 'accounts', icon: 'person' },
  { id: 'appearance', icon: 'paint_brush' },
  { id: 'notch', icon: 'desktop' },
  { id: 'alerts', icon: 'alert' },
  { id: 'forecast', icon: 'data_trending' },
  { id: 'privacy', icon: 'shield' },
  { id: 'about', icon: 'info' },
];

/**
 * Builds the rows of every section. `ctx` is the settings context (see controls.js);
 * `act` holds the actions that are not plain setting changes.
 */
export function buildSections(ctx, act) {
  const t = ctx.t;
  const sub = (dep) => ({ sub: true, dep });

  const screenOptions = () => [
    { value: 'primary', label: t('s.notch.screen.opt.primary') },
    { value: 'follow_mouse', label: t('s.notch.screen.opt.follow_mouse') },
    ...act.monitors().map((m) => ({ value: 'monitor:' + m.id, label: t('s.notch.screen.display', { n: m.index, w: m.width, h: m.height }) })),
  ];
  const screenValue = (cfg) => {
    if (cfg.notch.screen !== 'monitor') return cfg.notch.screen;
    return 'monitor:' + cfg.notch.monitor_id;
  };
  const screenOptionsWithSaved = (cfg) => {
    const options = screenOptions(), value = screenValue(cfg);
    // A saved monitor that is unplugged stays listed, so the choice is not lost.
    if (!options.some((o) => o.value === value)) options.push({ value, label: t('s.notch.screen.missing') });
    return options;
  };

  return {
    general: [
      [
        toggle(ctx, 'general.start_with_windows'),
        seg(ctx, 'general.app_icon', ['tray', 'hidden']),
        select(ctx, 'general.language', { options: () => ['system', 'en', 'fr'].map((v) => ({ value: v, label: t(`s.general.language.opt.${v}`) })) }),
        seg(ctx, 'general.refresh_minutes', [3, 5, 10, 15]),
      ],
      [
        hotkey(ctx, 'general.shortcut'),
        seg(ctx, 'general.tray_icons', ['per_provider', 'closest', 'none']),
      ],
    ],

    appearance: [
      [
        seg(ctx, 'appearance.theme', ['system', 'light', 'dark']),
        seg(ctx, 'appearance.size', ['s', 'm', 'l']),
        toggle(ctx, 'appearance.show_remaining'),
        seg(ctx, 'appearance.weekly_ring', ['off', 'inside', 'outside']),
        toggle(ctx, 'appearance.pace_tick'),
      ],
      [
        seg(ctx, 'appearance.color_mode', ['hard', 'smooth']),
        // Critical must stay above warning: each slider stops one short of the other.
        slider(ctx, 'appearance.warn_at', { min: 40, max: 95, step: 1, unit: '%', limit: (cfg) => [40, Math.min(95, cfg.appearance.crit_at - 1)] }),
        slider(ctx, 'appearance.crit_at', { min: 50, max: 99, step: 1, unit: '%', limit: (cfg) => [Math.max(50, cfg.appearance.warn_at + 1), 99] }),
        buttons(ctx, { label: t('s.appearance.colors.label'), desc: t('s.appearance.colors.desc'), list: [{ label: t('s.appearance.colors.reset'), onClick: act.resetColors }] }),
      ],
      [
        seg(ctx, 'appearance.glyphs', ['mono', 'color']),
        seg(ctx, 'appearance.motion', ['system', 'full', 'reduced']),
        seg(ctx, 'appearance.spring_feel', ['calm', 'default', 'playful'], { buttons: [{ label: t('s.appearance.spring_feel.preview'), onClick: act.previewSpring }] }),
      ],
    ],

    notch: [
      [
        seg(ctx, 'notch.show', ['always', 'hover', 'hidden']),
        seg(ctx, 'notch.edge', ['left', 'right', 'top', 'bottom']),
        select(ctx, 'notch.screen', { options: screenOptionsWithSaved, read: screenValue, write: act.setScreen }),
      ],
      [
        buttons(ctx, { label: t('s.notch.position.label'), desc: t('s.notch.position.desc'), list: [{ label: t('s.notch.position.recentre'), onClick: act.recentre }] }),
        toggle(ctx, 'notch.locked'),
        toggle(ctx, 'notch.hide_fullscreen'),
      ],
      [
        seg(ctx, 'notch.collapse_after_min', [0, 1, 5, 15]),
        slider(ctx, 'notch.hover_delay_ms', { min: 0, max: 600, step: 10, unit: t('unit.ms') }),
        toggle(ctx, 'notch.high_contrast'),
      ],
    ],

    alerts: [
      [
        toggle(ctx, 'alerts.warning', { labelArgs: (cfg) => ({ pct: cfg.appearance.warn_at }), label: t('s.alerts.warning.label', { pct: '…' }) }),
        toggle(ctx, 'alerts.critical', { labelArgs: (cfg) => ({ pct: cfg.appearance.crit_at }), label: t('s.alerts.critical.label', { pct: '…' }) }),
      ],
      [
        toggle(ctx, 'alerts.run_out'),
        seg(ctx, 'alerts.run_out_lead_min', [15, 30, 60], sub('alerts.run_out')),
      ],
      [
        toggle(ctx, 'alerts.reset'),
        toggle(ctx, 'alerts.reset_soon'),
      ],
      [
        seg(ctx, 'alerts.show_as', ['peek', 'toast', 'both']),
        toggle(ctx, 'alerts.sound'),
        quietHours(ctx),
      ],
    ],

    forecast: [
      [
        toggle(ctx, 'forecast.show'),
        seg(ctx, 'forecast.pace_window_min', [30, 60, 120]),
      ],
      [
        toggle(ctx, 'forecast.suggest'),
        slider(ctx, 'forecast.hysteresis_pts', { min: 5, max: 40, step: 1, unit: t('unit.pts'), ...sub('forecast.suggest') }),
      ],
    ],

    privacy: [
      [
        seg(ctx, 'history.retention_days', [0, 7, 30, 90]),
        buttons(ctx, {
          label: t('s.history.data.label'), desc: t('s.history.data.desc'),
          list: [{ label: t('s.history.data.export'), onClick: act.exportCsv }, { label: t('s.history.data.clear'), onClick: act.clearHistory, danger: true }],
        }),
        toggle(ctx, 'history.weekly_summary'),
      ],
      [
        buttons(ctx, { label: t('s.history.netlog.label'), desc: t('s.history.netlog.desc'), list: [{ label: t('s.history.netlog.open'), onClick: act.openNetworkLog }] }),
      ],
    ],

    about: [
      [
        buttons(ctx, { label: t('s.about.version.label'), desc: t('s.about.version.desc'), info: 'Droplet ' + act.version(), list: [{ label: t('s.about.version.check'), onClick: () => act.openPage('releases'), external: true }] }),
        buttons(ctx, { label: t('s.about.license.label'), desc: t('s.about.license.desc'), info: 'MIT', list: [{ label: t('s.about.license.view'), onClick: () => act.openPage('license'), external: true }] }),
        buttons(ctx, { label: t('s.about.source.label'), desc: 'github.com/IsmailMechkene/Droplet', list: [{ label: t('s.about.source.open'), onClick: () => act.openPage('source'), external: true }] }),
      ],
      [
        buttons(ctx, { label: t('s.about.diagnostics.label'), desc: t('s.about.diagnostics.desc'), list: [{ label: t('s.about.diagnostics.copy'), onClick: act.copyDiagnostics }] }),
        buttons(ctx, { label: t('s.about.config.label'), desc: act.configPath(), list: [{ label: t('s.about.config.open'), onClick: act.openConfig }] }),
      ],
    ],
  };
}
