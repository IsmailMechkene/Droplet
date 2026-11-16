//! One module per AI tool. Each exposes:
//! - `credential_path()` — the file it reads (shown in Accounts and the audit list),
//! - `fetch()` — read the credential, call the one allowed host, parse,
//! - `parse()` — pure JSON → limits, unit-tested with sample responses.
//!
//! Credentials stay inside `fetch()`: they are never stored, logged or returned.

pub mod claude;
pub mod codex;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::Config;
use crate::net::{Net, NetError};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderId {
    Claude,
    Codex,
    Cursor,
    Opencode,
    Kimi,
    Antigravity,
}

impl ProviderId {
    pub const ALL: [ProviderId; 6] =
        [ProviderId::Claude, ProviderId::Codex, ProviderId::Cursor, ProviderId::Opencode, ProviderId::Kimi, ProviderId::Antigravity];

    /// Config key, event id and history id.
    pub fn key(self) -> &'static str {
        match self {
            ProviderId::Claude => "claude",
            ProviderId::Codex => "codex",
            ProviderId::Cursor => "cursor",
            ProviderId::Opencode => "opencode",
            ProviderId::Kimi => "kimi",
            ProviderId::Antigravity => "antigravity",
        }
    }

    pub fn from_key(key: &str) -> Option<ProviderId> {
        ProviderId::ALL.into_iter().find(|id| id.key() == key)
    }

    /// Short display name (proper nouns, not translated).
    pub fn name(self) -> &'static str {
        match self {
            ProviderId::Claude => "Claude",
            ProviderId::Codex => "Codex",
            ProviderId::Cursor => "Cursor",
            ProviderId::Opencode => "OpenCode",
            ProviderId::Kimi => "Kimi",
            ProviderId::Antigravity => "Antigravity",
        }
    }

    /// The tool whose sign-in is read, for "via …" and "Open … to sign in again".
    pub fn tool(self) -> &'static str {
        match self {
            ProviderId::Claude => "Claude Code",
            ProviderId::Codex => "Codex CLI",
            ProviderId::Cursor => "Cursor",
            ProviderId::Opencode => "OpenCode",
            ProviderId::Kimi => "Kimi Code",
            ProviderId::Antigravity => "agy.exe",
        }
    }

    /// The only hosts this provider's credential may be sent to.
    pub fn hosts(self) -> &'static [&'static str] {
        match self {
            ProviderId::Claude => &["api.anthropic.com"],
            ProviderId::Codex => &["chatgpt.com"],
            ProviderId::Cursor => &["api2.cursor.sh", "cursor.com"],
            ProviderId::Opencode => &["opencode.ai"],
            ProviderId::Kimi => &["api.kimi.com"],
            ProviderId::Antigravity => &[],
        }
    }

    /// Window id drawn as the thin weekly ring, if the provider has one.
    pub fn weekly_window(self) -> Option<&'static str> {
        match self {
            ProviderId::Claude => Some("weekly"),
            ProviderId::Codex => Some("secondary"),
            ProviderId::Opencode => Some("weekly"),
            ProviderId::Kimi => Some("7d"),
            ProviderId::Cursor | ProviderId::Antigravity => None,
        }
    }
}

/// One usage limit ("window") of a provider.
#[derive(Debug, Clone, PartialEq)]
pub struct Limit {
    /// Stable id within the provider, e.g. "session". Stored in history.
    pub id: String,
    /// Locale key such as "limit.5h", or a literal name (Antigravity model groups).
    pub label: String,
    /// 0..=100, as reported. Never computed from guesses.
    pub used_pct: f64,
    /// Unix seconds.
    pub resets_at: Option<i64>,
    /// Length of the window, when known. Needed for even pace.
    pub window_secs: Option<i64>,
}

impl Limit {
    pub fn new(id: &str, label: &str, used_pct: f64, resets_at: Option<i64>, window_secs: Option<i64>) -> Limit {
        Limit { id: id.into(), label: label.into(), used_pct: used_pct.clamp(0.0, 100.0), resets_at, window_secs }
    }
}

pub const HOUR: i64 = 3600;
pub const DAY: i64 = 86_400;

/// A successful read.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Usage {
    pub plan: Option<String>,
    pub limits: Vec<Limit>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FetchError {
    /// No credential file / executable: the tool is not installed.
    NotInstalled,
    /// The credential is missing, expired or rejected. The user must sign in inside the tool.
    SignIn,
    /// Signed in, but the account has no plan with limits.
    NoSubscription,
    /// HTTP 429. `retry_after` is the server's hint in seconds.
    RateLimited { retry_after: Option<u64> },
    /// Anything else: shown as an error and the last reading is kept as stale.
    Failed(String),
}

impl From<NetError> for FetchError {
    fn from(e: NetError) -> Self {
        FetchError::Failed(e.0)
    }
}

/// Maps the common HTTP outcomes; returns the body on 2xx.
pub fn check_status(status: u16, retry_after: Option<u64>, body: String) -> Result<String, FetchError> {
    match status {
        200..=299 => Ok(body),
        401 | 403 => Err(FetchError::SignIn),
        429 => Err(FetchError::RateLimited { retry_after }),
        other => Err(FetchError::Failed(format!("HTTP {other}"))),
    }
}

pub fn credential_path(id: ProviderId, config: &Config) -> Option<PathBuf> {
    let _ = config;
    match id {
        ProviderId::Claude => claude::credential_path(),
        ProviderId::Codex => codex::credential_path(),
        _ => None,
    }
}

/// Whether this provider is implemented in the current build stage.
pub fn is_available(id: ProviderId) -> bool {
    matches!(id, ProviderId::Claude | ProviderId::Codex)
}

pub fn fetch(id: ProviderId, net: &Net, config: &Config) -> Result<Usage, FetchError> {
    let _ = config;
    match id {
        ProviderId::Claude => claude::fetch(net),
        ProviderId::Codex => codex::fetch(net),
        _ => Err(FetchError::NotInstalled),
    }
}

// ---- small JSON helpers shared by the parsers ----

/// A number that may arrive as a JSON number or a numeric string.
pub fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

pub fn parse_json(body: &str) -> Result<Value, FetchError> {
    serde_json::from_str(body).map_err(|e| FetchError::Failed(format!("unexpected response: {e}")))
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").map(PathBuf::from)
}

/// Reads a small credential file. Missing file → `NotInstalled`.
pub fn read_credential_file(path: &std::path::Path) -> Result<Value, FetchError> {
    let text = std::fs::read_to_string(path).map_err(|_| FetchError::NotInstalled)?;
    serde_json::from_str(&text).map_err(|_| FetchError::SignIn)
}
