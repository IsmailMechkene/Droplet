//! OpenCode (Go plan).
//! Reads  %USERPROFILE%\.local\share\opencode\auth.json
//!        %USERPROFILE%\.local\share\opencode\opencode.db (SQLite `credential` table, read-only)
//! Calls  GET https://opencode.ai/zen/go/v1/usage        (API key)
//!        GET https://opencode.ai/inference/go/v1/usage  (OAuth)

use std::path::PathBuf;

use serde_json::Value;

use super::{home_dir, num, parse_json, sqlite, FetchError, Limit, Usage, DAY, HOUR};
use crate::clock;
use crate::net::Net;
use crate::providers::ProviderId;

const KEY_URL: &str = "https://opencode.ai/zen/go/v1/usage";
const OAUTH_URL: &str = "https://opencode.ai/inference/go/v1/usage";
const KEY_PREFIX: &str = "oc_sk_";

fn data_dir() -> Option<PathBuf> {
    Some(home_dir()?.join(".local").join("share").join("opencode"))
}

pub fn auth_path() -> Option<PathBuf> {
    Some(data_dir()?.join("auth.json"))
}

pub fn database_path() -> Option<PathBuf> {
    Some(data_dir()?.join("opencode.db"))
}

#[derive(Debug, Clone, PartialEq)]
pub enum Credential {
    ApiKey(String),
    OAuth { access: String, expires_ms: Option<f64> },
}

/// A stored credential entry: `{"type":"api","key":…}` or `{"type":"oauth","access":…,"expires":…}`.
fn api_key(entry: &Value) -> Option<String> {
    entry["key"].as_str().filter(|k| !k.is_empty()).map(str::to_string)
}

fn oauth(entry: &Value) -> Option<Credential> {
    let access = entry["access"].as_str().filter(|a| !a.is_empty())?;
    Some(Credential::OAuth { access: access.to_string(), expires_ms: num(&entry["expires"]) })
}

/// Picks the credential, in this order:
/// 1. any "oc_sk_" key (database or auth.json, under "opencode-go" or "opencode"),
/// 2. auth.json "opencode-go" API key,
/// 3. database "opencode-go", else "opencode", OAuth,
/// 4. auth.json "opencode" OAuth.
///
/// `database` holds (name, entry) pairs from the `credential` table.
pub fn choose(auth: Option<&Value>, database: &[(String, Value)]) -> Option<Credential> {
    let names = ["opencode-go", "opencode"];
    let from_db = |name: &str| database.iter().find(|(n, _)| n == name).map(|(_, v)| v);
    let from_auth = |name: &str| auth.map(|a| &a[name]).filter(|v| v.is_object());

    let entries = names.iter().flat_map(|n| [from_db(n), from_auth(n)]).flatten();
    if let Some(key) = entries.filter_map(api_key).find(|k| k.starts_with(KEY_PREFIX)) {
        return Some(Credential::ApiKey(key));
    }
    if let Some(key) = from_auth("opencode-go").filter(|e| e["type"] == "api").and_then(api_key) {
        return Some(Credential::ApiKey(key));
    }
    names.iter().find_map(|n| from_db(n).and_then(oauth)).or_else(|| from_auth("opencode").and_then(oauth))
}

/// Reads the `credential` table: (integration_id, value) rows, active and newest first.
/// If the schema differs, falls back to any row where one text cell is a
/// credential name and another is a JSON object.
fn database_entries() -> Vec<(String, Value)> {
    let Some(db) = database_path().filter(|p| p.exists()).and_then(|p| sqlite::open_read_only(&p)) else {
        return Vec::new();
    };
    let known = db
        .prepare("SELECT integration_id, value FROM credential WHERE integration_id IN ('opencode-go', 'opencode') ORDER BY active DESC, time_updated DESC")
        .and_then(|mut statement| {
            let rows = statement.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
            Ok(rows.flatten().filter_map(|(name, value)| Some((name, serde_json::from_str::<Value>(&value).ok()?))).collect::<Vec<_>>())
        });
    if let Ok(entries) = known {
        return entries;
    }
    sqlite::text_rows(&db, "credential")
        .into_iter()
        .filter_map(|cells| {
            let name = cells.iter().find(|c| matches!(c.as_str(), "opencode-go" | "opencode"))?.clone();
            let entry = cells.iter().find_map(|c| serde_json::from_str::<Value>(c).ok().filter(Value::is_object))?;
            Some((name, entry))
        })
        .collect()
}

pub fn fetch(net: &Net) -> Result<Usage, FetchError> {
    let auth_file = auth_path().filter(|p| p.exists());
    let has_db = database_path().is_some_and(|p| p.exists());
    if auth_file.is_none() && !has_db {
        return Err(FetchError::NotInstalled);
    }
    let auth: Option<Value> = auth_file.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok());
    // No OpenCode credential at all (other integrations do not count): "not installed".
    let credential = choose(auth.as_ref(), &database_entries()).ok_or(FetchError::NotInstalled)?;

    let (url, token) = match credential {
        Credential::ApiKey(key) => (KEY_URL, key),
        Credential::OAuth { access, expires_ms } => {
            // An expired token is never sent; only OpenCode may refresh it.
            if expires_ms.is_some_and(|ms| clock::normalize_epoch(ms) <= clock::now()) {
                return Err(FetchError::SignIn);
            }
            (OAUTH_URL, access)
        }
    };
    let resp = net.get(ProviderId::Opencode, url, &[("Authorization", format!("Bearer {token}"))])?;
    match resp.status {
        // Signed in, but 401 here usually means the account has no Go plan.
        401 => Err(FetchError::NoSubscription),
        status => parse(&super::check_status(status, resp.retry_after, resp.body)?),
    }
}

