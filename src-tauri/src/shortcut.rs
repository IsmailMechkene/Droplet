//! The global shortcut that shows the usage card.
//! Registration fails when Windows or another app already owns the combination;
//! that failure is reported to the settings window instead of being saved.

use std::str::FromStr;
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut};

use crate::providers::ProviderId;
use crate::state::{main_limit, Core};
use crate::{notch, settings_window};

/// "Win+Shift+U" → a shortcut. `None` for empty or unknown text.
pub fn parse(text: &str) -> Option<Shortcut> {
    let mut modifiers = Modifiers::empty();
    let mut code = None;
    for part in text.split('+') {
        match part {
            "Win" => modifiers |= Modifiers::SUPER,
            "Ctrl" => modifiers |= Modifiers::CONTROL,
            "Alt" => modifiers |= Modifiers::ALT,
            "Shift" => modifiers |= Modifiers::SHIFT,
            key => code = Some(key_code(key)?),
        }
    }
    if modifiers.is_empty() {
        return None;
    }
    Some(Shortcut::new(Some(modifiers), code?))
}

/// "U" → KeyU, "7" → Digit7, "F5" / "Space" / "Home" by name.
fn key_code(key: &str) -> Option<Code> {
    let mut chars = key.chars();
    let name = match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphabetic() => format!("Key{}", c.to_ascii_uppercase()),
        (Some(c), None) if c.is_ascii_digit() => format!("Digit{c}"),
        _ => key.to_string(),
    };
    Code::from_str(&name).ok()
}

/// Swaps the registered shortcut. On failure the previous one is restored and
/// an error key for the UI is returned.
pub fn replace(app: &AppHandle, old: &str, new: &str) -> Result<(), String> {
    let manager = app.global_shortcut();
    if let Some(previous) = parse(old) {
        let _ = manager.unregister(previous);
    }
    if new.is_empty() {
        return Ok(());
    }
    let Some(shortcut) = parse(new) else {
        return Err("s.general.shortcut.need_modifier".into());
    };
    if manager.register(shortcut).is_err() {
        if let Some(previous) = parse(old) {
            let _ = manager.register(previous);
        }
        return Err("s.general.shortcut.taken".into());
    }
    Ok(())
}

/// The enabled provider with a reading that is closest to its limit.
fn closest_to_limit(core: &Core) -> Option<ProviderId> {
    let config = core.config();
    let providers = core.providers.lock().unwrap();
    config
        .providers
        .order
        .iter()
        .filter(|id| config.providers.get(**id).enabled)
        .filter_map(|id| {
            let usage = providers.get(id)?.usage.as_ref()?;
            Some((*id, main_limit(usage, &config.providers.get(*id).main_limit)?.used_pct))
        })
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(id, _)| id)
}

/// Shortcut pressed: open the card of the tool closest to its limit. With no
/// notch (hidden), open settings instead so the app stays reachable.
pub fn pressed(app: &AppHandle) {
    let core = app.state::<Arc<Core>>();
    if notch::window(app).is_none() {
        settings_window::open(app, None);
        return;
    }
    let id = closest_to_limit(&core).or_else(|| core.config().providers.order.first().copied());
    if let Some(id) = id {
        notch::focus(app);
        let _ = app.emit_to(notch::LABEL, "notch-command", serde_json::json!({ "open_card": id.key() }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_shortcuts() {
        let s = parse("Win+Shift+U").unwrap();
        assert_eq!(s.mods, Modifiers::SUPER | Modifiers::SHIFT);
        assert_eq!(s.key, Code::KeyU);
        assert_eq!(parse("Ctrl+Alt+7").unwrap().key, Code::Digit7);
        assert_eq!(parse("Ctrl+F5").unwrap().key, Code::F5);
        assert_eq!(parse("Alt+Space").unwrap().key, Code::Space);
        assert_eq!(parse("Ctrl+u").unwrap().key, Code::KeyU);
    }

    #[test]
    fn rejects_incomplete_shortcuts() {
        assert!(parse("").is_none());
        assert!(parse("U").is_none(), "a modifier is required");
        assert!(parse("Ctrl+Shift").is_none(), "a key is required");
        assert!(parse("Ctrl+NotAKey").is_none());
    }
}
