//! Miu's own local SQLite database (F0.4): settings now; songs (F3) and sync history (F4)
//! arrive as new entries in `MIGRATIONS`. Never edit a shipped migration — append a new one.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension};
use serde::de::DeserializeOwned;
use serde::Serialize;

pub const MIGRATIONS: &[&str] = &[
    // 1: key/value settings
    "CREATE TABLE settings (key TEXT PRIMARY KEY NOT NULL, value TEXT NOT NULL) STRICT;",
    // 2: watched folders and the songs found in them (F3.1); tags come in a later migration
    "CREATE TABLE watched_folders (
         id INTEGER PRIMARY KEY,
         path TEXT NOT NULL UNIQUE COLLATE NOCASE,
         added_at INTEGER NOT NULL
     ) STRICT;
     CREATE TABLE songs (
         id INTEGER PRIMARY KEY,
         folder_id INTEGER NOT NULL REFERENCES watched_folders(id) ON DELETE CASCADE,
         path TEXT NOT NULL UNIQUE COLLATE NOCASE,
         size INTEGER NOT NULL,
         mtime INTEGER NOT NULL,
         rating INTEGER NOT NULL DEFAULT 0 CHECK (rating IN (-1, 0, 1)),
         added_at INTEGER NOT NULL
     ) STRICT;
     CREATE INDEX songs_folder ON songs(folder_id);",
    // 3: detected file format (F3.2), as `format::Format::name()`
    "ALTER TABLE songs ADD COLUMN format TEXT NOT NULL DEFAULT 'unknown';",
    // 4: tags (F3.3); tags_read = 0 marks rows scanned before tags were read
    "ALTER TABLE songs ADD COLUMN title TEXT;
     ALTER TABLE songs ADD COLUMN artist TEXT;
     ALTER TABLE songs ADD COLUMN album TEXT;
     ALTER TABLE songs ADD COLUMN album_artist TEXT;
     ALTER TABLE songs ADD COLUMN genre TEXT;
     ALTER TABLE songs ADD COLUMN year INTEGER;
     ALTER TABLE songs ADD COLUMN track_no INTEGER;
     ALTER TABLE songs ADD COLUMN track_total INTEGER;
     ALTER TABLE songs ADD COLUMN disc_no INTEGER;
     ALTER TABLE songs ADD COLUMN duration_ms INTEGER;
     ALTER TABLE songs ADD COLUMN has_artwork INTEGER NOT NULL DEFAULT 0;
     ALTER TABLE songs ADD COLUMN tags_read INTEGER NOT NULL DEFAULT 0;",
    // 5: audio quality for the song list; existing songs are read again on the next rescan
    "ALTER TABLE songs ADD COLUMN sample_rate INTEGER;
     ALTER TABLE songs ADD COLUMN bit_depth INTEGER;
     ALTER TABLE songs ADD COLUMN bitrate_kbps INTEGER;
     UPDATE songs SET tags_read = 0;",
    // 6: audio fingerprint for duplicate detection (F3.6); NULL = not hashed yet, '' = unreadable
    "ALTER TABLE songs ADD COLUMN audio_hash TEXT;
     CREATE INDEX songs_audio_hash ON songs(audio_hash);",
    // 7: per-folder legacy tag encoding override (F8.1); NULL = detect
    "ALTER TABLE watched_folders ADD COLUMN tag_encoding TEXT;",
];

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("database schema v{found} is newer than this app supports (v{supported})")]
    NewerSchema { found: i64, supported: i64 },
}

pub type Result<T> = std::result::Result<T, DbError>;

#[derive(Debug)]
pub struct Db {
    conn: Connection,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut conn = Connection::open(path)?;
        conn.query_row("PRAGMA journal_mode = WAL", [], |_| Ok(()))?;
        Self::init(&mut conn)?;
        Ok(Self { conn })
    }

    pub fn open_in_memory() -> Result<Self> {
        let mut conn = Connection::open_in_memory()?;
        Self::init(&mut conn)?;
        Ok(Self { conn })
    }

    fn init(conn: &mut Connection) -> Result<()> {
        conn.pragma_update(None, "foreign_keys", true)?;
        migrate(conn, MIGRATIONS)
    }

    /// For library modules in this crate that run their own queries.
    pub(crate) fn conn(&self) -> &Connection {
        &self.conn
    }

    pub fn schema_version(&self) -> Result<i64> {
        Ok(self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    pub fn setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| r.get(0))
            .optional()?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }

    pub fn setting_json<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>> {
        match self.setting(key)? {
            Some(s) => Ok(Some(serde_json::from_str(&s)?)),
            None => Ok(None),
        }
    }

    pub fn set_setting_json<T: Serialize>(&self, key: &str, value: &T) -> Result<()> {
        self.set_setting(key, &serde_json::to_string(value)?)
    }
}

