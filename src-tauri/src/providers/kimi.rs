//! Kimi Code.
//! Reads  %KIMI_CODE_HOME% (else %USERPROFILE%\.kimi-code) \credentials\kimi-code.json
//! Calls  GET https://api.kimi.com/coding/v1/usages

use std::path::PathBuf;

use serde_json::Value;

use super::{check_status, home_dir, num, parse_json, read_credential_file, FetchError, Limit, Usage, DAY, HOUR};
use crate::clock;
use crate::net::Net;
use crate::providers::ProviderId;

const URL: &str = "https://api.kimi.com/coding/v1/usages";

pub fn credential_path() -> Option<PathBuf> {
    let home = match std::env::var_os("KIMI_CODE_HOME").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => home_dir()?.join(".kimi-code"),
    };
    Some(home.join("credentials").join("kimi-code.json"))
}

pub fn fetch(net: &Net) -> Result<Usage, FetchError> {
    let path = credential_path().ok_or(FetchError::NotInstalled)?;
    let file = read_credential_file(&path)?;
    // No token at all: not signed in on this PC, shown as "not installed".
    let token = file["access_token"].as_str().filter(|t| !t.is_empty()).ok_or(FetchError::NotInstalled)?;
    // An expired token is never sent; only Kimi Code may refresh it.
    if num(&file["expires_at"]).is_some_and(|at| clock::normalize_epoch(at) <= clock::now()) {
        return Err(FetchError::SignIn);
    }
    let headers = [("Authorization", format!("Bearer {token}")), ("Accept", "application/json".to_string())];
    let resp = net.get(ProviderId::Kimi, URL, &headers)?;
    parse(&check_status(resp.status, resp.retry_after, resp.body)?)
}

fn time_of(v: &Value) -> Option<i64> {
    v.as_str().and_then(clock::parse_rfc3339).or_else(|| num(v).map(clock::normalize_epoch))
}

/// Preferred shape: `{used_ratio, reset_time}`. A ratio is 0..=1; a value above 1 is already a percentage.
fn ratio_limit(v: &Value, id: &str, label: &str, secs: i64) -> Option<Limit> {
    let ratio = num(&v["used_ratio"])?;
    let pct = if ratio <= 1.0 { ratio * 100.0 } else { ratio };
    Some(Limit::new(id, label, pct, time_of(&v["reset_time"]), Some(secs)))
}

/// Legacy shape: `{limit, used, remaining, resetTime}`. `used` may be missing.
fn counted_limit(v: &Value, id: &str, label: &str, secs: i64) -> Option<Limit> {
    let limit = num(&v["limit"]).filter(|l| *l > 0.0)?;
    let used = num(&v["used"]).or_else(|| num(&v["remaining"]).map(|r| limit - r))?;
    Some(Limit::new(id, label, used / limit * 100.0, time_of(&v["resetTime"]), Some(secs)))
}

/// Seconds in a legacy `{duration, timeUnit}` window.
fn window_secs(window: &Value) -> Option<i64> {
    let duration = num(&window["duration"])? as i64;
    let unit = window["timeUnit"].as_str().unwrap_or("").to_ascii_uppercase();
    let per = if unit.contains("MINUTE") {
        60
    } else if unit.contains("HOUR") {
        HOUR
    } else if unit.contains("DAY") {
        DAY
    } else {
        return None;
    };
    Some(duration * per)
}

/// Accepts the current shape (`usages.limit_5h` …) and the legacy one
/// (`usage` = weekly, `limits[]` with a window). `boosterWallet` is ignored.
pub fn parse(body: &str) -> Result<Usage, FetchError> {
    let json = parse_json(body)?;
    let u = &json["usages"];
    let mut limits: Vec<Limit> = [
        ratio_limit(&u["limit_5h"], "5h", "limit.5h", 5 * HOUR),
        ratio_limit(&u["limit_7d"], "7d", "limit.weekly", 7 * DAY),
        ratio_limit(&u["limit_month_total"], "month_total", "limit.monthly", 30 * DAY),
        ratio_limit(&u["limit_month_code"], "month_code", "limit.monthly_code", 30 * DAY),
    ]
    .into_iter()
    .flatten()
    .collect();

    if limits.is_empty() {
        for item in json["limits"].as_array().map(Vec::as_slice).unwrap_or(&[]) {
            // 300 minutes is the 5-hour window.
            if window_secs(&item["window"]) == Some(5 * HOUR) {
                limits.extend(counted_limit(&item["detail"], "5h", "limit.5h", 5 * HOUR));
            }
        }
        limits.extend(counted_limit(&json["usage"], "7d", "limit.weekly", 7 * DAY));
    }
    if limits.is_empty() {
        return Err(FetchError::Failed("no usage limits in response".into()));
    }
    Ok(Usage { plan: None, limits })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_current_shape() {
        let body = r#"{"usages":{
            "limit_5h":{"used_ratio":0.27,"reset_time":"2026-11-16T17:00:00Z"},
            "limit_7d":{"used_ratio":"0.5","reset_time":1795277045},
            "limit_month_total":{"used_ratio":0,"reset_time":null},
            "limit_month_code":{"used_ratio":1,"reset_time":null}},
            "boosterWallet":{"balance":12}}"#;
        let u = parse(body).unwrap();
        let got: Vec<(&str, f64)> = u.limits.iter().map(|l| (l.id.as_str(), l.used_pct)).collect();
        assert_eq!(got, [("5h", 27.0), ("7d", 50.0), ("month_total", 0.0), ("month_code", 100.0)]);
        assert_eq!(u.limits[0].resets_at, Some(1_794_848_400));
        assert_eq!(u.limits[1].resets_at, Some(1_795_277_045));
    }

    #[test]
    fn parses_legacy_shape_with_string_numbers() {
        let body = r#"{"usage":{"limit":"1000","used":"270","remaining":"730","resetTime":"2026-11-19T21:00:00Z"},
            "limits":[{"window":{"duration":300,"timeUnit":"TIME_UNIT_MINUTE"},"detail":{"limit":"100","remaining":"88","resetTime":"2026-11-16T17:00:00Z"}},
                      {"window":{"duration":1,"timeUnit":"TIME_UNIT_DAY"},"detail":{"limit":"10","used":"1"}}]}"#;
        let u = parse(body).unwrap();
        let got: Vec<(&str, f64)> = u.limits.iter().map(|l| (l.id.as_str(), l.used_pct)).collect();
        assert_eq!(got, [("5h", 12.0), ("7d", 27.0)], "used = limit - remaining when missing; other windows ignored");
        assert_eq!(u.limits[0].resets_at, Some(1_794_848_400));
    }

    #[test]
    fn current_shape_is_preferred_over_legacy() {
        let body = r#"{"usages":{"limit_5h":{"used_ratio":0.1}},"usage":{"limit":100,"used":90}}"#;
        let u = parse(body).unwrap();
        assert_eq!(u.limits.len(), 1);
        assert_eq!(u.limits[0].used_pct, 10.0);
    }

    #[test]
    fn rejects_empty_responses() {
        assert!(parse("{}").is_err());
        assert!(parse(r#"{"usage":{"limit":0,"used":0}}"#).is_err());
        assert!(parse("<html>").is_err());
    }
}
