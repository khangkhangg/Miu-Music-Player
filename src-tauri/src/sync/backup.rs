//! Backup and restore of the phone's music database (F1.6, CLAUDE.md hard rule 1).
//!
//! A backup is `<root>\<udid8>\<YYYYMMDD-HHMMSS>\` holding `MediaLibrary.sqlitedb` (+ `-wal`, `-shm`
//! when present) and `manifest.json`. Restore always backs up the phone's current state first.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::device::files::DeviceFiles;
use crate::util::time::file_stamp;

pub const DB_DIR: &str = "/iTunes_Control/iTunes";
pub const DB_NAME: &str = "MediaLibrary.sqlitedb";
const COMPANIONS: [&str; 2] = ["MediaLibrary.sqlitedb-wal", "MediaLibrary.sqlitedb-shm"];
const MANIFEST: &str = "manifest.json";

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("the phone has no music database at {DB_DIR}/{DB_NAME}")]
    NoDatabase,
    #[error("phone: {0}")]
    Device(String),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("backup is damaged: {0}")]
    Corrupt(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub created_unix: u64,
    pub source_dir: String,
    /// File name → size in bytes.
    pub files: BTreeMap<String, u64>,
}

impl Manifest {
    pub fn load(dir: &Path) -> Result<Self, BackupError> {
        Ok(serde_json::from_slice(&fs::read(dir.join(MANIFEST))?)?)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Backup {
    pub dir: PathBuf,
    pub manifest: Manifest,
}

fn phone_dir(root: &Path, udid: &str) -> PathBuf {
    root.join(udid.chars().take(8).collect::<String>())
}

/// Copy the music database files off the phone. Leaves nothing behind if any step fails.
pub fn backup(phone: &dyn DeviceFiles, root: &Path, udid: &str, now_unix: u64) -> Result<Backup, BackupError> {
    let present = phone.list(DB_DIR).map_err(BackupError::Device)?;
    if !present.iter().any(|n| n == DB_NAME) {
        return Err(BackupError::NoDatabase);
    }
    let dir = phone_dir(root, udid).join(file_stamp(now_unix));
    fs::create_dir_all(&dir)?;
    match copy_off_phone(phone, &dir, &present, now_unix) {
        Ok(manifest) => Ok(Backup { dir, manifest }),
        Err(e) => {
            let _ = fs::remove_dir_all(&dir);
            Err(e)
        }
    }
}

fn copy_off_phone(phone: &dyn DeviceFiles, dir: &Path, present: &[String], now_unix: u64) -> Result<Manifest, BackupError> {
    let mut files = BTreeMap::new();
    for name in std::iter::once(DB_NAME).chain(COMPANIONS) {
        if !present.iter().any(|n| n == name) {
            continue;
        }
        let data = phone.read(&format!("{DB_DIR}/{name}")).map_err(BackupError::Device)?;
        write_atomic(&dir.join(name), &data)?;
        files.insert(name.to_string(), data.len() as u64);
    }
    let manifest = Manifest { created_unix: now_unix, source_dir: DB_DIR.into(), files };
    write_atomic(&dir.join(MANIFEST), &serde_json::to_vec_pretty(&manifest)?)?;
    Ok(manifest)
}

/// Newest complete backup for this phone.
pub fn latest_backup(root: &Path, udid: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(phone_dir(root, udid))
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join(MANIFEST).is_file())
        .collect();
    dirs.sort();
    dirs.pop()
}

/// Put a backup back on the phone. First checks the backup against its manifest, then backs up
/// what is on the phone now (returned, so it can be restored in turn), then writes.
pub fn restore(phone: &dyn DeviceFiles, from: &Path, root: &Path, udid: &str, now_unix: u64) -> Result<Backup, BackupError> {
    let manifest = Manifest::load(from)?;
    let mut contents = BTreeMap::new();
    for (name, size) in &manifest.files {
        let data = fs::read(from.join(name)).map_err(|_| BackupError::Corrupt(format!("{name} is missing")))?;
        if data.len() as u64 != *size {
            return Err(BackupError::Corrupt(format!("{name} is {} bytes, manifest says {size}", data.len())));
        }
        contents.insert(name.clone(), data);
    }
    if !contents.contains_key(DB_NAME) {
        return Err(BackupError::Corrupt(format!("no {DB_NAME}")));
    }

    let safety = backup(phone, root, udid, now_unix)?;

    let on_phone = phone.list(DB_DIR).map_err(BackupError::Device)?;
    for stale in COMPANIONS.iter().filter(|c| !contents.contains_key(**c) && on_phone.iter().any(|n| n == *c)) {
        phone.remove(&format!("{DB_DIR}/{stale}")).map_err(BackupError::Device)?;
    }
    for name in std::iter::once(DB_NAME).chain(COMPANIONS) {
        if let Some(data) = contents.get(name) {
            phone.write(&format!("{DB_DIR}/{name}"), data).map_err(BackupError::Device)?;
        }
    }
    Ok(safety)
}

/// Number of music items (`media_type = 8`) in a backup, read from a copy so the backup is untouched.
pub fn song_count(dir: &Path) -> Result<i64, BackupError> {
    let work = tempfile::tempdir()?;
    let conn = crate::sync::dbdiff::snapshot(&dir.join(DB_NAME), work.path()).map_err(BackupError::Corrupt)?;
    conn.query_row("SELECT count(*) FROM item WHERE media_type = 8", [], |r| r.get(0))
        .map_err(|e| BackupError::Corrupt(e.to_string()))
}

fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("partial");
    fs::write(&tmp, data)?;
    fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// In-memory stand-in for the phone's media partition.
    #[derive(Default)]
    struct FakePhone {
        files: RefCell<BTreeMap<String, Vec<u8>>>,
        fail_reads_of: Option<&'static str>,
    }

