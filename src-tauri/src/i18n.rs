//! Text for everything Rust words itself (cards, tooltips, menus, alerts).
//! The same JSON files in `ui/locales` are loaded by the settings window.

use std::collections::HashMap;

use crate::clock;
use crate::config::Language;

const EN: &str = include_str!("../../ui/locales/en.json");
const FR: &str = include_str!("../../ui/locales/fr.json");

pub struct I18n {
    pub lang: &'static str,
    strings: HashMap<String, String>,
    fallback: HashMap<String, String>,
}

fn load(text: &str) -> HashMap<String, String> {
    serde_json::from_str(text).unwrap_or_default()
}

impl I18n {
    /// `system_locale` is a tag like "fr-FR", used when the setting is "follow system".
    pub fn new(setting: Language, system_locale: &str) -> I18n {
        let lang = match setting {
            Language::En => "en",
            Language::Fr => "fr",
            Language::System if system_locale.to_ascii_lowercase().starts_with("fr") => "fr",
            Language::System => "en",
        };
        let strings = load(if lang == "fr" { FR } else { EN });
        I18n { lang, strings, fallback: load(EN) }
    }

    /// The text for `key`. Unknown keys come back unchanged, which is also how
    /// literal names (Antigravity model groups) pass through.
    pub fn t(&self, key: &str) -> String {
        self.strings.get(key).or_else(|| self.fallback.get(key)).cloned().unwrap_or_else(|| key.to_string())
    }

    /// `t` with `{name}` placeholders filled in.
    pub fn f(&self, key: &str, args: &[(&str, &str)]) -> String {
        let mut text = self.t(key);
        for (name, value) in args {
            text = text.replace(&format!("{{{name}}}"), value);
        }
        text
    }

    /// Every string of the active language (English fills any gaps), for the UIs.
    pub fn all(&self) -> HashMap<String, String> {
        let mut all = self.fallback.clone();
        all.extend(self.strings.clone());
        all
    }

    /// "3d 6h", "2h 14m", "12 min".
    pub fn duration(&self, secs: i64) -> String {
        self.duration_with("dur", secs)
    }

    /// "3d6h", "1h12m", "12m" — for the tray tooltip, which is limited to 127 characters.
    pub fn duration_compact(&self, secs: i64) -> String {
        self.duration_with("dur.compact", secs)
    }

    fn duration_with(&self, prefix: &str, secs: i64) -> String {
        let minutes = (secs.max(0) + 30) / 60;
        let (d, h, m) = (minutes / 1440, minutes % 1440 / 60, minutes % 60);
        if d > 0 {
            self.f(&format!("{prefix}.dh"), &[("d", &d.to_string()), ("h", &h.to_string())])
        } else if h > 0 {
            self.f(&format!("{prefix}.hm"), &[("h", &h.to_string()), ("m", &format!("{m:02}"))])
        } else {
            self.f(&format!("{prefix}.m"), &[("m", &m.to_string())])
        }
    }

    /// "1 hour 12 minutes" — for screen readers.
    pub fn duration_long(&self, secs: i64) -> String {
        let minutes = (secs.max(0) + 30) / 60;
        let (d, h, m) = (minutes / 1440, minutes % 1440 / 60, minutes % 60);
        let unit = |name: &str, n: i64| {
            let form = if n == 1 { "one" } else { "other" };
            self.f(&format!("dur.long.{name}.{form}"), &[("n", &n.to_string())])
        };
        let parts: Vec<String> = if d > 0 {
            vec![unit("day", d), unit("hour", h)]
        } else if h > 0 {
            vec![unit("hour", h), unit("minute", m)]
        } else {
            vec![unit("minute", m)]
        };
        match parts.as_slice() {
            [a, b] => self.f("dur.long.join", &[("a", a), ("b", b)]),
            _ => parts.concat(),
        }
    }

    /// Local "16:20" for a Unix time.
    pub fn clock(&self, unix: i64) -> String {
        clock::hhmm(unix, crate::win::utc_offset())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn en() -> I18n {
        I18n::new(Language::En, "en-US")
    }

    #[test]
    fn every_locale_has_every_key() {
        let (en, fr) = (load(EN), load(FR));
        assert!(!en.is_empty() && !fr.is_empty(), "locale files must parse");
        let mut missing: Vec<&String> = en.keys().filter(|k| !fr.contains_key(*k)).collect();
        missing.sort();
        assert!(missing.is_empty(), "missing in fr.json: {missing:?}");
        let mut extra: Vec<&String> = fr.keys().filter(|k| !en.contains_key(*k)).collect();
        extra.sort();
        assert!(extra.is_empty(), "not in en.json: {extra:?}");
    }

    #[test]
    fn language_selection() {
        assert_eq!(I18n::new(Language::System, "fr-CA").lang, "fr");
        assert_eq!(I18n::new(Language::System, "de-DE").lang, "en");
        assert_eq!(I18n::new(Language::En, "fr-FR").lang, "en");
        assert_eq!(I18n::new(Language::Fr, "en-US").lang, "fr");
    }

    #[test]
    fn durations() {
        let i = en();
        assert_eq!(i.duration(2 * 3600 + 14 * 60), "2h 14m");
        assert_eq!(i.duration(4 * 3600 + 2 * 60), "4h 02m");
        assert_eq!(i.duration(3 * 86_400 + 6 * 3600), "3d 6h");
        assert_eq!(i.duration(12 * 60), "12 min");
        assert_eq!(i.duration(-5), "0 min");
        assert_eq!(i.duration_compact(3600 + 12 * 60), "1h12m");
        assert_eq!(i.duration_long(3600 + 12 * 60), "1 hour 12 minutes");
        assert_eq!(i.duration_long(60), "1 minute");
        assert_eq!(i.duration_long(2 * 86_400 + 3600), "2 days 1 hour");
    }

    #[test]
    fn placeholders_and_unknown_keys() {
        let i = en();
        assert_eq!(i.f("card.title", &[("name", "Claude")]), "Claude usage");
        assert_eq!(i.t("Gemini 3 Pro"), "Gemini 3 Pro");
    }
}
