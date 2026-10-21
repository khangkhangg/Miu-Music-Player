//! Main window ⇄ mini player (F7.4): minimizing the main window shows the mini player (setting
//! `mini_on_minimize`); restoring it hides the mini player again. Closing the mini player hides it.

use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};

use crate::library::service::SharedDb;

pub fn setup(app: &AppHandle, db: SharedDb) {
    let Some(main) = app.get_webview_window("main") else {
        log::warn!("main window missing");
        return;
    };
    fit_main_window(&main);
    // The mini player is created the first time it is needed, not at start: one WebView2
    // renderer less while the app opens, and none at all for people who never use it.
    let (main2, app2) = (main.clone(), app.clone());
    main.on_window_event(move |event| {
        if let WindowEvent::Resized(_) = event {
            let minimized = main2.is_minimized().unwrap_or(false);
            let wanted = db.lock().map(|d| crate::settings::mini_on_minimize(&d)).unwrap_or(true);
            let change = on_main_resized(minimized, wanted);
            set_memory_low(&main2, change.memory_low);
            match change.mini {
                Some(true) => show_mini_later(&app2),
                Some(false) => {
                    if let Some(mini) = app2.get_webview_window("mini") {
                        let _ = mini.hide();
                        set_memory_low(&mini, true);
                    }
                }
                None => {}
            }
        }
    });
}

/// The mini player window, created from its configuration on first use.
/// Builds a window, so call it off the UI thread (it deadlocks in sync commands and event handlers on Windows).
fn mini_window(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    if let Some(mini) = app.get_webview_window("mini") {
        return Ok(mini);
    }
    let config = app.config().app.windows.iter().find(|w| w.label == "mini").ok_or("mini player not configured")?.clone();
    let mini = tauri::WebviewWindowBuilder::from_config(app, &config).map_err(|e| e.to_string())?.build().map_err(|e| e.to_string())?;
    let on_top = app.state::<SharedDb>().lock().map(|d| crate::settings::mini_on_top(&d)).unwrap_or(true);
    if let Err(e) = mini.set_always_on_top(on_top) {
        log::warn!("could not set mini player on top: {e}");
    }
    let (mini2, app2) = (mini.clone(), app.clone());
    mini.on_window_event(move |event| match event {
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            let _ = mini2.hide();
        }
        // The title bar's maximise button means "the full window, please".
        WindowEvent::Resized(_) if mini2.is_maximized().unwrap_or(false) => {
            let _ = mini2.unmaximize();
            let _ = open_main(app2.clone());
        }
        _ => {}
    });
    log::info!("mini player window created");
    Ok(mini)
}

fn show_mini_later(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        match mini_window(&app) {
            Ok(mini) => {
                set_memory_low(&mini, false);
                let _ = mini.show();
            }
            Err(e) => log::warn!("mini player: {e}"),
        }
    });
}

/// Ask WebView2 to give memory back while a window is out of sight (ICoreWebView2_19).
#[cfg(windows)]
fn set_memory_low(window: &tauri::WebviewWindow, low: bool) {
    let result = window.with_webview(move |webview| unsafe {
        use ::windows::core::Interface;
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            ICoreWebView2_19, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
        };
        let level = if low { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW } else { COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL };
        match webview.controller().CoreWebView2().and_then(|core| core.cast::<ICoreWebView2_19>()) {
            Ok(core) => {
                let _ = core.SetMemoryUsageTargetLevel(level);
            }
            Err(e) => log::debug!("memory target not supported by this WebView2: {e}"),
        }
    });
    if let Err(e) = result {
        log::debug!("memory target: {e}");
    }
}

#[cfg(not(windows))]
fn set_memory_low(_window: &tauri::WebviewWindow, _low: bool) {}

/// From the mini player: bring the full window back.
#[tauri::command]
pub fn open_main(app: AppHandle) -> Result<(), String> {
    let main = app.get_webview_window("main").ok_or("main window missing")?;
    main.unminimize().map_err(|e| e.to_string())?;
    main.show().map_err(|e| e.to_string())?;
    main.set_focus().map_err(|e| e.to_string())?;
    if let Some(mini) = app.get_webview_window("mini") {
        let _ = mini.hide();
    }
    Ok(())
}

