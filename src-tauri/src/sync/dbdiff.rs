//! Compare two snapshots of a SQLite database table by table (F1.6). Used to see exactly what a
//! sync wrote into MediaLibrary.sqlitedb. Inputs are copied (with -wal/-shm) before opening, so
//! diffing never modifies a backup.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use rusqlite::types::Value;
use rusqlite::Connection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    A,
    B,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableDiff {
    pub name: String,
    pub rows_a: usize,
    pub rows_b: usize,
    pub added: usize,
    pub removed: usize,
    pub changed: usize,
    pub only_in: Option<Side>,
}

impl TableDiff {
    pub fn summary(&self) -> String {
        let side = match self.only_in {
            Some(Side::A) => " [only in A]",
            Some(Side::B) => " [only in B]",
            None => "",
        };
        format!(
            "{}: {} → {} rows (+{} −{} ~{}){side}",
            self.name, self.rows_a, self.rows_b, self.added, self.removed, self.changed
        )
    }

    fn is_unchanged(&self) -> bool {
        self.only_in.is_none() && self.added == 0 && self.removed == 0 && self.changed == 0
    }
}

#[derive(Debug, Default)]
pub struct DbDiff {
    /// Only tables with differences, by name.
    pub tables: Vec<TableDiff>,
}

impl DbDiff {
    pub fn is_empty(&self) -> bool {
        self.tables.is_empty()
    }
}

type Rows = BTreeMap<String, Vec<Value>>;

pub fn diff(a: &Path, b: &Path) -> Result<DbDiff, String> {
    let work = tempfile::tempdir().map_err(|e| e.to_string())?;
    let ca = snapshot(a, &work.path().join("a"))?;
    let cb = snapshot(b, &work.path().join("b"))?;
    let (ta, tb) = (tables(&ca)?, tables(&cb)?);
    let mut out = DbDiff::default();
    for name in ta.union(&tb) {
        let ra = if ta.contains(name) { rows(&ca, name)? } else { Rows::new() };
        let rb = if tb.contains(name) { rows(&cb, name)? } else { Rows::new() };
        let t = TableDiff {
            name: name.clone(),
            rows_a: ra.len(),
            rows_b: rb.len(),
            added: rb.keys().filter(|k| !ra.contains_key(*k)).count(),
            removed: ra.keys().filter(|k| !rb.contains_key(*k)).count(),
            changed: ra.iter().filter(|(k, v)| rb.get(*k).is_some_and(|w| w != *v)).count(),
            only_in: match (ta.contains(name), tb.contains(name)) {
                (true, false) => Some(Side::A),
                (false, true) => Some(Side::B),
                _ => None,
            },
        };
        if !t.is_unchanged() {
            out.tables.push(t);
        }
    }
    Ok(out)
}

/// Copy `db` and its `-wal` / `-shm` into `dir` and open the copy.
pub(crate) fn snapshot(db: &Path, dir: &Path) -> Result<Connection, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let name = db.file_name().ok_or("not a file")?.to_string_lossy().into_owned();
    for suffix in ["", "-wal", "-shm"] {
        let src = db.with_file_name(format!("{name}{suffix}"));
        if src.is_file() {
            std::fs::copy(&src, dir.join(format!("db{suffix}"))).map_err(|e| format!("{}: {e}", src.display()))?;
        }
    }
    Connection::open(dir.join("db")).map_err(|e| e.to_string())
}

fn tables(conn: &Connection) -> Result<BTreeSet<String>, String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'")
        .map_err(|e| e.to_string())?;
    let names = stmt.query_map([], |r| r.get(0)).map_err(|e| e.to_string())?;
    names.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

