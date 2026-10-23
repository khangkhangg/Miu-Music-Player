//! Playlists (PL): the user's own song lists, in their own order. A song is in a playlist at most
//! once; deleting a song from the library takes it out of every playlist.

use serde::Serialize;

use crate::library::db::Db;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Playlist {
    pub id: i64,
    pub name: String,
    pub count: i64,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn clean(name: &str) -> Result<String, String> {
    let n: String = name.trim().chars().take(100).collect();
    if n.is_empty() {
        return Err("a playlist needs a name".into());
    }
    Ok(n)
}

/// Every playlist with how many songs it has, by name.
pub fn list(db: &Db) -> Result<Vec<Playlist>, String> {
    let mut stmt = db
        .conn()
        .prepare(
            "SELECT p.id, p.name, (SELECT COUNT(*) FROM playlist_songs s WHERE s.playlist_id = p.id)
             FROM playlists p ORDER BY p.name COLLATE NOCASE, p.id",
        )
        .map_err(err)?;
    let rows = stmt.query_map([], |r| Ok(Playlist { id: r.get(0)?, name: r.get(1)?, count: r.get(2)? })).map_err(err)?;
    rows.collect::<rusqlite::Result<_>>().map_err(err)
}

pub fn create(db: &Db, name: &str) -> Result<i64, String> {
    let name = clean(name)?;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    db.conn().execute("INSERT INTO playlists (name, created_at) VALUES (?1, ?2)", rusqlite::params![name, now]).map_err(err)?;
    Ok(db.conn().last_insert_rowid())
}

pub fn rename(db: &Db, id: i64, name: &str) -> Result<(), String> {
    let name = clean(name)?;
    db.conn().execute("UPDATE playlists SET name = ?2 WHERE id = ?1", rusqlite::params![id, name]).map_err(err)?;
    Ok(())
}

pub fn delete(db: &Db, id: i64) -> Result<(), String> {
    db.conn().execute("DELETE FROM playlists WHERE id = ?1", [id]).map_err(err)?;
    Ok(())
}

/// The playlist's songs in its order.
pub fn songs(db: &Db, id: i64) -> Result<Vec<i64>, String> {
    let mut stmt = db.conn().prepare("SELECT song_id FROM playlist_songs WHERE playlist_id = ?1 ORDER BY position").map_err(err)?;
    let rows = stmt.query_map([id], |r| r.get(0)).map_err(err)?;
    rows.collect::<rusqlite::Result<_>>().map_err(err)
}

/// Adds songs at the end; songs already in it stay where they are.
pub fn add(db: &Db, id: i64, song_ids: &[i64]) -> Result<(), String> {
    let mut next: i64 = db
        .conn()
        .query_row("SELECT COALESCE(MAX(position), -1) + 1 FROM playlist_songs WHERE playlist_id = ?1", [id], |r| r.get(0))
        .map_err(err)?;
    for &song in song_ids {
        let n = db
            .conn()
            .execute("INSERT OR IGNORE INTO playlist_songs (playlist_id, song_id, position) VALUES (?1, ?2, ?3)", rusqlite::params![id, song, next])
            .map_err(err)?;
        next += n as i64;
    }
    Ok(())
}

pub fn remove(db: &Db, id: i64, song_ids: &[i64]) -> Result<(), String> {
    for &song in song_ids {
        db.conn().execute("DELETE FROM playlist_songs WHERE playlist_id = ?1 AND song_id = ?2", rusqlite::params![id, song]).map_err(err)?;
    }
    Ok(())
}

/// A new order: the same songs, rearranged.
pub fn set_order(db: &Db, id: i64, song_ids: &[i64]) -> Result<(), String> {
    let mut now = songs(db, id)?;
    let mut wanted = song_ids.to_vec();
    now.sort_unstable();
    wanted.sort_unstable();
    if now != wanted {
        return Err("the new order must have the same songs".into());
    }
    for (pos, &song) in song_ids.iter().enumerate() {
        db.conn()
            .execute("UPDATE playlist_songs SET position = ?3 WHERE playlist_id = ?1 AND song_id = ?2", rusqlite::params![id, song, pos as i64])
            .map_err(err)?;
    }
    Ok(())
}

// ---- Commands ----

use crate::library::service::SharedDb;
use tauri::State;

fn with<T>(db: &State<'_, SharedDb>, f: impl FnOnce(&Db) -> Result<T, String>) -> Result<T, String> {
    f(&*db.lock().map_err(|e| e.to_string())?)
}

#[tauri::command]
pub fn playlists_list(db: State<'_, SharedDb>) -> Result<Vec<Playlist>, String> {
    with(&db, list)
}

#[tauri::command]
pub fn playlist_create(name: String, db: State<'_, SharedDb>) -> Result<i64, String> {
    with(&db, |d| create(d, &name))
}

#[tauri::command]
pub fn playlist_rename(id: i64, name: String, db: State<'_, SharedDb>) -> Result<(), String> {
    with(&db, |d| rename(d, id, &name))
}

#[tauri::command]
pub fn playlist_delete(id: i64, db: State<'_, SharedDb>) -> Result<(), String> {
    with(&db, |d| delete(d, id))
}

#[tauri::command]
pub fn playlist_songs(id: i64, db: State<'_, SharedDb>) -> Result<Vec<i64>, String> {
    with(&db, |d| songs(d, id))
}

#[tauri::command]
pub fn playlist_add(id: i64, song_ids: Vec<i64>, db: State<'_, SharedDb>) -> Result<(), String> {
    with(&db, |d| add(d, id, &song_ids))
}

#[tauri::command]
pub fn playlist_remove(id: i64, song_ids: Vec<i64>, db: State<'_, SharedDb>) -> Result<(), String> {
    with(&db, |d| remove(d, id, &song_ids))
}

#[tauri::command]
pub fn playlist_set_order(id: i64, song_ids: Vec<i64>, db: State<'_, SharedDb>) -> Result<(), String> {
    with(&db, |d| set_order(d, id, &song_ids))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::db::Db;
    use crate::library::folders::add_folder;

    fn song(db: &Db, path: &str) -> i64 {
        let f = match add_folder(db, r"E:\Nhạc", 1) {
            Ok(f) => f.id,
            Err(_) => db.conn().query_row("SELECT id FROM watched_folders", [], |r| r.get(0)).unwrap(),
        };
        db.conn()
            .execute(
                "INSERT INTO songs (folder_id, path, size, mtime, added_at, format, title, rating, tags_read) VALUES (?1, ?2, 1, 1, 1, 'mp3', ?2, 0, 1)",
                rusqlite::params![f, path],
            )
            .unwrap();
        db.conn().last_insert_rowid()
    }

    #[test]
    fn make_fill_and_list_playlists() {
        let db = Db::open_in_memory().unwrap();
        let (a, b, c) = (song(&db, "a.mp3"), song(&db, "b.mp3"), song(&db, "c.mp3"));
        let party = create(&db, "  Friday night  ").unwrap();
        let chill = create(&db, "Chill").unwrap();
        assert!(create(&db, "   ").is_err(), "a playlist needs a name");
        add(&db, party, &[b, a]).unwrap();
        add(&db, party, &[a, c]).unwrap();
        assert_eq!(songs(&db, party).unwrap(), vec![b, a, c], "added at the end, once each");
        let all = list(&db).unwrap();
        assert_eq!(all.iter().map(|p| (p.name.as_str(), p.count)).collect::<Vec<_>>(), vec![("Chill", 0), ("Friday night", 3)], "by name");
        assert_eq!(all[1].id, party);
        let _ = chill;
    }

    #[test]
    fn reorder_remove_rename_delete() {
        let db = Db::open_in_memory().unwrap();
        let (a, b, c) = (song(&db, "a.mp3"), song(&db, "b.mp3"), song(&db, "c.mp3"));
        let p = create(&db, "Wedding").unwrap();
        add(&db, p, &[a, b, c]).unwrap();
        set_order(&db, p, &[c, a, b]).unwrap();
        assert_eq!(songs(&db, p).unwrap(), vec![c, a, b]);
        assert!(set_order(&db, p, &[c, a]).is_err(), "a new order has every song, no more, no fewer");
        remove(&db, p, &[a]).unwrap();
        assert_eq!(songs(&db, p).unwrap(), vec![c, b]);
        rename(&db, p, "Wedding party").unwrap();
        assert_eq!(list(&db).unwrap()[0].name, "Wedding party");
        assert!(rename(&db, p, "").is_err());
        delete(&db, p).unwrap();
        assert!(list(&db).unwrap().is_empty());
        assert!(songs(&db, p).unwrap().is_empty());
    }

    #[test]
    fn songs_deleted_from_the_library_leave_their_playlists() {
        let db = Db::open_in_memory().unwrap();
        let (a, b) = (song(&db, "a.mp3"), song(&db, "b.mp3"));
        let p = create(&db, "Mix").unwrap();
        add(&db, p, &[a, b]).unwrap();
        db.conn().execute("DELETE FROM songs WHERE id = ?1", [a]).unwrap();
        assert_eq!(songs(&db, p).unwrap(), vec![b]);
        assert!(add(&db, p, &[999]).is_err(), "only songs in the library");
    }
}
