//! App glue for watched folders (F3.1): initial rescan, live watching, Tauri commands, and a
//! `library-changed` event carrying the folder list. The logic lives in `folders`, `scan`, `watch`.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{AppHandle, Emitter, State};

use crate::library::db::Db;
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

fn rescan(db: &SharedDb, ids: &[i64]) {
    for f in folder_list(db).into_iter().filter(|f| ids.contains(&f.id)) {
        let Ok(d) = db.lock() else { return };
        match rescan_folder(&d, f.id, &f.path, now_unix()) {
            Ok(diff) => log::info!("rescanned {}: +{} ~{} -{}", f.path, diff.added, diff.updated, diff.removed),
            Err(e) => log::warn!("rescan of {} failed: {e}", f.path),
        }
    }
}

/// (Re)start watching every folder; changes trigger a rescan and a `library-changed` event.
fn restart_watch(app: &AppHandle, db: &SharedDb, watch: &LibraryWatch) {
    let folders: Vec<(i64, String)> = folder_list(db).into_iter().map(|f| (f.id, f.path)).collect();
    let (app2, db2) = (app.clone(), db.clone());
    let watcher = FolderWatcher::start(folders, Duration::from_millis(800), move |ids| {
        rescan(&db2, &ids);
        let _ = app2.emit("library-changed", folder_list(&db2));
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
        rescan(&db, &ids);
        restart_watch(&app, &db, &watch);
        let _ = app.emit("library-changed", folder_list(&db));
    });
}

#[tauri::command]
pub fn list_songs(db: State<'_, SharedDb>) -> Result<Vec<crate::library::songs::SongRow>, String> {
    let d = db.lock().map_err(|e| e.to_string())?;
    crate::library::songs::list_songs(&d).map_err(|e| e.to_string())
}

/// `rating`: 1 liked, 0 not rated, -1 disliked.
#[tauri::command]
pub fn rate_song(id: i64, rating: i64, db: State<'_, SharedDb>) -> Result<(), String> {
    let rating = crate::library::songs::Rating::from_value(rating).ok_or("rating must be -1, 0 or 1")?;
    let d = db.lock().map_err(|e| e.to_string())?;
    crate::library::songs::set_rating(&d, id, rating)
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
    rescan(&db, &[added.id]);
    restart_watch(&app, &db, &watch);
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
