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

// ---- Offline listening (P.3): download queue and periodic upkeep ----

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Episodes waiting to download, worked through one at a time on a background thread.
#[derive(Default)]
pub struct Downloads {
    queue: Mutex<VecDeque<i64>>,
    running: AtomicBool,
}

pub type SharedDownloads = Arc<Downloads>;

fn podcasts_dir() -> std::path::PathBuf {
    crate::util::paths::local_appdata_root().join("MiuMusic").join("podcasts")
}

/// Queue episodes for download (already queued or downloaded ones are left alone) and make sure
/// the worker runs.
pub fn enqueue(app: &tauri::AppHandle, db: &SharedDb, downloads: &SharedDownloads, ids: &[i64]) {
    use tauri::Emitter;
    if let Ok(mut q) = downloads.queue.lock() {
        for id in ids {
            if !q.contains(id) {
                q.push_back(*id);
            }
        }
    }
    if downloads.running.swap(true, Ordering::AcqRel) {
        return;
    }
    let (app, db, downloads) = (app.clone(), db.clone(), downloads.clone());
    std::thread::spawn(move || {
        loop {
            let Some(id) = downloads.queue.lock().ok().and_then(|mut q| q.pop_front()) else { break };
            let Some(ep) = db.lock().ok().and_then(|d| store::episode(&d, id).ok().flatten()) else { continue };
            if ep.download_path.is_some() {
                continue;
            }
            let ext = crate::podcast::download::extension_for(&ep.audio_url, ep.audio_type.as_deref());
            let dest = podcasts_dir().join(ep.podcast_id.to_string()).join(format!("{id}.{ext}"));
            let title = ep.title.clone();
            let result = crate::podcast::download::download(&ep.audio_url, &dest, |done, total| {
                let _ = app.emit("library-progress", serde_json::json!({ "phase": "download", "done": done, "total": total.unwrap_or(0), "title": title }));
            });
            match result {
                Ok(_) => {
                    if let Ok(d) = db.lock() {
                        let _ = store::set_download(&d, id, Some(&dest.to_string_lossy()));
                    }
                }
                Err(e) => {
                    log::warn!("download of episode {id} failed: {e}");
                    let _ = app.emit("podcast-download-failed", serde_json::json!({ "id": id, "error": e }));
                }
            }
            let _ = app.emit("library-progress", serde_json::json!({ "phase": "download", "done": 0, "total": 0 }));
            let _ = app.emit("podcasts-changed", id);
        }
        downloads.running.store(false, Ordering::Release);
    });
}

/// Delete the downloaded file of an episode and forget it.
pub(crate) fn remove_download(db: &SharedDb, id: i64) -> Result<(), String> {
    let d = db.lock().map_err(|e| e.to_string())?;
    if let Some(path) = store::episode(&d, id).map_err(|e| e.to_string())?.and_then(|e| e.download_path) {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
            _ => {}
        }
    }
    store::set_download(&d, id, None).map_err(|e| e.to_string())
}

/// Check every show for new episodes, queue auto-downloads, clear old played downloads.
pub fn upkeep(app: &tauri::AppHandle, db: &SharedDb, downloads: &SharedDownloads) {
    use tauri::Emitter;
    let shows = db.lock().ok().and_then(|d| store::list_shows(&d).ok()).unwrap_or_default();
    let cleanup_days = db.lock().ok().and_then(|d| crate::settings::podcast_cleanup_days(&d));
    let mut wanted = Vec::new();
    for s in &shows {
        let validators = db.lock().ok().and_then(|d| store::validators(&d, s.id).ok().flatten());
        if let Some((url, etag, modified)) = validators {
            match fetch(&url, etag.as_deref(), modified.as_deref()) {
                Ok(Fetched::Feed { body, etag, last_modified }) => {
                    if let (Ok((show, eps)), Ok(d)) = (feed::parse(&url, &body), db.lock()) {
                        let _ = store::refresh(&d, s.id, &show, &eps, now_unix());
                        let _ = store::set_validators(&d, s.id, etag.as_deref(), last_modified.as_deref());
                    }
                }
                Ok(Fetched::NotModified) => {
                    if let Ok(d) = db.lock() {
                        let _ = store::touch(&d, s.id, now_unix());
                    }
                }
                Err(e) => log::warn!("refresh of {url} failed: {e}"),
            }
        }
        let eps = db.lock().ok().and_then(|d| store::list_episodes(&d, s.id).ok()).unwrap_or_default();
        if s.auto_download {
            wanted.extend(crate::podcast::policy::to_download(&eps, s.keep_latest.max(0) as usize));
        }
        if let Some(days) = cleanup_days {
            for id in crate::podcast::policy::to_clean_up(&eps, now_unix() as i64, days) {
                if let Err(e) = remove_download(db, id) {
                    log::warn!("could not clear episode {id}: {e}");
                }
            }
        }
    }
    let _ = app.emit("podcasts-changed", 0);
    enqueue(app, db, downloads, &wanted);
}

