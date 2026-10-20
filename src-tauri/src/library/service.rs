//! App glue for watched folders (F3.1): initial rescan, live watching, Tauri commands, and a
//! `library-changed` event carrying the folder list. The logic lives in `folders`, `scan`, `watch`.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Emitter, State};

use crate::library::db::Db;
use crate::library::duplicates::{pending_hashes, store_hash};
use crate::library::fingerprint::audio_hash;
use crate::library::folders::{self, WatchedFolder};
use crate::library::scan::rescan_folder;
use crate::library::watch::FolderWatcher;
use crate::util::time::now_unix;

pub type SharedDb = Arc<Mutex<Db>>;

/// Holds the current watcher so it can be replaced when folders change.
#[derive(Default)]
pub struct LibraryWatch(Mutex<Option<FolderWatcher>>);

fn folder_list(db: &SharedDb) -> Vec<WatchedFolder> {
    db.lock().ok().and_then(|d| folders::list_folders(&d).ok()).unwrap_or_default()
}

/// Background work for the progress bar: `phase` is "scan" or "fingerprint".
#[derive(Clone, serde::Serialize)]
pub struct Progress {
    phase: &'static str,
    done: usize,
    total: usize,
}

fn progress(app: &AppHandle, phase: &'static str, done: usize, total: usize) {
    let _ = app.emit("library-progress", Progress { phase, done, total });
}

fn rescan(app: &AppHandle, db: &SharedDb, ids: &[i64]) {
    let folders: Vec<WatchedFolder> = folder_list(db).into_iter().filter(|f| ids.contains(&f.id)).collect();
    let total = folders.len();
    for (i, f) in folders.into_iter().enumerate() {
        progress(app, "scan", i, total);
        let Ok(d) = db.lock() else { return };
        match rescan_folder(&d, f.id, &f.path, now_unix()) {
            Ok(diff) => log::info!("rescanned {}: +{} ~{} -{}", f.path, diff.added, diff.updated, diff.removed),
            Err(e) => log::warn!("rescan of {} failed: {e}", f.path),
        }
    }
    if let Ok(d) = db.lock() {
        match crate::library::sync_rules::apply_rules(&d) {
            Ok(liked) if !liked.is_empty() => log::info!("always-sync rules liked {} new songs", liked.len()),
            Ok(_) => {}
            Err(e) => log::warn!("applying always-sync rules failed: {e}"),
        }
    }
    progress(app, "scan", total, total);
}

static HASHING: AtomicBool = AtomicBool::new(false);

/// Fingerprint new and changed songs for duplicate detection (F3.6), a batch at a time with the
/// database unlocked while decoding; announces `library-changed` when done.
fn hash_in_background(app: &AppHandle, db: &SharedDb) {
    if HASHING.swap(true, Ordering::AcqRel) {
        return; // the running pass picks up new songs too
    }
    let (app, db) = (app.clone(), db.clone());
    std::thread::spawn(move || {
        let mut done = 0;
        let total = db.lock().ok().and_then(|d| crate::library::duplicates::count_pending(&d).ok()).unwrap_or(0);
        loop {
            progress(&app, "fingerprint", done, total.max(done));
            let todo = match db.lock() {
                Ok(d) => pending_hashes(&d, 25).unwrap_or_default(),
                Err(_) => break,
            };
            if todo.is_empty() {
                break;
            }
            let hashed: Vec<(i64, Option<String>)> = todo.iter().map(|(id, p)| (*id, audio_hash(p).ok())).collect();
            let Ok(d) = db.lock() else { break };
            for (id, h) in &hashed {
                let _ = store_hash(&d, *id, h.as_deref());
            }
            done += hashed.len();
        }
        HASHING.store(false, Ordering::Release);
        progress(&app, "fingerprint", done, done);
        if done > 0 {
            log::info!("fingerprinted {done} songs");
            let _ = app.emit("library-changed", folder_list(&db));
        }
    });
}

/// (Re)start watching every folder; changes trigger a rescan and a `library-changed` event.
fn restart_watch(app: &AppHandle, db: &SharedDb, watch: &LibraryWatch) {
    let folders: Vec<(i64, String)> = folder_list(db).into_iter().map(|f| (f.id, f.path)).collect();
    let (app2, db2) = (app.clone(), db.clone());
    let watcher = FolderWatcher::start(folders, Duration::from_millis(800), move |ids| {
        rescan(&app2, &db2, &ids);
        let _ = app2.emit("library-changed", folder_list(&db2));
        hash_in_background(&app2, &db2);
    });
    match watcher {
        Ok(w) => *watch.0.lock().unwrap() = Some(w),
        Err(e) => log::warn!("watching folders failed: {e}"),
    }
}

