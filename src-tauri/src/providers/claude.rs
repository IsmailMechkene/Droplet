//! Claude Code.
//! Reads  %USERPROFILE%\.claude\.credentials.json (claudeAiOauth.accessToken / expiresAt, ms)
//! Calls  GET https://api.anthropic.com/api/oauth/usage

use std::path::PathBuf;

use serde_json::Value;

use super::{check_status, home_dir, num, parse_json, read_credential_file, FetchError, Limit, Usage, DAY, HOUR};
use crate::clock;
use crate::net::Net;
use crate::providers::ProviderId;

const URL: &str = "https://api.anthropic.com/api/oauth/usage";

pub fn credential_path() -> Option<PathBuf> {
    Some(home_dir()?.join(".claude").join(".credentials.json"))
}

pub fn fetch(net: &Net) -> Result<Usage, FetchError> {
    let path = credential_path().ok_or(FetchError::NotInstalled)?;
    let file = read_credential_file(&path)?;
    let oauth = &file["claudeAiOauth"];
    let token = oauth["accessToken"].as_str().filter(|t| !t.is_empty()).ok_or(FetchError::SignIn)?;
    // An expired token is never sent; only Claude Code may refresh it.
    if let Some(expires_ms) = num(&oauth["expiresAt"]) {
        if clock::normalize_epoch(expires_ms) <= clock::now() {
            return Err(FetchError::SignIn);
        }
    }
    let plan = oauth["subscriptionType"].as_str().map(title_case);

    let headers = [("Authorization", format!("Bearer {token}")), ("anthropic-beta", "oauth-2025-04-20".to_string())];
    let resp = net.get(ProviderId::Claude, URL, &headers)?;
    let body = check_status(resp.status, resp.retry_after, resp.body)?;
    let mut usage = parse(&body)?;
    usage.plan = plan;
    Ok(usage)
}

fn title_case(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Maps an API kind to (window id, label key, window length).
fn window_for(kind: &str) -> Option<(&'static str, &'static str, i64)> {
    match kind {
        "session" | "five_hour" => Some(("session", "limit.5h", 5 * HOUR)),
        "weekly_all" | "seven_day" => Some(("weekly", "limit.weekly", 7 * DAY)),
        "weekly_opus" => Some(("weekly_opus", "limit.weekly_opus", 7 * DAY)),
        "weekly_scoped" => Some(("weekly_scoped", "limit.weekly_scoped", 7 * DAY)),
        _ => None,
    }
}

fn limit(kind: &str, percent: &Value, resets_at: &Value) -> Option<Limit> {
    let (id, label, secs) = window_for(kind)?;
    let pct = num(percent)?;
    let resets = resets_at.as_str().and_then(clock::parse_rfc3339);
    Some(Limit::new(id, label, pct, resets, Some(secs)))
}

/// Accepts both shapes:
/// - `limits: [{kind, percent, resets_at}]`
/// - `five_hour` / `seven_day`: `{utilization, resets_at}`
/// A window present in both is taken once (the `limits` entry wins).
pub fn parse(body: &str) -> Result<Usage, FetchError> {
    let json = parse_json(body)?;
    let mut limits: Vec<Limit> = Vec::new();

    if let Some(list) = json["limits"].as_array() {
        for item in list {
            let kind = item["kind"].as_str().unwrap_or("");
            if let Some(l) = limit(kind, &item["percent"], &item["resets_at"]) {
                if !limits.iter().any(|x| x.id == l.id) {
                    limits.push(l);
                }
            }
        }
    }
    for kind in ["five_hour", "seven_day"] {
        let item = &json[kind];
        if item.is_object() {
            if let Some(l) = limit(kind, &item["utilization"], &item["resets_at"]) {
                if !limits.iter().any(|x| x.id == l.id) {
                    limits.push(l);
                }
            }
        }
    }

    if limits.is_empty() {
        return Err(FetchError::Failed("no usage limits in response".into()));
    }
    let order = ["session", "weekly", "weekly_opus", "weekly_scoped"];
    limits.sort_by_key(|l| order.iter().position(|id| *id == l.id).unwrap_or(order.len()));
    Ok(Usage { plan: None, limits })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_limits_array() {
        let body = r#"{"limits":[
            {"kind":"weekly_all","percent":41,"resets_at":"2026-11-19T21:00:00Z"},
            {"kind":"session","percent":64.4,"resets_at":"2026-11-16T17:00:00+00:00"},
            {"kind":"weekly_opus","percent":12,"resets_at":"2026-11-19T21:00:00Z"},
            {"kind":"weekly_scoped","percent":3,"resets_at":null},
            {"kind":"something_new","percent":99,"resets_at":null}
        ]}"#;
        let u = parse(body).unwrap();
        let ids: Vec<&str> = u.limits.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["session", "weekly", "weekly_opus", "weekly_scoped"]);
        assert_eq!(u.limits[0].used_pct, 64.4);
        assert_eq!(u.limits[0].resets_at, Some(1_794_848_400));
        assert_eq!(u.limits[0].window_secs, Some(5 * HOUR));
        assert_eq!(u.limits[1].window_secs, Some(7 * DAY));
        assert_eq!(u.limits[3].resets_at, None);
    }

    #[test]
    fn parses_fallback_shape() {
        let body = r#"{"five_hour":{"utilization":8.0,"resets_at":"2026-11-16T17:00:00Z"},
                       "seven_day":{"utilization":55,"resets_at":"2026-11-19T21:00:00Z"},
                       "seven_day_opus":null}"#;
        let u = parse(body).unwrap();
        assert_eq!(u.limits.len(), 2);
        assert_eq!((u.limits[0].id.as_str(), u.limits[0].used_pct), ("session", 8.0));
        assert_eq!((u.limits[1].id.as_str(), u.limits[1].used_pct), ("weekly", 55.0));
    }

    #[test]
    fn dedupes_when_both_shapes_are_present() {
        let body = r#"{"limits":[{"kind":"session","percent":30,"resets_at":"2026-11-16T17:00:00Z"}],
                       "five_hour":{"utilization":31,"resets_at":"2026-11-16T17:00:00Z"},
                       "seven_day":{"utilization":50,"resets_at":"2026-11-19T21:00:00Z"}}"#;
        let u = parse(body).unwrap();
        assert_eq!(u.limits.len(), 2);
        assert_eq!(u.limits[0].used_pct, 30.0, "the limits[] entry wins");
        assert_eq!(u.limits[1].id, "weekly");
    }

    #[test]
    fn zero_is_valid_and_values_are_clamped() {
        let u = parse(r#"{"five_hour":{"utilization":0,"resets_at":null}}"#).unwrap();
        assert_eq!(u.limits[0].used_pct, 0.0);
        let u = parse(r#"{"five_hour":{"utilization":104.2,"resets_at":null}}"#).unwrap();
        assert_eq!(u.limits[0].used_pct, 100.0);
    }

    #[test]
    fn rejects_empty_or_broken_responses() {
        assert!(parse("{}").is_err());
        assert!(parse("<html>").is_err());
        assert!(parse(r#"{"limits":[]}"#).is_err());
    }
}