    impl FakePhone {
        fn with(files: &[(&str, &[u8])]) -> Self {
            let p = FakePhone::default();
            for (path, data) in files {
                p.files.borrow_mut().insert(path.to_string(), data.to_vec());
            }
            p
        }
        fn get(&self, path: &str) -> Option<Vec<u8>> {
            self.files.borrow().get(path).cloned()
        }
    }

    impl DeviceFiles for FakePhone {
        fn list(&self, dir: &str) -> Result<Vec<String>, String> {
            let prefix = format!("{dir}/");
            Ok(self.files.borrow().keys().filter_map(|k| k.strip_prefix(&prefix)).filter(|n| !n.contains('/')).map(String::from).collect())
        }
        fn read(&self, path: &str) -> Result<Vec<u8>, String> {
            if self.fail_reads_of == Some(path) {
                return Err("cable pulled".into());
            }
            self.get(path).ok_or_else(|| format!("{path} not found"))
        }
        fn write(&self, path: &str, data: &[u8]) -> Result<(), String> {
            self.files.borrow_mut().insert(path.into(), data.into());
            Ok(())
        }
        fn remove(&self, path: &str) -> Result<(), String> {
            self.files.borrow_mut().remove(path).map(|_| ()).ok_or_else(|| format!("{path} not found"))
        }
    }

    const DB: &str = "/iTunes_Control/iTunes/MediaLibrary.sqlitedb";
    const WAL: &str = "/iTunes_Control/iTunes/MediaLibrary.sqlitedb-wal";
    const SHM: &str = "/iTunes_Control/iTunes/MediaLibrary.sqlitedb-shm";
    const T0: u64 = 1_792_465_445; // 2026-10-20T03:04:05Z

    fn phone() -> FakePhone {
        FakePhone::with(&[(DB, b"db-v1"), (WAL, b"wal-v1"), (SHM, b"shm-v1"), ("/iTunes_Control/iTunes/Other.plist", b"x")])
    }

    #[test]
    fn backup_folder_is_named_after_the_phone_and_utc_time() {
        let root = tempfile::tempdir().unwrap();
        let b = backup(&phone(), root.path(), "e51e30a5aaaabbbbccccdddd", T0).unwrap();
        assert_eq!(b.dir, root.path().join("e51e30a5").join("20261020-030405"));
    }

    #[test]
    fn backup_copies_the_database_and_its_wal_and_shm() {
        let root = tempfile::tempdir().unwrap();
        let b = backup(&phone(), root.path(), "e51e30a5", T0).unwrap();
        assert_eq!(fs::read(b.dir.join("MediaLibrary.sqlitedb")).unwrap(), b"db-v1");
        assert_eq!(fs::read(b.dir.join("MediaLibrary.sqlitedb-wal")).unwrap(), b"wal-v1");
        assert_eq!(fs::read(b.dir.join("MediaLibrary.sqlitedb-shm")).unwrap(), b"shm-v1");
        assert!(!b.dir.join("Other.plist").exists());
    }

    #[test]
    fn manifest_records_files_and_sizes() {
        let root = tempfile::tempdir().unwrap();
        let b = backup(&phone(), root.path(), "e51e30a5", T0).unwrap();
        let m = Manifest::load(&b.dir).unwrap();
        assert_eq!(m.files, BTreeMap::from([
            ("MediaLibrary.sqlitedb".to_string(), 5),
            ("MediaLibrary.sqlitedb-shm".to_string(), 6),
            ("MediaLibrary.sqlitedb-wal".to_string(), 6),
        ]));
        assert_eq!(m.created_unix, T0);
    }

