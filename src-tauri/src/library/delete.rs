//! Deleting disliked songs (F7.3): the file goes to the Recycle Bin (never a permanent delete,
//! CLAUDE.md), then the song leaves the library. Removal from the iPhone waits for sync (paused).

use std::path::Path;

use rusqlite::OptionalExtension;
use serde::Serialize;

use crate::library::db::Db;

/// Where deleted files go. The app uses the Windows Recycle Bin; tests use a fake.
pub trait Trash {
    fn trash(&self, path: &Path) -> Result<(), String>;
}

/// The Windows Recycle Bin (via the shell, so files can be restored from Explorer).
pub struct RecycleBin;

impl Trash for RecycleBin {
    fn trash(&self, path: &Path) -> Result<(), String> {
        trash::delete(path).map_err(|e| e.to_string())
    }
}

#[derive(Debug, Default, Serialize)]
pub struct DeleteReport {
    pub deleted: Vec<i64>,
    /// (song id, reason) for songs that were left alone.
    pub failed: Vec<(i64, String)>,
}

/// Move the files of disliked songs `ids` to the trash and remove them from the library.
/// Songs that are not disliked are refused, so a stray id can never delete a kept song.
pub fn delete_disliked(db: &Db, ids: &[i64], trash: &dyn Trash) -> rusqlite::Result<DeleteReport> {
    let mut report = DeleteReport::default();
    for &id in ids {
        let row: Option<(String, i64)> = db
            .conn()
            .query_row("SELECT path, rating FROM songs WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        let Some((path, rating)) = row else {
            report.failed.push((id, "not in the library".into()));
            continue;
        };
        if rating != -1 {
            report.failed.push((id, "only disliked songs can be deleted".into()));
            continue;
        }
        let path = Path::new(&path);
        if path.exists() {
            if let Err(e) = trash.trash(path) {
                report.failed.push((id, e));
                continue;
            }
        }
        db.conn().execute("DELETE FROM songs WHERE id = ?1", [id])?;
        report.deleted.push(id);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::folders::add_folder;
    use crate::library::songs::{list_songs, set_rating, Rating};
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    /// Records what would go to the Recycle Bin; can be told to refuse a path.
    #[derive(Default)]
    struct FakeTrash {
        trashed: RefCell<Vec<PathBuf>>,
        refuse: Option<PathBuf>,
    }

    impl Trash for FakeTrash {
        fn trash(&self, path: &Path) -> Result<(), String> {
            if self.refuse.as_deref() == Some(path) {
                return Err("file is in use".into());
            }
            self.trashed.borrow_mut().push(path.to_path_buf());
            std::fs::remove_file(path).map_err(|e| e.to_string())
        }
    }

    /// Three real files: a.mp3 (disliked), b.mp3 (disliked), c.mp3 (liked). Returns ids in that order.
    fn library(dir: &Path, db: &Db) -> Vec<i64> {
        let f = add_folder(db, &dir.to_string_lossy(), 1).unwrap();
        let mut ids = Vec::new();
        for (name, rating) in [("a.mp3", Rating::Disliked), ("b.mp3", Rating::Disliked), ("c.mp3", Rating::Liked)] {
            let p = dir.join(name);
            std::fs::write(&p, b"x").unwrap();
            db.conn()
                .execute("INSERT INTO songs (folder_id, path, size, mtime, added_at) VALUES (?1, ?2, 1, 1, 1)", rusqlite::params![f.id, p.to_string_lossy()])
                .unwrap();
            let id = db.conn().last_insert_rowid();
            set_rating(db, id, rating).unwrap();
            ids.push(id);
        }
        ids
    }

    #[test]
    fn deleting_disliked_songs_trashes_the_files_and_removes_them_from_the_library() {
        let d = tempfile::tempdir().unwrap();
        let db = Db::open_in_memory().unwrap();
        let ids = library(d.path(), &db);
        let trash = FakeTrash::default();
        let report = delete_disliked(&db, &ids[..2], &trash).unwrap();
        assert_eq!(report.deleted, vec![ids[0], ids[1]]);
        assert!(report.failed.is_empty());
        assert_eq!(trash.trashed.borrow().clone(), vec![d.path().join("a.mp3"), d.path().join("b.mp3")]);
        assert_eq!(list_songs(&db).unwrap().len(), 1);
    }

    #[test]
    fn songs_that_are_not_disliked_are_never_deleted() {
        let d = tempfile::tempdir().unwrap();
        let db = Db::open_in_memory().unwrap();
        let ids = library(d.path(), &db);
        let trash = FakeTrash::default();
        let report = delete_disliked(&db, &[ids[2]], &trash).unwrap();
        assert!(report.deleted.is_empty());
        assert_eq!(report.failed.len(), 1);
        assert!(trash.trashed.borrow().is_empty());
        assert!(d.path().join("c.mp3").exists());
    }

    #[test]
    fn a_file_already_gone_is_just_removed_from_the_library() {
        let d = tempfile::tempdir().unwrap();
        let db = Db::open_in_memory().unwrap();
        let ids = library(d.path(), &db);
        std::fs::remove_file(d.path().join("a.mp3")).unwrap();
        let report = delete_disliked(&db, &[ids[0]], &FakeTrash::default()).unwrap();
        assert_eq!(report.deleted, vec![ids[0]]);
        assert_eq!(list_songs(&db).unwrap().len(), 2);
    }

    #[test]
    fn if_the_trash_refuses_the_song_stays_and_the_error_is_reported() {
        let d = tempfile::tempdir().unwrap();
        let db = Db::open_in_memory().unwrap();
        let ids = library(d.path(), &db);
        let trash = FakeTrash { refuse: Some(d.path().join("a.mp3")), ..Default::default() };
        let report = delete_disliked(&db, &ids[..2], &trash).unwrap();
        assert_eq!(report.deleted, vec![ids[1]]);
        assert_eq!(report.failed, vec![(ids[0], "file is in use".to_string())]);
        assert!(d.path().join("a.mp3").exists());
        assert_eq!(list_songs(&db).unwrap().len(), 2);
    }

    #[test]
    fn unknown_ids_are_reported_not_fatal() {
        let db = Db::open_in_memory().unwrap();
        let report = delete_disliked(&db, &[999], &FakeTrash::default()).unwrap();
        assert_eq!(report.failed.len(), 1);
    }
}
