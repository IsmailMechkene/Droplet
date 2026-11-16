//! Time helpers. Everything is Unix seconds (UTC); only display code converts to local time.

use std::time::{SystemTime, UNIX_EPOCH};

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// Days since 1970-01-01 for a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Parses an RFC 3339 timestamp such as "2026-11-16T15:04:05.123Z" or
/// "2026-11-16T15:04:05+01:00" into Unix seconds.
pub fn parse_rfc3339(text: &str) -> Option<i64> {
    let b = text.trim().as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't' | b' ') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |from: usize, to: usize| -> Option<i64> { std::str::from_utf8(&b[from..to]).ok()?.parse().ok() };
    let (year, month, day) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hour, minute, second) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let mut i = 19;
    if b[i] == b'.' {
        i += 1;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    }
    let offset = match *b.get(i)? {
        b'Z' | b'z' => 0,
        sign @ (b'+' | b'-') => {
            if b.len() < i + 6 || b[i + 3] != b':' {
                return None;
            }
            let secs = num(i + 1, i + 3)? * 3600 + num(i + 4, i + 6)? * 60;
            if sign == b'+' { secs } else { -secs }
        }
        _ => return None,
    };
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

/// Accepts Unix seconds or milliseconds (anything above year ~5000 in seconds is taken as ms).
pub fn normalize_epoch(value: f64) -> i64 {
    if value > 1.0e11 {
        (value / 1000.0) as i64
    } else {
        value as i64
    }
}

/// Minutes since local midnight and weekday (0 = Monday) for a Unix time, given the local UTC offset.
pub fn local_minutes_and_weekday(unix: i64, offset_secs: i64) -> (u32, u32) {
    let local = unix + offset_secs;
    let days = local.div_euclid(86_400);
    let minutes = (local.rem_euclid(86_400) / 60) as u32;
    // 1970-01-01 was a Thursday (3 when Monday is 0).
    let weekday = (days + 3).rem_euclid(7) as u32;
    (minutes, weekday)
}

/// "16:20" for a Unix time in local time.
pub fn hhmm(unix: i64, offset_secs: i64) -> String {
    let (minutes, _) = local_minutes_and_weekday(unix, offset_secs);
    format!("{:02}:{:02}", minutes / 60, minutes % 60)
}

/// "2026-11-16 15:04:05" in local time, for logs and CSV.
pub fn datetime(unix: i64, offset_secs: i64) -> String {
    let local = unix + offset_secs;
    let days = local.div_euclid(86_400);
    let secs = local.rem_euclid(86_400);
    // Inverse of days_from_civil.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", secs / 3600, secs % 3600 / 60, secs % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rfc3339() {
        assert_eq!(parse_rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_rfc3339("2026-11-16T15:04:05Z"), Some(1_794_841_445));
        assert_eq!(parse_rfc3339("2026-11-16T15:04:05.987654Z"), Some(1_794_841_445));
        assert_eq!(parse_rfc3339("2026-11-16T16:04:05+01:00"), Some(1_794_841_445));
        assert_eq!(parse_rfc3339("2026-11-16T10:04:05.5-05:00"), Some(1_794_841_445));
        assert_eq!(parse_rfc3339("2024-02-29T12:00:00Z"), Some(1_709_208_000));
        assert_eq!(parse_rfc3339("nonsense"), None);
        assert_eq!(parse_rfc3339("2026-13-16T15:04:05Z"), None);
        assert_eq!(parse_rfc3339("2026-11-16T15:04:05"), None);
    }

    #[test]
    fn formats_local_time() {
        assert_eq!(hhmm(1_794_841_445, 3600), "16:04");
        assert_eq!(datetime(1_794_841_445, 0), "2026-11-16 15:04:05");
        assert_eq!(datetime(1_709_208_000, 0), "2024-02-29 12:00:00");
        // 2026-11-16 is a Monday.
        assert_eq!(local_minutes_and_weekday(1_794_841_445, 0), (15 * 60 + 4, 0));
    }

    #[test]
    fn epoch_units() {
        assert_eq!(normalize_epoch(1_794_841_445.0), 1_794_841_445);
        assert_eq!(normalize_epoch(1_794_841_445_000.0), 1_794_841_445);
    }
}