#[tauri::command]
pub fn mini_on_top(db: State<'_, SharedDb>) -> Result<bool, String> {
    let d = db.lock().map_err(|e| e.to_string())?;
    Ok(crate::settings::mini_on_top(&d))
}

/// Pin / unpin the mini player: saved, applied, and announced so every control shows it.
#[tauri::command]
pub fn set_mini_on_top(on: bool, app: AppHandle, db: State<'_, SharedDb>) -> Result<(), String> {
    {
        let d = db.lock().map_err(|e| e.to_string())?;
        crate::settings::set_mini_on_top(&d, on).map_err(|e| e.to_string())?;
    }
    if let Some(mini) = app.get_webview_window("mini") {
        mini.set_always_on_top(on).map_err(|e| e.to_string())?;
    }
    let _ = app.emit("mini-on-top", on);
    Ok(())
}

#[tauri::command]
pub fn set_mini_on_minimize(on: bool, db: State<'_, SharedDb>) -> Result<(), String> {
    let d = db.lock().map_err(|e| e.to_string())?;
    crate::settings::set_mini_on_minimize(&d, on).map_err(|e| e.to_string())
}

/// The interface language setting ("auto", "en" or "vi").
#[tauri::command]
pub fn ui_language(db: State<'_, SharedDb>) -> Result<String, String> {
    let d = db.lock().map_err(|e| e.to_string())?;
    Ok(crate::settings::ui_language(&d))
}

/// Save the interface language and tell every window, which reload in it.
#[tauri::command]
pub fn set_ui_language(language: String, app: AppHandle, db: State<'_, SharedDb>) -> Result<(), String> {
    {
        let d = db.lock().map_err(|e| e.to_string())?;
        crate::settings::set_ui_language(&d, &language)?;
    }
    app.emit("language-changed", &language).map_err(|e| e.to_string())
}

/// Every setting for the settings page (F5.5).
#[tauri::command]
pub fn app_settings(db: State<'_, SharedDb>) -> Result<crate::settings::AppSettings, String> {
    let d = db.lock().map_err(|e| e.to_string())?;
    Ok(crate::settings::all(&d))
}

/// Change a setting that needs no other action (theme, conversion) and tell every window.
#[tauri::command]
pub fn set_app_setting(name: String, value: serde_json::Value, app: AppHandle, db: State<'_, SharedDb>) -> Result<crate::settings::AppSettings, String> {
    if !["theme", "conversion", "podcastCleanupDays"].contains(&name.as_str()) {
        return Err(format!("{name} has its own command"));
    }
    let all = {
        let d = db.lock().map_err(|e| e.to_string())?;
        crate::settings::change(&d, &name, &value)?;
        crate::settings::all(&d)
    };
    let _ = app.emit("settings-changed", &all);
    Ok(all)
}

/// Open the Apple Devices app's page in the Microsoft Store (the drivers Miu needs). The address is
/// fixed here: the UI cannot ask to open anything else.
#[tauri::command]
pub fn open_store_page() -> Result<(), String> {
    const APPLE_DEVICES: &str = "ms-windows-store://pdp/?productid=9NP83LWLPZ9K";
    std::process::Command::new("explorer.exe").arg(APPLE_DEVICES).spawn().map(|_| ()).map_err(|e| e.to_string())
}

/// Save a diagnostics report (logs + device facts, no databases or account ids) to `path` (F6.4).
#[tauri::command]
pub fn export_diagnostics(path: String, device: String) -> Result<(), String> {
    let app_data = crate::util::paths::app_data_dir_in(&crate::util::paths::appdata_root());
    let version = format!("Miu Music {} ({})", env!("CARGO_PKG_VERSION"), std::env::consts::OS);
    crate::util::diagnostics::export(&app_data, &version, &device, std::path::Path::new(&path)).map_err(|e| e.to_string())
}