/// Rows keyed by rowid; tables WITHOUT ROWID are keyed by their whole content.
fn rows(conn: &Connection, table: &str) -> Result<Rows, String> {
    let quoted = format!("\"{}\"", table.replace('"', "\"\""));
    let (sql, keyed) = match conn.prepare(&format!("SELECT rowid, * FROM {quoted}")) {
        Ok(_) => (format!("SELECT rowid, * FROM {quoted}"), true),
        Err(_) => (format!("SELECT * FROM {quoted}"), false),
    };
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let n = stmt.column_count();
    let mut out = Rows::new();
    let mut q = stmt.query([]).map_err(|e| e.to_string())?;
    while let Some(row) = q.next().map_err(|e| e.to_string())? {
        let values: Vec<Value> = (0..n).map(|i| row.get(i)).collect::<Result<_, _>>().map_err(|e| e.to_string())?;
        if keyed {
            let key = format!("{:?}", values[0]);
            out.insert(key, values[1..].to_vec());
        } else {
            out.insert(format!("{values:?}"), values);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Synthetic stand-in for MediaLibrary.sqlitedb (no private data).
    fn make_db(dir: &Path, name: &str, setup: &str) -> PathBuf {
        let path = dir.join(name);
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE item (item_pid INTEGER PRIMARY KEY, title TEXT);
             INSERT INTO item VALUES (1, 'Mùa thu cho em'), (2, 'Diễm xưa'), (3, 'Bolero');
             CREATE TABLE album (album_pid INTEGER PRIMARY KEY, album TEXT);
             INSERT INTO album VALUES (10, 'Tình khúc vàng');",
        )
        .unwrap();
        conn.execute_batch(setup).unwrap();
        path
    }

    #[test]
    fn identical_databases_have_no_differences() {
        let d = tempfile::tempdir().unwrap();
        let a = make_db(d.path(), "a.db", "");
        let b = make_db(d.path(), "b.db", "");
        assert!(diff(&a, &b).unwrap().is_empty());
    }

    #[test]
    fn counts_added_removed_and_changed_rows_per_table() {
        let d = tempfile::tempdir().unwrap();
        let a = make_db(d.path(), "a.db", "");
        let b = make_db(d.path(), "b.db",
            "INSERT INTO item VALUES (4, 'Hà Nội mùa vắng'); DELETE FROM item WHERE item_pid = 3;
             UPDATE item SET title = 'Diễm Xưa' WHERE item_pid = 2;");
        let result = diff(&a, &b).unwrap();
        let item = result.tables.iter().find(|t| t.name == "item").unwrap();
        assert_eq!((item.rows_a, item.rows_b), (3, 3));
        assert_eq!((item.added, item.removed, item.changed), (1, 1, 1));
        assert!(result.tables.iter().all(|t| t.name != "album"), "unchanged tables are left out");
    }

    #[test]
    fn reports_tables_present_on_one_side_only() {
        let d = tempfile::tempdir().unwrap();
        let a = make_db(d.path(), "a.db", "");
        let b = make_db(d.path(), "b.db", "CREATE TABLE lyrics (item_pid INTEGER PRIMARY KEY, text TEXT); INSERT INTO lyrics VALUES (1, '');");
        let lyrics = diff(&a, &b).unwrap().tables.into_iter().find(|t| t.name == "lyrics").unwrap();
        assert_eq!(lyrics.only_in, Some(Side::B));
        assert_eq!(lyrics.added, 1);
    }

    #[test]
    fn diffing_leaves_the_input_files_untouched() {
        let d = tempfile::tempdir().unwrap();
        let a = make_db(d.path(), "a.db", "PRAGMA journal_mode = WAL;");
        let b = make_db(d.path(), "b.db", "");
        let before = std::fs::read(&a).unwrap();
        diff(&a, &b).unwrap();
        assert_eq!(std::fs::read(&a).unwrap(), before);
    }

    #[test]
    fn summary_reads_like_a_report() {
        let t = TableDiff { name: "item".into(), rows_a: 69, rows_b: 70, added: 1, removed: 0, changed: 2, only_in: None };
        assert_eq!(t.summary(), "item: 69 → 70 rows (+1 −0 ~2)");
    }
}
