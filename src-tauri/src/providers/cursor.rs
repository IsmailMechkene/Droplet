//! Cursor.
//! Reads  %APPDATA%\Cursor\User\globalStorage\state.vscdb (SQLite, opened read-only)
//!        %APPDATA%\Cursor\Local State (the DPAPI-wrapped key for encrypted values)
//! Calls  POST https://api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage
//!        GET  https://cursor.com/api/usage-summary (fallback)

use std::path::PathBuf;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use serde_json::Value;

use super::{num, parse_json, sqlite, FetchError, Limit, Usage};
use crate::clock;
use crate::net::Net;
use crate::providers::ProviderId;
use crate::win::dpapi;

const PRIMARY_URL: &str = "https://api2.cursor.sh/aiserver.v1.DashboardService/GetCurrentPeriodUsage";
const FALLBACK_URL: &str = "https://cursor.com/api/usage-summary";
const ENCRYPTED_PREFIX: &str = "cursor-safe-storage:v1:";

fn app_data() -> Option<PathBuf> {
    Some(PathBuf::from(std::env::var_os("APPDATA")?).join("Cursor"))
}

pub fn database_path() -> Option<PathBuf> {
    Some(app_data()?.join("User").join("globalStorage").join("state.vscdb"))
}

pub fn local_state_path() -> Option<PathBuf> {
    Some(app_data()?.join("Local State"))
}

/// Decrypts a stored value. Plain values pass through. Encrypted ones are
/// base64 of "v10" | 12-byte nonce | AES-256-GCM ciphertext + tag.
pub fn decrypt_value(value: &str, key: Option<&[u8]>) -> Option<String> {
    let Some(encoded) = value.strip_prefix(ENCRYPTED_PREFIX) else {
        return Some(value.to_string());
    };
    let blob = BASE64.decode(encoded.trim()).ok()?;
    let body = blob.strip_prefix(b"v10")?;
    if body.len() < 12 + 16 {
        return None;
    }
    let (nonce, ciphertext) = body.split_at(12);
    let cipher = Aes256Gcm::new_from_slice(key?).ok()?;
    let plain = cipher.decrypt(Nonce::from_slice(nonce), ciphertext).ok()?;
    String::from_utf8(plain).ok()
}

/// The AES key: `os_crypt.encrypted_key` in Local State is base64 of "DPAPI" + a
/// blob only the current Windows user can unwrap.
fn storage_key() -> Option<Vec<u8>> {
    let text = std::fs::read_to_string(local_state_path()?).ok()?;
    let json: Value = serde_json::from_str(&text).ok()?;
    let wrapped = BASE64.decode(json["os_crypt"]["encrypted_key"].as_str()?).ok()?;
    dpapi::unprotect(wrapped.strip_prefix(b"DPAPI")?)
}

struct Credential {
    token: String,
    auth_id: Option<String>,
    membership: Option<String>,
}

fn read_credential() -> Result<Credential, FetchError> {
    let path = database_path().filter(|p| p.exists()).ok_or(FetchError::NotInstalled)?;
    let db = sqlite::open_read_only(&path).ok_or_else(|| FetchError::Failed("could not open Cursor's settings database".into()))?;
    let mut key: Option<Option<Vec<u8>>> = None; // fetched once, and only if a value is encrypted
    // Outer None: no such value. Inner None: the value exists but could not be decrypted.
    let mut read = |name: &str| -> Option<Option<String>> {
        let raw: String = sqlite::text_value(&db, "SELECT value FROM ItemTable WHERE key = ?1", name)?;
        if raw.starts_with(ENCRYPTED_PREFIX) {
            let k = key.get_or_insert_with(storage_key);
            Some(decrypt_value(&raw, k.as_deref()))
        } else {
            Some(Some(raw))
        }
    };
    // No stored token at all means Cursor is not signed in on this PC: treated as "not installed".
    let token = match read("cursorAuth/accessToken") {
        None => return Err(FetchError::NotInstalled),
        Some(None) => return Err(FetchError::Failed("could not decrypt Cursor's sign-in".into())),
        Some(Some(token)) if token.is_empty() => return Err(FetchError::NotInstalled),
        Some(Some(token)) => token,
    };
    Ok(Credential {
        token,
        auth_id: read("cursorAuth/stripeMembershipAuthId").flatten(),
        membership: read("cursorAuth/stripeMembershipType").flatten(),
    })
}

