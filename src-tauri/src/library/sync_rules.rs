//! "Always sync" for an artist or an album: every song of it is liked now, and songs that turn up
//! later (new files, tag edits) are liked too. Songs the user disliked stay disliked.
//! Matching follows the Albums and Artists views (`src/js/library.js`): an artist is the song's
//! artist tag; an album is its album tag plus album artist (or artist when that is empty).

use serde::{Deserialize, Serialize};

use crate::library::db::Db;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncRule {
    /// "artist" or "album".
    pub kind: String,
    pub artist: String,
    /// Albums only.
    #[serde(default)]
    pub album: Option<String>,
}

/// Like every not-rated song of `rule` and remember it; returns the songs that became liked.
pub fn add_rule(db: &Db, rule: &SyncRule) -> Result<Vec<i64>, String> {
    let (kind, artist, album) = key(rule)?;
    db.conn()
        .execute("INSERT OR IGNORE INTO sync_rules (kind, artist, album) VALUES (?1, ?2, ?3)", rusqlite::params![kind, artist, album])
        .map_err(|e| e.to_string())?;
    apply_rules(db)
}

/// Stop liking new songs of `rule`. Songs it already liked stay liked.
pub fn remove_rule(db: &Db, rule: &SyncRule) -> Result<(), String> {
    let (kind, artist, album) = key(rule)?;
    db.conn()
        .execute("DELETE FROM sync_rules WHERE kind = ?1 AND artist = ?2 AND album = ?3", rusqlite::params![kind, artist, album])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn list_rules(db: &Db) -> Result<Vec<SyncRule>, String> {
    let mut stmt = db
        .conn()
        .prepare("SELECT kind, artist, album FROM sync_rules ORDER BY kind, artist, album")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            let album: String = r.get(2)?;
            Ok(SyncRule { kind: r.get(0)?, artist: r.get(1)?, album: (!album.is_empty()).then_some(album) })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<_>>().map_err(|e| e.to_string())
}

/// Like every not-rated song that a rule covers; returns the songs that became liked.
pub fn apply_rules(db: &Db) -> Result<Vec<i64>, String> {
    let mut stmt = db
        .conn()
        .prepare(
            "UPDATE songs SET rating = 1 WHERE rating = 0 AND EXISTS (
                 SELECT 1 FROM sync_rules r WHERE
                     (r.kind = 'artist' AND songs.artist = r.artist)
                  OR (r.kind = 'album' AND songs.album = r.album
                      AND COALESCE(NULLIF(songs.album_artist, ''), NULLIF(songs.artist, ''), '') = r.artist))
             RETURNING id",
        )
        .map_err(|e| e.to_string())?;
    let ids = stmt.query_map([], |r| r.get(0)).map_err(|e| e.to_string())?;
    ids.collect::<rusqlite::Result<_>>().map_err(|e| e.to_string())
}

