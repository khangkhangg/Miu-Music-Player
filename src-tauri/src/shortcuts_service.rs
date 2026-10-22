//! App glue for keyboard shortcuts (K.1): registers the saved keys with Windows (through Tauri's
//! global-shortcut plugin, so they work while other apps are in front) and runs the actions.
//! Playback goes straight to the player; like, dislike and sync press the main window's own
//! buttons (a `shortcut` event), so they behave exactly as clicked.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::library::service::SharedDb;
use crate::shortcuts::Action;

/// Registered shortcuts by id, and why an action's keys could not be registered.
#[derive(Default)]
struct Registered {
    by_id: HashMap<u32, Action>,
    failed: HashMap<Action, String>,
}

static REGISTERED: Mutex<Option<Registered>> = Mutex::new(None);

pub fn plugin() -> tauri::plugin::TauriPlugin<tauri::Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| {
            if event.state() != ShortcutState::Pressed {
                return;
            }
            let action = REGISTERED.lock().ok().and_then(|r| r.as_ref()?.by_id.get(&shortcut.id()).copied());
            log::info!("shortcut {shortcut} pressed: {action:?}");
            if let Some(action) = action {
                run(app, action);
            }
        })
        .build()
}

/// The plugin's spelling of Miu's keys ("Win" is "Super" there).
fn plugin_keys(keys: &str) -> String {
    keys.replace("Win+", "Super+")
}

/// Register the saved shortcuts afresh (none when they are turned off).
pub fn apply(app: &AppHandle, db: &SharedDb) {
    let (enabled, bindings) = match db.lock() {
        Ok(d) => (crate::settings::shortcuts_enabled(&d), crate::settings::shortcut_bindings(&d)),
        Err(_) => return,
    };
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    let mut reg = Registered::default();
    if enabled {
        for (action, keys) in bindings {
            let Some(keys) = keys else { continue };
            let result = plugin_keys(&keys).parse::<Shortcut>().map_err(|e| e.to_string()).and_then(|s| gs.register(s).map(|_| s).map_err(|e| e.to_string()));
            match result {
                Ok(s) => {
                    reg.by_id.insert(s.id(), action);
                }
                Err(e) => {
                    log::warn!("shortcut {keys} for {action:?} not registered: {e}");
                    reg.failed.insert(action, e);
                }
            }
        }
    }
    if let Ok(mut r) = REGISTERED.lock() {
        *r = Some(reg);
    }
}

fn run(app: &AppHandle, action: Action) {
    let player = app.state::<crate::audio::service::SharedPlayer>();
    let result = match action {
        Action::PlayPause if player.is_playing() => crate::audio::service::player_pause(player),
        Action::PlayPause => crate::audio::service::player_resume(player),
        Action::Next => crate::audio::service::player_skip(1, player),
        Action::Previous => crate::audio::service::player_skip(-1, player),
        Action::MiniPlayer => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = crate::windows::open_mini(app).await {
                    log::warn!("mini player shortcut: {e}");
                }
            });
            Ok(())
        }
        Action::Restore => crate::windows::open_main(app.clone()),
        Action::Like | Action::Dislike | Action::Sync => app.emit_to("main", "shortcut", action).map_err(|e| e.to_string()),
    };
    if let Err(e) = result {
        log::debug!("shortcut {action:?}: {e}");
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutItem {
    action: Action,
    keys: Option<String>,
    default_keys: &'static str,
    /// Windows would not take the keys (another app has them).
    taken: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutsState {
    enabled: bool,
    items: Vec<ShortcutItem>,
}

#[tauri::command]
pub fn shortcuts_state(db: State<'_, SharedDb>) -> Result<ShortcutsState, String> {
    let d = db.lock().map_err(|e| e.to_string())?;
    let failed = REGISTERED.lock().ok().and_then(|r| r.as_ref().map(|r| r.failed.keys().copied().collect::<Vec<_>>())).unwrap_or_default();
    Ok(ShortcutsState {
        enabled: crate::settings::shortcuts_enabled(&d),
        items: crate::settings::shortcut_bindings(&d)
            .into_iter()
            .map(|(action, keys)| ShortcutItem { taken: failed.contains(&action), default_keys: action.default_keys(), action, keys })
            .collect(),
    })
}

/// New keys for one action (`None` turns it off); errors say why (bad keys, already used).
#[tauri::command]
pub fn set_shortcut_keys(action: Action, keys: Option<String>, app: AppHandle, db: State<'_, SharedDb>) -> Result<ShortcutsState, String> {
    crate::settings::set_shortcut(&*db.lock().map_err(|e| e.to_string())?, action, keys.as_deref())?;
    apply(&app, db.inner());
    shortcuts_state(db)
}

#[tauri::command]
pub fn reset_shortcut_keys(app: AppHandle, db: State<'_, SharedDb>) -> Result<ShortcutsState, String> {
    crate::settings::reset_shortcuts(&*db.lock().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    apply(&app, db.inner());
    shortcuts_state(db)
}

#[tauri::command]
pub fn set_shortcuts_enabled(on: bool, app: AppHandle, db: State<'_, SharedDb>) -> Result<ShortcutsState, String> {
    crate::settings::change(&*db.lock().map_err(|e| e.to_string())?, "shortcutsEnabled", &serde_json::json!(on))?;
    apply(&app, db.inner());
    shortcuts_state(db)
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_default_reads_as_a_shortcut_for_the_plugin() {
        for a in crate::shortcuts::ACTIONS {
            let keys = super::plugin_keys(a.default_keys());
            assert!(keys.parse::<super::Shortcut>().is_ok(), "{keys}");
        }
        assert!(super::plugin_keys("Ctrl+Shift+Win+F5").parse::<super::Shortcut>().is_ok());
    }
}

/// While the listener is choosing new keys in Settings, Windows must not swallow them: pause all
/// shortcuts, then bring them back (nothing is saved either way).
#[tauri::command]
pub fn pause_shortcuts(paused: bool, app: AppHandle, db: State<'_, SharedDb>) -> Result<(), String> {
    if paused {
        app.global_shortcut().unregister_all().map_err(|e| e.to_string())
    } else {
        apply(&app, db.inner());
        Ok(())
    }
}
