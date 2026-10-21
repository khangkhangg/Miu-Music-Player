//! App glue for music players: the drive watcher (`players` event), setting players up, preview
//! and sync (`player-sync-progress` events).

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::library::service::SharedDb;
use crate::player::drives::{device_id, players, volume_of, Player};
use crate::player::profile::{forget_player as forget, set_player, Profile};
use crate::player::sync::{plan_for, run, DriveFs, Report, Stop, MARGIN_BYTES};
use crate::sync::plan::{check_space, history, Plan, RunSummary};

fn root_dirs(root: &Path) -> Vec<String> {
    std::fs::read_dir(root)
        .map(|rd| rd.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default()
}

fn current(db: &SharedDb) -> Result<Vec<Player>, String> {
    let known = crate::player::profile::players(&*db.lock().map_err(|e| e.to_string())?);
    Ok(players(&crate::player::volumes::list(), &known, &root_dirs))
}

/// Watch for drives coming and going (letters change), and send the players then.
pub fn start(app: AppHandle, db: SharedDb) {
    let spawned = std::thread::Builder::new().name("player-watch".into()).spawn(move || {
        let mut last = None;
        loop {
            let now = crate::player::volumes::letters();
            if last != Some(now) {
                last = Some(now);
                match current(&db) {
                    Ok(list) => {
                        log::info!("music players: {}", list.iter().map(|p| format!("{} {}", p.root.display(), p.id)).collect::<Vec<_>>().join(", "));
                        let _ = app.emit("players", list);
                    }
                    Err(e) => log::warn!("listing players failed: {e}"),
                }
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
    if let Err(e) = spawned {
        log::error!("could not start the player watcher: {e}");
    }
}

#[tauri::command]
pub fn list_players(db: State<'_, SharedDb>) -> Result<Vec<Player>, String> {
    current(&db)
}

/// Use the drive holding `folder` as a player that Miu fills from that folder.
#[tauri::command]
pub fn add_player(folder: String, db: State<'_, SharedDb>) -> Result<Vec<Player>, String> {
    let volumes = crate::player::volumes::list();
    let (volume, rel) = volume_of(&volumes, Path::new(&folder)).ok_or("That folder is not on a USB drive, memory card or second disk.")?;
    let id = device_id(volume.serial);
    {
        let d = db.lock().map_err(|e| e.to_string())?;
        let existing = crate::player::profile::players(&d).remove(&id).unwrap_or_default();
        set_player(&d, &id, Profile { folder: rel, ..existing })?;
    }
    log::info!("player {id} set up at {folder}");
    current(&db)
}

#[tauri::command]
pub fn save_player(id: String, profile: Profile, db: State<'_, SharedDb>) -> Result<Vec<Player>, String> {
    set_player(&*db.lock().map_err(|e| e.to_string())?, &id, profile)?;
    current(&db)
}

#[tauri::command]
pub fn forget_player(id: String, db: State<'_, SharedDb>) -> Result<Vec<Player>, String> {
    forget(&*db.lock().map_err(|e| e.to_string())?, &id)?;
    current(&db)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerPreview {
    plan: Plan,
    shortage: Option<Stop>,
    last_run: Option<RunSummary>,
}

fn find(db: &SharedDb, id: &str) -> Result<Player, String> {
    current(db)?.into_iter().find(|p| p.id == id).ok_or_else(|| "The player is not connected.".into())
}

#[tauri::command]
pub fn player_sync_preview(id: String, db: State<'_, SharedDb>) -> Result<PlayerPreview, String> {
    let player = find(&db, &id)?;
    let d = db.lock().map_err(|e| e.to_string())?;
    let plan = plan_for(&d, &id, &player.profile).map_err(|e| e.to_string())?;
    let shortage = check_space(&plan, player.free_bytes, MARGIN_BYTES).err().map(|s| Stop::NoSpace { need: s.need, available: s.available });
    let last_run = history(&d, &id).map_err(|e| e.to_string())?.into_iter().next();
    Ok(PlayerPreview { plan, shortage, last_run })
}

static SYNCING: AtomicBool = AtomicBool::new(false);

/// Copy the liked songs to the player. A suggested player is set up by its first sync.
#[tauri::command]
pub async fn player_sync_now(id: String, app: AppHandle, db: State<'_, SharedDb>) -> Result<Report, Stop> {
    let fail = |e: String| Stop::Failed { reason: e };
    if SYNCING.swap(true, Ordering::AcqRel) {
        return Err(fail("A sync to this player is already running.".into()));
    }
    let shared = db.inner().clone();
    let result = tauri::async_runtime::spawn_blocking(move || sync_blocking(&id, &shared, &app)).await.unwrap_or_else(|e| Err(fail(e.to_string())));
    SYNCING.store(false, Ordering::Release);
    result
}

fn sync_blocking(id: &str, shared: &SharedDb, app: &AppHandle) -> Result<Report, Stop> {
    let fail = |e: String| Stop::Failed { reason: e };
    let player = find(shared, id).map_err(fail)?;
    if !player.configured {
        set_player(&*shared.lock().map_err(|e| fail(e.to_string()))?, id, player.profile.clone()).map_err(fail)?;
    }
    // Its own connection: the library stays usable while files are copied.
    let appdata = crate::util::paths::appdata_root();
    let db = crate::library::db::Db::open(&crate::util::paths::db_path_in(&appdata)).map_err(|e| fail(e.to_string()))?;
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default();
    let ffmpeg = crate::library::transcode::ffmpeg_path(&exe_dir).map(|exe| crate::library::transcode::Ffmpeg { exe });
    let cache = crate::library::transcode::Cache { dir: crate::util::paths::local_appdata_root().join("MiuMusic").join("cache") };
    let convert = |s: &crate::sync::itunes::SongSource| match &ffmpeg {
        Some(ff) => cache.converted(ff, &s.path, &s.audio_key, &s.tags_key, s.action, s.cover.as_deref()),
        None => Err("needs converting, and the ffmpeg sidecar is missing".into()),
    };
    let root = player.root.clone();
    let fs = DriveFs { root: root.clone(), free: Box::new(move || crate::player::volumes::free_bytes(&root)) };
    log::info!("player sync to {} ({}) into {:?}", player.root.display(), id, player.profile.folder);
    let report = run(&db, id, &player.profile, &fs, Some(&convert), &mut |stage| {
        let _ = app.emit("player-sync-progress", stage);
    }, crate::util::time::now_unix());
    let _ = cache.trim(4 * 1024 * 1024 * 1024);
    match &report {
        Ok(r) => log::info!("player sync done: +{} ~{} −{} failed {} left alone {}", r.added, r.replaced + r.updated, r.removed, r.failed.len(), r.left_alone),
        Err(e) => log::warn!("player sync stopped: {e:?}"),
    }
    if let Ok(list) = current(shared) {
        let _ = app.emit("players", list);
    }
    report
}
