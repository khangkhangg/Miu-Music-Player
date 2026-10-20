//! Podcast subscriptions in the library database (P.1): shows and their episodes. Episodes are
//! keyed by the feed's guid, so a refresh adds only what is new and keeps play progress.

use rusqlite::{params, OptionalExtension};
use serde::Serialize;

use crate::library::db::Db;
use crate::podcast::feed::{Episode, Show};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShowRow {
    pub id: i64,
    pub feed_url: String,
    pub title: String,
    pub author: Option<String>,
    pub artwork_url: Option<String>,
    pub last_checked: Option<i64>,
    pub episodes: i64,
    pub unplayed: i64,
    pub auto_download: bool,
    pub keep_latest: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EpisodeRow {
    pub id: i64,
    pub podcast_id: i64,
    pub guid: String,
    pub title: String,
    pub description: Option<String>,
    pub published: Option<i64>,
    pub duration_ms: Option<i64>,
    pub audio_url: String,
    pub position_ms: i64,
    pub played: bool,
    /// When it was finished (seconds since 1970).
    pub played_at: Option<i64>,
    pub download_path: Option<String>,
}

fn upsert_show(tx: &rusqlite::Transaction<'_>, s: &Show, now: u64) -> rusqlite::Result<i64> {
    tx.execute(
        "INSERT INTO podcasts (feed_url, title, author, description, artwork_url, link, language, added_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(feed_url) DO UPDATE SET title = excluded.title, author = excluded.author,
             description = excluded.description, artwork_url = excluded.artwork_url, link = excluded.link, language = excluded.language",
        params![s.feed_url, s.title, s.author, s.description, s.artwork_url, s.link, s.language, now as i64],
    )?;
    tx.query_row("SELECT id FROM podcasts WHERE feed_url = ?1", [&s.feed_url], |r| r.get(0))
}

/// Insert the episodes the show does not have yet; returns how many were new.
pub(crate) fn add_episodes(tx: &rusqlite::Transaction<'_>, id: i64, eps: &[Episode]) -> rusqlite::Result<usize> {
    let mut added = 0;
    for e in eps {
        added += tx.execute(
            "INSERT OR IGNORE INTO episodes (podcast_id, guid, title, description, published, duration_ms, audio_url, audio_type, audio_bytes, season, number)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![id, e.guid, e.title, e.description, e.published, e.duration_ms.map(|d| d as i64), e.audio_url, e.audio_type,
                    e.audio_bytes.map(|b| b as i64), e.season, e.number],
        )?;
    }
    Ok(added)
}

/// Subscribe to a show (or update it if already subscribed). Returns its id.
pub fn subscribe(db: &Db, show: &Show, eps: &[Episode], now: u64) -> rusqlite::Result<i64> {
    let tx = db.conn().unchecked_transaction()?;
    let id = upsert_show(&tx, show, now)?;
    add_episodes(&tx, id, eps)?;
    tx.execute("UPDATE podcasts SET last_checked = ?1 WHERE id = ?2", params![now as i64, id])?;
    tx.commit()?;
    Ok(id)
}

/// Apply a fresh copy of the feed: show details updated, new episodes added (count returned).
pub fn refresh(db: &Db, id: i64, show: &Show, eps: &[Episode], now: u64) -> rusqlite::Result<usize> {
    let tx = db.conn().unchecked_transaction()?;
    tx.execute(
        "UPDATE podcasts SET title = ?1, author = ?2, description = ?3, artwork_url = ?4, link = ?5, language = ?6, last_checked = ?7 WHERE id = ?8",
        params![show.title, show.author, show.description, show.artwork_url, show.link, show.language, now as i64, id],
    )?;
    let added = add_episodes(&tx, id, eps)?;
    tx.commit()?;
    Ok(added)
}

pub fn unsubscribe(db: &Db, id: i64) -> rusqlite::Result<()> {
    db.conn().execute("DELETE FROM podcasts WHERE id = ?1", [id])?;
    Ok(())
}

pub fn list_shows(db: &Db) -> rusqlite::Result<Vec<ShowRow>> {
    let mut stmt = db.conn().prepare(
        "SELECT p.id, p.feed_url, p.title, p.author, p.artwork_url, p.last_checked,
                (SELECT count(*) FROM episodes e WHERE e.podcast_id = p.id),
                (SELECT count(*) FROM episodes e WHERE e.podcast_id = p.id AND e.played = 0),
                p.auto_download, p.keep_latest
         FROM podcasts p WHERE p.kind = 'podcast' ORDER BY p.title COLLATE NOCASE",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok(ShowRow { id: r.get(0)?, feed_url: r.get(1)?, title: r.get(2)?, author: r.get(3)?, artwork_url: r.get(4)?,
                     last_checked: r.get(5)?, episodes: r.get(6)?, unplayed: r.get(7)?,
                     auto_download: r.get(8)?, keep_latest: r.get(9)? })
    })?;
    rows.collect()
}

