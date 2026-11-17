//! Display data for Settings › Accounts and the "files this app reads" list.
//! Paths are shown; file contents never leave the provider modules.

use std::path::PathBuf;

use serde::Serialize;

use crate::config::validate::main_limit_options;
use crate::i18n::I18n;
use crate::providers::{self, ProviderId};
use crate::state::{Core, ProviderState, Status};

#[derive(Debug, Clone, Serialize)]
pub struct LimitOption {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AccountView {
    pub id: &'static str,
    pub name: &'static str,
    /// "Connected · Pro", "Signed out", …
    pub status: String,
    /// ok | stale | off — the colour of the status pill.
    pub kind: &'static str,
    /// The credential file (or agy.exe) this provider reads.
    pub path: String,
    pub path_found: bool,
    pub updated: String,
    pub installed: bool,
    pub loading: bool,
    pub limits: Vec<LimitOption>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileView {
    pub path: String,
    pub found: bool,
}

/// Label for a main-limit option when no reading names it yet.
fn default_label(id: ProviderId, limit: &str) -> &'static str {
    match (id, limit) {
        (_, "session" | "rolling" | "5h") => "limit.5h",
        (_, "weekly" | "7d") => "limit.weekly",
        (_, "monthly" | "month_total" | "total") => "limit.monthly",
        (_, "primary") => "limit.primary",
        (_, "secondary") => "limit.secondary",
        _ => "limit.primary",
    }
}

fn limit_options(id: ProviderId, state: Option<&ProviderState>, i18n: &I18n) -> Vec<LimitOption> {
    let reported = state.and_then(|s| s.usage.as_ref()).map(|u| u.limits.as_slice()).unwrap_or(&[]);
    let known = main_limit_options(id);
    if known.is_empty() {
        // Antigravity: whatever model groups agy.exe reported.
        return reported.iter().map(|l| LimitOption { id: l.id.clone(), label: i18n.t(&l.label) }).collect();
    }
    known
        .iter()
        .map(|limit| {
            let label = reported.iter().find(|l| l.id == *limit).map(|l| l.label.as_str()).unwrap_or(default_label(id, limit));
            LimitOption { id: limit.to_string(), label: i18n.t(label) }
        })
        .collect()
}

fn age(i18n: &I18n, ago: &str, just_now: &str, at: Option<i64>, now: i64) -> String {
    match at {
        Some(at) if now - at >= 45 => i18n.f(ago, &[("time", &i18n.duration(now - at))]),
        Some(_) => i18n.t(just_now),
        None => i18n.t("acc.not_checked"),
    }
}

pub fn accounts(core: &Core, now: i64) -> Vec<AccountView> {
    let config = core.config();
    let i18n = core.i18n.read().unwrap();
    let all = core.providers.lock().unwrap();
    config
        .providers
        .order
        .iter()
        .map(|id| {
            let state = all.get(id);
            let status = state.map(|s| s.status.clone()).unwrap_or(Status::Unknown);
            let loading = state.is_some_and(|s| s.loading);
            let stale = state.is_some_and(|s| s.is_stale());
            let plan = state.and_then(|s| s.usage.as_ref()).and_then(|u| u.plan.clone());
            let (text, kind) = match &status {
                _ if stale => (i18n.t("status.stale"), "stale"),
                Status::Ok => match plan {
                    Some(plan) => (i18n.f("status.ok_plan", &[("plan", &plan)]), "ok"),
                    None => (i18n.t("status.ok"), "ok"),
                },
                Status::SignIn => (i18n.t("status.signin"), "off"),
                Status::NoSubscription => (i18n.t("status.nosub"), "off"),
                Status::NotInstalled => (i18n.t("status.missing"), "off"),
                Status::Error(_) => (i18n.t("status.error"), "stale"),
                Status::Unknown => (i18n.t("status.checking"), "off"),
            };
            let path: Option<PathBuf> = providers::files(*id, &config).into_iter().next();
            let updated = match state {
                Some(s) if stale => age(&i18n, "acc.last_updated", "acc.updated_now", s.fetched_at, now),
                Some(s) if s.status == Status::Ok => age(&i18n, "acc.updated", "acc.updated_now", s.fetched_at, now),
                Some(s) => age(&i18n, "acc.checked", "acc.checked_now", s.checked_at, now),
                None => i18n.t("acc.not_checked"),
            };
            AccountView {
                id: id.key(),
                name: id.name(),
                status: text,
                kind,
                path_found: path.as_ref().is_some_and(|p| p.exists()),
                path: match path {
                    Some(p) => p.display().to_string(),
                    None => i18n.t("acc.agy_missing"),
                },
                updated,
                installed: !matches!(status, Status::NotInstalled | Status::Unknown),
                loading,
                limits: limit_options(*id, state, &i18n),
            }
        })
        .collect()
}

/// Every file Droplet reads from other apps, with whether it exists right now.
pub fn files(core: &Core) -> Vec<FileView> {
    let config = core.config();
    config
        .providers
        .order
        .iter()
        .flat_map(|id| providers::files(*id, &config))
        .map(|path| FileView { found: path.exists(), path: path.display().to_string() })
        .collect()
}