pub fn fetch(net: &Net) -> Result<Usage, FetchError> {
    let cred = read_credential()?;
    let now = clock::now();

    let headers = [("Authorization", format!("Bearer {}", cred.token)), ("Content-Type", "application/json".to_string())];
    let primary = net.post_json(ProviderId::Cursor, PRIMARY_URL, &headers, "{}");
    let mut usage = match primary {
        Ok(resp) if resp.status == 429 => return Err(FetchError::RateLimited { retry_after: resp.retry_after }),
        Ok(resp) if (200..300).contains(&resp.status) => parse_primary(&resp.body, now),
        Ok(resp) => Err(FetchError::Failed(format!("HTTP {}", resp.status))),
        Err(e) => Err(e.into()),
    };
    // The dashboard API moves around; the website summary is the fallback.
    if !matches!(usage, Ok(_) | Err(FetchError::NoSubscription)) {
        if let Some(auth_id) = cred.auth_id.as_deref().filter(|a| !a.is_empty()) {
            let cookie = format!("WorkosCursorSessionToken={auth_id}%3A%3A{}", cred.token);
            let headers = [("Cookie", cookie), ("Accept", "application/json".to_string())];
            let resp = net.get(ProviderId::Cursor, FALLBACK_URL, &headers)?;
            let body = super::check_status(resp.status, resp.retry_after, resp.body)?;
            usage = parse_fallback(&body, now);
        }
    }
    let mut usage = usage?;
    usage.plan = cred.membership.filter(|m| !m.is_empty()).map(|m| super::title_case(&m.replace('_', " ")));
    Ok(usage)
}

/// Milliseconds, seconds or an RFC 3339 string → Unix seconds.
fn time_of(v: &Value) -> Option<i64> {
    v.as_str().and_then(clock::parse_rfc3339).or_else(|| num(v).map(clock::normalize_epoch))
}

/// Builds the limits from already-computed percentages. 0 % is a valid reading.
fn limits(total: f64, api: Option<f64>, start: Option<i64>, end: Option<i64>, on_demand: Option<f64>) -> Vec<Limit> {
    let window = match (start, end) {
        (Some(s), Some(e)) if e > s => Some(e - s),
        _ => None,
    };
    let mut list = vec![Limit::new("total", "limit.monthly", total, end, window)];
    if let Some(api) = api.filter(|a| *a > 0.0) {
        list.push(Limit::new("api", "limit.api", api, end, window));
    }
    if let Some(pct) = on_demand {
        list.push(Limit::new("ondemand", "limit.ondemand", pct, end, window));
    }
    list
}

pub fn parse_primary(body: &str, _now: i64) -> Result<Usage, FetchError> {
    let json = parse_json(body)?;
    let plan = &json["planUsage"];
    let total = num(&plan["totalPercentUsed"]).ok_or_else(|| FetchError::Failed("no plan usage in response".into()))?;
    let (start, end) = (time_of(&json["billingCycleStart"]), time_of(&json["billingCycleEnd"]));
    Ok(Usage { plan: None, limits: limits(total, num(&plan["apiPercentUsed"]), start, end, None) })
}

