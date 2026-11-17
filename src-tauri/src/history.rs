//! Usage history in %APPDATA%\Droplet\history.db (SQLite, Droplet's own file).
//! One row per limit per successful reading. Used for sparklines, for pace
//! across restarts, the weekly summary and CSV export. Pruned by retention.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};

use crate::clock;
use crate::providers::{ProviderId, Usage, DAY, HOUR};

pub const FILE_NAME: &str = "history.db";

pub struct History {
    db: Connection,
}

/// One stored reading.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub provider: String,
    pub window_id: String,
    pub used_pct: f64,
    pub resets_at: Option<i64>,
    pub fetched_at: i64,
}

impl History {
    pub fn open(dir: &Path) -> rusqlite::Result<History> {
        Self::with(Connection::open(dir.join(FILE_NAME))?)
    }

    #[cfg(test)]
    pub fn in_memory() -> rusqlite::Result<History> {
        Self::with(Connection::open_in_memory()?)
    }

    fn with(db: Connection) -> rusqlite::Result<History> {
        db.execute_batch(
            "PRAGMA journal_mode = WAL;
             CREATE TABLE IF NOT EXISTS readings (
                 provider   TEXT    NOT NULL,
                 window_id  TEXT    NOT NULL,
                 used_pct   REAL    NOT NULL,
                 resets_at  INTEGER,
                 fetched_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS readings_by_time ON readings (provider, window_id, fetched_at);
             CREATE TABLE IF NOT EXISTS meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);",
        )?;
        Ok(History { db })
    }

    pub fn record(&self, id: ProviderId, usage: &Usage, now: i64) -> rusqlite::Result<()> {
        let mut insert = self.db.prepare_cached("INSERT INTO readings VALUES (?1, ?2, ?3, ?4, ?5)")?;
        for limit in &usage.limits {
            insert.execute(params![id.key(), limit.id, limit.used_pct, limit.resets_at, now])?;
        }
        Ok(())
    }

    /// Deletes rows older than `days` (0 deletes everything).
    pub fn prune(&self, days: u32, now: i64) -> rusqlite::Result<usize> {
        let cutoff = if days == 0 { i64::MAX } else { now - days as i64 * DAY };
        self.db.execute("DELETE FROM readings WHERE fetched_at < ?1", [cutoff])
    }

    pub fn clear(&self) -> rusqlite::Result<()> {
        self.db.execute_batch("DELETE FROM readings; DELETE FROM meta WHERE key LIKE 'summary%'; VACUUM;")
    }

    /// Readings of one limit since `since`, oldest first.
    pub fn series(&self, id: &str, window_id: &str, since: i64) -> Vec<(i64, f64, Option<i64>)> {
        let Ok(mut q) = self.db.prepare_cached(
            "SELECT fetched_at, used_pct, resets_at FROM readings WHERE provider = ?1 AND window_id = ?2 AND fetched_at >= ?3 ORDER BY fetched_at",
        ) else {
            return Vec::new();
        };
        q.query_map(params![id, window_id, since], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }

    /// The window ids stored for a provider since `since`.
    pub fn windows(&self, id: &str, since: i64) -> Vec<String> {
        let Ok(mut q) = self.db.prepare_cached("SELECT DISTINCT window_id FROM readings WHERE provider = ?1 AND fetched_at >= ?2") else {
            return Vec::new();
        };
        q.query_map(params![id, since], |r| r.get(0)).map(|rows| rows.flatten().collect()).unwrap_or_default()
    }

    pub fn all_rows(&self) -> Vec<Row> {
        let Ok(mut q) = self.db.prepare("SELECT provider, window_id, used_pct, resets_at, fetched_at FROM readings ORDER BY fetched_at, provider, window_id") else {
            return Vec::new();
        };
        q.query_map([], |r| Ok(Row { provider: r.get(0)?, window_id: r.get(1)?, used_pct: r.get(2)?, resets_at: r.get(3)?, fetched_at: r.get(4)? }))
            .map(|rows| rows.flatten().collect())
            .unwrap_or_default()
    }

    pub fn meta(&self, key: &str) -> Option<String> {
        self.db.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0)).ok()
    }

    pub fn set_meta(&self, key: &str, value: &str) {
        let _ = self.db.execute("INSERT OR REPLACE INTO meta VALUES (?1, ?2)", [key, value]);
    }
}

/// 24 hourly points for the last day (the last reading in each hour, carried
/// forward). Empty unless there are at least two readings, so a sparkline is
/// never drawn from a single point.
pub fn sparkline(readings: &[(i64, f64, Option<i64>)], now: i64) -> Vec<f64> {
    let start = now - 24 * HOUR;
    let recent: Vec<&(i64, f64, Option<i64>)> = readings.iter().filter(|r| r.0 > start && r.0 <= now).collect();
    if recent.len() < 2 {
        return Vec::new();
    }
    let mut points = Vec::with_capacity(24);
    let mut last: Option<f64> = readings.iter().filter(|r| r.0 <= start).last().map(|r| r.1);
    let mut i = 0;
    for hour in 1..=24 {
        let end = start + hour * HOUR;
        while i < recent.len() && recent[i].0 <= end {
            last = Some(recent[i].1);
            i += 1;
        }
        // Before the first reading of the day, start from the first value.
        points.push(last.unwrap_or(recent[0].1));
    }
    points
}

fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n']) { format!("\"{}\"", text.replace('"', "\"\"")) } else { text.to_string() }
}

