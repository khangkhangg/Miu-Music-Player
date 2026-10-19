//! Main window ⇄ mini player (F7.4): minimizing the main window shows the mini player (setting
//! `mini_on_minimize`); restoring it hides the mini player again. Closing the mini player hides it.

use tauri::{AppHandle, Emitter, Manager, State, WindowEvent};

use crate::library::service::SharedDb;

pub fn setup(app: &AppHandle, db: SharedDb) {
    let (Some(main), Some(mini)) = (app.get_webview_window("main"), app.get_webview_window("mini")) else {
        log::warn!("mini player window missing");
        return;
    };
    let on_top = db.lock().map(|d| crate::settings::mini_on_top(&d)).unwrap_or(true);
    if let Err(e) = mini.set_always_on_top(on_top) {
        log::warn!("could not set mini player on top: {e}");
    }
    let (main2, mini2) = (main.clone(), mini.clone());
    main.on_window_event(move |event| {
        if let WindowEvent::Resized(_) = event {
            let minimized = main2.is_minimized().unwrap_or(false);
            let wanted = db.lock().map(|d| crate::settings::mini_on_minimize(&d)).unwrap_or(true);
            if minimized && wanted {
                let _ = mini2.show();
            } else if !minimized {
                let _ = mini2.hide();
            }
        }
    });
    let mini3 = mini.clone();
    mini.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = mini3.hide();
        }
    });
}

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
