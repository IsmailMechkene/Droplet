//! Every command a UI may call. UIs get finished data back; no command returns
//! a credential or the contents of another app's file.

use std::collections::HashMap;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, State};

use crate::clock;
use crate::config::Config;
use crate::notch::dock::{Placement, Shape};
use crate::providers::ProviderId;
use crate::scheduler::{Msg, Reason};
use crate::state::{Core, Snapshot};
use crate::{notch, settings};

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
