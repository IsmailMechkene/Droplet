//! Settings: Rust is the single owner. Load → migrate → validate; every change
//! is validated as a whole config and written atomically.

pub mod migrate;
pub mod schema;
pub mod validate;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde_json::Value;

pub use schema::*;

pub const FILE_NAME: &str = "config.json";
pub const BAD_FILE_NAME: &str = "config.bad.json";

/// Parses, migrates and validates config text.
pub fn parse(text: &str) -> Result<Config, String> {
    let raw: Value = serde_json::from_str(text).map_err(|e| format!("not valid JSON: {e}"))?;
    let raw = migrate::migrate(raw)?;
    let config: Config = serde_json::from_value(raw).map_err(|e| e.to_string())?;
    validate::validate(&config)?;
    Ok(config)
}

/// Loads the config. A missing file gives defaults; an invalid or corrupt one
/// is kept as `config.bad.json` and replaced by defaults.
pub fn load(dir: &Path) -> Config {
    let path = dir.join(FILE_NAME);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => return Config::default(),
    };
    match parse(&text) {
        Ok(config) => config,
        Err(_) => {
            let _ = fs::rename(&path, dir.join(BAD_FILE_NAME));
            Config::default()
        }
    }
}

/// Writes to a temp file in the same folder, then renames it over the target.
pub fn save(dir: &Path, config: &Config) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    let tmp: PathBuf = dir.join(format!("{FILE_NAME}.tmp"));
    let mut file = fs::File::create(&tmp).map_err(|e| e.to_string())?;
    file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    fs::rename(&tmp, dir.join(FILE_NAME)).map_err(|e| e.to_string())
}

/// Returns a copy of `config` with the value at a dotted `path` replaced
/// (for example "appearance.warn_at"). The result is fully re-validated, so a
/// wrong type, unknown key, bad enum or out-of-range number is rejected.
pub fn with_change(config: &Config, path: &str, value: Value) -> Result<Config, String> {
    let mut raw = serde_json::to_value(config).map_err(|e| e.to_string())?;
    let mut slot = &mut raw;
    for key in path.split('.') {
        slot = slot
            .as_object_mut()
            .and_then(|map| map.get_mut(key))
            .ok_or_else(|| format!("unknown setting: {path}"))?;
    }
    if path == "version" {
        return Err("version cannot be changed".into());
    }
    *slot = value;
    let next: Config = serde_json::from_value(raw).map_err(|e| format!("{path}: {e}"))?;
    validate::validate(&next)?;
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn defaults_are_valid() {
        validate::validate(&Config::default()).unwrap();
    }

    #[test]
    fn empty_object_migrates_to_defaults() {
        assert_eq!(parse("{}").unwrap(), Config::default());
    }

    #[test]
    fn round_trips_through_json() {
        let text = serde_json::to_string(&Config::default()).unwrap();
        assert_eq!(parse(&text).unwrap(), Config::default());
    }

    #[test]
    fn v0_flat_keys_are_migrated() {
        let c = parse(r#"{"refresh_minutes": 10, "theme": "dark", "edge": "left"}"#).unwrap();
        assert_eq!(c.version, CURRENT_VERSION);
        assert_eq!(c.general.refresh_minutes, 10);
        assert_eq!(c.appearance.theme, Theme::Dark);
        assert_eq!(c.notch.edge, Edge::Left);
    }

    #[test]
    fn newer_version_is_rejected() {
        assert!(parse(r#"{"version": 99}"#).is_err());
    }

    #[test]
    fn corrupt_and_wrong_types_are_rejected() {
        assert!(parse("{not json").is_err());
        assert!(parse("[]").is_err());
        assert!(parse(r#"{"version":1,"general":{"refresh_minutes":"5"}}"#).is_err());
        assert!(parse(r#"{"version":1,"appearance":{"theme":"neon"}}"#).is_err());
        assert!(parse(r#"{"version":1,"surprise":true}"#).is_err());
    }

    #[test]
    fn ranges_are_enforced() {
        let c = Config::default();
        assert!(with_change(&c, "general.refresh_minutes", json!(4)).is_err());
        assert!(with_change(&c, "general.refresh_minutes", json!(3)).is_ok());
        assert!(with_change(&c, "notch.hover_delay_ms", json!(601)).is_err());
        assert!(with_change(&c, "forecast.hysteresis_pts", json!(4)).is_err());
        assert!(with_change(&c, "forecast.hysteresis_pts", json!(40)).is_ok());
        assert!(with_change(&c, "notch.position.left", json!(1.5)).is_err());
        assert!(with_change(&c, "alerts.quiet.from", json!("25:00")).is_err());
        assert!(with_change(&c, "history.retention_days", json!(14)).is_err());
    }

    #[test]
    fn critical_must_stay_above_warning() {
        let c = Config::default();
        assert!(with_change(&c, "appearance.warn_at", json!(90)).is_err());
        assert!(with_change(&c, "appearance.crit_at", json!(70)).is_err());
        assert!(with_change(&c, "appearance.warn_at", json!(89)).is_ok());
    }

    #[test]
    fn change_rejects_unknown_paths_and_types() {
        let c = Config::default();
        assert!(with_change(&c, "nope.key", json!(1)).is_err());
        assert!(with_change(&c, "appearance.nope", json!(1)).is_err());
        assert!(with_change(&c, "appearance.theme", json!(3)).is_err());
        assert!(with_change(&c, "version", json!(2)).is_err());
        assert_eq!(with_change(&c, "appearance.theme", json!("dark")).unwrap().appearance.theme, Theme::Dark);
    }

    #[test]
    fn provider_rules() {
        let c = Config::default();
        assert!(with_change(&c, "providers.claude.main_limit", json!("weekly")).is_ok());
        assert!(with_change(&c, "providers.claude.main_limit", json!("rolling")).is_err());
        assert!(with_change(&c, "providers.order", json!(["claude", "codex"])).is_err());
        assert!(with_change(&c, "providers.order", json!(["codex", "claude", "cursor", "opencode", "kimi", "antigravity"])).is_ok());
        assert!(with_change(&c, "providers.codex.agy_path", json!("C:\\x.exe")).is_err());
    }

    #[test]
    fn shortcut_format() {
        assert!(validate::shortcut_is_well_formed(""));
        assert!(validate::shortcut_is_well_formed("Win+Shift+U"));
        assert!(validate::shortcut_is_well_formed("Ctrl+F9"));
        assert!(!validate::shortcut_is_well_formed("U"));
        assert!(!validate::shortcut_is_well_formed("Ctrl+Shift"));
        assert!(!validate::shortcut_is_well_formed("Ctrl+A+B"));
    }

    #[test]
    fn bad_file_is_backed_up_and_defaults_used() {
        let dir = std::env::temp_dir().join(format!("droplet-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(FILE_NAME), "{broken").unwrap();
        assert_eq!(load(&dir), Config::default());
        assert!(dir.join(BAD_FILE_NAME).exists());
        assert!(!dir.join(FILE_NAME).exists());

        let mut changed = Config::default();
        changed.appearance.warn_at = 60;
        save(&dir, &changed).unwrap();
        assert_eq!(load(&dir), changed);
        assert!(!dir.join(format!("{FILE_NAME}.tmp")).exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