/// From the main window: switch to the mini player (shown even if "open on minimise" is off).
/// Async, because the mini player may have to be created first.
#[tauri::command]
pub async fn open_mini(app: AppHandle) -> Result<(), String> {
    let mini = mini_window(&app)?;
    set_memory_low(&mini, false);
    mini.show().map_err(|e| e.to_string())?;
    if let Some(main) = app.get_webview_window("main") {
        main.minimize().map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Previous / play-pause / next under the taskbar preview (Windows thumbnail toolbar).
#[cfg(windows)]
#[tauri::command]
pub fn taskbar_init(labels: crate::taskbar_win::Labels, app: AppHandle) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("no main window")?;
    let hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as isize;
    let handle = app.clone();
    app.run_on_main_thread(move || {
        if let Err(e) = crate::taskbar_win::init(&handle, hwnd, labels) {
            log::warn!("taskbar buttons: {e}");
        }
    })
    .map_err(|e| e.to_string())
}

/// Swap the middle taskbar button between play and pause.
#[cfg(windows)]
#[tauri::command]
pub fn taskbar_playing(playing: bool, app: AppHandle) -> Result<(), String> {
    app.run_on_main_thread(move || {
        if let Err(e) = crate::taskbar_win::set_playing(playing) {
            log::warn!("taskbar buttons: {e}");
        }
    })
    .map_err(|e| e.to_string())
}

/// What follows the main window being minimised or restored.
#[derive(Debug, PartialEq, Eq)]
pub struct MainChange {
    /// Show (true) or hide (false) the mini player; None leaves it alone.
    pub mini: Option<bool>,
    /// Ask WebView2 to keep as little memory as it can while nobody looks at the window.
    pub memory_low: bool,
}

pub fn on_main_resized(minimized: bool, mini_wanted: bool) -> MainChange {
    match (minimized, mini_wanted) {
        (true, true) => MainChange { mini: Some(true), memory_low: true },
        (true, false) => MainChange { mini: None, memory_low: true },
        (false, _) => MainChange { mini: Some(false), memory_low: false },
    }
}

/// Height the main window opens at: 90 % of the screen's usable height, between 600 and 960.
pub fn start_height(work_area_height: f64) -> f64 {
    ((work_area_height * 0.9 * 10.0).round() / 10.0).clamp(600.0, 960.0)
}

/// Open the main window taller, fitted to the screen it is on, and centred.
fn fit_main_window(main: &tauri::WebviewWindow) {
    let Ok(Some(monitor)) = main.current_monitor() else { return };
    let scale = monitor.scale_factor();
    let height = start_height(monitor.work_area().size.height as f64 / scale);
    let width = main.outer_size().map(|s| s.width as f64 / scale).unwrap_or(1280.0);
    if main.set_size(tauri::LogicalSize::new(width, height)).is_ok() {
        let _ = main.center();
    }
}

#[cfg(test)]
mod tests {
    use super::{on_main_resized, start_height, MainChange};

    #[test]
    fn minimising_shows_the_mini_player_and_trims_memory() {
        assert_eq!(on_main_resized(true, true), MainChange { mini: Some(true), memory_low: true });
        assert_eq!(on_main_resized(true, false), MainChange { mini: None, memory_low: true }, "mini player off: just trim");
        assert_eq!(on_main_resized(false, true), MainChange { mini: Some(false), memory_low: false });
        assert_eq!(on_main_resized(false, false), MainChange { mini: Some(false), memory_low: false });
    }

    #[test]
    fn the_window_opens_taller_but_never_past_the_screen() {
        assert_eq!(start_height(1400.0), 960.0, "capped on tall screens");
        assert_eq!(start_height(1040.0), 936.0, "1080p at 100 %: 90 % of the work area");
        assert_eq!(start_height(824.0), 741.6, "1080p at 125 %");
        assert_eq!(start_height(500.0), 600.0, "never below the minimum size");
    }
}
