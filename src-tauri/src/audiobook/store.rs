//! Audiobooks in the library database (A.2): a book is a row in `podcasts` with kind 'audiobook'
//! and its chapters are `episodes`, so downloads, progress and playback are the podcast ones.
//! Books stay out of the podcast list (and its feed refreshes and clean-up).

use rusqlite::params;
use serde::Serialize;

use crate::audiobook::librivox::Book;
use crate::library::db::Db;
use crate::podcast::feed::Episode;
use crate::podcast::store::{add_episodes, EpisodeRow};

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BookRow {
    pub id: i64,
    pub feed_url: String,
    pub title: String,
    pub author: Option<String>,
    pub description: Option<String>,
    pub artwork_url: Option<String>,
    pub language: Option<String>,
    pub chapters: i64,
    pub downloaded: i64,
    pub finished: i64,
    pub duration_ms: Option<i64>,
    /// Where listening goes on: the first unfinished chapter (`None` once the book is done).
    pub resume_chapter: Option<i64>,
    pub resume_title: Option<String>,
    pub resume_ms: i64,
}

/// Add a LibriVox book and its chapters (from its feed). Adding it again keeps progress.
pub fn add_book(db: &Db, book: &Book, chapters: &[Episode], now: u64) -> rusqlite::Result<i64> {
    let tx = db.conn().unchecked_transaction()?;
    tx.execute(
        "INSERT INTO podcasts (feed_url, title, author, description, artwork_url, link, language, added_at, last_checked, kind)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, 'audiobook')
         ON CONFLICT(feed_url) DO UPDATE SET title = excluded.title, author = excluded.author, description = excluded.description,
             artwork_url = excluded.artwork_url, link = excluded.link, language = excluded.language, kind = 'audiobook'",
        params![book.rss_url, book.title, Some(&book.author).filter(|a| !a.is_empty()), book.description, book.cover_url, book.page_url,
                Some(&book.language).filter(|l| !l.is_empty()), now as i64],
    )?;
    let id = tx.query_row("SELECT id FROM podcasts WHERE feed_url = ?1", [&book.rss_url], |r| r.get(0))?;
    add_episodes(&tx, id, chapters)?;
    tx.commit()?;
    Ok(id)
}

pub fn remove_book(db: &Db, id: i64) -> rusqlite::Result<()> {
    db.conn().execute("DELETE FROM podcasts WHERE id = ?1 AND kind = 'audiobook'", [id])?;
    Ok(())
}

/// A book's chapters in reading order.
pub fn chapters_of(db: &Db, book_id: i64) -> rusqlite::Result<Vec<EpisodeRow>> {
    let mut stmt = db.conn().prepare(
        "SELECT id, podcast_id, guid, title, description, published, duration_ms, audio_url, position_ms, played, download_path, played_at
         FROM episodes WHERE podcast_id = ?1 ORDER BY number IS NULL, number, id",
    )?;
    let rows = stmt.query_map([book_id], |r| {
        Ok(EpisodeRow { id: r.get(0)?, podcast_id: r.get(1)?, guid: r.get(2)?, title: r.get(3)?, description: r.get(4)?,
                        published: r.get(5)?, duration_ms: r.get(6)?, audio_url: r.get(7)?, position_ms: r.get(8)?,
                        played: r.get(9)?, download_path: r.get(10)?, played_at: r.get(11)? })
    })?;
    rows.collect()
}

/// The first unfinished chapter and where in it the listener is; `None` when every one is done.
pub fn resume_point(chapters: &[EpisodeRow]) -> Option<(i64, i64)> {
    chapters.iter().find(|c| !c.played).map(|c| (c.id, c.position_ms))
}

/// Chapters on this PC from `start` on, in order, until the first one that is not.
pub fn play_queue(chapters: &[EpisodeRow], start: i64) -> Vec<(i64, String)> {
    chapters
        .iter()
        .skip_while(|c| c.id != start)
        .map_while(|c| c.download_path.clone().map(|p| (c.id, p)))
        .collect()
}

pub fn not_downloaded(chapters: &[EpisodeRow]) -> Vec<i64> {
    chapters.iter().filter(|c| c.download_path.is_none()).map(|c| c.id).collect()
}

