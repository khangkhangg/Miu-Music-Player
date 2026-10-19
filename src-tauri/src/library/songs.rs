//! The song list the UI shows (F5.2). Sorted by title for now (lowercase compare);
//! language-aware collation is F8.5.

use std::path::Path;

use serde::Serialize;

use crate::library::db::Db;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongRow {
    pub id: i64,
    /// Title tag, or the file name without extension.
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_ms: Option<i64>,
    pub format: String,
    pub rating: i64,
}

pub fn list_songs(db: &Db) -> rusqlite::Result<Vec<SongRow>> {
    let mut stmt = db
        .conn()
        .prepare("SELECT id, path, title, artist, album, duration_ms, format, rating FROM songs")?;
    let mut rows: Vec<SongRow> = stmt
        .query_map([], |r| {
            let path: String = r.get(1)?;
            let title: Option<String> = r.get(2)?;
            let title = title
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| Path::new(&path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or(path.clone()));
            Ok(SongRow {
                id: r.get(0)?,
                title,
                artist: r.get(3)?,
                album: r.get(4)?,
                duration_ms: r.get(5)?,
                format: r.get(6)?,
                rating: r.get(7)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    rows.sort_by_cached_key(|s| s.title.to_lowercase());
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::folders::add_folder;

    fn insert(db: &Db, folder: i64, path: &str, title: Option<&str>, artist: Option<&str>, duration_ms: Option<i64>) {
        db.conn()
            .execute(
                "INSERT INTO songs (folder_id, path, size, mtime, added_at, format, title, artist, album, duration_ms, tags_read)
                 VALUES (?1, ?2, 1, 1, 1, 'mp3', ?3, ?4, 'Album', ?5, 1)",
                rusqlite::params![folder, path, title, artist, duration_ms],
            )
            .unwrap();
    }

    #[test]
    fn empty_library_lists_nothing() {
        assert!(list_songs(&Db::open_in_memory().unwrap()).unwrap().is_empty());
    }

    #[test]
    fn lists_songs_with_display_titles_sorted_by_title() {
        let db = Db::open_in_memory().unwrap();
        let f = add_folder(&db, r"E:\Nhạc", 1).unwrap();
        insert(&db, f.id, r"E:\Nhạc\Diễm xưa.mp3", None, Some("Khánh Ly"), Some(303_000));
        insert(&db, f.id, r"E:\Nhạc\b.mp3", Some("Mùa thu cho em"), Some("Ngọc Lan"), Some(252_000));
        insert(&db, f.id, r"E:\Nhạc\c.mp3", Some("bolero"), None, None);
        let songs = list_songs(&db).unwrap();
        let titles: Vec<&str> = songs.iter().map(|s| s.title.as_str()).collect();
        assert_eq!(titles, ["bolero", "Diễm xưa", "Mùa thu cho em"]);
        let diem = &songs[1];
        assert_eq!(diem.artist.as_deref(), Some("Khánh Ly"));
        assert_eq!(diem.album.as_deref(), Some("Album"));
        assert_eq!(diem.duration_ms, Some(303_000));
        assert_eq!(diem.format, "mp3");
        assert_eq!(diem.rating, 0);
    }

    #[test]
    fn serializes_camel_case_for_the_ui() {
        let db = Db::open_in_memory().unwrap();
        let f = add_folder(&db, r"D:\M", 1).unwrap();
        insert(&db, f.id, r"D:\M\a.mp3", Some("A"), None, Some(1000));
        let json = serde_json::to_value(&list_songs(&db).unwrap()[0]).unwrap();
        assert_eq!(json["durationMs"], 1000);
        assert!(json["artist"].is_null());
    }
}
