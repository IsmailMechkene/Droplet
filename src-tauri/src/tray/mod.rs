//! Tray icons, per the settings: one ring per provider, one for the provider
//! closest to its limit, or just the Droplet logo. "App icon: hidden" removes
//! them all. Icons are redrawn only when their picture or text changes.

pub mod draw;
pub mod font;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tauri::image::Image;
use tauri::menu::{Menu, MenuBuilder, MenuItemBuilder, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Wry};
use windows::core::w;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
use windows::Win32::UI::WindowsAndMessaging::SM_CXSMICON;

use crate::config::{AppIcon, Config, TrayIcons};
use crate::i18n::I18n;
use crate::state::{Core, ProviderView, Snapshot};
use crate::{notch, shortcut};
use draw::RingIcon;

const LOGO_ID: &str = "logo";
/// Windows truncates tray tooltips at 127 characters.
const TOOLTIP_MAX: usize = 127;

/// What one tray icon should show.
#[derive(Debug, Clone, PartialEq)]
struct Want {
    id: String,
    /// `None` = the app logo.
    picture: Option<(Vec<u8>, u32)>,
    tooltip: String,
}

#[derive(Default)]
pub struct Tray {
    /// id → what is currently shown, to skip redundant updates.
    shown: Mutex<HashMap<String, Want>>,
    /// The suggestion line the menus were built with.
    menu_line: Mutex<Option<String>>,
}

/// Icon size Windows wants for the tray at the current DPI.
fn icon_size() -> u32 {
    let px = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem()) };
    draw::size_for(px.max(16) as u32)
}

/// True when the taskbar uses the light theme (dark ink on the icon).
fn light_taskbar() -> bool {
    let mut value = 0u32;
    let mut size = std::mem::size_of::<u32>() as u32;
    let read = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("SystemUsesLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some(&mut value as *mut u32 as *mut _),
            Some(&mut size),
        )
    };
    read.is_ok() && value == 1
}

fn truncate(text: String) -> String {
    if text.chars().count() <= TOOLTIP_MAX {
        return text;
    }
    text.chars().take(TOOLTIP_MAX - 1).collect::<String>() + "…"
}

/// "Claude 64% · resets 1h12m · out at 16:20"
pub fn tooltip(view: &ProviderView, i18n: &I18n, now: i64) -> String {
    let mut text = i18n.f("tray.tip", &[("name", view.name), ("pct", &format!("{:.0}", view.pct.round()))]);
    if view.ring == "stale" {
        text += &i18n.t("tray.tip_stale");
    } else if let Some(reset) = view.main_resets_at {
        text += &i18n.f("tray.tip_resets", &[("time", &i18n.duration_compact(reset - now))]);
    }
    if let Some(at) = &view.out_at {
        text += &i18n.f("tray.tip_out", &[("at", at)]);
    }
    truncate(text)
}

/// Which icons should exist right now.
fn wanted(config: &Config, snapshot: &Snapshot, i18n: &I18n, now: i64) -> Vec<Want> {
    if config.general.app_icon == AppIcon::Hidden {
        return Vec::new();
    }
    let logo = || Want { id: LOGO_ID.into(), picture: None, tooltip: i18n.t("tray.tip_idle") };
    let with_reading: Vec<&ProviderView> =
        snapshot.providers.iter().filter(|v| v.enabled && matches!(v.card, "ok" | "stale")).collect();
    let chosen: Vec<&ProviderView> = match config.general.tray_icons {
        TrayIcons::None => Vec::new(),
        TrayIcons::PerProvider => with_reading,
        TrayIcons::Closest => with_reading.into_iter().max_by(|a, b| a.pct.total_cmp(&b.pct)).into_iter().collect(),
    };
    if chosen.is_empty() {
        return vec![logo()];
    }
    let (size, light) = (icon_size(), light_taskbar());
    chosen
        .into_iter()
        .map(|v| {
            let ring = RingIcon { pct: v.pct, pace: v.pace.filter(|_| config.appearance.pace_tick), stale: v.ring == "stale", light_taskbar: light };
            Want { id: v.id.to_string(), picture: Some((draw::render(&ring, size, &config.appearance), size)), tooltip: tooltip(v, i18n, now) }
        })
        .collect()
}

/// Right-click menu: suggestion line · Refresh now · Settings… · Network log · Quit.
fn menu(app: &AppHandle, line: &str, i18n: &I18n) -> tauri::Result<Menu<Wry>> {
    MenuBuilder::new(app)
        .item(&MenuItemBuilder::with_id("suggestion", line).enabled(false).build(app)?)
        .item(&PredefinedMenuItem::separator(app)?)
        .item(&MenuItemBuilder::with_id("refresh", i18n.t("menu.refresh")).build(app)?)
        .item(&MenuItemBuilder::with_id("settings", i18n.t("menu.settings")).build(app)?)
        .item(&MenuItemBuilder::with_id("netlog", i18n.t("menu.netlog")).build(app)?)
        .item(&PredefinedMenuItem::separator(app)?)
        .item(&MenuItemBuilder::with_id("quit", i18n.t("menu.quit")).build(app)?)
        .build()
}

/// Left click: open that provider's card in the notch (or settings when there is no notch).
fn clicked(app: &AppHandle, id: &str) {
    if notch::window(app).is_none() || id == LOGO_ID {
        shortcut::pressed(app);
        return;
    }
    notch::focus(app);
    let _ = app.emit_to(notch::LABEL, "notch-command", serde_json::json!({ "open_card": id }));
}