pub fn to_csv(rows: &[Row], utc_offset: i64) -> String {
    let mut out = String::from("time,provider,limit,used_percent,resets_at\r\n");
    for r in rows {
        let resets = r.resets_at.map(|t| clock::datetime(t, utc_offset)).unwrap_or_default();
        out += &format!(
            "{},{},{},{},{}\r\n",
            clock::datetime(r.fetched_at, utc_offset),
            csv_field(&r.provider),
            csv_field(&r.window_id),
            r.used_pct,
            resets
        );
    }
    out
}

/// A file name in `dir` that does not exist yet: droplet-history-2026-11-17.csv, then (2), (3)…
pub fn export_path(dir: &Path, now: i64, utc_offset: i64) -> PathBuf {
    let day = &clock::datetime(now, utc_offset)[..10];
    let mut path = dir.join(format!("droplet-history-{day}.csv"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("droplet-history-{day} ({n}).csv"));
        n += 1;
    }
    path
}

/// Last week's summary per provider: (name, average %, peak %) of the main limit, Monday to Sunday.
pub fn weekly_summary(history: &History, ids: &[(ProviderId, String)], week_start: i64) -> Vec<(&'static str, f64, f64)> {
    ids.iter()
        .filter_map(|(id, window)| {
            let rows = history.series(id.key(), window, week_start);
            let values: Vec<f64> = rows.iter().filter(|r| r.0 < week_start + 7 * DAY).map(|r| r.1).collect();
            if values.is_empty() {
                return None;
            }
            let avg = values.iter().sum::<f64>() / values.len() as f64;
            let peak = values.iter().cloned().fold(0.0, f64::max);
            Some((id.name(), avg, peak))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::Limit;

    const T0: i64 = 1_794_800_000;

    fn usage(pct: f64) -> Usage {
        Usage { plan: None, limits: vec![Limit::new("session", "limit.5h", pct, Some(T0 + 5 * HOUR), Some(5 * HOUR)), Limit::new("weekly", "limit.weekly", 10.0, None, None)] }
    }

    #[test]
    fn records_reads_and_prunes() {
        let h = History::in_memory().unwrap();
        h.record(ProviderId::Claude, &usage(20.0), T0 - 10 * DAY).unwrap();
        h.record(ProviderId::Claude, &usage(30.0), T0 - DAY).unwrap();
        h.record(ProviderId::Codex, &usage(5.0), T0).unwrap();
        assert_eq!(h.all_rows().len(), 6);
        assert_eq!(h.series("claude", "session", 0), vec![(T0 - 10 * DAY, 20.0, Some(T0 + 5 * HOUR)), (T0 - DAY, 30.0, Some(T0 + 5 * HOUR))]);
        let mut windows = h.windows("claude", 0);
        windows.sort();
        assert_eq!(windows, ["session", "weekly"]);

        assert_eq!(h.prune(7, T0).unwrap(), 2, "the 10-day-old reading (two limits) goes");
        assert_eq!(h.series("claude", "session", 0).len(), 1);
        assert_eq!(h.prune(0, T0).unwrap(), 4, "retention off keeps nothing");
        assert!(h.all_rows().is_empty());
    }

    #[test]
    fn clear_and_meta() {
        let h = History::in_memory().unwrap();
        h.record(ProviderId::Claude, &usage(20.0), T0).unwrap();
        h.set_meta("summary_week", "123");
        h.set_meta("other", "x");
        h.clear().unwrap();
        assert!(h.all_rows().is_empty());
        assert_eq!(h.meta("summary_week"), None);
        assert_eq!(h.meta("other").as_deref(), Some("x"));
    }

    #[test]
    fn sparkline_has_24_points_and_needs_two_readings() {
        let now = T0;
        assert!(sparkline(&[(now - HOUR, 10.0, None)], now).is_empty(), "one reading is not a line");
        let readings = [(now - 30 * HOUR, 5.0, None), (now - 20 * HOUR + 60, 10.0, None), (now - 2 * HOUR + 60, 40.0, None), (now - 60, 64.0, None)];
        let points = sparkline(&readings, now);
        assert_eq!(points.len(), 24);
        assert_eq!(points[0], 5.0, "carried in from before the day");
        assert_eq!(points[4], 10.0);
        assert_eq!(points[22], 40.0);
        assert_eq!(points[23], 64.0);
    }

    #[test]
    fn csv_export() {
        let rows = [Row { provider: "claude".into(), window_id: "session".into(), used_pct: 64.5, resets_at: Some(1_794_848_400), fetched_at: 1_794_841_445 }];
        let csv = to_csv(&rows, 0);
        assert_eq!(csv, "time,provider,limit,used_percent,resets_at\r\n2026-11-16 15:04:05,claude,session,64.5,2026-11-16 17:00:00\r\n");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"hi\""), "\"say \"\"hi\"\"\"");
    }

    #[test]
    fn export_names_do_not_overwrite() {
        let dir = std::env::temp_dir().join(format!("droplet-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = export_path(&dir, 1_794_841_445, 0);
        assert!(first.ends_with("droplet-history-2026-11-16.csv"));
        std::fs::write(&first, "x").unwrap();
        assert!(export_path(&dir, 1_794_841_445, 0).ends_with("droplet-history-2026-11-16 (2).csv"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn weekly_summary_per_provider() {
        let h = History::in_memory().unwrap();
        for (d, pct) in [(1, 20.0), (3, 60.0), (5, 100.0), (9, 99.0)] {
            h.record(ProviderId::Claude, &usage(pct), T0 + d * DAY).unwrap();
        }
        let summary = weekly_summary(&h, &[(ProviderId::Claude, "session".into()), (ProviderId::Codex, "primary".into())], T0);
        assert_eq!(summary, vec![("Claude", 60.0, 100.0)], "day 9 is next week; Codex has no data");
    }
}