/// At startup: catch up with changes made while Miu was closed, then watch.
pub fn start(app: AppHandle, db: SharedDb, watch: Arc<LibraryWatch>) {
    std::thread::spawn(move || {
        let ids: Vec<i64> = folder_list(&db).iter().map(|f| f.id).collect();
        rescan(&app, &db, &ids);
        restart_watch(&app, &db, &watch);
        let _ = app.emit("library-changed", folder_list(&db));
        hash_in_background(&app, &db);
    });
}

#[tauri::command]
pub fn list_songs(db: State<'_, SharedDb>) -> Result<Vec<crate::library::songs::SongRow>, String> {
    let d = db.lock().map_err(|e| e.to_string())?;
    crate::library::songs::list_songs(&d).map_err(|e| e.to_string())
}

/// `rating`: 1 liked, 0 not rated, -1 disliked.
#[tauri::command]
pub fn rate_song(id: i64, rating: i64, app: AppHandle, db: State<'_, SharedDb>) -> Result<(), String> {
    let value = crate::library::songs::Rating::from_value(rating).ok_or("rating must be -1, 0 or 1")?;
    {
        let d = db.lock().map_err(|e| e.to_string())?;
        crate::library::songs::set_rating(&d, id, value)?;
    }
    // Both windows (main and mini) keep their song lists in step.
    let _ = app.emit("song-rated", serde_json::json!({ "id": id, "rating": rating }));
    Ok(())
}

/// The "always sync" artists and albums.
#[tauri::command]
pub fn sync_rules(db: State<'_, SharedDb>) -> Result<Vec<crate::library::sync_rules::SyncRule>, String> {
    let d = db.lock().map_err(|e| e.to_string())?;
    crate::library::sync_rules::list_rules(&d)
}

/// Turn "always sync" on (likes every not-rated song of it now and later) or off for an artist or album.
#[tauri::command]
pub fn set_sync_rule(
    rule: crate::library::sync_rules::SyncRule,
    on: bool,
    app: AppHandle,
    db: State<'_, SharedDb>,
) -> Result<Vec<crate::library::sync_rules::SyncRule>, String> {
    let d = db.lock().map_err(|e| e.to_string())?;
    if on {
        for id in crate::library::sync_rules::add_rule(&d, &rule)? {
            let _ = app.emit("song-rated", serde_json::json!({ "id": id, "rating": 1 }));
        }
    } else {
        crate::library::sync_rules::remove_rule(&d, &rule)?;
    }
    crate::library::sync_rules::list_rules(&d)
}

/// Move disliked songs' files to the Recycle Bin and remove them from the library.
#[tauri::command]
pub fn delete_songs(ids: Vec<i64>, db: State<'_, SharedDb>) -> Result<crate::library::delete::DeleteReport, String> {
    let d = db.lock().map_err(|e| e.to_string())?;
    let report = crate::library::delete::delete_disliked(&d, &ids, &crate::library::delete::RecycleBin).map_err(|e| e.to_string())?;
    log::info!("moved {} disliked songs to the Recycle Bin ({} left alone)", report.deleted.len(), report.failed.len());
    Ok(report)
}

#[tauri::command]
pub fn list_folders(db: State<'_, SharedDb>) -> Vec<WatchedFolder> {
    folder_list(&db)
}

#[tauri::command]
pub fn add_folder(
    path: String,
    app: AppHandle,
    db: State<'_, SharedDb>,
    watch: State<'_, Arc<LibraryWatch>>,
) -> Result<Vec<WatchedFolder>, String> {
    if !Path::new(&path).is_dir() {
        return Err(format!("{path} is not a folder"));
    }
    let added = {
        let d = db.lock().map_err(|e| e.to_string())?;
        folders::add_folder(&d, &path, now_unix()).map_err(|e| e.to_string())?
    };
    log::info!("watching {}", added.path);
    rescan(&app, &db, &[added.id]);
    restart_watch(&app, &db, &watch);
    hash_in_background(&app, &db);
    Ok(folder_list(&db))
}

