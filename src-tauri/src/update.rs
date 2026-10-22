//! In-app updates through tauri-plugin-updater. The app looks for `latest.json` on the newest
//! GitHub release shortly after start and then at most once a day (setting "autoUpdate"), and
//! tells the page with `update-available`. Nothing is downloaded or installed until the user
//! clicks "Update and restart" (`install_update`): the plugin checks the signature against the
//! public key in tauri.conf.json, runs the NSIS installer in passive mode and Miu restarts.

use crate::library::service::SharedDb;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

/// What the page shows about a new version.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub current: String,
    pub notes: String,
}

/// The update found by the last check, kept for `install_update`.
#[derive(Default)]
pub struct Pending(Mutex<Option<Update>>);

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

async fn check(app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
    if let Ok(db) = app.state::<SharedDb>().lock() {
        let _ = crate::settings::set_last_update_check(&db, now());
    }
    let update = app.updater().map_err(|e| e.to_string())?.check().await.map_err(|e| e.to_string())?;
    let info = update.as_ref().map(|u| UpdateInfo {
        version: u.version.clone(),
        current: u.current_version.clone(),
        notes: u.body.clone().unwrap_or_default(),
    });
    if let Ok(mut pending) = app.state::<Pending>().0.lock() {
        *pending = update;
    }
    match &info {
        Some(i) => {
            log::info!("update available: {} (running {})", i.version, i.current);
            let _ = app.emit("update-available", i);
        }
        None => log::info!("no update available"),
    }
    Ok(info)
}

/// "Check now" in Settings: the new version, or `None` when Miu is up to date.
#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    check(&app).await
}

/// Download the update found by the last check with progress (`update-progress` {done, total}),
/// install it and restart. Only ever called from the user's click.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    let update = app.state::<Pending>().0.lock().ok().and_then(|mut p| p.take());
    let Some(update) = update else { return Err("no update to install: check again".into()) };
    log::info!("installing update {}", update.version);
    let mut done = 0u64;
    update
        .download_and_install(
            |chunk, total| {
                done += chunk as u64;
                let _ = app.emit("update-progress", serde_json::json!({ "done": done, "total": total }));
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;
    app.restart()
}

/// Background checks: one shortly after start (so startup is never slowed), then every hour
/// look whether a day has passed since the last one.
pub fn start(app: AppHandle, db: SharedDb) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(20));
        loop {
            let due = db.lock().is_ok_and(|d| crate::settings::update_check_due(&d, now()));
            if due {
                if let Err(e) = tauri::async_runtime::block_on(check(&app)) {
                    log::warn!("update check failed: {e}");
                }
            }
            std::thread::sleep(Duration::from_secs(3600));
        }
    });
}
