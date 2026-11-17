//! "Open log": writes %APPDATA%\Droplet\network-log.txt and opens it with the
//! default editor. It lists the files Droplet reads, the programs it may start,
//! the hosts each sign-in may be sent to, then every recent request (time,
//! provider, method, host + path, status, duration, size). Never headers,
//! tokens or bodies: the in-memory log does not hold them in the first place.

use std::fmt::Write as _;

use crate::clock;
use crate::config::Config;
use crate::net::LogEntry;
use crate::providers::{self, ProviderId};
use crate::win::{self, shell};

pub const FILE_NAME: &str = "network-log.txt";

/// The audit list, shared with Settings and the README.
pub fn audit(config: &Config) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "Files this app reads");
    for id in ProviderId::ALL {
        for path in providers::files(id, config) {
            let state = if path.exists() { "found" } else { "not found" };
            let _ = writeln!(out, "  {:<12} {:<9} {}", id.name(), state, path.display());
        }
    }
    let _ = writeln!(out, "  {:<12} {:<9} %APPDATA%\\Droplet (its own settings, history and this log)", "Droplet", "");
    let _ = writeln!(out);
    let _ = writeln!(out, "Processes this app may start");
    let _ = writeln!(out, "  agy.exe      Antigravity's CLI, hidden, killed after 20 s. Currently never started:");
    let _ = writeln!(out, "               this agy.exe version has no usage command.");
    let _ = writeln!(out, "  Through Windows (no process of Droplet's own): Explorer to show a file, the default");
    let _ = writeln!(out, "  browser for this project's GitHub pages, the default editor for files Droplet wrote.");
    let _ = writeln!(out);
    let _ = writeln!(out, "Where each sign-in may be sent");
    for id in ProviderId::ALL {
        let hosts = id.hosts();
        let _ = writeln!(out, "  {:<12} {}", id.name(), if hosts.is_empty() { "nowhere (no network request)".to_string() } else { hosts.join(", ") });
    }
    out
}

/// The request table, oldest first.
pub fn requests(entries: &[LogEntry], utc_offset: i64) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{:<19}  {:<11}  {:<4}  {:>6}  {:>7}  {:>8}  {}", "time", "provider", "verb", "status", "ms", "bytes", "host + path");
    for e in entries {
        let status = if e.status == 0 { "failed".to_string() } else { e.status.to_string() };
        let _ = writeln!(
            out,
            "{:<19}  {:<11}  {:<4}  {:>6}  {:>7}  {:>8}  {}",
            clock::datetime(e.time, utc_offset),
            e.provider.name(),
            e.method,
            status,
            e.duration_ms,
            e.bytes,
            e.target
        );
    }
    if entries.is_empty() {
        let _ = writeln!(out, "(no requests yet)");
    }
    out
}

pub fn render(config: &Config, entries: &[LogEntry], now: i64, utc_offset: i64) -> String {
    format!(
        "Droplet network log, written {}\r\nThe last {} requests since Droplet started. Never headers, tokens or bodies.\r\n\r\n{}\r\nRequests\r\n{}",
        clock::datetime(now, utc_offset),
        entries.len(),
        audit(config),
        requests(entries, utc_offset)
    )
    .replace("\r\n", "\n")
    .replace('\n', "\r\n")
}

/// Writes the log next to the config and opens it.
pub fn open(dir: &std::path::Path, config: &Config, entries: &[LogEntry]) -> bool {
    let text = render(config, entries, clock::now(), win::utc_offset());
    std::fs::create_dir_all(dir).is_ok() && std::fs::write(dir.join(FILE_NAME), text).is_ok() && shell::open_own_file(dir, FILE_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_files_processes_and_hosts() {
        let text = audit(&Config::default());
        assert!(text.contains("Files this app reads"));
        assert!(text.contains(".credentials.json"));
        assert!(text.contains("Processes this app may start"));
        assert!(text.contains("agy.exe"));
        assert!(text.contains("api.anthropic.com"));
        assert!(text.contains("chatgpt.com"));
    }

    #[test]
    fn request_rows_hold_only_the_allowed_fields() {
        let entries = [LogEntry {
            time: 1_794_841_445,
            provider: ProviderId::Claude,
            method: "GET",
            target: "api.anthropic.com/api/oauth/usage".into(),
            status: 200,
            duration_ms: 312,
            bytes: 640,
        }];
        let table = requests(&entries, 0);
        assert!(table.contains("2026-11-16 15:04:05  Claude       GET      200      312       640  api.anthropic.com/api/oauth/usage"), "{table}");
        let empty = requests(&[], 0);
        assert!(empty.contains("(no requests yet)"));
        let failed = requests(&[LogEntry { status: 0, ..entries[0].clone() }], 0);
        assert!(failed.contains("failed"));
    }

    #[test]
    fn uses_windows_line_endings() {
        let text = render(&Config::default(), &[], 0, 0);
        assert!(text.contains("\r\n"));
        assert!(!text.replace("\r\n", "").contains('\n'));
    }
}