    #[test]
    fn wal_and_shm_are_optional() {
        let root = tempfile::tempdir().unwrap();
        let b = backup(&FakePhone::with(&[(DB, b"db-only")]), root.path(), "e51e30a5", T0).unwrap();
        assert_eq!(Manifest::load(&b.dir).unwrap().files.len(), 1);
    }

    #[test]
    fn no_database_means_no_backup_and_no_folder() {
        let root = tempfile::tempdir().unwrap();
        let err = backup(&FakePhone::with(&[(WAL, b"w")]), root.path(), "e51e30a5", T0).unwrap_err();
        assert!(matches!(err, BackupError::NoDatabase), "{err:?}");
        assert!(!root.path().join("e51e30a5").join("20261020-030405").exists());
    }

    #[test]
    fn a_failed_read_removes_the_partial_backup() {
        let root = tempfile::tempdir().unwrap();
        let mut p = phone();
        p.fail_reads_of = Some(SHM);
        assert!(matches!(backup(&p, root.path(), "e51e30a5", T0), Err(BackupError::Device(_))));
        assert!(!root.path().join("e51e30a5").join("20261020-030405").exists());
    }

    #[test]
    fn latest_backup_is_the_newest_folder() {
        let root = tempfile::tempdir().unwrap();
        backup(&phone(), root.path(), "e51e30a5", T0).unwrap();
        let newer = backup(&phone(), root.path(), "e51e30a5", T0 + 60).unwrap();
        assert_eq!(latest_backup(root.path(), "e51e30a5").unwrap(), newer.dir);
    }

    #[test]
    fn restore_backs_up_the_current_state_first_then_writes_the_backup() {
        let root = tempfile::tempdir().unwrap();
        let p = phone();
        let old = backup(&p, root.path(), "e51e30a5", T0).unwrap();
        p.write(DB, b"db-v2-broken").unwrap();

        let safety = restore(&p, &old.dir, root.path(), "e51e30a5", T0 + 60).unwrap();

        assert_eq!(p.get(DB).unwrap(), b"db-v1");
        assert_eq!(p.get(WAL).unwrap(), b"wal-v1");
        assert_eq!(fs::read(safety.dir.join("MediaLibrary.sqlitedb")).unwrap(), b"db-v2-broken");
    }

    #[test]
    fn restore_removes_wal_and_shm_the_backup_did_not_have() {
        let root = tempfile::tempdir().unwrap();
        let p = FakePhone::with(&[(DB, b"db-only")]);
        let old = backup(&p, root.path(), "e51e30a5", T0).unwrap();
        p.write(WAL, b"newer-wal").unwrap();
        restore(&p, &old.dir, root.path(), "e51e30a5", T0 + 60).unwrap();
        assert_eq!(p.get(WAL), None);
        assert_eq!(p.get(DB).unwrap(), b"db-only");
    }

    #[test]
    fn restore_refuses_a_corrupt_backup_and_writes_nothing() {
        let root = tempfile::tempdir().unwrap();
        let p = phone();
        let old = backup(&p, root.path(), "e51e30a5", T0).unwrap();
        fs::write(old.dir.join("MediaLibrary.sqlitedb"), b"truncated").unwrap();
        p.write(DB, b"current").unwrap();
        assert!(matches!(restore(&p, &old.dir, root.path(), "e51e30a5", T0 + 60), Err(BackupError::Corrupt(_))));
        assert_eq!(p.get(DB).unwrap(), b"current");
    }
}

#[cfg(test)]
mod count_tests {
    use super::*;

    #[test]
    fn song_count_opens_the_backup_and_counts_music_items() {
        let dir = tempfile::tempdir().unwrap();
        let conn = rusqlite::Connection::open(dir.path().join(DB_NAME)).unwrap();
        conn.execute_batch(
            "CREATE TABLE item (item_pid INTEGER PRIMARY KEY, media_type INTEGER);
             INSERT INTO item VALUES (1, 8), (2, 8), (3, 2048);",
        )
        .unwrap();
        drop(conn);
        assert_eq!(song_count(dir.path()).unwrap(), 2, "media_type 8 = music");
    }
}

#[cfg(all(test, windows))]
mod machine_tests {
    use super::*;
    use crate::apple::afc::tests_support::open_first_trusted;

    /// Read-only on the phone: copies the music DB off and counts songs. The test device holds
    /// 69 MP3s synced by 3uTools (CLAUDE.md). Run with `--include-ignored --test-threads=1`.
    #[test]
    #[ignore]
    fn backs_up_the_real_music_database() {
        let (afc, udid) = open_first_trusted();
        let root = tempfile::tempdir().unwrap();
        let b = backup(&afc, root.path(), &udid, crate::util::time::now_unix()).unwrap();
        println!("backup: {:?}", b.manifest.files);
        assert!(b.manifest.files.contains_key(DB_NAME));
        assert_eq!(song_count(&b.dir).unwrap(), 69);
    }
}