#[tauri::command]
pub fn remove_folder(
    id: i64,
    app: AppHandle,
    db: State<'_, SharedDb>,
    watch: State<'_, Arc<LibraryWatch>>,
) -> Result<Vec<WatchedFolder>, String> {
    {
        let d = db.lock().map_err(|e| e.to_string())?;
        folders::remove_folder(&d, id).map_err(|e| e.to_string())?;
    }
    restart_watch(&app, &db, &watch);
    Ok(folder_list(&db))
}

/// Force (or clear, with `None`) a folder's legacy tag encoding, then re-read its tags (F8.1).
#[tauri::command]
pub fn set_folder_encoding(id: i64, encoding: Option<String>, app: AppHandle, db: State<'_, SharedDb>) -> Result<Vec<WatchedFolder>, String> {
    {
        let d = db.lock().map_err(|e| e.to_string())?;
        folders::set_tag_encoding(&d, id, encoding.as_deref().filter(|e| !e.is_empty()))?;
    }
    rescan(&app, &db, &[id]);
    let list = folder_list(&db);
    let _ = app.emit("library-changed", &list);
    Ok(list)
}

/// The undo edits of the last save (F9.4), so "Undo" can put the tags back.
#[derive(Default)]
pub struct LastEdit(Mutex<Vec<crate::library::edit::Edit>>);

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SongTags {
    id: i64,
    path: String,
    tags: crate::library::tags::TrackTags,
}

/// Current tags of the given songs, read from their files (for the editor).
#[tauri::command]
pub fn song_tags(ids: Vec<i64>, db: State<'_, SharedDb>) -> Result<Vec<SongTags>, String> {
    let found = {
        let d = db.lock().map_err(|e| e.to_string())?;
        crate::library::songs::paths_for(&d, &ids).map_err(|e| e.to_string())?
    };
    Ok(found
        .into_iter()
        .map(|(id, p)| SongTags { id, tags: crate::library::tags::read_tags(&p).unwrap_or_default(), path: p.to_string_lossy().into_owned() })
        .collect())
}

fn save(edits: &[crate::library::edit::Edit], app: &AppHandle, db: &SharedDb, last: &LastEdit) -> Result<crate::library::edit::EditReport, String> {
    let report = {
        let d = db.lock().map_err(|e| e.to_string())?;
        let report = crate::library::edit::edit_songs(&d, edits)?;
        // Edited tags can put a song under an "always sync" artist or album.
        crate::library::sync_rules::apply_rules(&d)?;
        report
    };
    *last.0.lock().map_err(|e| e.to_string())? = report.undo.clone();
    let _ = app.emit("library-changed", folder_list(db));
    Ok(report)
}

/// Write tag edits into the files (F9.1–F9.4); the previous tags are kept for `undo_edit`.
#[tauri::command]
pub fn save_edits(edits: Vec<crate::library::edit::Edit>, app: AppHandle, db: State<'_, SharedDb>, last: State<'_, LastEdit>) -> Result<crate::library::edit::EditReport, String> {
    save(&edits, &app, &db, &last)
}

/// Put back the tags from before the last save.
#[tauri::command]
pub fn undo_edit(app: AppHandle, db: State<'_, SharedDb>, last: State<'_, LastEdit>) -> Result<crate::library::edit::EditReport, String> {
    let undo = std::mem::take(&mut *last.0.lock().map_err(|e| e.to_string())?);
    let report = save(&undo, &app, &db, &last)?;
    last.0.lock().map_err(|e| e.to_string())?.clear(); // undo is not itself undoable
    Ok(report)
}

/// `miuart` protocol (redesign): `/<song id>-<size>` → that song's cover thumbnail, or 404.
pub fn art_response(db: &SharedDb, uri_path: &str) -> tauri::http::Response<Vec<u8>> {
    let not_found = || tauri::http::Response::builder().status(404).body(Vec::new()).unwrap();
    let Some((id, size)) = crate::library::art::parse_request(uri_path) else { return not_found() };
    let cover = db.lock().ok().and_then(|d| crate::library::artfetch::cover_of(&d, id));
    let cache = crate::util::paths::local_appdata_root().join("MiuMusic").join("art");
    // Embedded first; otherwise the cover found online for the song's album.
    let made = match cover {
        Some(crate::library::artfetch::Cover::Embedded { path, mtime }) => crate::library::art::cached_thumbnail(&cache, id, mtime, Path::new(&path), size),
        Some(crate::library::artfetch::Cover::Fetched(file)) => crate::library::art::cached_file_thumbnail(&cache, &file, size),
        None => return not_found(),
    };
    match made {
        Ok(Some(jpg)) => tauri::http::Response::builder()
            .header("Content-Type", "image/jpeg")
            .header("Cache-Control", "max-age=86400")
            .body(jpg)
            .unwrap(),
        _ => not_found(),
    }
}

