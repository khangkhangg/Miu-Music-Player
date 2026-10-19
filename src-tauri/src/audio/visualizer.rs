//! Visualizer feed (F7.6): while the gate is open (on, mini player visible, playing), turn what the
//! device just played into band levels and emit them as `spectrum` (bytes, ~30/s). Otherwise idle.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::audio::service::SharedPlayer;
use crate::audio::spectrum::{to_bytes, FrameGate, Spectrum};
use crate::library::service::SharedDb;

const BANDS: usize = 28;

/// Cached "visualizer on" so the 10 ms loop never touches the database.
pub type VisualizerOn = Arc<AtomicBool>;

pub fn start(app: AppHandle, player: SharedPlayer, on: VisualizerOn) {
    std::thread::spawn(move || {
        // The device rate changes with the output device, and per song in exclusive mode.
        let mut rate = 0;
        let mut spectrum = Spectrum::new(48_000, BANDS);
        let mut gate = FrameGate::default();
        let started = Instant::now();
        loop {
            std::thread::sleep(Duration::from_millis(10));
            let visible = app.get_webview_window("mini").and_then(|w| w.is_visible().ok()).unwrap_or(false);
            let playing = player.is_playing();
            if gate.tick(started.elapsed().as_millis() as u64, on.load(Ordering::Relaxed), visible, playing) {
                match player.rate() {
                    Some(r) if r > 0 && r != rate => {
                        rate = r;
                        spectrum = Spectrum::new(r, BANDS);
                    }
                    _ => {}
                }
                let levels = spectrum.levels(&player.recent_mono());
                let _ = app.emit_to("mini", "spectrum", to_bytes(&levels));
            }
        }
    });
}

#[derive(Clone, Serialize)]
pub struct VisualizerSettings {
    on: bool,
    style: String,
}

#[tauri::command]
pub fn visualizer_settings(db: State<'_, SharedDb>) -> Result<VisualizerSettings, String> {
    visualizer_settings_now(&db)
}

#[tauri::command]
pub fn set_visualizer_on(on: bool, app: AppHandle, db: State<'_, SharedDb>, flag: State<'_, VisualizerOn>) -> Result<(), String> {
    {
        let d = db.lock().map_err(|e| e.to_string())?;
        crate::settings::set_visualizer_on(&d, on).map_err(|e| e.to_string())?;
    }
    flag.store(on, Ordering::Relaxed);
    let _ = app.emit("visualizer-settings", visualizer_settings_now(&db)?);
    Ok(())
}

#[tauri::command]
pub fn set_visualizer_style(style: String, app: AppHandle, db: State<'_, SharedDb>) -> Result<(), String> {
    {
        let d = db.lock().map_err(|e| e.to_string())?;
        crate::settings::set_visualizer_style(&d, &style)?;
    }
    let _ = app.emit("visualizer-settings", visualizer_settings_now(&db)?);
    Ok(())
}

fn visualizer_settings_now(db: &SharedDb) -> Result<VisualizerSettings, String> {
    let d = db.lock().map_err(|e| e.to_string())?;
    Ok(VisualizerSettings { on: crate::settings::visualizer_on(&d), style: crate::settings::visualizer_style(&d) })
}
