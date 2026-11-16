//! Upgrades older config files, one version step at a time, on the raw JSON.

use serde_json::{json, Value};

use super::schema::CURRENT_VERSION;

/// Brings `raw` up to `CURRENT_VERSION`. Fails for files from a newer Droplet,
/// which we cannot safely interpret.
pub fn migrate(mut raw: Value) -> Result<Value, String> {
    if !raw.is_object() {
        return Err("config is not a JSON object".into());
    }
    let mut version = raw.get("version").and_then(Value::as_u64).unwrap_or(0) as u32;
    if version > CURRENT_VERSION {
        return Err(format!("config version {version} is newer than this app understands"));
    }
    while version < CURRENT_VERSION {
        raw = match version {
            0 => v0_to_v1(raw),
            _ => unreachable!("no migration from version {version}"),
        };
        version += 1;
        raw["version"] = json!(version);
    }
    Ok(raw)
}

/// Version 0 is any file without a "version" field. Flat top-level keys are
/// moved into the section they belong to now; everything else is kept as is.
fn v0_to_v1(mut old: Value) -> Value {
    const MOVES: [(&str, &str); 6] = [
        ("refresh_minutes", "general"),
        ("start_with_windows", "general"),
        ("theme", "appearance"),
        ("warn_at", "appearance"),
        ("crit_at", "appearance"),
        ("edge", "notch"),
    ];
    for (key, section) in MOVES {
        if let Some(value) = old.as_object_mut().and_then(|map| map.remove(key)) {
            if !old[section].is_object() {
                old[section] = json!({});
            }
            old[section][key] = value;
        }
    }
    old
}