// ---- Covers found online for albums without one ----

use crate::library::artfetch::{self, AlbumKey, Http, Outcome};

static FETCHING_ART: AtomicBool = AtomicBool::new(false);
static STOP_ART: AtomicBool = AtomicBool::new(false);
/// iTunes allows about 20 searches a minute; MusicBrainz one a second.
const ART_PAUSE: Duration = Duration::from_secs(3);
const MAX_ART_BYTES: u64 = 10 * 1024 * 1024;

/// HTTP for the cover services. MusicBrainz asks clients to say who they are.
struct WebHttp(ureq::Agent);

impl WebHttp {
    fn new() -> Self {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .user_agent(concat!("MiuMusic/", env!("CARGO_PKG_VERSION"), " ( https://musicplayer.vietnamcos.com )"))
            .build()
            .into();
        Self(agent)
    }
}

impl Http for WebHttp {
    fn get(&self, url: &str) -> Result<Vec<u8>, String> {
        let mut res = self.0.get(url).call().map_err(|e| e.to_string())?;
        let status = res.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(format!("{url} answered {status}"));
        }
        res.body_mut().with_config().limit(MAX_ART_BYTES).read_to_vec().map_err(|e| e.to_string())
    }
}

fn covers_dir() -> std::path::PathBuf {
    crate::util::paths::app_data_dir_in(&crate::util::paths::appdata_root()).join("covers")
}

/// Look one album up now (the album page's "Find artwork"). Returns whether a cover was found.
#[tauri::command]
pub async fn find_album_art(artist: String, album: String, app: AppHandle, db: State<'_, SharedDb>) -> Result<bool, String> {
    let db = db.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let key = AlbumKey { artist, album };
        let outcome = artfetch::fetch_one(&WebHttp::new(), &key, &covers_dir())?;
        let found = matches!(outcome, Outcome::Found { .. });
        {
            let d = db.lock().map_err(|e| e.to_string())?;
            artfetch::record(&d, &key, &outcome, now_unix())?;
        }
        if found {
            let _ = app.emit("library-changed", folder_list(&db));
        }
        Ok(found)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Look up every album without a cover, in the background (progress as `library-progress`, phase "art").
/// Returns how many albums will be looked up.
#[tauri::command]
pub fn fetch_missing_art(app: AppHandle, db: State<'_, SharedDb>) -> Result<usize, String> {
    let todo = {
        let d = db.lock().map_err(|e| e.to_string())?;
        artfetch::missing_albums(&d, now_unix())?
    };
    if todo.is_empty() || FETCHING_ART.swap(true, Ordering::AcqRel) {
        return Ok(0);
    }
    STOP_ART.store(false, Ordering::Release);
    let (db, total) = (db.inner().clone(), todo.len());
    std::thread::spawn(move || {
        let http = WebHttp::new();
        let mut found = 0;
        for (i, key) in todo.iter().enumerate() {
            if STOP_ART.load(Ordering::Acquire) {
                break;
            }
            progress(&app, "art", i, total);
            match artfetch::fetch_one(&http, key, &covers_dir()) {
                Ok(outcome) => {
                    if matches!(outcome, Outcome::Found { .. }) {
                        found += 1;
                    }
                    if let Ok(d) = db.lock() {
                        let _ = artfetch::record(&d, key, &outcome, now_unix());
                    }
                    // Show covers as they arrive, a few at a time.
                    if found > 0 && found % 5 == 0 {
                        let _ = app.emit("library-changed", folder_list(&db));
                    }
                }
                Err(e) => log::warn!("cover lookup for {} – {} failed: {e}", key.artist, key.album),
            }
            std::thread::sleep(ART_PAUSE);
        }
        log::info!("album covers: {found} found for {total} albums");
        progress(&app, "art", total, total);
        let _ = app.emit("library-changed", folder_list(&db));
        FETCHING_ART.store(false, Ordering::Release);
    });
    Ok(total)
}

/// Stop the background cover lookup after the album in progress.
#[tauri::command]
pub fn cancel_art_fetch() {
    STOP_ART.store(true, Ordering::Release);
}
