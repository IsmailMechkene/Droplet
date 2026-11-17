//! Every command a UI may call. UIs get finished data back; no command returns
//! a credential or the contents of another app's file.

use std::collections::HashMap;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;

use crate::clock;
use crate::config::Config;
use crate::notch::dock::{Placement, Shape};
use crate::providers::ProviderId;
use crate::scheduler::{Msg, Reason};
use crate::state::{Core, Snapshot};
use crate::accounts::{self, AccountView, FileView};
use crate::notch::monitors::{self, MonitorChoice};
use crate::win::shell::{self, ProjectPage};
use crate::{notch, providers, settings, settings_window};

/// Everything a window needs to draw its first frame.
#[derive(Serialize)]
pub struct Bootstrap {
    config: Config,
    snapshot: Snapshot,
    /// UI strings in the active language.
    strings: HashMap<String, String>,
    lang: &'static str,
    placement: Option<Placement>,
}

#[tauri::command]
pub fn bootstrap(app: AppHandle, core: State<Arc<Core>>) -> Bootstrap {
    crate::trace!("bootstrap");
    let i18n = core.i18n.read().unwrap();
    Bootstrap {
        config: core.config(),
        snapshot: core.snapshot(clock::now()),
        strings: i18n.all(),
        lang: i18n.lang,
        placement: notch::placement(&app),
    }
}

/// A fresh snapshot (the UI asks when a card opens, so ages like "2 min ago" are current).
#[tauri::command]
pub fn get_snapshot(core: State<Arc<Core>>) -> Snapshot {
    core.snapshot(clock::now())
}

#[tauri::command]
pub fn set_setting(app: AppHandle, path: String, value: Value) -> Result<(), String> {
    settings::change(&app, &path, value)
}

/// Refreshes one provider, or all of them when `id` is omitted.
#[tauri::command]
pub fn refresh(core: State<Arc<Core>>, id: Option<String>) {
    match id.as_deref().and_then(ProviderId::from_key) {
        Some(id) => core.send(Msg::RefreshOne(id, Reason::Manual)),
        None => core.send(Msg::RefreshAll(Reason::Manual)),
    }
}

/// A card was opened: send a snapshot with current ages ("2 min ago").
#[tauri::command]
pub fn card_opened(core: State<Arc<Core>>, id: String) {
    if let Some(id) = ProviderId::from_key(&id) {
        core.send(Msg::RefreshOne(id, Reason::CardOpen));
    }
}

/// The notch UI asks for a window shape and learns where it is docked.
#[tauri::command]
pub fn notch_shape(app: AppHandle, shape: Shape) -> Option<Placement> {
    notch::set_shape(&app, shape)
}

#[tauri::command]
pub fn quit(app: AppHandle) {
    app.exit(0);
}

// ---------------------------------------------------------------- notch

#[tauri::command]
pub fn notch_menu(app: AppHandle, keep_open: bool, jiggle: bool) {
    notch::popup_menu(&app, keep_open, jiggle);
}

/// Escape in the notch: give up keyboard focus again.
#[tauri::command]
pub fn notch_blur(app: AppHandle) {
    notch::blur(&app);
}

/// Moves the notch back to the middle of its edge.
#[tauri::command]
pub fn notch_recentre(app: AppHandle, core: State<Arc<Core>>) -> Result<(), String> {
    if notch::window(&app).is_some() {
        notch::drag::recentre(&app); // springs there, then saves
        return Ok(());
    }
    let old = core.config();
    let mut new = old.clone();
    new.notch.position.set(old.notch.edge, 0.5);
    settings::commit(&app, old, new)
}

// ---------------------------------------------------------------- settings window

/// Everything the settings window needs besides `bootstrap`.
#[derive(Serialize)]
pub struct SettingsData {
    accounts: Vec<AccountView>,
    files: Vec<FileView>,
    monitors: Vec<MonitorChoice>,
    version: &'static str,
    config_path: String,
}

