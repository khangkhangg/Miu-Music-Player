//! The song list the UI shows (F5.2). Sorted by title for now (lowercase compare);
//! language-aware collation is F8.5.

use std::path::Path;

use rusqlite::OptionalExtension;
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
    pub sample_rate: Option<u32>,
    /// Lossless files only.
    pub bit_depth: Option<u32>,
    pub bitrate_kbps: Option<u32>,
}

pub fn list_songs(db: &Db) -> rusqlite::Result<Vec<SongRow>> {
    let mut stmt = db
        .conn()
        .prepare("SELECT id, path, title, artist, album, duration_ms, format, rating, sample_rate, bit_depth, bitrate_kbps FROM songs")?;
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
                sample_rate: r.get(8)?,
                bit_depth: r.get(9)?,
                bitrate_kbps: r.get(10)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    rows.sort_by_cached_key(|s| s.title.to_lowercase());
    Ok(rows)
}

/// Liked songs sync to the iPhone; disliked ones never do (F7.2, product decisions in CLAUDE.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rating {
    Disliked = -1,
    NotRated = 0,
    Liked = 1,
}

impl Rating {
    pub fn from_value(v: i64) -> Option<Self> {
        match v {
            -1 => Some(Self::Disliked),
            0 => Some(Self::NotRated),
            1 => Some(Self::Liked),
            _ => None,
        }
    }
}

pub fn set_rating(db: &Db, id: i64, rating: Rating) -> Result<(), String> {
    let changed = db
        .conn()
        .execute("UPDATE songs SET rating = ?1 WHERE id = ?2", rusqlite::params![rating as i64, id])
        .map_err(|e| e.to_string())?;
    if changed == 0 {
        return Err(format!("song {id} is not in the library"));
    }
    Ok(())
}

/// File paths for `ids`, in the same order; ids no longer in the library are skipped.
pub fn paths_for(db: &Db, ids: &[i64]) -> rusqlite::Result<Vec<(i64, std::path::PathBuf)>> {
    let mut stmt = db.conn().prepare_cached("SELECT path FROM songs WHERE id = ?1")?;
    let mut out = Vec::with_capacity(ids.len());
    for &id in ids {
        if let Some(path) = stmt.query_row([id], |r| r.get::<_, String>(0)).optional()? {
            out.push((id, std::path::PathBuf::from(path)));
        }
    }
    Ok(out)
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

#[cfg(test)]
mod path_tests {
    use super::*;
    use crate::library::folders::add_folder;

    #[test]
    fn paths_come_back_in_the_order_asked_and_unknown_ids_are_skipped() {
        let db = Db::open_in_memory().unwrap();
        let f = add_folder(&db, r"E:\Nhạc", 1).unwrap();
        for p in [r"E:\Nhạc\a.mp3", r"E:\Nhạc\Diễm xưa.flac", r"E:\Nhạc\c.mp3"] {
            db.conn().execute("INSERT INTO songs (folder_id, path, size, mtime, added_at) VALUES (?1, ?2, 1, 1, 1)", rusqlite::params![f.id, p]).unwrap();
        }
        let got = paths_for(&db, &[3, 999, 1, 2]).unwrap();
        let ids: Vec<i64> = got.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, vec![3, 1, 2]);
        assert_eq!(got[2].1, std::path::PathBuf::from(r"E:\Nhạc\Diễm xưa.flac"));
    }
}

#[cfg(test)]
mod rating_tests {
    use super::*;
    use crate::library::folders::add_folder;
    use crate::library::scan::rescan_folder;

    fn one_song(db: &Db) -> i64 {
        let f = add_folder(db, r"D:\M", 1).unwrap();
        db.conn().execute(r"INSERT INTO songs (folder_id, path, size, mtime, added_at) VALUES (?1, 'D:\M\a.mp3', 1, 1, 1)", [f.id]).unwrap();
        db.conn().last_insert_rowid()
    }

    #[test]
    fn ratings_are_saved_and_listed() {
        let db = Db::open_in_memory().unwrap();
        let id = one_song(&db);
        set_rating(&db, id, Rating::Liked).unwrap();
        assert_eq!(list_songs(&db).unwrap()[0].rating, 1);
        set_rating(&db, id, Rating::Disliked).unwrap();
        assert_eq!(list_songs(&db).unwrap()[0].rating, -1);
        set_rating(&db, id, Rating::NotRated).unwrap();
        assert_eq!(list_songs(&db).unwrap()[0].rating, 0);
    }

    #[test]
    fn rating_values_map_both_ways() {
        assert_eq!(Rating::from_value(1), Some(Rating::Liked));
        assert_eq!(Rating::from_value(0), Some(Rating::NotRated));
        assert_eq!(Rating::from_value(-1), Some(Rating::Disliked));
        assert_eq!(Rating::from_value(5), None);
    }

    #[test]
    fn rating_an_unknown_song_is_an_error() {
        let db = Db::open_in_memory().unwrap();
        assert!(set_rating(&db, 42, Rating::Liked).is_err());
    }

    #[test]
    fn ratings_survive_a_rescan_of_the_folder() {
        let d = tempfile::tempdir().unwrap();
        crate::library::tags::fixtures::silent_mp3(d.path(), "a.mp3");
        let db = Db::open_in_memory().unwrap();
        let f = add_folder(&db, &d.path().to_string_lossy(), 1).unwrap();
        rescan_folder(&db, f.id, &f.path, 1).unwrap();
        let id = list_songs(&db).unwrap()[0].id;
        set_rating(&db, id, Rating::Liked).unwrap();
        rescan_folder(&db, f.id, &f.path, 2).unwrap();
        assert_eq!(list_songs(&db).unwrap()[0].rating, 1);
    }
}
