//! App glue for podcasts (P.1): commands. Network work runs off the UI thread and never holds the
//! library database while downloading.

use tauri::State;

use crate::library::service::SharedDb;
use crate::podcast::fetch::{fetch, Fetched};
use crate::podcast::feed;
use crate::podcast::store::{self, EpisodeRow, ShowRow};
use crate::util::time::now_unix;

async fn off_thread<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

/// Subscribe to the feed at `url`; returns every show.
#[tauri::command]
pub async fn podcast_subscribe(url: String, db: State<'_, SharedDb>) -> Result<Vec<ShowRow>, String> {
    let db = db.inner().clone();
    off_thread(move || {
        let Fetched::Feed { body, etag, last_modified } = fetch(&url, None, None)? else {
            return Err("the server sent no feed".into());
        };
        let (show, episodes) = feed::parse(url.trim(), &body)?;
        let d = db.lock().map_err(|e| e.to_string())?;
        let id = store::subscribe(&d, &show, &episodes, now_unix()).map_err(|e| e.to_string())?;
        store::set_validators(&d, id, etag.as_deref(), last_modified.as_deref()).map_err(|e| e.to_string())?;
        store::list_shows(&d).map_err(|e| e.to_string())
    })
    .await
}

/// Check a show for new episodes; returns how many arrived.
#[tauri::command]
pub async fn podcast_refresh(id: i64, db: State<'_, SharedDb>) -> Result<usize, String> {
    let db = db.inner().clone();
    off_thread(move || {
        let (url, etag, modified) = {
            let d = db.lock().map_err(|e| e.to_string())?;
            store::validators(&d, id).map_err(|e| e.to_string())?.ok_or("no such podcast")?
        };
        let fetched = fetch(&url, etag.as_deref(), modified.as_deref())?;
        let d = db.lock().map_err(|e| e.to_string())?;
        match fetched {
            Fetched::NotModified => store::touch(&d, id, now_unix()).map(|_| 0).map_err(|e| e.to_string()),
            Fetched::Feed { body, etag, last_modified } => {
                let (show, episodes) = feed::parse(&url, &body)?;
                let added = store::refresh(&d, id, &show, &episodes, now_unix()).map_err(|e| e.to_string())?;
                store::set_validators(&d, id, etag.as_deref(), last_modified.as_deref()).map_err(|e| e.to_string())?;
                Ok(added)
            }
        }
    })
    .await
}

#[tauri::command]
pub fn podcast_list(db: State<'_, SharedDb>) -> Result<Vec<ShowRow>, String> {
    store::list_shows(&*db.lock().map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn podcast_episodes(id: i64, db: State<'_, SharedDb>) -> Result<Vec<EpisodeRow>, String> {
    store::list_episodes(&*db.lock().map_err(|e| e.to_string())?, id).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn podcast_unsubscribe(id: i64, db: State<'_, SharedDb>) -> Result<Vec<ShowRow>, String> {
    let d = db.lock().map_err(|e| e.to_string())?;
    store::unsubscribe(&d, id).map_err(|e| e.to_string())?;
    store::list_shows(&d).map_err(|e| e.to_string())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportResult {
    report: crate::podcast::opml::ImportReport,
    shows: Vec<ShowRow>,
}

/// Subscribe to every feed in an OPML file (from another podcast app), with progress events.
#[tauri::command]
pub async fn podcast_import_opml(path: String, app: tauri::AppHandle, db: State<'_, SharedDb>) -> Result<ImportResult, String> {
    use crate::podcast::opml::{import_feeds, parse, Downloaded};
    use tauri::Emitter;
    let db = db.inner().clone();
    off_thread(move || {
        let feeds = parse(&std::fs::read(&path).map_err(|e| e.to_string())?)?;
        let known: std::collections::HashSet<String> = {
            let d = db.lock().map_err(|e| e.to_string())?;
            store::list_shows(&d).map_err(|e| e.to_string())?.into_iter().map(|s| s.feed_url).collect()
        };
        let report = import_feeds(
            &feeds,
            |url| known.contains(url),
            |url| match fetch(url, None, None)? {
                Fetched::Feed { body, etag, last_modified } => {
                    let (show, episodes) = feed::parse(url, &body)?;
                    Ok(Downloaded { show, episodes, etag, last_modified })
                }
                Fetched::NotModified => Err("the server sent no feed".into()),
            },
            |d| {
                let db = db.lock().map_err(|e| e.to_string())?;
                let id = store::subscribe(&db, &d.show, &d.episodes, now_unix()).map_err(|e| e.to_string())?;
                store::set_validators(&db, id, d.etag.as_deref(), d.last_modified.as_deref()).map_err(|e| e.to_string())
            },
            |done, total| {
                let _ = app.emit("library-progress", serde_json::json!({ "phase": "import", "done": done, "total": total }));
            },
        );
        let shows = store::list_shows(&*db.lock().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        Ok(ImportResult { report, shows })
    })
    .await
}

/// Save every subscription as an OPML file other podcast apps can import.
#[tauri::command]
pub fn podcast_export_opml(path: String, db: State<'_, SharedDb>) -> Result<usize, String> {
    use crate::podcast::opml::{export, OpmlFeed};
    let shows = store::list_shows(&*db.lock().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let feeds: Vec<OpmlFeed> = shows.into_iter().map(|s| OpmlFeed { url: s.feed_url, title: Some(s.title) }).collect();
    std::fs::write(&path, export(&feeds)).map_err(|e| e.to_string())?;
    Ok(feeds.len())
}