/// A show's episodes, newest first.
pub fn list_episodes(db: &Db, podcast_id: i64) -> rusqlite::Result<Vec<EpisodeRow>> {
    let mut stmt = db.conn().prepare(
        "SELECT id, podcast_id, guid, title, description, published, duration_ms, audio_url, position_ms, played, download_path, played_at
         FROM episodes WHERE podcast_id = ?1 ORDER BY published DESC, id DESC",
    )?;
    let rows = stmt.query_map([podcast_id], |r| {
        Ok(EpisodeRow { id: r.get(0)?, podcast_id: r.get(1)?, guid: r.get(2)?, title: r.get(3)?, description: r.get(4)?,
                        published: r.get(5)?, duration_ms: r.get(6)?, audio_url: r.get(7)?, position_ms: r.get(8)?,
                        played: r.get(9)?, download_path: r.get(10)?, played_at: r.get(11)? })
    })?;
    rows.collect()
}

/// Where the listener is in an episode, and whether it is finished (finishing records `now`).
pub fn set_progress(db: &Db, episode_id: i64, position_ms: u64, played: bool, now: u64) -> rusqlite::Result<()> {
    db.conn().execute(
        "UPDATE episodes SET position_ms = ?1, played = ?2,
             played_at = CASE WHEN ?2 THEN COALESCE(played_at, ?3) ELSE NULL END
         WHERE id = ?4",
        params![position_ms as i64, played, now as i64, episode_id],
    )?;
    Ok(())
}

/// Record that the feed was checked (nothing changed).
pub fn touch(db: &Db, id: i64, now: u64) -> rusqlite::Result<()> {
    db.conn().execute("UPDATE podcasts SET last_checked = ?1 WHERE id = ?2", params![now as i64, id])?;
    Ok(())
}