/// Upkeep shortly after start-up, then every three hours.
pub fn start(app: tauri::AppHandle, db: SharedDb, downloads: SharedDownloads) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(20));
        upkeep(&app, &db, &downloads);
        std::thread::sleep(std::time::Duration::from_secs(3 * 60 * 60));
    });
}

#[tauri::command]
pub fn podcast_download(id: i64, app: tauri::AppHandle, db: State<'_, SharedDb>, downloads: State<'_, SharedDownloads>) {
    enqueue(&app, &db, &downloads, &[id]);
}

#[tauri::command]
pub fn podcast_delete_download(id: i64, db: State<'_, SharedDb>) -> Result<(), String> {
    remove_download(&db, id)
}

#[tauri::command]
pub fn podcast_set_played(id: i64, played: bool, db: State<'_, SharedDb>) -> Result<(), String> {
    let d = db.lock().map_err(|e| e.to_string())?;
    store::set_progress(&d, id, 0, played, now_unix()).map_err(|e| e.to_string())
}

/// Turn auto-download on or off for a show; turning it on fetches the newest episodes now.
#[tauri::command]
pub fn podcast_set_options(
    id: i64,
    auto_download: bool,
    keep_latest: u32,
    app: tauri::AppHandle,
    db: State<'_, SharedDb>,
    downloads: State<'_, SharedDownloads>,
) -> Result<Vec<ShowRow>, String> {
    let keep = keep_latest.clamp(1, 50);
    let (shows, wanted) = {
        let d = db.lock().map_err(|e| e.to_string())?;
        store::set_show_options(&d, id, auto_download, keep).map_err(|e| e.to_string())?;
        let eps = store::list_episodes(&d, id).map_err(|e| e.to_string())?;
        let wanted = if auto_download { crate::podcast::policy::to_download(&eps, keep as usize) } else { Vec::new() };
        (store::list_shows(&d).map_err(|e| e.to_string())?, wanted)
    };
    enqueue(&app, &db, &downloads, &wanted);
    Ok(shows)
}

// ---- Discover (P.4): Apple charts and search ----

use crate::podcast::discover::{self, Found};

/// Charts per country, kept for a few hours (they change daily at most).
#[derive(Default)]
pub struct DiscoverCache(Mutex<std::collections::HashMap<String, (std::time::Instant, Vec<Found>)>>);

const CHART_TTL: std::time::Duration = std::time::Duration::from_secs(6 * 60 * 60);

fn get_text(url: &str) -> Result<String, String> {
    match fetch(url, None, None)? {
        Fetched::Feed { body, .. } => String::from_utf8(body).map_err(|e| e.to_string()),
        Fetched::NotModified => Err("no answer".into()),
    }
}

/// Top podcasts in a country ("vn", "us"), cached for six hours.
#[tauri::command]
pub async fn podcast_chart(country: String, cache: State<'_, Arc<DiscoverCache>>) -> Result<Vec<Found>, String> {
    let key = country.to_ascii_lowercase();
    if let Some((at, shows)) = cache.0.lock().map_err(|e| e.to_string())?.get(&key) {
        if at.elapsed() < CHART_TTL {
            return Ok(shows.clone());
        }
    }
    let url = discover::chart_url(&key, 50)?;
    let shows = off_thread(move || discover::parse_chart(&get_text(&url)?)).await?;
    cache.0.lock().map_err(|e| e.to_string())?.insert(key, (std::time::Instant::now(), shows.clone()));
    Ok(shows)
}

#[tauri::command]
pub async fn podcast_search(term: String, country: String) -> Result<Vec<Found>, String> {
    if term.trim().is_empty() {
        return Ok(Vec::new());
    }
    let url = discover::search_url(&term, &country)?;
    off_thread(move || discover::parse_search(&get_text(&url)?)).await
}

/// Subscribe to a Discover result: its feed address, looked up from Apple when a chart gave none.
#[tauri::command]
pub async fn podcast_subscribe_found(apple_id: i64, feed_url: Option<String>, db: State<'_, SharedDb>) -> Result<Vec<ShowRow>, String> {
    let url = match feed_url {
        Some(u) => u,
        None => off_thread(move || discover::parse_lookup(&get_text(&discover::lookup_url(apple_id))?).ok_or_else(|| "this podcast has no public feed".to_string())).await?,
    };
    podcast_subscribe(url, db).await
}

/// Title, show and artwork of an episode, for the now-playing column (P.5).
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NowEpisode {
    title: String,
    show: String,
    artwork_url: Option<String>,
    duration_ms: Option<i64>,
}

#[tauri::command]
pub fn podcast_now(id: i64, db: State<'_, SharedDb>) -> Result<NowEpisode, String> {
    let d = db.lock().map_err(|e| e.to_string())?;
    d.conn()
        .query_row(
            "SELECT e.title, p.title, p.artwork_url, e.duration_ms FROM episodes e JOIN podcasts p ON p.id = e.podcast_id WHERE e.id = ?1",
            [id],
            |r| Ok(NowEpisode { title: r.get(0)?, show: r.get(1)?, artwork_url: r.get(2)?, duration_ms: r.get(3)? }),
        )
        .map_err(|e| e.to_string())
}
