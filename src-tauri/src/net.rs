//! The only place that talks to the network.
//!
//! Rules enforced here:
//! - a provider's request may only go to that provider's own hosts, over HTTPS,
//! - redirects are never followed (a credential cannot leak to another host),
//! - every request is recorded in the in-memory network log: time, provider,
//!   method, host + path, status, duration and size. Never headers or bodies.

use std::collections::VecDeque;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use ureq::tls::{RootCerts, TlsConfig, TlsProvider};
use ureq::Agent;

use crate::clock;
use crate::providers::ProviderId;

const TIMEOUT: Duration = Duration::from_secs(15);
const MAX_BODY_BYTES: u64 = 1024 * 1024;
pub const LOG_CAPACITY: usize = 500;

#[derive(Debug, Clone, PartialEq)]
pub struct NetError(pub String);

pub struct Response {
    pub status: u16,
    pub body: String,
    /// `Retry-After` in seconds, when the server sent one.
    pub retry_after: Option<u64>,
}

/// One line of the network log.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub time: i64,
    pub provider: ProviderId,
    pub method: &'static str,
    /// Host and path only. Query strings are dropped.
    pub target: String,
    /// HTTP status, or 0 when the request failed before a response.
    pub status: u16,
    pub duration_ms: u64,
    pub bytes: u64,
}

pub struct Net {
    agent: Agent,
    log: Mutex<VecDeque<LogEntry>>,
}

/// Splits an https URL into (host, path-without-query). Anything else is refused.
pub fn split_https_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("https://")?;
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    if host.is_empty() || host.contains('@') || host.contains(':') {
        return None;
    }
    let path = path.split(['?', '#']).next().unwrap_or("/");
    Some((host, path))
}

/// The host allowlist check, separated so it can be unit-tested.
pub fn target_for(provider: ProviderId, url: &str) -> Result<String, NetError> {
    let (host, path) = split_https_url(url).ok_or_else(|| NetError("only https URLs are allowed".into()))?;
    if !provider.hosts().contains(&host) {
        return Err(NetError(format!("{host} is not an allowed host for {}", provider.name())));
    }
    Ok(format!("{host}{path}"))
}

impl Net {
    pub fn new() -> Net {
        let tls = TlsConfig::builder()
            .provider(TlsProvider::NativeTls) // Windows Schannel
            .root_certs(RootCerts::PlatformVerifier) // the Windows certificate store
            .build();
        let agent: Agent = Agent::config_builder()
            .timeout_global(Some(TIMEOUT))
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .proxy(None)
            .user_agent("Droplet")
            .tls_config(tls)
            .build()
            .into();
        Net { agent, log: Mutex::new(VecDeque::with_capacity(LOG_CAPACITY)) }
    }

    pub fn get(&self, provider: ProviderId, url: &str, headers: &[(&str, String)]) -> Result<Response, NetError> {
        self.request(provider, "GET", url, headers, None)
    }

    pub fn post_json(&self, provider: ProviderId, url: &str, headers: &[(&str, String)], body: &str) -> Result<Response, NetError> {
        self.request(provider, "POST", url, headers, Some(body))
    }

    fn request(
        &self,
        provider: ProviderId,
        method: &'static str,
        url: &str,
        headers: &[(&str, String)],
        body: Option<&str>,
    ) -> Result<Response, NetError> {
        let target = target_for(provider, url)?;
        let started = Instant::now();
        let result = self.send(url, headers, body);
        let (status, bytes) = match &result {
            Ok(r) => (r.status, r.body.len() as u64),
            Err(_) => (0, 0),
        };
        self.record(LogEntry {
            time: clock::now(),
            provider,
            method,
            target,
            status,
            duration_ms: started.elapsed().as_millis() as u64,
            bytes,
        });
        result
    }

