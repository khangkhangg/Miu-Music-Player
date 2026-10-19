//! Watched folders (F3.1). Folders may not repeat or nest, so every song belongs to exactly one.
//! Removing a folder removes its songs from Miu's library only — never from disk.

use serde::Serialize;

use crate::library::db::Db;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchedFolder {
    pub id: i64,
    pub path: String,
    pub song_count: i64,
    /// Legacy tag encoding forced for this folder (F8.1); `None` = detect per song.
    pub tag_encoding: Option<String>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum FolderError {
    #[error("use a full path such as D:\\Music")]
    NotAbsolute,
    #[error("{0} is already watched")]
    AlreadyWatched(String),
    #[error("it is inside {0}, which is already watched")]
    InsideWatched(String),
    #[error("it contains {0}, which is already watched — remove that one first")]
    ContainsWatched(String),
    #[error("database: {0}")]
    Db(String),
}

fn db_err(e: impl std::fmt::Display) -> FolderError {
    FolderError::Db(e.to_string())
}

/// Backslashes, no trailing separator (except a drive root like `D:\`).
pub fn normalize(path: &str) -> String {
    let p = path.replace('/', "\\");
    let trimmed = p.trim_end_matches('\\');
    if trimmed.len() == 2 && trimmed.ends_with(':') {
        format!("{trimmed}\\")
    } else {
        trimmed.to_string()
    }
}

/// `child` is strictly inside `parent` (case-insensitive, whole path components).
pub(crate) fn is_inside(child: &str, parent: &str) -> bool {
    let (c, p) = (child.to_lowercase(), parent.to_lowercase());
    let prefix = if p.ends_with('\\') { p } else { format!("{p}\\") };
    c.starts_with(&prefix) && c.len() > prefix.len()
}

pub fn add_folder(db: &Db, path: &str, now_unix: u64) -> Result<WatchedFolder, FolderError> {
    let path = normalize(path);
    if !std::path::Path::new(&path).is_absolute() {
        return Err(FolderError::NotAbsolute);
    }
    for existing in list_folders(db)? {
        if existing.path.to_lowercase() == path.to_lowercase() {
            return Err(FolderError::AlreadyWatched(existing.path));
        }
        if is_inside(&path, &existing.path) {
            return Err(FolderError::InsideWatched(existing.path));
        }
        if is_inside(&existing.path, &path) {
            return Err(FolderError::ContainsWatched(existing.path));
        }
    }
    db.conn()
        .execute("INSERT INTO watched_folders (path, added_at) VALUES (?1, ?2)", rusqlite::params![path, now_unix as i64])
        .map_err(db_err)?;
    Ok(WatchedFolder { id: db.conn().last_insert_rowid(), path, song_count: 0, tag_encoding: None })
}

/// Force a legacy tag encoding for a folder (`None` = detect). Its songs' tags are read again
/// on the next rescan.
pub fn set_tag_encoding(db: &Db, folder_id: i64, label: Option<&str>) -> Result<(), String> {
    let name = match label {
        Some(l) => Some(crate::library::legacy::encoding_named(l).ok_or_else(|| format!("unknown encoding {l:?}"))?.name()),
        None => None,
    };
    let c = db.conn();
    c.execute("UPDATE watched_folders SET tag_encoding = ?1 WHERE id = ?2", rusqlite::params![name.map(str::to_lowercase), folder_id])
        .map_err(|e| e.to_string())?;
    c.execute("UPDATE songs SET tags_read = 0 WHERE folder_id = ?1", [folder_id]).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn list_folders(db: &Db) -> Result<Vec<WatchedFolder>, FolderError> {
    let mut stmt = db
        .conn()
        .prepare(
            "SELECT f.id, f.path, (SELECT count(*) FROM songs s WHERE s.folder_id = f.id), f.tag_encoding
             FROM watched_folders f ORDER BY f.path COLLATE NOCASE",
        )
        .map_err(db_err)?;
    let rows = stmt
        .query_map([], |r| Ok(WatchedFolder { id: r.get(0)?, path: r.get(1)?, song_count: r.get(2)?, tag_encoding: r.get(3)? }))
        .map_err(db_err)?;
    rows.collect::<Result<_, _>>().map_err(db_err)
}

pub fn remove_folder(db: &Db, id: i64) -> Result<(), FolderError> {
    db.conn().execute("DELETE FROM watched_folders WHERE id = ?1", [id]).map_err(db_err)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn db() -> Db {
        Db::open_in_memory().unwrap()
    }

    fn add_song(db: &Db, folder_id: i64, path: &str) {
        db.conn()
            .execute(
                "INSERT INTO songs (folder_id, path, size, mtime, added_at) VALUES (?1, ?2, 1, 1, 1)",
                rusqlite::params![folder_id, path],
            )
            .unwrap();
    }

    #[test]
    fn normalizes_separators_and_trailing_slashes() {
        assert_eq!(normalize(r"D:/Music/"), r"D:\Music");
        assert_eq!(normalize(r"D:\Music\\"), r"D:\Music");
        assert_eq!(normalize(r"D:\"), r"D:\");
        assert_eq!(normalize(r"E:\Nhạc Trịnh"), r"E:\Nhạc Trịnh");
    }

    #[test]
    fn adds_and_lists_a_folder() {
        let db = db();
        let f = add_folder(&db, r"D:\Music", 100).unwrap();
        let all = list_folders(&db).unwrap();
        assert_eq!(all, vec![WatchedFolder { id: f.id, path: r"D:\Music".into(), song_count: 0, tag_encoding: None }]);
    }

    #[test]
    fn the_same_folder_twice_is_rejected_ignoring_case_and_slashes() {
        let db = db();
        add_folder(&db, r"D:\Music", 1).unwrap();
        assert_eq!(add_folder(&db, r"d:/music/", 2), Err(FolderError::AlreadyWatched(r"D:\Music".into())));
    }

    #[test]
    fn a_folder_inside_a_watched_one_is_rejected() {
        let db = db();
        add_folder(&db, r"D:\Music", 1).unwrap();
        assert_eq!(add_folder(&db, r"D:\Music\Jazz", 2), Err(FolderError::InsideWatched(r"D:\Music".into())));
    }

    #[test]
    fn a_folder_containing_a_watched_one_is_rejected() {
        let db = db();
        add_folder(&db, r"D:\Music\Jazz", 1).unwrap();
        assert_eq!(add_folder(&db, r"D:\Music", 2), Err(FolderError::ContainsWatched(r"D:\Music\Jazz".into())));
    }

    #[test]
    fn a_sibling_with_a_common_prefix_is_fine() {
        let db = db();
        add_folder(&db, r"D:\Music", 1).unwrap();
        assert!(add_folder(&db, r"D:\Music2", 2).is_ok());
    }

    #[test]
    fn relative_paths_are_rejected() {
        assert_eq!(add_folder(&db(), r"Music", 1), Err(FolderError::NotAbsolute));
    }

    #[test]
    fn folders_list_their_song_counts() {
        let db = db();
        let a = add_folder(&db, r"D:\Music", 1).unwrap();
        let b = add_folder(&db, r"E:\Nhạc Trịnh", 2).unwrap();
        add_song(&db, a.id, r"D:\Music\a.mp3");
        add_song(&db, a.id, r"D:\Music\b.flac");
        add_song(&db, b.id, r"E:\Nhạc Trịnh\Diễm xưa.mp3");
        let counts: Vec<(String, i64)> = list_folders(&db).unwrap().into_iter().map(|f| (f.path, f.song_count)).collect();
        assert_eq!(counts, vec![(r"D:\Music".into(), 2), (r"E:\Nhạc Trịnh".into(), 1)]);
    }

    #[test]
    fn removing_a_folder_removes_its_songs_from_the_library() {
        let db = db();
        let a = add_folder(&db, r"D:\Music", 1).unwrap();
        add_song(&db, a.id, r"D:\Music\a.mp3");
        remove_folder(&db, a.id).unwrap();
        assert!(list_folders(&db).unwrap().is_empty());
        let songs: i64 = db.conn().query_row("SELECT count(*) FROM songs", [], |r| r.get(0)).unwrap();
        assert_eq!(songs, 0);
    }
}
