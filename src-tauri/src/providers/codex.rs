//! Codex CLI.
//! Reads  %USERPROFILE%\.codex\auth.json (tokens.access_token, tokens.account_id)
//! Calls  GET https://chatgpt.com/backend-api/wham/usage

use std::path::PathBuf;

use serde_json::Value;

use super::{check_status, home_dir, num, parse_json, read_credential_file, FetchError, Limit, Usage, DAY, HOUR};
use crate::clock;
use crate::net::Net;
use crate::providers::ProviderId;

const URL: &str = "https://chatgpt.com/backend-api/wham/usage";

pub fn credential_path() -> Option<PathBuf> {
    Some(home_dir()?.join(".codex").join("auth.json"))
}

pub fn fetch(net: &Net) -> Result<Usage, FetchError> {
    let path = credential_path().ok_or(FetchError::NotInstalled)?;
    let file = read_credential_file(&path)?;
    let tokens = &file["tokens"];
    // No token at all: not signed in on this PC, shown as "not installed".
    let token = tokens["access_token"].as_str().filter(|t| !t.is_empty()).ok_or(FetchError::NotInstalled)?;
    let mut headers = vec![("Authorization", format!("Bearer {token}"))];
    if let Some(account) = tokens["account_id"].as_str().filter(|a| !a.is_empty()) {
        headers.push(("ChatGPT-Account-Id", account.to_string()));
    }
    let resp = net.get(ProviderId::Codex, URL, &headers)?;
    let body = check_status(resp.status, resp.retry_after, resp.body)?;
    parse(&body, clock::now())
}

/// Label for a window from its length: 5 h and 7 d get their usual names.
fn label_for(window_secs: Option<i64>, fallback: &'static str) -> &'static str {
    match window_secs {
        Some(s) if s == 5 * HOUR => "limit.5h",
        Some(s) if s == 7 * DAY => "limit.weekly",
        Some(s) if s == DAY => "limit.daily",
        Some(s) if (28 * DAY..=31 * DAY).contains(&s) => "limit.monthly",
        _ => fallback,
    }
}

fn window(id: &str, fallback_label: &'static str, v: &Value, now: i64) -> Option<Limit> {
    let pct = num(&v["used_percent"])?;
    let window_secs = num(&v["limit_window_seconds"]).map(|s| s as i64).filter(|s| *s > 0);
    let resets_at = num(&v["reset_at"])
        .map(clock::normalize_epoch)
        .or_else(|| num(&v["reset_after_seconds"]).map(|s| now + s as i64));
    Some(Limit::new(id, label_for(window_secs, fallback_label), pct, resets_at, window_secs))
}

pub fn parse(body: &str, now: i64) -> Result<Usage, FetchError> {
    let json = parse_json(body)?;
    let rate = &json["rate_limit"];
    let mut limits = Vec::new();
    limits.extend(window("primary", "limit.primary", &rate["primary_window"], now));
    limits.extend(window("secondary", "limit.secondary", &rate["secondary_window"], now));
    if limits.is_empty() {
        return Err(FetchError::Failed("no rate limits in response".into()));
    }
    let plan = json["plan_type"].as_str().filter(|p| !p.is_empty()).map(super::title_case);
    Ok(Usage { plan, limits })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_794_841_445;

    #[test]
    fn parses_both_windows() {
        let body = r#"{"plan_type":"plus","rate_limit":{"allowed":true,
            "primary_window":{"used_percent":12,"limit_window_seconds":18000,"reset_after_seconds":14520,"reset_at":1794855965},
            "secondary_window":{"used_percent":30.5,"limit_window_seconds":604800,"reset_at":1795277045}}}"#;
        let u = parse(body, NOW).unwrap();
        assert_eq!(u.plan.as_deref(), Some("Plus"));
        assert_eq!(u.limits.len(), 2);
        let p = &u.limits[0];
        assert_eq!((p.id.as_str(), p.label.as_str(), p.used_pct), ("primary", "limit.5h", 12.0));
        assert_eq!(p.resets_at, Some(1_794_855_965));
        assert_eq!(p.window_secs, Some(18_000));
        let s = &u.limits[1];
        assert_eq!((s.id.as_str(), s.label.as_str(), s.used_pct), ("secondary", "limit.weekly", 30.5));
    }

    #[test]
    fn falls_back_to_reset_after_seconds() {
        let body = r#"{"rate_limit":{"primary_window":{"used_percent":"7","limit_window_seconds":18000,"reset_after_seconds":600}}}"#;
        let u = parse(body, NOW).unwrap();
        assert_eq!(u.plan, None);
        assert_eq!(u.limits.len(), 1);
        assert_eq!(u.limits[0].used_pct, 7.0);
        assert_eq!(u.limits[0].resets_at, Some(NOW + 600));
    }

    #[test]
    fn secondary_may_be_null_and_zero_is_valid() {
        let body = r#"{"plan_type":"pro","rate_limit":{"primary_window":{"used_percent":0,"limit_window_seconds":18000,"reset_at":1794855965},"secondary_window":null}}"#;
        let u = parse(body, NOW).unwrap();
        assert_eq!(u.limits.len(), 1);
        assert_eq!(u.limits[0].used_pct, 0.0);
    }

    #[test]
    fn unusual_window_keeps_generic_label() {
        let body = r#"{"rate_limit":{"primary_window":{"used_percent":5,"limit_window_seconds":7200,"reset_at":1794855965}}}"#;
        assert_eq!(parse(body, NOW).unwrap().limits[0].label, "limit.primary");
    }

    #[test]
    fn rejects_responses_without_limits() {
        assert!(parse(r#"{"plan_type":"free"}"#, NOW).is_err());
        assert!(parse("not json", NOW).is_err());
    }
}