pub fn parse_fallback(body: &str, _now: i64) -> Result<Usage, FetchError> {
    let json = parse_json(body)?;
    if json["isUnlimited"].as_bool() == Some(true) {
        return Err(FetchError::NoSubscription);
    }
    let individual = &json["individualUsage"];
    let plan = &individual["plan"];
    let total = num(&plan["totalPercentUsed"]).ok_or_else(|| FetchError::Failed("no plan usage in response".into()))?;
    // On demand is shown only when it is switched on and has a limit.
    let od = &individual["onDemand"];
    let on_demand = match (od["enabled"].as_bool(), num(&od["used"]), num(&od["limit"])) {
        (Some(true), Some(used), Some(limit)) if limit > 0.0 => Some(used / limit * 100.0),
        _ => None,
    };
    let (start, end) = (time_of(&json["billingCycleStart"]), time_of(&json["billingCycleEnd"]));
    Ok(Usage { plan: None, limits: limits(total, num(&plan["apiPercentUsed"]), start, end, on_demand) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_primary_response() {
        let body = r#"{"billingCycleStart":"1793491200000","billingCycleEnd":"1796083200000",
                       "planUsage":{"totalSpend":1234,"totalPercentUsed":72.5,"apiPercentUsed":3.25}}"#;
        let u = parse_primary(body, 0).unwrap();
        assert_eq!(u.limits.len(), 2);
        let total = &u.limits[0];
        assert_eq!((total.id.as_str(), total.used_pct), ("total", 72.5));
        assert_eq!(total.resets_at, Some(1_796_083_200));
        assert_eq!(total.window_secs, Some(30 * 86_400));
        assert_eq!((u.limits[1].id.as_str(), u.limits[1].used_pct), ("api", 3.25));
    }

    #[test]
    fn zero_percent_is_valid_and_api_zero_is_hidden() {
        let body = r#"{"billingCycleStart":1793491200000,"billingCycleEnd":1796083200000,"planUsage":{"totalPercentUsed":0,"apiPercentUsed":0}}"#;
        let u = parse_primary(body, 0).unwrap();
        assert_eq!(u.limits.len(), 1);
        assert_eq!(u.limits[0].used_pct, 0.0);
    }

    #[test]
    fn primary_without_plan_usage_fails() {
        assert!(parse_primary(r#"{"billingCycleEnd":"1796083200000"}"#, 0).is_err());
        assert!(parse_primary("oops", 0).is_err());
    }

    #[test]
    fn parses_fallback_summary() {
        let body = r#"{"billingCycleStart":"2026-11-01T00:00:00.000Z","billingCycleEnd":"2026-12-01T00:00:00.000Z","isUnlimited":false,
            "individualUsage":{"plan":{"used":150,"limit":500,"totalPercentUsed":41,"apiPercentUsed":0},
                               "onDemand":{"enabled":true,"used":5,"limit":20}}}"#;
        let u = parse_fallback(body, 0).unwrap();
        let ids: Vec<&str> = u.limits.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(ids, ["total", "ondemand"]);
        assert_eq!(u.limits[0].used_pct, 41.0, "the reported percentage is used, not used/limit");
        assert_eq!(u.limits[1].used_pct, 25.0);
        assert_eq!(u.limits[0].resets_at, clock::parse_rfc3339("2026-12-01T00:00:00Z"));
    }

    #[test]
    fn fallback_on_demand_needs_enabled_and_a_limit() {
        let disabled = r#"{"individualUsage":{"plan":{"totalPercentUsed":10},"onDemand":{"enabled":false,"used":5,"limit":20}}}"#;
        assert_eq!(parse_fallback(disabled, 0).unwrap().limits.len(), 1);
        let no_limit = r#"{"individualUsage":{"plan":{"totalPercentUsed":10},"onDemand":{"enabled":true,"used":5,"limit":null}}}"#;
        assert_eq!(parse_fallback(no_limit, 0).unwrap().limits.len(), 1);
    }

    #[test]
    fn unlimited_plan_has_nothing_to_track() {
        assert_eq!(parse_fallback(r#"{"isUnlimited":true}"#, 0), Err(FetchError::NoSubscription));
    }

    #[test]
    fn decrypts_safe_storage_values() {
        let key = [7u8; 32];
        let nonce = [9u8; 12];
        let cipher = Aes256Gcm::new_from_slice(&key).unwrap();
        let sealed = cipher.encrypt(Nonce::from_slice(&nonce), b"secret-token".as_ref()).unwrap();
        let mut blob = b"v10".to_vec();
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&sealed);
        let stored = format!("{ENCRYPTED_PREFIX}{}", BASE64.encode(&blob));

        assert_eq!(decrypt_value(&stored, Some(&key)).as_deref(), Some("secret-token"));
        assert_eq!(decrypt_value(&stored, Some(&[8u8; 32])), None, "wrong key");
        assert_eq!(decrypt_value(&stored, None), None, "no key");
        assert_eq!(decrypt_value("plain-token", None).as_deref(), Some("plain-token"));
        assert_eq!(decrypt_value(&format!("{ENCRYPTED_PREFIX}AAAA"), Some(&key)), None, "truncated");
    }
}
