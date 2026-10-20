//! App glue for the sync plan (F5.3): a preview of what a sync would change on a phone.

use serde::Serialize;
use tauri::State;

use crate::library::service::SharedDb;
use crate::sync::plan::{check_space, plan, Plan};

/// Room left on the phone after a sync, so iOS keeps working comfortably.
const MARGIN_BYTES: u64 = 200_000_000;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shortage {
    need: u64,
    available: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPreview {
    plan: Plan,
    /// Set when the plan does not fit in `free_bytes`.
    shortage: Option<Shortage>,
}

#[tauri::command]
pub fn sync_preview(udid: String, free_bytes: Option<u64>, db: State<'_, SharedDb>) -> Result<SyncPreview, String> {
    let d = db.lock().map_err(|e| e.to_string())?;
    let p = plan(&d, &udid, &crate::settings::conversion_policy(&d)).map_err(|e| e.to_string())?;
    let shortage = free_bytes.and_then(|free| check_space(&p, free, MARGIN_BYTES).err()).map(|s| Shortage { need: s.need, available: s.available });
    Ok(SyncPreview { plan: p, shortage })
}

// ---- Sync through iTunes (S.3) ----

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Emitter};

use crate::apple::afc::Afc;
use crate::sync::guard::{copy_off, phone_tracks, CopyReport, PhoneTrack};
use crate::sync::run::{run_sync, Options, PhoneSide, Report, SyncError};

static SYNCING: AtomicBool = AtomicBool::new(false);

/// The phone's files over AFC; backups and copies go under `root\<udid8>\`.
struct RealPhone {
    afc: Afc,
    root: PathBuf,
    udid: String,
}

impl RealPhone {
    fn tracks_in(&self, root: &std::path::Path) -> Result<Vec<PhoneTrack>, String> {
        let b = crate::sync::backup::backup(&self.afc, root, &self.udid, crate::util::time::now_unix()).map_err(|e| e.to_string())?;
        phone_tracks(&b.dir)
    }
}

impl PhoneSide for RealPhone {
    fn backup(&self) -> Result<Vec<PhoneTrack>, String> {
        self.tracks_in(&self.root)
    }
    fn peek(&self) -> Result<Vec<PhoneTrack>, String> {
        let scratch = tempfile::tempdir().map_err(|e| e.to_string())?;
        self.tracks_in(scratch.path())
    }
    fn copy_off(&self, tracks: &[&PhoneTrack]) -> Result<CopyReport, String> {
        let media = self.root.join(self.udid.chars().take(8).collect::<String>()).join("media");
        copy_off(&self.afc, tracks, &media)
    }
}

/// Why a sync stopped, for the UI.
#[derive(Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum SyncStop {
    Blocked { reasons: Vec<String> },
    Failed { reason: String },
}

fn failed(reason: impl Into<String>) -> SyncStop {
    SyncStop::Failed { reason: reason.into() }
}

/// Sync the liked songs to the phone through iTunes, with every safeguard (`sync::run`).
/// Progress arrives as `sync-progress` events.
#[tauri::command]
pub async fn sync_now(udid: String, app: AppHandle) -> Result<Report, SyncStop> {
    if SYNCING.swap(true, Ordering::AcqRel) {
        return Err(failed("A sync is already running."));
    }
    let result = tauri::async_runtime::spawn_blocking(move || sync_blocking(&udid, &app)).await.unwrap_or_else(|e| Err(failed(e.to_string())));
    SYNCING.store(false, Ordering::Release);
    result
}

fn sync_blocking(udid: &str, app: &AppHandle) -> Result<Report, SyncStop> {
    let appdata = crate::util::paths::appdata_root();
    // Its own connection: the library stays usable while the sync runs for minutes.
    let db = crate::library::db::Db::open(&crate::util::paths::db_path_in(&appdata)).map_err(|e| failed(e.to_string()))?;
    let policy = crate::settings::conversion_policy(&db);
    let (afc, found) = crate::apple::afc::open_first_trusted(Duration::from_secs(10)).map_err(failed)?;
    if found != udid {
        return Err(failed("The iPhone to sync is not the one plugged in by USB."));
    }
    let phone = RealPhone { afc, root: crate::util::paths::backups_dir_in(&appdata), udid: udid.to_string() };
    let itunes = crate::apple::itunes_com::ITunesCom::connect().map_err(failed)?;
    let options = Options { poll_every: Duration::from_secs(5), max_polls: 60 };
    // Songs the Music app cannot play go as converted copies, cached between syncs (F3.5).
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(std::path::Path::to_path_buf)).unwrap_or_default();
    let ffmpeg = crate::library::transcode::ffmpeg_path(&exe_dir).map(|exe| crate::library::transcode::Ffmpeg { exe });
    if ffmpeg.is_none() {
        log::warn!("ffmpeg sidecar not found next to {}: songs that need converting will be skipped", exe_dir.display());
    }
    let cache = crate::library::transcode::Cache { dir: crate::util::paths::local_appdata_root().join("MiuMusic").join("cache") };
    let convert = |s: &crate::sync::itunes::SongSource| match &ffmpeg {
        Some(ff) => cache.converted(ff, &s.path, &s.audio_key, &s.tags_key, s.action),
        None => Err("needs converting for the iPhone, and the ffmpeg sidecar is missing".into()),
    };
    let report = run_sync(
        &db,
        udid,
        &policy,
        &phone,
        &itunes,
        Some(&convert),
        &options,
        &std::thread::sleep,
        &mut |stage| {
            log::info!("sync: {stage:?}");
            let _ = app.emit("sync-progress", &stage);
        },
        crate::util::time::now_unix(),
    );
    match report {
        Ok(r) => {
            log::info!("sync done: +{} −{} adopted {} failed {} lost {}", r.added, r.removed, r.adopted, r.failed.len(), r.unexpected_removed.len());
            Ok(r)
        }
        Err(SyncError::Blocked(reasons)) => {
            log::warn!("sync blocked: {reasons:?}");
            Err(SyncStop::Blocked { reasons })
        }
        Err(SyncError::Failed(reason)) => {
            log::error!("sync failed: {reason}");
            Err(failed(reason))
        }
    }
}
