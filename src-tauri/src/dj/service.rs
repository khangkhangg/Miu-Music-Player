//! DJ mode for the app: the background analyser (D.1), which works through the library one song
//! at a time at background priority so DJ mode knows every song's tempo, key and energy (progress
//! goes out as `dj-analysis-progress` { done, total }), and the commands that run the DJ engine (D.2).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, State};

use super::actions::{Action, Deck as Side};
use super::analysis::{self, Analysis};
use super::engine::{DjEngine, EngineStatus};
use super::store;
use crate::audio::service::SharedPlayer;
use crate::library::service::SharedDb;
use crate::settings;

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


/// The running DJ engine, while DJ mode is open.
pub type SharedDj = Arc<Mutex<Option<DjEngine>>>;

fn engine_do<T>(dj: &SharedDj, f: impl FnOnce(&DjEngine) -> T) -> Result<T, String> {
    let g = dj.lock().map_err(|e| e.to_string())?;
    g.as_ref().map(f).ok_or_else(|| "DJ mode is not open".to_string())
}

/// Opens DJ mode: the normal player pauses and the DJ engine takes the speakers (the player's
/// output) and the chosen headphones.
#[tauri::command]
pub fn dj_enter(db: State<'_, SharedDb>, player: State<'_, SharedPlayer>, dj: State<'_, SharedDj>) -> Result<EngineStatus, String> {
    let mut g = dj.lock().map_err(|e| e.to_string())?;
    if let Some(e) = g.as_ref() {
        return Ok(e.status());
    }
    let _ = player.run(crate::media_controls::Command::Pause);
    let (master, phones, exclusive) = {
        let d = db.lock().map_err(|e| e.to_string())?;
        (settings::output_device(&d), settings::dj_phones(&d), settings::exclusive_output(&d))
    };
    let engine = DjEngine::start(master, phones).map_err(|e| {
        if exclusive { format!("{e} (exclusive mode holds the speakers: turn it off for DJ mode)") } else { e }
    })?;
    let s = engine.status();
    *g = Some(engine);
    Ok(s)
}

/// Closes DJ mode and gives the speakers back to the normal player.
#[tauri::command]
pub fn dj_leave(dj: State<'_, SharedDj>) -> Result<(), String> {
    let old = dj.lock().map_err(|e| e.to_string())?.take();
    drop(old); // waits for the engine to close its devices
    Ok(())
}

#[tauri::command]
pub fn dj_action(action: Action, dj: State<'_, SharedDj>) -> Result<(), String> {
    engine_do(&dj, |e| e.apply(action))
}

/// Puts a library song on a deck; it plays from its first beat when started.
#[tauri::command]
pub fn dj_load(deck: Side, song_id: i64, db: State<'_, SharedDb>, dj: State<'_, SharedDj>) -> Result<(), String> {
    let (path, analysis) = {
        let d = db.lock().map_err(|e| e.to_string())?;
        let path = crate::library::songs::paths_for(&d, &[song_id]).map_err(|e| e.to_string())?.pop().map(|(_, p)| p);
        (path.ok_or(format!("song {song_id} is not in the library"))?, store::get(&d, song_id).map_err(|e| e.to_string())?)
    };
    engine_do(&dj, |e| e.load(deck, path, song_id, analysis))
}

/// Decks, meters and devices for the DJ screen (`None` when DJ mode is closed).
#[tauri::command]
pub fn dj_state(dj: State<'_, SharedDj>) -> Result<Option<EngineStatus>, String> {
    Ok(dj.lock().map_err(|e| e.to_string())?.as_ref().map(DjEngine::status))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DjOutputs {
    pub devices: Vec<crate::audio::player::OutputDevice>,
    pub phones: Option<String>,
}

#[tauri::command]
pub fn dj_outputs(db: State<'_, SharedDb>) -> Result<DjOutputs, String> {
    let phones = settings::dj_phones(&*db.lock().map_err(|e| e.to_string())?);
    Ok(DjOutputs { devices: crate::audio::player::output_devices(), phones })
}

/// Chooses the headphones (`None`: no headphone cue); switches at once when DJ mode is open.
#[tauri::command]
pub fn dj_set_phones(name: Option<String>, db: State<'_, SharedDb>, dj: State<'_, SharedDj>) -> Result<(), String> {
    settings::set_dj_phones(&*db.lock().map_err(|e| e.to_string())?, name.as_deref()).map_err(|e| e.to_string())?;
    if let Some(e) = dj.lock().map_err(|e| e.to_string())?.as_ref() {
        e.set_phones(name);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::db::Db;

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
