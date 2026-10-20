//! App glue for audiobooks (A.1–A.3): LibriVox search and browse, adding a book (its chapter feed
//! read like a podcast feed, chapters downloaded by the podcast download queue), removing it.

use std::sync::{Arc, Mutex};

use tauri::State;

use crate::audiobook::librivox::{self, Book};
use crate::audiobook::store::{self, BookRow};
use crate::library::service::SharedDb;
use crate::podcast::fetch::{fetch, Fetched};
use crate::podcast::service::{enqueue, remove_download, SharedDownloads};
use crate::podcast::store::EpisodeRow;
use crate::util::time::now_unix;

async fn off_thread<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

/// Search titles and authors; `language` ("English", …, or empty for all) narrows the results.
#[tauri::command]
pub async fn audiobook_search(term: String, language: String) -> Result<Vec<Book>, String> {
    let urls = librivox::search_urls(&term, 50);
    off_thread(move || {
        let lists = urls.iter().map(|u| librivox::parse_books(&librivox::get(u)?)).collect::<Result<Vec<_>, String>>()?;
        Ok(librivox::in_language(librivox::merge(lists), &language))
    })
    .await
}

/// The catalogue's last two months, kept for a few hours.
#[derive(Default)]
pub struct RecentCache(Mutex<Option<(std::time::Instant, Vec<Book>)>>);

const RECENT_TTL: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);

/// Newest books first, optionally in one language.
#[tauri::command]
pub async fn audiobook_recent(language: String, cache: State<'_, Arc<RecentCache>>) -> Result<Vec<Book>, String> {
    let cached = cache.0.lock().map_err(|e| e.to_string())?.as_ref().filter(|(at, _)| at.elapsed() < RECENT_TTL).map(|(_, b)| b.clone());
    let books = match cached {
        Some(b) => b,
        None => {
            let url = librivox::recent_url(now_unix() as i64 - 60 * 86_400, 300);
            let books = off_thread(move || Ok(librivox::newest_first(librivox::parse_books(&librivox::get(&url)?)?))).await?;
            *cache.0.lock().map_err(|e| e.to_string())? = Some((std::time::Instant::now(), books.clone()));
            books
        }
    };
    Ok(librivox::in_language(books, &language))
}

/// Add a book to the library and download its chapters for offline listening.
#[tauri::command]
pub async fn audiobook_add(
    book: Book,
    app: tauri::AppHandle,
    db: State<'_, SharedDb>,
    downloads: State<'_, SharedDownloads>,
) -> Result<Vec<BookRow>, String> {
    let shared = db.inner().clone();
    let (books, wanted) = off_thread(move || {
        let Fetched::Feed { body, .. } = fetch(&book.rss_url, None, None)? else {
            return Err("LibriVox sent no chapter list".into());
        };
        let (_, chapters) = crate::podcast::feed::parse(&book.rss_url, &body)?;
        if chapters.is_empty() {
            return Err("this book has no chapters to download yet".into());
        }
        let d = shared.lock().map_err(|e| e.to_string())?;
        let id = store::add_book(&d, &book, &chapters, now_unix()).map_err(|e| e.to_string())?;
        let wanted = store::not_downloaded(&store::chapters_of(&d, id).map_err(|e| e.to_string())?);
        Ok((store::list_books(&d).map_err(|e| e.to_string())?, wanted))
    })
    .await?;
    enqueue(&app, &db, &downloads, &wanted);
    Ok(books)
}

#[tauri::command]
pub fn audiobook_list(db: State<'_, SharedDb>) -> Result<Vec<BookRow>, String> {
    store::list_books(&*db.lock().map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn audiobook_chapters(id: i64, db: State<'_, SharedDb>) -> Result<Vec<EpisodeRow>, String> {
    store::chapters_of(&*db.lock().map_err(|e| e.to_string())?, id).map_err(|e| e.to_string())
}

/// Download the chapters that are not on this PC yet (after a failure, or a removed download).
#[tauri::command]
pub fn audiobook_download(id: i64, app: tauri::AppHandle, db: State<'_, SharedDb>, downloads: State<'_, SharedDownloads>) -> Result<(), String> {
    let wanted = store::not_downloaded(&store::chapters_of(&*db.lock().map_err(|e| e.to_string())?, id).map_err(|e| e.to_string())?);
    enqueue(&app, &db, &downloads, &wanted);
    Ok(())
}

/// Delete every downloaded chapter of a book (it stays in the library, with its progress).
fn remove_files(db: &SharedDb, id: i64) -> Result<(), String> {
    let chapters = store::chapters_of(&*db.lock().map_err(|e| e.to_string())?, id).map_err(|e| e.to_string())?;
    for c in chapters.iter().filter(|c| c.download_path.is_some()) {
        remove_download(db, c.id)?;
    }
    Ok(())
}

#[tauri::command]
pub fn audiobook_remove_download(id: i64, db: State<'_, SharedDb>) -> Result<Vec<BookRow>, String> {
    remove_files(&db, id)?;
    store::list_books(&*db.lock().map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// Take a book out of the library, with its downloaded chapters and progress.
#[tauri::command]
pub fn audiobook_remove(id: i64, db: State<'_, SharedDb>) -> Result<Vec<BookRow>, String> {
    remove_files(&db, id)?;
    let d = db.lock().map_err(|e| e.to_string())?;
    store::remove_book(&d, id).map_err(|e| e.to_string())?;
    store::list_books(&d).map_err(|e| e.to_string())
}
