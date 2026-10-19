pub mod apple;
pub mod audio;
pub mod device;
pub mod library;
pub mod settings;
mod windows;
pub mod sync;
pub mod util;

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct AppInfo {
    pub name: &'static str,
    pub version: &'static str,
}

#[tauri::command]
fn app_info() -> AppInfo {
    AppInfo {
        name: "Miu Music",
        version: env!("CARGO_PKG_VERSION"),
    }
}

#[tauri::command]
fn check_prereqs() -> apple::locate::PrereqReport {
    let report = apple::locate::PrereqReport::from(apple::locate::locate_on_system());
    if report.ready {
        log::info!("Apple DLLs found: {:?}", report.paths);
    } else {
        log::warn!("Apple DLLs missing: {:?}", report.missing);
    }
    report
}

#[cfg(windows)]
#[tauri::command]
fn list_devices(registry: tauri::State<'_, device::service::SharedRegistry>) -> Vec<device::registry::Device> {
    device::service::snapshot(&registry)
}

pub fn run() {
    let logs = util::paths::logs_dir_in(&util::paths::appdata_root());
    if let Err(e) = util::log::init(&logs, log::LevelFilter::Info) {
        eprintln!("logging disabled: {e}");
    }
    log::info!("Miu Music {} started", env!("CARGO_PKG_VERSION"));

    let db_path = util::paths::db_path_in(&util::paths::appdata_root());
    let db = match library::db::Db::open(&db_path) {
        Ok(db) => {
            log::info!("database ready: {} (schema v{})", db_path.display(), db.schema_version().unwrap_or(-1));
            db
        }
        Err(e) => {
            // Keep the app usable this session; nothing will be saved.
            log::error!("database unavailable at {}: {e}; using a temporary in-memory one", db_path.display());
            library::db::Db::open_in_memory().expect("in-memory database")
        }
    };
    let visualizer_on: audio::visualizer::VisualizerOn = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(settings::visualizer_on(&db)));
    let db: library::service::SharedDb = std::sync::Arc::new(std::sync::Mutex::new(db));
    let library_watch = std::sync::Arc::new(library::service::LibraryWatch::default());

    let registry = device::service::SharedRegistry::default();
    let saved_output = db.lock().ok().and_then(|d| settings::output_device(&d));
    let saved_exclusive = db.lock().is_ok_and(|d| settings::exclusive_output(&d));
    let player: audio::service::SharedPlayer = std::sync::Arc::new(audio::service::PlayerHandle::new(saved_output.as_deref(), saved_exclusive));
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(registry.clone())
        .manage(db.clone())
        .manage(library_watch.clone())
        .manage(player.clone())
        .manage(visualizer_on.clone())
        .setup(move |app| {
            device::service::start(app.handle().clone(), registry);
            windows::setup(app.handle(), db.clone());
            library::service::start(app.handle().clone(), db.clone(), library_watch);
            audio::eq_service::start(&db, &player);
            audio::service::start(app.handle().clone(), player.clone());
            audio::visualizer::start(app.handle().clone(), player, visualizer_on);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            check_prereqs,
            list_devices,
            library::service::list_folders,
            library::service::list_songs,
            library::service::rate_song,
            library::service::delete_songs,
            library::service::add_folder,
            library::service::remove_folder,
            audio::service::play_songs,
            audio::service::player_pause,
            audio::service::player_resume,
            audio::service::player_seek,
            audio::service::player_volume,
            audio::service::player_skip,
            audio::service::output_state,
            audio::service::set_output,
            audio::service::set_exclusive,
            windows::open_main,
            audio::eq_service::eq_state,
            audio::eq_service::set_eq,
            audio::eq_service::apply_eq_preset,
            audio::eq_service::save_eq_preset,
            audio::eq_service::delete_eq_preset,
            audio::visualizer::visualizer_settings,
            audio::visualizer::set_visualizer_on,
            audio::visualizer::set_visualizer_style,
            windows::mini_on_top,
            windows::set_mini_on_top,
            windows::set_mini_on_minimize,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Miu Music");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_info_reports_product_name_and_crate_version() {
        let info = app_info();
        assert_eq!(info.name, "Miu Music");
        assert_eq!(info.version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn app_info_matches_tauri_config() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let info = app_info();
        assert_eq!(conf["productName"], info.name);
        assert_eq!(conf["version"], info.version);
    }
}
