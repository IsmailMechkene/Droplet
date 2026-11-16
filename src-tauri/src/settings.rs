//! Applying a settings change: validate → save atomically → broadcast → apply effects.

use std::sync::Arc;

use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

use crate::config::{self, Config};
use crate::i18n::I18n;
use crate::scheduler::Msg;
use crate::state::Core;
use crate::{notch, win};

/// Changes one setting by its dotted path. Nothing is changed if validation fails.
pub fn change(app: &AppHandle, path: &str, value: Value) -> Result<(), String> {
    let core = app.state::<Arc<Core>>();
    let old = core.config();
    let new = config::with_change(&old, path, value)?;
    commit(app, old, new)
}

/// Replaces the whole config (already validated by the caller's construction).
pub fn commit(app: &AppHandle, old: Config, new: Config) -> Result<(), String> {
    if old == new {
        return Ok(());
    }
    config::validate::validate(&new)?;
    let core = app.state::<Arc<Core>>();
    config::save(&core.dir, &new)?;
    *core.config.write().unwrap() = new.clone();

    if old.general.language != new.general.language {
        *core.i18n.write().unwrap() = I18n::new(new.general.language, &win::system_locale());
    }
    let _ = app.emit("settings-changed", &new);
    notch::on_config_changed(app, &old, &new);
    core.send(Msg::ConfigChanged);
    Ok(())
}