/// Cached-feed validators for a conditional refresh (ETag / Last-Modified).
pub fn validators(db: &Db, id: i64) -> rusqlite::Result<Option<(String, Option<String>, Option<String>)>> {
    db.conn()
        .query_row("SELECT feed_url, etag, last_modified FROM podcasts WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .optional()
}

pub fn set_validators(db: &Db, id: i64, etag: Option<&str>, last_modified: Option<&str>) -> rusqlite::Result<()> {
    db.conn().execute("UPDATE podcasts SET etag = ?1, last_modified = ?2 WHERE id = ?3", params![etag, last_modified, id])?;
    Ok(())
}

/// What the downloader needs to know about one episode.
#[derive(Debug, Clone, PartialEq)]
pub struct EpisodeFile {
    pub id: i64,
    pub podcast_id: i64,
    pub title: String,
    pub audio_url: String,
    pub audio_type: Option<String>,
    pub download_path: Option<String>,
}

pub fn episode(db: &Db, id: i64) -> rusqlite::Result<Option<EpisodeFile>> {
    db.conn()
        .query_row("SELECT id, podcast_id, title, audio_url, audio_type, download_path FROM episodes WHERE id = ?1", [id], |r| {
            Ok(EpisodeFile { id: r.get(0)?, podcast_id: r.get(1)?, title: r.get(2)?, audio_url: r.get(3)?, audio_type: r.get(4)?, download_path: r.get(5)? })
        })
        .optional()
}

/// Where the episode's audio is on disk (`None` = not downloaded).
pub fn set_download(db: &Db, id: i64, path: Option<&str>) -> rusqlite::Result<()> {
    db.conn().execute("UPDATE episodes SET download_path = ?1 WHERE id = ?2", params![path, id])?;
    Ok(())
}

/// Download new episodes on their own, keeping the newest `keep_latest`.
pub fn set_show_options(db: &Db, id: i64, auto_download: bool, keep_latest: u32) -> rusqlite::Result<()> {
    db.conn().execute("UPDATE podcasts SET auto_download = ?1, keep_latest = ?2 WHERE id = ?3", params![auto_download, keep_latest, id])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::db::Db;
    use crate::podcast::feed::{Episode, Show};

    fn show(title: &str) -> Show {
        Show { feed_url: "https://example.vn/feed.xml".into(), title: title.into(), author: Some("Đài Mẫu".into()), description: None,
               artwork_url: Some("https://example.vn/a.jpg".into()), link: None, language: Some("vi".into()) }
    }

    fn ep(guid: &str, published: i64) -> Episode {
        Episode { guid: guid.into(), title: format!("Tập {guid}"), description: None, published: Some(published), duration_ms: Some(60_000),
                  audio_url: format!("https://cdn.example.vn/{guid}.mp3"), audio_type: Some("audio/mpeg".into()), audio_bytes: Some(1_000),
                  season: None, number: None }
    }

    #[test]
    fn subscribing_stores_the_show_and_its_episodes_newest_first() {
        let db = Db::open_in_memory().unwrap();
        let id = subscribe(&db, &show("Hôm Nay Nghe Gì"), &[ep("1", 100), ep("2", 200)], 1_000).unwrap();
        let shows = list_shows(&db).unwrap();
        assert_eq!(shows.len(), 1);
        assert_eq!((shows[0].id, shows[0].title.as_str(), shows[0].episodes, shows[0].unplayed), (id, "Hôm Nay Nghe Gì", 2, 2));
        let eps = list_episodes(&db, id).unwrap();
        assert_eq!(eps.iter().map(|e| e.guid.as_str()).collect::<Vec<_>>(), ["2", "1"]);
        assert_eq!(eps[0].audio_url, "https://cdn.example.vn/2.mp3");
    }

    #[test]
    fn the_same_feed_twice_is_one_subscription() {
        let db = Db::open_in_memory().unwrap();
        let a = subscribe(&db, &show("A"), &[ep("1", 1)], 1).unwrap();
        let b = subscribe(&db, &show("A again"), &[ep("1", 1)], 2).unwrap();
        assert_eq!(a, b);
        assert_eq!(list_shows(&db).unwrap().len(), 1);
    }

    #[test]
    fn refreshing_adds_only_new_episodes_and_keeps_progress() {
        let db = Db::open_in_memory().unwrap();
        let id = subscribe(&db, &show("Show"), &[ep("1", 100)], 1).unwrap();
        let first = list_episodes(&db, id).unwrap()[0].id;
        set_progress(&db, first, 42_000, false, 1).unwrap();
        let added = refresh(&db, id, &show("Show (renamed)"), &[ep("1", 100), ep("2", 200)], 5).unwrap();
        assert_eq!(added, 1);
        let eps = list_episodes(&db, id).unwrap();
        assert_eq!(eps.len(), 2);
        let old = eps.iter().find(|e| e.guid == "1").unwrap();
        assert_eq!((old.position_ms, old.played), (42_000, false), "progress kept");
        let s = &list_shows(&db).unwrap()[0];
        assert_eq!((s.title.as_str(), s.last_checked), ("Show (renamed)", Some(5)));
    }

    #[test]
    fn a_finished_episode_counts_as_played() {
        let db = Db::open_in_memory().unwrap();
        let id = subscribe(&db, &show("Show"), &[ep("1", 1), ep("2", 2)], 1).unwrap();
        let e = list_episodes(&db, id).unwrap()[0].id;
        set_progress(&db, e, 60_000, true, 1).unwrap();
        assert_eq!(list_shows(&db).unwrap()[0].unplayed, 1);
    }

    #[test]
    fn unsubscribing_removes_the_show_and_its_episodes() {
        let db = Db::open_in_memory().unwrap();
        let id = subscribe(&db, &show("Show"), &[ep("1", 1)], 1).unwrap();
        unsubscribe(&db, id).unwrap();
        assert!(list_shows(&db).unwrap().is_empty());
        assert!(list_episodes(&db, id).unwrap().is_empty());
    }
}

#[cfg(test)]
mod offline_tests {
    use super::*;
    use crate::library::db::Db;
    use crate::podcast::feed::{Episode, Show};

    fn setup() -> (Db, i64, i64) {
        let db = Db::open_in_memory().unwrap();
        let show = Show { feed_url: "https://x/f".into(), title: "S".into(), author: None, description: None, artwork_url: None, link: None, language: None };
        let ep = Episode { guid: "1".into(), title: "E".into(), description: None, published: Some(1), duration_ms: None,
                           audio_url: "https://x/1.mp3".into(), audio_type: Some("audio/mpeg".into()), audio_bytes: None, season: None, number: None };
        let id = subscribe(&db, &show, &[ep], 1).unwrap();
        let e = list_episodes(&db, id).unwrap()[0].id;
        (db, id, e)
    }

    #[test]
    fn shows_do_not_auto_download_until_asked_and_keep_three() {
        let (db, id, _) = setup();
        let s = &list_shows(&db).unwrap()[0];
        assert_eq!((s.auto_download, s.keep_latest), (false, 3));
        set_show_options(&db, id, true, 5).unwrap();
        let s = &list_shows(&db).unwrap()[0];
        assert_eq!((s.auto_download, s.keep_latest), (true, 5));
    }

    #[test]
    fn a_download_is_recorded_and_can_be_cleared() {
        let (db, _, e) = setup();
        set_download(&db, e, Some(r"C:\podcasts\1\1.mp3")).unwrap();
        let row = episode(&db, e).unwrap().unwrap();
        assert_eq!(row.download_path.as_deref(), Some(r"C:\podcasts\1\1.mp3"));
        assert_eq!(row.audio_type.as_deref(), Some("audio/mpeg"));
        set_download(&db, e, None).unwrap();
        assert_eq!(episode(&db, e).unwrap().unwrap().download_path, None);
    }

    #[test]
    fn finishing_records_the_time_once_and_unplaying_clears_it() {
        let (db, id, e) = setup();
        set_progress(&db, e, 10, true, 100).unwrap();
        set_progress(&db, e, 10, true, 200).unwrap();
        assert_eq!(list_episodes(&db, id).unwrap()[0].played_at, Some(100), "first finish counts");
        set_progress(&db, e, 0, false, 300).unwrap();
        assert_eq!(list_episodes(&db, id).unwrap()[0].played_at, None);
    }
}