#[tauri::command]
pub fn open_settings(app: AppHandle, section: Option<String>) {
    settings_window::open(&app, section.as_deref());
}

#[tauri::command]
pub fn settings_data(core: State<Arc<Core>>) -> SettingsData {
    crate::trace!("settings_data");
    SettingsData {
        accounts: accounts::accounts(&core, clock::now()),
        files: accounts::files(&core),
        monitors: monitors::choices(),
        version: env!("CARGO_PKG_VERSION"),
        config_path: "%APPDATA%\\Droplet\\config.json".into(),
    }
}

/// Window controls for the custom title bar: ready | minimize | maximize | close.
#[tauri::command]
pub fn settings_window(app: AppHandle, action: String) {
    match action.as_str() {
        "ready" => settings_window::show(&app),
        "minimize" => settings_window::minimize(&app),
        "maximize" => settings_window::toggle_maximize(&app),
        "close" => settings_window::close(&app),
        _ => {}
    }
}

/// Selects a provider's credential file in Explorer. The file is not read.
#[tauri::command]
pub fn show_file_location(core: State<Arc<Core>>, id: String) -> bool {
    let Some(id) = ProviderId::from_key(&id) else { return false };
    providers::files(id, &core.config()).first().is_some_and(|path| shell::reveal(path))
}

/// Opens one of this project's GitHub pages: releases | license | source.
#[tauri::command]
pub fn open_project_page(page: String) -> bool {
    ProjectPage::from_key(&page).is_some_and(shell::open_project_page)
}

/// Opens Droplet's own config file with the default editor.
#[tauri::command]
pub fn open_config_file(core: State<Arc<Core>>) -> bool {
    let config = core.config();
    if !core.dir.join(crate::config::FILE_NAME).exists() {
        let _ = crate::config::save(&core.dir, &config);
    }
    shell::open_own_file(&core.dir, crate::config::FILE_NAME)
}

/// File picker for agy.exe. The chosen path is saved as the Antigravity override.
#[tauri::command]
pub async fn pick_agy_path(app: AppHandle) -> Result<(), String> {
    let picked = app.dialog().file().add_filter("agy.exe", &["exe"]).blocking_pick_file();
    let Some(path) = picked.and_then(|p| p.into_path().ok()) else { return Ok(()) };
    settings::change(&app, "providers.antigravity.agy_path", path.display().to_string().into())
}

/// "Preview" next to Spring feel: gives the real notch a little bounce.
#[tauri::command]
pub fn notch_preview(app: AppHandle) {
    let _ = app.emit_to(notch::LABEL, "notch-command", "kick");
}

/// The grip was pressed: Rust follows the cursor until the button is released.
#[tauri::command]
pub fn notch_drag_start(app: AppHandle) {
    notch::drag::start(&app);
}

/// Writes network-log.txt (with the audit list) and opens it in the default editor.
#[tauri::command]
pub fn open_network_log(core: State<Arc<Core>>) -> bool {
    crate::netlog::open(&core.dir, &core.config(), &core.net.log_snapshot())
}

/// Writes the history as CSV to Downloads and selects it in Explorer. False when there is nothing to export.
#[tauri::command]
pub fn export_history(core: State<Arc<Core>>) -> bool {
    let rows = match core.history.lock().unwrap().as_ref() {
        Some(history) => history.all_rows(),
        None => return false,
    };
    let Some(downloads) = crate::win::downloads_dir().filter(|_| !rows.is_empty()) else { return false };
    let offset = crate::win::utc_offset();
    let path = crate::history::export_path(&downloads, clock::now(), offset);
    std::fs::write(&path, crate::history::to_csv(&rows, offset)).is_ok() && shell::reveal(&path)
}

/// Deletes all stored history (after the confirmation in Settings).
#[tauri::command]
pub fn clear_history(core: State<Arc<Core>>) {
    if let Some(history) = core.history.lock().unwrap().as_ref() {
        let _ = history.clear();
    }
    core.send(Msg::HistoryCleared);
}
