//! Finding songs in a watched folder and keeping the `songs` table in step with the disk (F3.1).
//! Detection is by extension for now; content-based detection is F3.2.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::library::db::Db;

const AUDIO_EXTENSIONS: &[&str] = &["mp3", "m4a", "aac", "alac", "flac", "wav", "aif", "aiff", "ogg", "oga", "opus", "wma", "ape", "wv"];

#[derive(Debug, Clone, PartialEq)]
pub struct FileEntry {
    pub path: PathBuf,
    pub size: u64,
    /// Seconds since 1970.
    pub mtime: i64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScanDiff {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

pub fn is_audio_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTENSIONS.iter().any(|a| a.eq_ignore_ascii_case(e)))
}

/// Every audio file under `root`, sorted by path. Folders starting with `.` are skipped;
/// unreadable folders are skipped silently (a watched drive may be partly inaccessible).
pub fn scan_folder(root: &Path) -> Vec<FileEntry> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                if !entry.file_name().to_string_lossy().starts_with('.') {
                    stack.push(path);
                }
            } else if meta.is_file() && is_audio_path(&path) {
                let mtime = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
                out.push(FileEntry { path, size: meta.len(), mtime });
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Make the folder's rows in `songs` match `found`: insert new files, update changed ones
/// (keeping rating), delete files that are gone. One transaction.
pub fn reconcile(db: &Db, folder_id: i64, found: &[FileEntry], now_unix: u64) -> rusqlite::Result<ScanDiff> {
    let conn = db.conn();
    let tx = conn.unchecked_transaction()?;
    let mut known: HashMap<String, (i64, i64, i64)> = HashMap::new();
    {
        let mut stmt = tx.prepare("SELECT id, path, size, mtime FROM songs WHERE folder_id = ?1")?;
        let rows = stmt.query_map([folder_id], |r| Ok((r.get::<_, String>(1)?.to_lowercase(), (r.get(0)?, r.get(2)?, r.get(3)?))))?;
        for row in rows {
            let (path, v) = row?;
            known.insert(path, v);
        }
    }
    let mut diff = ScanDiff::default();
    for f in found {
        let path = f.path.to_string_lossy();
        match known.remove(&path.to_lowercase()) {
            None => {
                tx.execute(
                    "INSERT INTO songs (folder_id, path, size, mtime, added_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![folder_id, path, f.size as i64, f.mtime, now_unix as i64],
                )?;
                diff.added += 1;
            }
            Some((id, size, mtime)) if size != f.size as i64 || mtime != f.mtime => {
                tx.execute("UPDATE songs SET size = ?1, mtime = ?2 WHERE id = ?3", rusqlite::params![f.size as i64, f.mtime, id])?;
                diff.updated += 1;
            }
            Some(_) => {}
        }
    }
    for (id, _, _) in known.into_values() {
        tx.execute("DELETE FROM songs WHERE id = ?1", [id])?;
        diff.removed += 1;
    }
    tx.commit()?;
    Ok(diff)
}

pub fn rescan_folder(db: &Db, folder_id: i64, root: &str, now_unix: u64) -> rusqlite::Result<ScanDiff> {
    reconcile(db, folder_id, &scan_folder(Path::new(root)), now_unix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::folders::add_folder;
    use std::fs;

    fn touch(path: &Path, bytes: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![0u8; bytes]).unwrap();
    }

    fn rating_of(db: &Db, path: &Path) -> i64 {
        db.conn()
            .query_row("SELECT rating FROM songs WHERE path = ?1", [path.to_string_lossy()], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn recognises_audio_extensions_in_any_case() {
        for p in ["a.mp3", "b.M4A", "c.flac", "d.wav", "e.aiff", "f.aif", "g.ogg", "h.opus", "i.wma", "j.ape", "k.wv", "l.aac"] {
            assert!(is_audio_path(Path::new(p)), "{p}");
        }
        for p in ["cover.jpg", "notes.txt", "noext", "playlist.m3u"] {
            assert!(!is_audio_path(Path::new(p)), "{p}");
        }
    }

    #[test]
    fn finds_audio_files_recursively_with_unicode_names_and_skips_hidden_folders() {
        let d = tempfile::tempdir().unwrap();
        touch(&d.path().join("Mùa thu cho em.mp3"), 10);
        touch(&d.path().join("Trịnh").join("Diễm xưa.flac"), 20);
        touch(&d.path().join("cover.jpg"), 5);
        touch(&d.path().join(".trash").join("old.mp3"), 5);
        let found = scan_folder(d.path());
        let names: Vec<String> = found.iter().map(|e| e.path.strip_prefix(d.path()).unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["Mùa thu cho em.mp3".to_string(), format!("Trịnh{}Diễm xưa.flac", std::path::MAIN_SEPARATOR)]);
        assert_eq!(found[1].size, 20);
        assert!(found[0].mtime > 0);
    }

    #[test]
    fn first_reconcile_adds_every_file_and_a_rerun_changes_nothing() {
        let d = tempfile::tempdir().unwrap();
        touch(&d.path().join("a.mp3"), 1);
        touch(&d.path().join("b.mp3"), 2);
        let db = Db::open_in_memory().unwrap();
        let f = add_folder(&db, &d.path().to_string_lossy(), 1).unwrap();
        assert_eq!(reconcile(&db, f.id, &scan_folder(d.path()), 100).unwrap(), ScanDiff { added: 2, updated: 0, removed: 0 });
        assert_eq!(reconcile(&db, f.id, &scan_folder(d.path()), 200).unwrap(), ScanDiff::default());
    }

    #[test]
    fn changed_files_are_updated_and_keep_their_rating() {
        let d = tempfile::tempdir().unwrap();
        let song = d.path().join("a.mp3");
        touch(&song, 1);
        let db = Db::open_in_memory().unwrap();
        let f = add_folder(&db, &d.path().to_string_lossy(), 1).unwrap();
        reconcile(&db, f.id, &scan_folder(d.path()), 100).unwrap();
        db.conn().execute("UPDATE songs SET rating = 1", []).unwrap();
        touch(&song, 999);
        assert_eq!(reconcile(&db, f.id, &scan_folder(d.path()), 200).unwrap(), ScanDiff { added: 0, updated: 1, removed: 0 });
        assert_eq!(rating_of(&db, &song), 1);
    }

    #[test]
    fn files_gone_from_disk_are_removed_from_the_library() {
        let d = tempfile::tempdir().unwrap();
        touch(&d.path().join("a.mp3"), 1);
        touch(&d.path().join("b.mp3"), 1);
        let db = Db::open_in_memory().unwrap();
        let f = add_folder(&db, &d.path().to_string_lossy(), 1).unwrap();
        reconcile(&db, f.id, &scan_folder(d.path()), 100).unwrap();
        fs::remove_file(d.path().join("b.mp3")).unwrap();
        assert_eq!(reconcile(&db, f.id, &scan_folder(d.path()), 200).unwrap(), ScanDiff { added: 0, updated: 0, removed: 1 });
    }

    #[test]
    fn reconcile_only_touches_its_own_folder() {
        let (d1, d2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        touch(&d1.path().join("a.mp3"), 1);
        touch(&d2.path().join("b.mp3"), 1);
        let db = Db::open_in_memory().unwrap();
        let f1 = add_folder(&db, &d1.path().to_string_lossy(), 1).unwrap();
        let f2 = add_folder(&db, &d2.path().to_string_lossy(), 1).unwrap();
        reconcile(&db, f1.id, &scan_folder(d1.path()), 100).unwrap();
        reconcile(&db, f2.id, &scan_folder(d2.path()), 100).unwrap();
        assert_eq!(reconcile(&db, f1.id, &[], 200).unwrap().removed, 1);
        let left: i64 = db.conn().query_row("SELECT count(*) FROM songs", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 1);
    }
}

#[cfg(test)]
mod rescan_tests {
    use super::*;
    use crate::library::folders::add_folder;

    #[test]
    fn rescan_folder_scans_and_reconciles_in_one_step() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.mp3"), b"x").unwrap();
        let db = Db::open_in_memory().unwrap();
        let f = add_folder(&db, &d.path().to_string_lossy(), 1).unwrap();
        assert_eq!(rescan_folder(&db, f.id, &f.path, 10).unwrap().added, 1);
        assert_eq!(rescan_folder(&db, f.id, &f.path, 20).unwrap(), ScanDiff::default());
    }
}