fn image(want: &Want, app: &AppHandle) -> Option<Image<'static>> {
    match &want.picture {
        Some((rgba, size)) => Some(Image::new_owned(rgba.clone(), *size, *size)),
        None => app.default_window_icon().map(|i| i.clone().to_owned()),
    }
}

/// Brings the tray in line with the settings and the latest snapshot.
pub fn update(app: &AppHandle, snapshot: &Snapshot) {
    let core = app.state::<Arc<Core>>();
    let config = core.config();
    let tray = app.state::<Tray>();
    let i18n = core.i18n.read().unwrap();
    let now = crate::clock::now();
    let wants = wanted(&config, snapshot, &i18n, now);
    let line = snapshot.suggestion.as_ref().map(|s| s.line.clone()).unwrap_or_else(|| i18n.t("suggest.none"));

    let mut shown = tray.shown.lock().unwrap();
    let mut menu_line = tray.menu_line.lock().unwrap();
    let menu_changed = menu_line.as_deref() != Some(line.as_str());

    // Remove icons that are no longer wanted.
    for id in shown.keys().filter(|id| !wants.iter().any(|w| &w.id == *id)).cloned().collect::<Vec<_>>() {
        let _ = app.remove_tray_by_id(id.as_str());
        shown.remove(&id);
    }
    for want in wants {
        let current = shown.get(&want.id);
        match app.tray_by_id(want.id.as_str()) {
            Some(icon) => {
                if current.map(|c| &c.picture) != Some(&want.picture) {
                    let _ = icon.set_icon(image(&want, app));
                }
                if current.map(|c| &c.tooltip) != Some(&want.tooltip) {
                    let _ = icon.set_tooltip(Some(&want.tooltip));
                }
                if menu_changed {
                    if let Ok(m) = menu(app, &line, &i18n) {
                        let _ = icon.set_menu(Some(m));
                    }
                }
            }
            None => {
                let Ok(m) = menu(app, &line, &i18n) else { continue };
                let mut builder = TrayIconBuilder::with_id(want.id.as_str()).tooltip(&want.tooltip).menu(&m).show_menu_on_left_click(false).on_tray_icon_event(
                    |icon, event| {
                        if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                            clicked(icon.app_handle(), icon.id().as_ref());
                        }
                    },
                );
                if let Some(img) = image(&want, app) {
                    builder = builder.icon(img);
                }
                if builder.build(app).is_err() {
                    continue;
                }
            }
        }
        shown.insert(want.id.clone(), want);
    }
    *menu_line = Some(line);
}

/// Makes the next `update` redraw every icon, for example after a DPI or taskbar theme change.
pub fn invalidate(app: &AppHandle) {
    for want in app.state::<Tray>().shown.lock().unwrap().values_mut() {
        want.picture = Some((Vec::new(), 0));
        want.tooltip.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Language;

    fn view(pct: f64) -> ProviderView {
        ProviderView {
            id: "claude",
            name: "Claude",
            ring: "ok",
            card: "ok",
            enabled: true,
            loading: false,
            pct,
            weekly: None,
            pace: None,
            period: 0,
            plan: None,
            title: String::new(),
            rows: Vec::new(),
            forecast: None,
            message: None,
            spark: Vec::new(),
            footer: String::new(),
            label: String::new(),
            out_at: None,
            main_resets_at: None,
        }
    }

    #[test]
    fn tooltip_text() {
        let i18n = I18n::new(Language::En, "en-US");
        let now = 1_794_841_445;
        let mut v = view(64.4);
        v.main_resets_at = Some(now + 3600 + 12 * 60);
        v.out_at = Some("16:20".into());
        assert_eq!(tooltip(&v, &i18n, now), "Claude 64% · resets 1h12m · out at 16:20");
        v.ring = "stale";
        v.out_at = None;
        assert_eq!(tooltip(&v, &i18n, now), "Claude 64% · stale");
    }

    #[test]
    fn tooltip_is_capped() {
        let long = truncate("x".repeat(300));
        assert_eq!(long.chars().count(), TOOLTIP_MAX);
        assert!(long.ends_with('…'));
        assert_eq!(truncate("short".into()), "short");
    }

    #[test]
    fn icon_choice_follows_settings() {
        let i18n = I18n::new(Language::En, "en-US");
        let mut a = view(40.0);
        let mut b = view(80.0);
        a.id = "codex";
        b.id = "claude";
        let snapshot = Snapshot { providers: vec![a, b], suggestion: None };
        let mut config = Config::default();
        let ids = |c: &Config| wanted(c, &snapshot, &i18n, 0).into_iter().map(|w| w.id).collect::<Vec<_>>();

        config.general.tray_icons = TrayIcons::Closest;
        assert_eq!(ids(&config), ["claude"]);
        config.general.tray_icons = TrayIcons::PerProvider;
        assert_eq!(ids(&config), ["codex", "claude"]);
        config.general.tray_icons = TrayIcons::None;
        assert_eq!(ids(&config), [LOGO_ID]);
        config.general.app_icon = AppIcon::Hidden;
        assert!(ids(&config).is_empty());
        config.general.app_icon = AppIcon::Tray;
        config.general.tray_icons = TrayIcons::Closest;
        assert_eq!(wanted(&config, &Snapshot::default(), &i18n, 0)[0].id, LOGO_ID, "no readings yet: the logo");
    }
}