pub fn list_books(db: &Db) -> rusqlite::Result<Vec<BookRow>> {
    let mut stmt = db.conn().prepare(
        "SELECT id, feed_url, title, author, description, artwork_url, language FROM podcasts WHERE kind = 'audiobook' ORDER BY title COLLATE NOCASE",
    )?;
    let heads = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)))?
        .collect::<rusqlite::Result<Vec<(i64, String, String, Option<String>, Option<String>, Option<String>, Option<String>)>>>()?;
    heads
        .into_iter()
        .map(|(id, feed_url, title, author, description, artwork_url, language)| {
            let ch = chapters_of(db, id)?;
            let resume = resume_point(&ch);
            let total: i64 = ch.iter().filter_map(|c| c.duration_ms).sum();
            Ok(BookRow {
                id,
                feed_url,
                title,
                author,
                description,
                artwork_url,
                language,
                chapters: ch.len() as i64,
                downloaded: ch.iter().filter(|c| c.download_path.is_some()).count() as i64,
                finished: ch.iter().filter(|c| c.played).count() as i64,
                duration_ms: (total > 0).then_some(total),
                resume_chapter: resume.map(|r| r.0),
                resume_title: resume.and_then(|(rid, _)| ch.iter().find(|c| c.id == rid)).map(|c| c.title.clone()),
                resume_ms: resume.map_or(0, |r| r.1),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audiobook::librivox::Book;
    use crate::library::db::Db;
    use crate::podcast::feed::Episode;
    use crate::podcast::store as podcasts;

    fn book() -> Book {
        Book { id: 449, title: "Treasure Island".into(), author: "Robert Louis Stevenson".into(), language: "English".into(),
               description: Some("Buccaneers and buried gold.".into()), duration_ms: Some(27_008_000), chapters: 3,
               rss_url: "https://librivox.org/rss/449".into(), page_url: None, cover_url: Some("https://archive.org/x.jpg".into()) }
    }

    fn chapter(n: u32, ms: u64) -> Episode {
        Episode { guid: format!("https://archive.org/ti_{n:02}.mp3"), title: format!("Chapter {n}"), description: None, published: None,
                  duration_ms: Some(ms), audio_url: format!("https://archive.org/ti_{n:02}.mp3"), audio_type: Some("audio/mpeg".into()),
                  audio_bytes: None, season: None, number: Some(n) }
    }

    /// The feed lists chapters in order, but nothing should depend on it.
    fn chapters() -> Vec<Episode> {
        vec![chapter(2, 1_759_000), chapter(1, 1_520_000), chapter(3, 1_693_000)]
    }

    #[test]
    fn a_book_is_stored_with_its_catalogue_details_and_chapters_in_order() {
        let db = Db::open_in_memory().unwrap();
        let id = add_book(&db, &book(), &chapters(), 1_000).unwrap();
        let books = list_books(&db).unwrap();
        assert_eq!(books.len(), 1);
        let b = &books[0];
        assert_eq!((b.id, b.title.as_str(), b.author.as_deref()), (id, "Treasure Island", Some("Robert Louis Stevenson")));
        assert_eq!(b.artwork_url.as_deref(), Some("https://archive.org/x.jpg"));
        assert_eq!((b.chapters, b.downloaded, b.finished), (3, 0, 0));
        assert_eq!(b.duration_ms, Some(1_520_000 + 1_759_000 + 1_693_000));
        let ch = chapters_of(&db, id).unwrap();
        assert_eq!(ch.iter().map(|c| c.title.as_str()).collect::<Vec<_>>(), ["Chapter 1", "Chapter 2", "Chapter 3"]);
    }

    #[test]
    fn books_are_not_podcasts() {
        let db = Db::open_in_memory().unwrap();
        add_book(&db, &book(), &chapters(), 1).unwrap();
        assert!(podcasts::list_shows(&db).unwrap().is_empty(), "no feed refresh, auto-download or clean-up for books");
    }

    #[test]
    fn adding_the_same_book_again_keeps_progress() {
        let db = Db::open_in_memory().unwrap();
        let id = add_book(&db, &book(), &chapters(), 1).unwrap();
        let first = chapters_of(&db, id).unwrap()[0].id;
        podcasts::set_progress(&db, first, 60_000, false, 2).unwrap();
        assert_eq!(add_book(&db, &book(), &chapters(), 3).unwrap(), id);
        assert_eq!(chapters_of(&db, id).unwrap()[0].position_ms, 60_000);
        assert_eq!(list_books(&db).unwrap().len(), 1);
    }

    #[test]
    fn a_book_resumes_in_the_first_unfinished_chapter() {
        let db = Db::open_in_memory().unwrap();
        let id = add_book(&db, &book(), &chapters(), 1).unwrap();
        let ch = chapters_of(&db, id).unwrap();
        assert_eq!(resume_point(&ch), Some((ch[0].id, 0)), "a new book starts at chapter 1");
        podcasts::set_progress(&db, ch[0].id, 0, true, 2).unwrap();
        podcasts::set_progress(&db, ch[1].id, 95_000, false, 3).unwrap();
        let ch = chapters_of(&db, id).unwrap();
        assert_eq!(resume_point(&ch), Some((ch[1].id, 95_000)));
        let b = &list_books(&db).unwrap()[0];
        assert_eq!(b.finished, 1);
        assert_eq!((b.resume_chapter, b.resume_title.as_deref(), b.resume_ms), (Some(ch[1].id), Some("Chapter 2"), 95_000));
        for c in &ch {
            podcasts::set_progress(&db, c.id, 0, true, 4).unwrap();
        }
        assert_eq!(resume_point(&chapters_of(&db, id).unwrap()), None, "finished");
        assert_eq!(list_books(&db).unwrap()[0].resume_chapter, None);
    }

    #[test]
    fn playing_on_goes_through_the_downloaded_chapters_that_follow() {
        let db = Db::open_in_memory().unwrap();
        let id = add_book(&db, &book(), &chapters(), 1).unwrap();
        let ch = chapters_of(&db, id).unwrap();
        for c in &ch[..2] {
            podcasts::set_download(&db, c.id, Some(&format!(r"C:\books\{}.mp3", c.id))).unwrap();
        }
        let ch = chapters_of(&db, id).unwrap();
        assert_eq!(list_books(&db).unwrap()[0].downloaded, 2);
        let q = play_queue(&ch, ch[0].id);
        assert_eq!(q.iter().map(|(i, _)| *i).collect::<Vec<_>>(), [ch[0].id, ch[1].id], "chapter 3 is not on this PC");
        assert_eq!(q[1].1, format!(r"C:\books\{}.mp3", ch[1].id));
        assert_eq!(play_queue(&ch, ch[1].id).len(), 1, "starts at the chosen chapter");
        assert!(play_queue(&ch, ch[2].id).is_empty(), "not downloaded");
        assert_eq!(not_downloaded(&ch), [ch[2].id]);
    }

    #[test]
    fn removing_a_book_takes_its_chapters_too() {
        let db = Db::open_in_memory().unwrap();
        let id = add_book(&db, &book(), &chapters(), 1).unwrap();
        remove_book(&db, id).unwrap();
        assert!(list_books(&db).unwrap().is_empty());
        assert!(chapters_of(&db, id).unwrap().is_empty());
    }
}
