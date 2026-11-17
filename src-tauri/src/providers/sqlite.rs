//! Read-only access to other apps' SQLite files. Never opened read-write, so
//! Droplet cannot change, lock for writing, or create journal files for them.

use std::path::Path;

use rusqlite::{Connection, OpenFlags};

/// "C:\dir\file.db" → "file:///C:/dir/file.db" with URI-special characters escaped.
fn file_uri(path: &Path) -> String {
    let mut uri = String::from("file:///");
    for c in path.to_string_lossy().chars() {
        match c {
            '\\' => uri.push('/'),
            '%' => uri.push_str("%25"),
            '?' => uri.push_str("%3F"),
            '#' => uri.push_str("%23"),
            c => uri.push(c),
        }
    }
    uri
}

/// Opens with `?mode=ro`; if that fails (for example the owner holds a lock),
/// retries with `?immutable=1`, which reads the file without any locking.
pub fn open_read_only(path: &Path) -> Option<Connection> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let uri = file_uri(path);
    for mode in ["mode=ro", "immutable=1"] {
        if let Ok(db) = Connection::open_with_flags(format!("{uri}?{mode}"), flags) {
            // Opening is lazy: make sure the file can actually be read.
            if db.query_row("SELECT count(*) FROM sqlite_master", [], |row| row.get::<_, i64>(0)).is_ok() {
                return Some(db);
            }
        }
    }
    None
}

/// First column of the first row as text (TEXT or BLOB), for a query with one parameter.
pub fn text_value(db: &Connection, sql: &str, param: &str) -> Option<String> {
    db.query_row(sql, [param], |row| {
        Ok(match row.get_ref(0)? {
            rusqlite::types::ValueRef::Text(t) | rusqlite::types::ValueRef::Blob(t) => String::from_utf8_lossy(t).into_owned(),
            _ => String::new(),
        })
    })
    .ok()
    .filter(|s| !s.is_empty())
}

/// Every row of a table as its text cells, for tables whose columns are not known in advance.
pub fn text_rows(db: &Connection, table: &str) -> Vec<Vec<String>> {
    // `table` is a fixed name from our own code, never user input.
    let Ok(mut statement) = db.prepare(&format!("SELECT * FROM \"{table}\"")) else { return Vec::new() };
    let columns = statement.column_count();
    let rows = statement.query_map([], |row| {
        Ok((0..columns)
            .filter_map(|i| match row.get_ref(i).ok()? {
                rusqlite::types::ValueRef::Text(t) | rusqlite::types::ValueRef::Blob(t) => Some(String::from_utf8_lossy(t).into_owned()),
                _ => None,
            })
            .collect::<Vec<String>>())
    });
    match rows {
        Ok(rows) => rows.flatten().collect(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_file_uris() {
        assert_eq!(file_uri(Path::new("C:\\Users\\me\\a b\\state.vscdb")), "file:///C:/Users/me/a b/state.vscdb");
        assert_eq!(file_uri(Path::new("C:\\odd#dir\\50%?.db")), "file:///C:/odd%23dir/50%25%3F.db");
    }

    #[test]
    fn opens_read_only_and_cannot_write() {
        let path = std::env::temp_dir().join(format!("droplet-ro-test-{}.db", std::process::id()));
        {
            let db = Connection::open(&path).unwrap();
            db.execute_batch("CREATE TABLE ItemTable(key TEXT, value BLOB); INSERT INTO ItemTable VALUES('k', 'v');").unwrap();
        }
        let db = open_read_only(&path).expect("opens");
        assert_eq!(text_value(&db, "SELECT value FROM ItemTable WHERE key = ?1", "k").as_deref(), Some("v"));
        assert_eq!(text_value(&db, "SELECT value FROM ItemTable WHERE key = ?1", "missing"), None);
        assert_eq!(text_rows(&db, "ItemTable"), vec![vec!["k".to_string(), "v".to_string()]]);
        assert!(db.execute("INSERT INTO ItemTable VALUES('x', 'y')", []).is_err(), "writes must be refused");
        drop(db);
        let _ = std::fs::remove_file(&path);
        assert!(open_read_only(&path).is_none(), "a missing file is not created");
        assert!(!path.exists());
    }
}
