//! Antigravity.
//!
//! The plan was to run `agy.exe --print /usage` in a hidden pseudo console and
//! parse its output. agy.exe 1.5.9 has no usage command: `--print /usage` sends
//! "/usage" to the model as a prompt, which spends quota and lets the agent try
//! to run tools. So `USAGE_ARGS` is empty and Droplet never starts agy.exe.
//! When a real usage command exists, put its arguments there; the runner
//! (hidden ConPTY, Job Object, 20 s, 64 KB) and the parser below are ready.
//!
//! agy.exe uses about 200 MB for about 12 s, so the scheduler only runs it at
//! startup, on manual refresh and when its card opens, at most once per 10 min.

use std::path::PathBuf;
use std::time::Duration;

use super::{home_dir, FetchError, Limit, Usage, DAY, HOUR};
use crate::clock;
use crate::config::Config;
use crate::win::conpty;

const TIMEOUT: Duration = Duration::from_secs(20);
const MAX_OUTPUT: usize = 64 * 1024;
pub const MIN_INTERVAL_SECS: i64 = 10 * 60;

/// Where agy.exe is: the settings override, the two usual install folders, then PATH.
pub fn executable(config: &Config) -> Option<PathBuf> {
    let custom = config.providers.antigravity.agy_path.trim();
    if !custom.is_empty() {
        let path = PathBuf::from(custom);
        return path.is_file().then_some(path);
    }
    let mut candidates = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local).join("agy").join("bin").join("agy.exe"));
    }
    if let Some(home) = home_dir() {
        candidates.push(home.join(".gemini").join("bin").join("agy.exe"));
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(std::env::split_paths(&path).map(|dir| dir.join("agy.exe")));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// Arguments that make agy.exe print its usage without talking to a model.
/// None known yet (see the module comment), so agy.exe is never started.
const USAGE_ARGS: Option<&[&str]> = None;

pub fn fetch(config: &Config) -> Result<Usage, FetchError> {
    let exe = executable(config).ok_or(FetchError::NotInstalled)?;
    let Some(args) = USAGE_ARGS else {
        return Err(FetchError::Failed("This agy.exe has no usage command yet, so Droplet does not run it".into()));
    };
    let output = conpty::run_hidden(&exe, args, TIMEOUT, MAX_OUTPUT)
        .ok_or_else(|| FetchError::Failed("could not start agy.exe".into()))?;
    parse(&output, clock::now())
}

/// Removes ANSI escape sequences (colours, cursor moves, window titles) and carriage returns.
pub fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                // CSI: ESC [ … final byte in @..~
                Some('[') => {
                    for n in chars.by_ref() {
                        if ('@'..='~').contains(&n) {
                            break;
                        }
                    }
                }
                // OSC: ESC ] … terminated by BEL or ESC \
                Some(']') => {
                    while let Some(n) = chars.next() {
                        if n == '\u{7}' || (n == '\u{1b}' && chars.next_if_eq(&'\\').is_some()) {
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// "3h 20m", "2d 4h", "45m", "1 hour 5 minutes" → seconds.
fn duration_secs(text: &str) -> Option<i64> {
    let mut total = 0i64;
    let mut number = String::new();
    let mut found = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c.is_ascii_digit() {
            number.push(c);
            continue;
        }
        if number.is_empty() {
            continue;
        }
        if c == ' ' && chars.peek().is_some_and(|n| n.is_ascii_alphabetic()) {
            continue; // "1 hour"
        }
        let n: i64 = number.parse().ok()?;
        number.clear();
        let unit = match c.to_ascii_lowercase() {
            'd' => DAY,
            'h' => HOUR,
            'm' => 60,
            's' => 1,
            _ => continue,
        };
        total += n * unit;
        found = true;
        // Skip the rest of the unit word ("hours", "min").
        while chars.peek().is_some_and(|n| n.is_ascii_alphabetic()) {
            chars.next();
        }
    }
    found.then_some(total)
}

/// The first "NN%" or "NN.N%" in a line, with the text before it.
fn percent_in(line: &str) -> Option<(f64, &str)> {
    let end = line.find('%')?;
    let start = line[..end].rfind(|c: char| !(c.is_ascii_digit() || c == '.')).map_or(0, |i| i + 1);
    let value: f64 = line[start..end].parse().ok()?;
    (0.0..=100.0).contains(&value).then_some((value, &line[..start]))
}

fn slug(name: &str) -> String {
    let mut id: String = name.to_ascii_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    while id.contains("__") {
        id = id.replace("__", "_");
    }
    id.trim_matches('_').chars().take(48).collect()
}

/// One limit per model group: a line with a name and a percentage. The reset
/// time is taken from the same line or the next one. A percentage described as
/// "left" / "remaining" is converted to used.
pub fn parse(output: &str, now: i64) -> Result<Usage, FetchError> {
    let text = strip_ansi(output);
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let mut limits: Vec<Limit> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some((pct, before)) = percent_in(line) else { continue };
        // The name is the text before the number, without bar characters and separators.
        let name: String = before.chars().filter(|c| c.is_alphanumeric() || matches!(c, ' ' | '.' | '-' | '/')).collect::<String>().trim().to_string();
        let id = slug(&name);
        if id.is_empty() || !name.chars().any(|c| c.is_alphabetic()) || limits.iter().any(|l| l.id == id) {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        let used = if lower.contains("left") || lower.contains("remaining") { 100.0 - pct } else { pct };
        let reset_text = [Some(*line), lines.get(i + 1).copied()]
            .into_iter()
            .flatten()
            .find_map(|l| l.to_ascii_lowercase().find("reset").map(|at| l[at..].to_string()));
        let resets_at = reset_text.as_deref().and_then(duration_secs).map(|secs| now + secs);
        limits.push(Limit::new(&id, &name, used, resets_at, None));
    }
    if limits.is_empty() {
        return Err(FetchError::Failed("agy.exe printed no usage".into()));
    }
    Ok(Usage { plan: None, limits })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_794_841_445;

    #[test]
    fn strips_ansi_sequences() {
        let raw = "\u{1b}]0;agy\u{7}\u{1b}[1m\u{1b}[32mGemini 3 Pro\u{1b}[0m  40% used\r\n\u{1b}[2K\u{1b}[?25hdone";
        assert_eq!(strip_ansi(raw), "Gemini 3 Pro  40% used\ndone");
    }

    #[test]
    fn parses_durations() {
        assert_eq!(duration_secs("resets in 3h 20m"), Some(3 * 3600 + 20 * 60));
        assert_eq!(duration_secs("Resets in 2d 4h"), Some(2 * 86_400 + 4 * 3600));
        assert_eq!(duration_secs("resets in 45m"), Some(45 * 60));
        assert_eq!(duration_secs("resets in 1 hour 5 minutes"), Some(3600 + 300));
        assert_eq!(duration_secs("resets soon"), None);
    }

    #[test]
    fn parses_model_groups() {
        let raw = "\u{1b}[1mUsage\u{1b}[0m\r\n\
                   Gemini 3 Pro      ████░░░░░░  40% used · resets in 3h 20m\r\n\
                   Claude Sonnet 4.5 ██░░░░░░░░  18.5% used\r\n\
                   \u{1b}[2m  resets in 2d 4h\u{1b}[0m\r\n\
                   Flash             ░░░░░░░░░░  0% used\r\n";
        let u = parse(raw, NOW).unwrap();
        let got: Vec<(&str, &str, f64)> = u.limits.iter().map(|l| (l.id.as_str(), l.label.as_str(), l.used_pct)).collect();
        assert_eq!(got, [("gemini_3_pro", "Gemini 3 Pro", 40.0), ("claude_sonnet_4_5", "Claude Sonnet 4.5", 18.5), ("flash", "Flash", 0.0)]);
        assert_eq!(u.limits[0].resets_at, Some(NOW + 3 * 3600 + 20 * 60));
        assert_eq!(u.limits[1].resets_at, Some(NOW + 2 * 86_400 + 4 * 3600), "reset on the following line");
        assert_eq!(u.limits[2].resets_at, None);
    }

    #[test]
    fn remaining_percentages_are_converted_to_used() {
        let u = parse("Pro models: 70% left (resets in 5h)\n", NOW).unwrap();
        assert_eq!(u.limits[0].used_pct, 30.0);
        assert_eq!(u.limits[0].resets_at, Some(NOW + 5 * 3600));
    }

    #[test]
    fn output_without_usage_is_an_error() {
        assert!(parse("Please sign in to continue.\n", NOW).is_err());
        assert!(parse("", NOW).is_err());
        assert!(parse("100% \n 50%\n", NOW).is_err(), "a percentage without a name is not a limit");
    }
}