    /// GET when `body` is `None`, POST otherwise.
    fn send(&self, url: &str, headers: &[(&str, String)], body: Option<&str>) -> Result<Response, NetError> {
        let sent = match body {
            None => {
                let mut req = self.agent.get(url);
                for (name, value) in headers {
                    req = req.header(*name, value.as_str());
                }
                req.call()
            }
            Some(body) => {
                let mut req = self.agent.post(url);
                for (name, value) in headers {
                    req = req.header(*name, value.as_str());
                }
                req.send(body)
            }
        };
        // ureq's error text describes the transport failure; it never contains headers.
        let mut resp = sent.map_err(|e| NetError(short_error(&e.to_string())))?;
        let status = resp.status().as_u16();
        let retry_after = resp.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse().ok());
        let body = resp
            .body_mut()
            .with_config()
            .limit(MAX_BODY_BYTES)
            .read_to_string()
            .map_err(|e| NetError(short_error(&e.to_string())))?;
        Ok(Response { status, body, retry_after })
    }

    fn record(&self, entry: LogEntry) {
        let mut log = self.log.lock().unwrap();
        if log.len() == LOG_CAPACITY {
            log.pop_front();
        }
        log.push_back(entry);
    }

    pub fn log_snapshot(&self) -> Vec<LogEntry> {
        self.log.lock().unwrap().iter().cloned().collect()
    }
}

fn short_error(text: &str) -> String {
    let line = text.lines().next().unwrap_or("network error");
    line.chars().take(120).collect()
}

/// Wait before the next attempt after `consecutive` HTTP 429 answers:
/// 60 s × 2ⁿ capped at 15 min. A longer Retry-After from the server is
/// respected (bounded to one hour so a bad header cannot park a provider).
pub fn backoff_secs(consecutive: u32, retry_after: Option<u64>) -> u64 {
    let doubling = consecutive.saturating_sub(1).min(4);
    let exponential = (60u64 << doubling).min(15 * 60);
    exponential.max(retry_after.unwrap_or(0).min(3600))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_splitting() {
        assert_eq!(split_https_url("https://api.anthropic.com/api/oauth/usage"), Some(("api.anthropic.com", "/api/oauth/usage")));
        assert_eq!(split_https_url("https://cursor.com/api/usage-summary?x=1"), Some(("cursor.com", "/api/usage-summary")));
        assert_eq!(split_https_url("https://chatgpt.com"), Some(("chatgpt.com", "/")));
        assert_eq!(split_https_url("http://chatgpt.com/x"), None);
        assert_eq!(split_https_url("https://user@evil.example/x"), None);
        assert_eq!(split_https_url("https://chatgpt.com:8443/x"), None);
    }

    #[test]
    fn tokens_only_go_to_their_own_host() {
        assert!(target_for(ProviderId::Claude, "https://api.anthropic.com/api/oauth/usage").is_ok());
        assert!(target_for(ProviderId::Claude, "https://chatgpt.com/backend-api/wham/usage").is_err());
        assert!(target_for(ProviderId::Codex, "https://api.anthropic.com/api/oauth/usage").is_err());
        assert!(target_for(ProviderId::Codex, "https://chatgpt.com.evil.example/x").is_err());
        assert!(target_for(ProviderId::Antigravity, "https://example.com/").is_err());
    }

    #[test]
    fn backoff_doubles_and_caps() {
        assert_eq!(backoff_secs(1, None), 60);
        assert_eq!(backoff_secs(2, None), 120);
        assert_eq!(backoff_secs(3, None), 240);
        assert_eq!(backoff_secs(4, None), 480);
        assert_eq!(backoff_secs(5, None), 900);
        assert_eq!(backoff_secs(30, None), 900);
        assert_eq!(backoff_secs(1, Some(300)), 300, "Retry-After is respected");
        assert_eq!(backoff_secs(1, Some(10)), 60);
        assert_eq!(backoff_secs(1, Some(999_999)), 3600, "an absurd Retry-After is bounded");
    }
}
