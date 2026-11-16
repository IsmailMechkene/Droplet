//! Range and consistency checks that serde cannot express.

use super::schema::Config;
use crate::providers::ProviderId;

fn one_of(name: &str, value: u32, allowed: &[u32]) -> Result<(), String> {
    if allowed.contains(&value) {
        Ok(())
    } else {
        Err(format!("{name} must be one of {allowed:?}, got {value}"))
    }
}

fn in_range(name: &str, value: u32, min: u32, max: u32) -> Result<(), String> {
    if (min..=max).contains(&value) {
        Ok(())
    } else {
        Err(format!("{name} must be between {min} and {max}, got {value}"))
    }
}

/// "HH:MM" → minutes since midnight.
pub fn parse_hhmm(text: &str) -> Option<u32> {
    let (h, m) = text.split_once(':')?;
    if h.len() != 2 || m.len() != 2 {
        return None;
    }
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h < 24 && m < 60).then_some(h * 60 + m)
}

/// Shortcut text is "Mod+Mod+Key": at least one modifier and exactly one key.
pub fn shortcut_is_well_formed(text: &str) -> bool {
    if text.is_empty() {
        return true;
    }
    let is_mod = |p: &str| matches!(p, "Win" | "Ctrl" | "Alt" | "Shift");
    let parts: Vec<&str> = text.split('+').collect();
    let mods = parts.iter().filter(|p| is_mod(p)).count();
    let keys: Vec<&str> = parts.iter().copied().filter(|p| !is_mod(p)).collect();
    mods >= 1 && keys.len() == 1 && !keys[0].is_empty() && keys[0].len() <= 12 && keys[0].chars().all(|c| c.is_ascii_alphanumeric())
}

/// Window ids each provider can use as its main limit. Antigravity's groups
/// come from agy.exe output, so any short id is accepted there.
pub fn main_limit_options(id: ProviderId) -> &'static [&'static str] {
    match id {
        ProviderId::Claude => &["session", "weekly"],
        ProviderId::Codex => &["primary", "secondary"],
        ProviderId::Cursor => &["total"],
        ProviderId::Opencode => &["rolling", "weekly", "monthly"],
        ProviderId::Kimi => &["5h", "7d", "month_total"],
        ProviderId::Antigravity => &[],
    }
}

pub fn validate(c: &Config) -> Result<(), String> {
    one_of("general.refresh_minutes", c.general.refresh_minutes, &[3, 5, 10, 15])?;
    if !shortcut_is_well_formed(&c.general.shortcut) {
        return Err("general.shortcut must look like Ctrl+Shift+U".into());
    }

    let order = &c.providers.order;
    let complete = order.len() == ProviderId::ALL.len() && ProviderId::ALL.iter().all(|id| order.contains(id));
    if !complete {
        return Err("providers.order must list every provider exactly once".into());
    }
    for id in ProviderId::ALL {
        let p = c.providers.get(id);
        let options = main_limit_options(id);
        let ok = if options.is_empty() {
            !p.main_limit.is_empty() && p.main_limit.len() <= 64
        } else {
            options.contains(&p.main_limit.as_str())
        };
        if !ok {
            return Err(format!("providers.{}.main_limit is not a known limit", id.key()));
        }
        if p.agy_path.len() > 520 {
            return Err("agy_path is too long".into());
        }
        if id != ProviderId::Antigravity && !p.agy_path.is_empty() {
            return Err("agy_path only applies to antigravity".into());
        }
    }

    in_range("appearance.warn_at", c.appearance.warn_at, 40, 95)?;
    in_range("appearance.crit_at", c.appearance.crit_at, 50, 99)?;
    if c.appearance.crit_at <= c.appearance.warn_at {
        return Err("appearance.crit_at must be greater than appearance.warn_at".into());
    }

    let pos = &c.notch.position;
    for (name, v) in [("left", pos.left), ("right", pos.right), ("top", pos.top), ("bottom", pos.bottom)] {
        if !(0.0..=1.0).contains(&v) {
            return Err(format!("notch.position.{name} must be between 0 and 1"));
        }
    }
    one_of("notch.collapse_after_min", c.notch.collapse_after_min, &[0, 1, 5, 15])?;
    in_range("notch.hover_delay_ms", c.notch.hover_delay_ms, 0, 600)?;
    if c.notch.monitor_id.len() > 400 {
        return Err("notch.monitor_id is too long".into());
    }

    one_of("alerts.run_out_lead_min", c.alerts.run_out_lead_min, &[15, 30, 60])?;
    if parse_hhmm(&c.alerts.quiet.from).is_none() || parse_hhmm(&c.alerts.quiet.to).is_none() {
        return Err("alerts.quiet times must be HH:MM".into());
    }

    one_of("forecast.pace_window_min", c.forecast.pace_window_min, &[30, 60, 120])?;
    in_range("forecast.hysteresis_pts", c.forecast.hysteresis_pts, 5, 40)?;
    one_of("history.retention_days", c.history.retention_days, &[0, 7, 30, 90])?;
    Ok(())
}
