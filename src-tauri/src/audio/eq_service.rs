//! App glue for the equalizer (F7.7): commands, persistence, and an `eq-changed` event.

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::audio::eq::{EqSettings, BANDS_HZ};
use crate::audio::presets::{built_in, clip_risk, match_preset, Preset};
use crate::audio::service::SharedPlayer;
use crate::library::service::SharedDb;
use crate::settings;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EqState {
    pub settings: EqSettings,
    pub bands_hz: [f32; 10],
    pub built_in: Vec<Preset>,
    pub user: Vec<Preset>,
    /// Name of the matching preset, or None for "Custom".
    pub preset: Option<String>,
    pub clip_risk: bool,
}

fn state(db: &SharedDb) -> Result<EqState, String> {
    let d = db.lock().map_err(|e| e.to_string())?;
    let s = settings::eq_settings(&d);
    let user = settings::user_presets(&d);
    Ok(EqState { preset: match_preset(&s, &user), clip_risk: clip_risk(&s), settings: s, bands_hz: BANDS_HZ, built_in: built_in(), user })
}

fn apply(app: &AppHandle, db: &SharedDb, player: &SharedPlayer, s: EqSettings) -> Result<EqState, String> {
    {
        let d = db.lock().map_err(|e| e.to_string())?;
        settings::set_eq_settings(&d, &s).map_err(|e| e.to_string())?;
    }
    player.set_eq(s.clamped());
    let st = state(db)?;
    let _ = app.emit("eq-changed", &st);
    Ok(st)
}

/// At startup: the engine gets the saved settings.
pub fn start(db: &SharedDb, player: &SharedPlayer) {
    if let Ok(d) = db.lock() {
        player.set_eq(settings::eq_settings(&d));
    }
}

#[tauri::command]
pub fn eq_state(db: State<'_, SharedDb>) -> Result<EqState, String> {
    state(&db)
}

#[tauri::command]
pub fn set_eq(settings: EqSettings, app: AppHandle, db: State<'_, SharedDb>, player: State<'_, SharedPlayer>) -> Result<EqState, String> {
    apply(&app, &db, &player, settings)
}

#[tauri::command]
pub fn apply_eq_preset(name: String, app: AppHandle, db: State<'_, SharedDb>, player: State<'_, SharedPlayer>) -> Result<EqState, String> {
    let current = state(&db)?;
    let preset = current
        .built_in
        .iter()
        .chain(&current.user)
        .find(|p| p.name.eq_ignore_ascii_case(&name))
        .ok_or_else(|| format!("no preset called {name}"))?;
    let s = EqSettings { enabled: current.settings.enabled, ..preset.settings() };
    apply(&app, &db, &player, s)
}

#[tauri::command]
pub fn save_eq_preset(name: String, app: AppHandle, db: State<'_, SharedDb>) -> Result<EqState, String> {
    {
        let d = db.lock().map_err(|e| e.to_string())?;
        let s = settings::eq_settings(&d);
        settings::save_user_preset(&d, &name, &s)?;
    }
    let st = state(&db)?;
    let _ = app.emit("eq-changed", &st);
    Ok(st)
}

#[tauri::command]
pub fn delete_eq_preset(name: String, app: AppHandle, db: State<'_, SharedDb>) -> Result<EqState, String> {
    {
        let d = db.lock().map_err(|e| e.to_string())?;
        settings::delete_user_preset(&d, &name).map_err(|e| e.to_string())?;
    }
    let st = state(&db)?;
    let _ = app.emit("eq-changed", &st);
    Ok(st)
}