/// Apply `steps[user_version..]`, each in its own transaction; `user_version` = steps applied.
pub fn migrate(conn: &mut Connection, steps: &[&str]) -> Result<()> {
    let supported = steps.len() as i64;
    let found: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if found > supported {
        return Err(DbError::NewerSchema { found, supported });
    }
    for (i, sql) in steps.iter().enumerate().skip(found as usize) {
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", i as i64 + 1)?;
        tx.commit()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table_exists(db: &Db, name: &str) -> bool {
        db.conn
            .query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name=?1", [name], |r| r.get::<_, i64>(0))
            .unwrap()
            == 1
    }

    #[test]
    fn fresh_database_is_migrated_to_latest_version() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.schema_version().unwrap(), MIGRATIONS.len() as i64);
        assert!(table_exists(&db, "settings"));
    }

    #[test]
    fn reopening_keeps_data_and_does_not_rerun_migrations() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("miumusic.db");
        Db::open(&path).unwrap().set_setting("theme", "dark").unwrap();
        let db = Db::open(&path).unwrap();
        assert_eq!(db.schema_version().unwrap(), MIGRATIONS.len() as i64);
        assert_eq!(db.setting("theme").unwrap().as_deref(), Some("dark"));
    }

    #[test]
    fn file_database_uses_wal_journal() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("miumusic.db")).unwrap();
        let mode: String = db.conn.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, "wal");
    }

    #[test]
    fn creates_parent_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("MiuMusic").join("miumusic.db");
        Db::open(&path).unwrap();
        assert!(path.exists());
    }

    #[test]
    fn refuses_a_database_from_a_newer_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("miumusic.db");
        {
            let conn = rusqlite::Connection::open(&path).unwrap();
            conn.pragma_update(None, "user_version", 999).unwrap();
        }
        let err = Db::open(&path).unwrap_err();
        assert!(matches!(err, DbError::NewerSchema { found: 999, .. }), "{err:?}");
    }

    #[test]
    fn failed_migration_rolls_back_and_keeps_earlier_ones() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        let steps = ["CREATE TABLE a (x)", "CREATE TABLE b (", "CREATE TABLE c (x)"];
        assert!(migrate(&mut conn, &steps).is_err());
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(version, 1);
        let tables: i64 = conn.query_row("SELECT count(*) FROM sqlite_master WHERE name IN ('a','b','c')", [], |r| r.get(0)).unwrap();
        assert_eq!(tables, 1);
    }

    #[test]
    fn missing_setting_is_none() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.setting("nope").unwrap(), None);
    }

    #[test]
    fn setting_overwrites_and_keeps_unicode() {
        let db = Db::open_in_memory().unwrap();
        db.set_setting("last_folder", r"D:\Music").unwrap();
        db.set_setting("last_folder", r"E:\Nhạc Trịnh").unwrap();
        assert_eq!(db.setting("last_folder").unwrap().as_deref(), Some(r"E:\Nhạc Trịnh"));
    }

    #[test]
    fn typed_settings_round_trip_through_json() {
        let db = Db::open_in_memory().unwrap();
        db.set_setting_json("always_on_top", &true).unwrap();
        db.set_setting_json("eq_gains", &vec![-4, 6, 5]).unwrap();
        assert_eq!(db.setting_json::<bool>("always_on_top").unwrap(), Some(true));
        assert_eq!(db.setting_json::<Vec<i32>>("eq_gains").unwrap(), Some(vec![-4, 6, 5]));
        assert_eq!(db.setting_json::<bool>("missing").unwrap(), None);
    }
}

#[cfg(test)]
mod quality_migration_tests {
    use super::*;

    #[test]
    fn upgrading_adds_audio_quality_and_marks_songs_for_a_tag_reread() {
        let mut conn = Connection::open_in_memory().unwrap();
        migrate(&mut conn, &MIGRATIONS[..4]).unwrap();
        conn.execute_batch(
            "INSERT INTO watched_folders (path, added_at) VALUES ('C:/M', 1);
             INSERT INTO songs (folder_id, path, size, mtime, added_at, tags_read) VALUES (1, 'C:/M/a.flac', 1, 1, 1, 1);",
        )
        .unwrap();
        migrate(&mut conn, MIGRATIONS).unwrap();
        let (read, rate): (i64, Option<i64>) =
            conn.query_row("SELECT tags_read, sample_rate FROM songs", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
        assert_eq!((read, rate), (0, None));
    }
}
