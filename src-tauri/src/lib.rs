pub mod apple;
pub mod audio;
pub mod device;
pub mod dj;
pub mod library;
pub mod lyrics;
pub mod media_controls;
#[cfg(windows)]
pub mod media_controls_win;
pub mod player;
pub mod podcast;
pub mod audiobook;
pub mod settings;
pub mod shortcuts;
pub mod shortcuts_service;
mod windows;
pub mod sync;
pub mod taskbar;
#[cfg(windows)]
pub mod taskbar_win;
pub mod util;
pub mod update;

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
    // Before any window: Windows reads the app identity when the window is made.
    #[cfg(windows)]
    media_controls_win::declare_app_id();

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
    let (saved_shuffle, saved_repeat) = db.lock().map(|d| (settings::shuffle(&d), settings::repeat(&d))).unwrap_or_default();
    let saved_second = db.lock().ok().and_then(|d| audio::service::second_config(&d));
    let player: audio::service::SharedPlayer = audio::service::PlayerHandle::new_deferred(saved_output, saved_exclusive, saved_second, saved_shuffle, saved_repeat);
    let podcast_downloads: podcast::service::SharedDownloads = std::sync::Arc::new(podcast::service::Downloads::default());
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(shortcuts_service::plugin())
        .manage(update::Pending::default())
        .manage(registry.clone())
        .manage(db.clone())
        .manage(library_watch.clone())
        .manage(player.clone())
        .manage(visualizer_on.clone())
        .manage(library::service::LastEdit::default())
        .manage(podcast_downloads.clone())
        .manage(dj::service::SharedDj::default())
        .manage(std::sync::Arc::new(podcast::service::DiscoverCache::default()))
        .manage(std::sync::Arc::new(audiobook::service::RecentCache::default()))
        .register_uri_scheme_protocol("miuart", {
            let db = db.clone();
            move |_ctx, request| library::service::art_response(&db, request.uri().path())
        })
        .setup(move |app| {
            device::service::start(app.handle().clone(), registry);
            podcast::service::start(app.handle().clone(), db.clone(), podcast_downloads.clone());
            player::service::start(app.handle().clone(), db.clone());
            windows::setup(app.handle(), db.clone());
            library::service::start(app.handle().clone(), db.clone(), library_watch);
            update::start(app.handle().clone(), db.clone());
            dj::service::start(app.handle().clone(), db.clone());
            shortcuts_service::apply(app.handle(), &db);
            audio::eq_service::start(&db, &player);
            #[cfg(windows)]
            media_controls_win::start(app.handle(), player.clone());
            audio::service::start(app.handle().clone(), player.clone(), db.clone());
            audio::visualizer::start(app.handle().clone(), player, visualizer_on);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            update::check_update,
            update::install_update,
            shortcuts_service::shortcuts_state,
            shortcuts_service::set_shortcut_keys,
            shortcuts_service::reset_shortcut_keys,
            shortcuts_service::set_shortcuts_enabled,
            shortcuts_service::pause_shortcuts,
            check_prereqs,
            list_devices,
            library::service::list_folders,
            library::service::list_songs,
            library::service::rate_song,
            library::service::find_album_art,
            library::service::fetch_missing_art,
            library::service::cancel_art_fetch,
            library::service::sync_rules,
            library::service::set_sync_rule,
            library::service::delete_songs,
            library::service::add_folder,
            library::service::remove_folder,
            library::service::set_folder_encoding,
            library::service::song_tags,
            library::service::save_edits,
            library::service::undo_edit,
            sync::service::sync_preview,
            sync::service::sync_now,
            player::service::list_players,
            player::service::add_player,
            player::service::save_player,
            player::service::forget_player,
            player::service::player_sync_preview,
            player::service::player_sync_now,
            podcast::service::podcast_subscribe,
            podcast::service::podcast_refresh,
            podcast::service::podcast_list,
            podcast::service::podcast_episodes,
            podcast::service::podcast_unsubscribe,
            podcast::service::podcast_import_opml,
            podcast::service::podcast_export_opml,
            podcast::service::podcast_download,
            podcast::service::podcast_delete_download,
            podcast::service::podcast_pending_downloads,
            podcast::service::podcast_set_played,
            podcast::service::podcast_set_options,
            podcast::service::podcast_chart,
            podcast::service::podcast_now,
            podcast::service::podcast_search,
            podcast::service::podcast_subscribe_found,
            audio::service::play_songs,
            audio::service::player_pause,
            audio::service::player_resume,
            audio::service::player_seek,
            audio::service::player_volume,
            audio::service::player_skip,
            audio::service::output_state,
            audio::service::set_output,
            audio::service::set_exclusive,
            audio::service::set_second_output,
            audio::service::set_dual_mode,
            audio::service::set_dual_delays,
            audio::service::set_dual_balance,
            audio::service::output_align,
            audio::service::calibrate_outputs,
            lyrics::service::song_lyrics,
            dj::service::dj_analysis_progress,
            dj::service::dj_song_analysis,
            dj::service::dj_enter,
            dj::service::dj_leave,
            dj::service::dj_action,
            dj::service::dj_load,
            dj::service::dj_state,
            dj::service::dj_set_auto,
            dj::service::dj_overview,
            dj::service::dj_library,
            dj::service::dj_controllers,
            dj::service::dj_midi_learn,
            dj::service::dj_midi_forget,
            dj::service::dj_outputs,
            dj::service::dj_set_phones,
            audio::service::player_shuffle,
            audio::service::play_episode,
            audio::service::play_chapters,
            audiobook::service::audiobook_search,
            audiobook::service::audiobook_recent,
            audiobook::service::audiobook_add,
            audiobook::service::audiobook_list,
            audiobook::service::audiobook_chapters,
            audiobook::service::audiobook_download,
            audiobook::service::audiobook_remove_download,
            audiobook::service::audiobook_remove,
            audio::service::player_repeat,
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
            windows::ui_language,
            windows::set_ui_language,
            windows::app_settings,
            windows::set_app_setting,
            windows::open_store_page,
            windows::open_mini,
            windows::taskbar_init,
            windows::taskbar_playing,
            windows::export_diagnostics,
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

    #[test]
    fn updates_come_signed_from_the_latest_github_release_and_install_quietly() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let updater = &conf["plugins"]["updater"];
        assert_eq!(
            updater["endpoints"],
            serde_json::json!(["https://github.com/khangkhangg/Miu-Music-Player/releases/latest/download/latest.json"])
        );
        assert!(updater["pubkey"].as_str().is_some_and(|k| k.len() > 40), "the public key is set");
        assert_eq!(updater["windows"]["installMode"], "passive");
        assert_eq!(conf["bundle"]["createUpdaterArtifacts"], true);
    }
}
