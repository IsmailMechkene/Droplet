//! Delivers alerts as a notch peek and/or a Windows notification, per
//! Settings › Alerts › Show as. Without a notch window, peeks fall back to
//! notifications so nothing is lost.

use std::sync::Arc;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;

use crate::alerts::Kind;
use crate::config::ShowAs;
use crate::i18n::I18n;
use crate::notch;
use crate::state::{Core, ProviderView};

/// What the notch peek shows: "Claude 81%" "· out at 16:20".
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Peek {
    pub id: String,
    pub a: String,
    pub b: String,
    /// A CSS colour token for the dot.
    pub dot: &'static str,
}

/// Words an alert for a provider. `a` is the headline, `b` the detail.
pub fn words(kind: Kind, view: &ProviderView, i18n: &I18n) -> (String, String, &'static str) {
    let pct = format!("{:.0}", view.pct.round());
    let level = i18n.f("alert.level", &[("name", view.name), ("pct", &pct)]);
    let out_at = view.out_at.as_ref().map(|at| i18n.f("alert.out_at", &[("at", at)]));
    match kind {
        Kind::Critical => (level, out_at.unwrap_or_else(|| i18n.t("alert.critical")), "var(--dp-danger)"),
        Kind::Warning => (level, out_at.unwrap_or_else(|| i18n.t("alert.warning")), "var(--dp-warn)"),
        Kind::RunOut => {
            let detail = view.forecast.as_ref().map(|f| f.text.clone()).or(out_at).unwrap_or_default();
            (level, detail, "var(--dp-warn)")
        }
        Kind::Reset => (i18n.f("alert.reset.a", &[("name", view.name)]), i18n.t("alert.reset.b"), "var(--dp-ok)"),
        Kind::ResetSoon => (i18n.f("alert.soon.a", &[("name", view.name)]), i18n.f("alert.soon.b", &[("pct", &pct)]), "var(--dp-ink3)"),
    }
}

/// Shows a Windows notification. Silent unless Settings › Alerts › Sound is on.
pub fn toast(app: &AppHandle, title: &str, body: &str) {
    let sound = app.state::<Arc<Core>>().config().alerts.sound;
    let mut builder = app.notification().builder().title(title).body(body);
    if sound {
        builder = builder.sound("Default");
    }
    let _ = builder.show();
}

pub fn send(app: &AppHandle, kind: Kind, view: &ProviderView) {
    let core = app.state::<Arc<Core>>();
    let show_as = core.config().alerts.show_as;
    let (a, b, dot) = words(kind, view, &core.i18n.read().unwrap());
    let has_notch = notch::window(app).is_some();
    let peek = has_notch && matches!(show_as, ShowAs::Peek | ShowAs::Both);
    let toast_too = !has_notch || matches!(show_as, ShowAs::Toast | ShowAs::Both);
    crate::trace!("alert {:?} for {}", kind, view.id);
    if peek {
        let _ = app.emit_to(notch::LABEL, "notch-peek", Peek { id: view.id.to_string(), a: a.clone(), b: format!("· {b}"), dot });
    }
    if toast_too {
        toast(app, &a, &b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Language;
    use crate::state::ForecastView;

    fn view() -> ProviderView {
        ProviderView {
            id: "claude",
            name: "Claude",
            ring: "ok",
            card: "ok",
            enabled: true,
            loading: false,
            pct: 81.4,
            weekly: None,
            pace: None,
            period: 0,
            plan: None,
            title: String::new(),
            rows: Vec::new(),
            forecast: Some(ForecastView { kind: "warn", text: "At this pace: out at 16:20 (40 min before reset)".into() }),
            message: None,
            spark: Vec::new(),
            footer: String::new(),
            label: String::new(),
            out_at: Some("16:20".into()),
            main_resets_at: None,
        }
    }

    #[test]
    fn alert_wording() {
        let i18n = I18n::new(Language::En, "en-US");
        let v = view();
        assert_eq!(words(Kind::Warning, &v, &i18n), ("Claude 81%".into(), "Out at 16:20".into(), "var(--dp-warn)"));
        assert_eq!(words(Kind::RunOut, &v, &i18n).1, "At this pace: out at 16:20 (40 min before reset)");
        assert_eq!(words(Kind::Reset, &v, &i18n), ("Claude reset".into(), "100% available".into(), "var(--dp-ok)"));
        assert_eq!(words(Kind::ResetSoon, &v, &i18n).0, "Claude resets in 10 min");
        let calm = ProviderView { out_at: None, ..view() };
        assert_eq!(words(Kind::Critical, &calm, &i18n).1, "Critical level reached");
    }
}