fn window(json: &Value, name: &str, id: &str, label: &str, secs: i64) -> Option<Limit> {
    let w = &json["usage"][name];
    let pct = num(&w["percent"])?;
    let resets = w["resetsAt"].as_str().and_then(clock::parse_rfc3339).or_else(|| num(&w["resetsAt"]).map(clock::normalize_epoch));
    Some(Limit::new(id, label, pct, resets, Some(secs)))
}

/// `{"usage":{"rolling":{"percent","resetsAt"},"weekly":{…},"monthly":{…}}}` — percent is the share used.
pub fn parse(body: &str) -> Result<Usage, FetchError> {
    let json = parse_json(body)?;
    let limits: Vec<Limit> = [
        window(&json, "rolling", "rolling", "limit.5h", 5 * HOUR),
        window(&json, "weekly", "weekly", "limit.weekly", 7 * DAY),
        window(&json, "monthly", "monthly", "limit.monthly", 30 * DAY),
    ]
    .into_iter()
    .flatten()
    .collect();
    if limits.is_empty() {
        return Err(FetchError::Failed("no usage in response".into()));
    }
    Ok(Usage { plan: Some("Go".into()), limits })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_usage() {
        let body = r#"{"usage":{"rolling":{"percent":38,"resetsAt":"2026-11-16T17:00:00Z"},
                               "weekly":{"percent":22.5,"resetsAt":1795277045000},
                               "monthly":{"percent":"4","resetsAt":null}}}"#;
        let u = parse(body).unwrap();
        assert_eq!(u.limits.len(), 3);
        assert_eq!((u.limits[0].id.as_str(), u.limits[0].used_pct, u.limits[0].resets_at), ("rolling", 38.0, Some(1_794_848_400)));
        assert_eq!((u.limits[1].id.as_str(), u.limits[1].used_pct, u.limits[1].resets_at), ("weekly", 22.5, Some(1_795_277_045)));
        assert_eq!((u.limits[2].id.as_str(), u.limits[2].used_pct, u.limits[2].resets_at), ("monthly", 4.0, None));
    }

    #[test]
    fn missing_windows_are_skipped_and_empty_fails() {
        let u = parse(r#"{"usage":{"rolling":{"percent":0,"resetsAt":null}}}"#).unwrap();
        assert_eq!(u.limits.len(), 1);
        assert_eq!(u.limits[0].used_pct, 0.0);
        assert!(parse(r#"{"usage":{}}"#).is_err());
        assert!(parse("nope").is_err());
    }

    fn db(name: &str, entry: Value) -> (String, Value) {
        (name.to_string(), entry)
    }

    #[test]
    fn an_oc_sk_key_always_wins() {
        let auth = json!({"opencode-go": {"type": "api", "key": "plain-key"}, "opencode": {"type": "api", "key": "oc_sk_from_auth"}});
        let database = [db("opencode-go", json!({"type": "oauth", "access": "tok", "expires": 1}))];
        assert_eq!(choose(Some(&auth), &database), Some(Credential::ApiKey("oc_sk_from_auth".into())));
        let database = [db("opencode", json!({"type": "api", "key": "oc_sk_from_db"}))];
        assert_eq!(choose(None, &database), Some(Credential::ApiKey("oc_sk_from_db".into())));
    }

    #[test]
    fn then_the_go_api_key_from_auth_json() {
        let auth = json!({"opencode-go": {"type": "api", "key": "plain-key"}});
        let database = [db("opencode-go", json!({"type": "oauth", "access": "tok", "expires": 1}))];
        assert_eq!(choose(Some(&auth), &database), Some(Credential::ApiKey("plain-key".into())));
    }

    #[test]
    fn then_database_oauth_go_before_plain() {
        let database = [db("opencode", json!({"type": "oauth", "access": "plain", "expires": 5})), db("opencode-go", json!({"type": "oauth", "access": "go", "expires": 9}))];
        assert_eq!(choose(None, &database), Some(Credential::OAuth { access: "go".into(), expires_ms: Some(9.0) }));
        assert_eq!(choose(None, &database[..1]), Some(Credential::OAuth { access: "plain".into(), expires_ms: Some(5.0) }));
    }

    #[test]
    fn finally_auth_json_oauth() {
        let auth = json!({"opencode": {"type": "oauth", "access": "a", "refresh": "r", "expires": 7}});
        assert_eq!(choose(Some(&auth), &[]), Some(Credential::OAuth { access: "a".into(), expires_ms: Some(7.0) }));
        assert_eq!(choose(Some(&json!({})), &[]), None);
        assert_eq!(choose(None, &[]), None);
    }
}