fn key(rule: &SyncRule) -> Result<(&str, &str, &str), String> {
    if rule.artist.is_empty() {
        return Err("a sync rule needs an artist".into());
    }
    match (rule.kind.as_str(), rule.album.as_deref()) {
        ("artist", _) => Ok(("artist", &rule.artist, "")),
        ("album", Some(album)) if !album.is_empty() => Ok(("album", &rule.artist, album)),
        ("album", _) => Err("an album rule needs the album title".into()),
        (other, _) => Err(format!("unknown sync rule kind {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::folders::add_folder;

    fn song(db: &Db, path: &str, artist: Option<&str>, album: Option<&str>, album_artist: Option<&str>, rating: i64) -> i64 {
        let f = match add_folder(db, r"E:\Nhạc", 1) {
            Ok(f) => f.id,
            Err(_) => db.conn().query_row("SELECT id FROM watched_folders", [], |r| r.get(0)).unwrap(),
        };
        db.conn()
            .execute(
                "INSERT INTO songs (folder_id, path, size, mtime, added_at, format, title, artist, album, album_artist, rating, tags_read)
                 VALUES (?1, ?2, 1, 1, 1, 'mp3', ?2, ?3, ?4, ?5, ?6, 1)",
                rusqlite::params![f, path, artist, album, album_artist, rating],
            )
            .unwrap();
        db.conn().last_insert_rowid()
    }

    fn rating(db: &Db, id: i64) -> i64 {
        db.conn().query_row("SELECT rating FROM songs WHERE id = ?1", [id], |r| r.get(0)).unwrap()
    }

    fn artist(name: &str) -> SyncRule {
        SyncRule { kind: "artist".into(), artist: name.into(), album: None }
    }

    fn album(artist: &str, title: &str) -> SyncRule {
        SyncRule { kind: "album".into(), artist: artist.into(), album: Some(title.into()) }
    }

    #[test]
    fn an_artist_rule_likes_that_artists_songs_but_not_disliked_ones() {
        let db = Db::open_in_memory().unwrap();
        let a = song(&db, "a.mp3", Some("Hà Nhi"), None, None, 0);
        let b = song(&db, "b.mp3", Some("Hà Nhi"), Some("X"), None, 1);
        let c = song(&db, "c.mp3", Some("Hà Nhi"), None, None, -1);
        let other = song(&db, "d.mp3", Some("Ngọc Lan"), None, None, 0);
        let liked = add_rule(&db, &artist("Hà Nhi")).unwrap();
        assert_eq!(liked, vec![a], "only the not-rated song changes");
        assert_eq!([rating(&db, a), rating(&db, b), rating(&db, c), rating(&db, other)], [1, 1, -1, 0]);
    }

    #[test]
    fn an_album_rule_matches_album_artist_or_artist_like_the_albums_view() {
        let db = Db::open_in_memory().unwrap();
        let by_album_artist = song(&db, "1.mp3", Some("Guest"), Some("Tình ca"), Some("Khánh Ly"), 0);
        let by_artist = song(&db, "2.mp3", Some("Khánh Ly"), Some("Tình ca"), Some(""), 0);
        let same_title_other_artist = song(&db, "3.mp3", Some("Ngọc Lan"), Some("Tình ca"), None, 0);
        let other_album = song(&db, "4.mp3", Some("Khánh Ly"), Some("Diễm xưa"), None, 0);
        let mut liked = add_rule(&db, &album("Khánh Ly", "Tình ca")).unwrap();
        liked.sort();
        assert_eq!(liked, vec![by_album_artist, by_artist]);
        assert_eq!(rating(&db, same_title_other_artist), 0);
        assert_eq!(rating(&db, other_album), 0);
    }

    #[test]
    fn songs_that_arrive_later_are_liked_when_rules_are_applied() {
        let db = Db::open_in_memory().unwrap();
        add_rule(&db, &artist("Hà Nhi")).unwrap();
        add_rule(&db, &album("Khánh Ly", "Tình ca")).unwrap();
        let new_song = song(&db, "new.mp3", Some("Hà Nhi"), None, None, 0);
        let new_track = song(&db, "t.mp3", Some("Khánh Ly"), Some("Tình ca"), None, 0);
        let unrelated = song(&db, "u.mp3", Some("Ngọc Lan"), None, None, 0);
        let mut liked = apply_rules(&db).unwrap();
        liked.sort();
        assert_eq!(liked, vec![new_song, new_track]);
        assert_eq!(rating(&db, unrelated), 0);
        assert!(apply_rules(&db).unwrap().is_empty(), "nothing left to do the second time");
    }

    #[test]
    fn rules_are_listed_kept_once_and_can_be_removed() {
        let db = Db::open_in_memory().unwrap();
        add_rule(&db, &artist("Hà Nhi")).unwrap();
        add_rule(&db, &artist("Hà Nhi")).unwrap();
        add_rule(&db, &album("Khánh Ly", "Tình ca")).unwrap();
        assert_eq!(list_rules(&db).unwrap(), vec![album("Khánh Ly", "Tình ca"), artist("Hà Nhi")]);
        remove_rule(&db, &artist("Hà Nhi")).unwrap();
        assert_eq!(list_rules(&db).unwrap(), vec![album("Khánh Ly", "Tình ca")]);
        let later = song(&db, "later.mp3", Some("Hà Nhi"), None, None, 0);
        apply_rules(&db).unwrap();
        assert_eq!(rating(&db, later), 0, "a removed rule no longer likes new songs");
    }

    #[test]
    fn a_rule_must_name_an_artist_and_albums_need_a_title() {
        let db = Db::open_in_memory().unwrap();
        assert!(add_rule(&db, &artist("")).is_err());
        assert!(add_rule(&db, &SyncRule { kind: "album".into(), artist: "Khánh Ly".into(), album: None }).is_err());
        assert!(add_rule(&db, &SyncRule { kind: "genre".into(), artist: "x".into(), album: None }).is_err());
    }
}
