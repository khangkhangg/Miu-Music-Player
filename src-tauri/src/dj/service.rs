//! The background analyser (D.1): works through the library one song at a time at background
//! priority, so DJ mode knows every song's tempo, key and energy. Progress goes out as
//! `dj-analysis-progress` { done, total }.

use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{AppHandle, Emitter, State};

use super::analysis::{self, Analysis};
use super::store;
use crate::library::service::SharedDb;

static RUNNING: AtomicBool = AtomicBool::new(false);

/// Analyses the next waiting song with `analyse`. Returns false when nothing is left.
/// A file that can't be decoded is recorded as failed so it isn't tried again until it changes.
pub fn step(db: &SharedDb, analyse: impl Fn(&str) -> Result<Analysis, String>, now_unix: i64) -> bool {
    let next = match db.lock() {
        Ok(d) => store::pending(&d, 1).unwrap_or_default().pop(),
        Err(_) => return false,
    };
    let Some(song) = next else { return false };
    let result = analyse(&song.path); // without the database lock: this takes a while
    let Ok(d) = db.lock() else { return false };
    match result {
        Ok(a) => store::save(&d, song.song_id, song.size, song.mtime, &a, now_unix).is_ok(),
        Err(e) => {
            log::warn!("DJ analysis of {} failed: {e}", song.path);
            store::save_failed(&d, song.song_id, song.size, song.mtime, now_unix).is_ok()
        }
    }
}

fn analyse_file(path: &str) -> Result<Analysis, String> {
    let (mono, rate) = analysis::decode_mono(std::path::Path::new(path))?;
    Ok(analysis::analyze(&mono, rate))
}

#[cfg(windows)]
fn background_priority() {
    use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN};
    // SAFETY: the pseudo handle of the calling thread is always valid.
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) };
}

#[cfg(not(windows))]
fn background_priority() {}

fn emit_progress(app: &AppHandle, db: &SharedDb) {
    if let Some((done, total)) = db.lock().ok().and_then(|d| store::progress(&d).ok()) {
        let _ = app.emit("dj-analysis-progress", serde_json::json!({ "done": done, "total": total }));
    }
}

/// Starts a pass over the library unless one is running (that one picks up new songs too).
pub fn analyse_library(app: &AppHandle, db: &SharedDb) {
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    let (app, db) = (app.clone(), db.clone());
    std::thread::spawn(move || {
        background_priority();
        let mut n = 0;
        loop {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
            if !step(&db, analyse_file, now) {
                break;
            }
            n += 1;
            if n % 5 == 0 {
                emit_progress(&app, &db);
            }
        }
        RUNNING.store(false, Ordering::Release);
        emit_progress(&app, &db);
        if n > 0 {
            log::info!("DJ analysis: {n} songs analysed");
        }
    });
}

/// Starts the first pass a minute after launch, when the library scan has settled, so DJ mode is
/// ready by the time someone opens it.
pub fn start(app: AppHandle, db: SharedDb) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(60));
        analyse_library(&app, &db);
    });
}

#[derive(serde::Serialize)]
pub struct Progress {
    pub done: u32,
    pub total: u32,
}

/// How far the analysis is; opening DJ mode calls this, which also starts a pass.
#[tauri::command]
pub fn dj_analysis_progress(app: AppHandle, db: State<'_, SharedDb>) -> Result<Progress, String> {
    let (done, total) = store::progress(&*db.lock().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if done < total {
        analyse_library(&app, &db);
    }
    Ok(Progress { done, total })
}

#[tauri::command]
pub fn dj_song_analysis(song_id: i64, db: State<'_, SharedDb>) -> Result<Option<Analysis>, String> {
    store::get(&*db.lock().map_err(|e| e.to_string())?, song_id).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::db::Db;
    use std::sync::{Arc, Mutex};

    fn library(paths: &[&str]) -> SharedDb {
        let db = Db::open_in_memory().unwrap();
        db.conn().execute("INSERT INTO watched_folders (id, path, added_at) VALUES (1, 'C:/M', 1)", []).unwrap();
        for p in paths {
            db.conn()
                .execute("INSERT INTO songs (folder_id, path, size, mtime, added_at) VALUES (1, ?1, 1, 1, 1)", [p])
                .unwrap();
        }
        Arc::new(Mutex::new(db))
    }

    fn fake(path: &str) -> Result<Analysis, String> {
        if path.ends_with(".bad") {
            return Err("cannot decode".into());
        }
        Ok(analysis::analyze(&vec![0.0; 11025], 11025))
    }

    #[test]
    fn works_through_every_song_once_and_records_failures() {
        let db = library(&["C:/M/a.mp3", "C:/M/b.bad", "C:/M/c.flac"]);
        let mut steps = 0;
        while step(&db, fake, 7) {
            steps += 1;
            assert!(steps <= 3, "each song is analysed once");
        }
        assert_eq!(steps, 3);
        let d = db.lock().unwrap();
        assert_eq!(store::progress(&d).unwrap(), (3, 3));
        assert!(store::get(&d, 1).unwrap().is_some());
        assert_eq!(store::get(&d, 2).unwrap(), None, "a failed file has no analysis");
    }
}
