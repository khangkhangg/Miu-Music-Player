//! App glue for playback (F7.1): Tauri commands and a `player` event with what is playing.
//! The engine is `audio::player`; songs are looked up in the library database.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::audio::player::Player;
use crate::library::service::SharedDb;
use crate::library::songs::paths_for;

pub struct PlayerHandle {
    player: Option<Player>,
    /// Song ids of the current queue, by queue index.
    queue: Mutex<Vec<i64>>,
}

pub type SharedPlayer = Arc<PlayerHandle>;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlayerEvent {
    pub available: bool,
    pub playing: bool,
    pub song_id: Option<i64>,
    pub position_ms: u64,
    pub volume: f32,
}

impl PlayerHandle {
    pub fn new() -> Self {
        let player = match Player::start() {
            Ok(p) => Some(p),
            Err(e) => {
                log::error!("no audio output: {e}");
                None
            }
        };
        Self { player, queue: Mutex::new(Vec::new()) }
    }

    fn event(&self) -> PlayerEvent {
        let Some(p) = &self.player else {
            return PlayerEvent { available: false, playing: false, song_id: None, position_ms: 0, volume: 0.0 };
        };
        let s = p.status();
        let song_id = s.track.and_then(|t| self.queue.lock().ok()?.get(t).copied());
        PlayerEvent { available: true, playing: s.playing, song_id, position_ms: s.position_ms, volume: s.volume }
    }

    pub fn set_eq(&self, settings: crate::audio::eq::EqSettings) {
        if let Some(p) = &self.player {
            p.set_eq(settings);
        }
    }

    pub fn rate(&self) -> Option<u32> {
        self.player.as_ref().map(Player::rate)
    }

    pub fn is_playing(&self) -> bool {
        self.player.as_ref().is_some_and(|p| p.status().playing)
    }

    pub fn recent_mono(&self) -> Vec<f32> {
        self.player.as_ref().map(Player::recent_mono).unwrap_or_default()
    }

    fn with_player(&self, f: impl FnOnce(&Player)) -> Result<(), String> {
        self.player.as_ref().map(f).ok_or_else(|| "no audio output device".to_string())
    }
}

/// Emit `player` events while something changes (every 250 ms at most).
pub fn start(app: AppHandle, handle: SharedPlayer) {
    std::thread::spawn(move || {
        let mut last: Option<PlayerEvent> = None;
        loop {
            let ev = handle.event();
            if last.as_ref() != Some(&ev) {
                let _ = app.emit("player", &ev);
                last = Some(ev);
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    });
}

/// Play `ids` as a queue, starting at `start`.
#[tauri::command]
pub fn play_songs(ids: Vec<i64>, start: usize, db: State<'_, SharedDb>, player: State<'_, SharedPlayer>) -> Result<(), String> {
    let found = {
        let d = db.lock().map_err(|e| e.to_string())?;
        paths_for(&d, &ids).map_err(|e| e.to_string())?
    };
    let start_id = ids.get(start).copied();
    let start = found.iter().position(|(id, _)| Some(*id) == start_id).unwrap_or(0);
    let (queue, paths): (Vec<i64>, Vec<_>) = found.into_iter().unzip();
    *player.queue.lock().map_err(|e| e.to_string())? = queue;
    player.with_player(|p| p.play(paths, start))
}

#[tauri::command]
pub fn player_pause(player: State<'_, SharedPlayer>) -> Result<(), String> {
    player.with_player(Player::pause)
}

#[tauri::command]
pub fn player_resume(player: State<'_, SharedPlayer>) -> Result<(), String> {
    player.with_player(Player::resume)
}

#[tauri::command]
pub fn player_seek(ms: u64, player: State<'_, SharedPlayer>) -> Result<(), String> {
    player.with_player(|p| p.seek(ms))
}

#[tauri::command]
pub fn player_volume(volume: f32, player: State<'_, SharedPlayer>) -> Result<(), String> {
    player.with_player(|p| p.set_volume(volume))
}

#[tauri::command]
pub fn player_skip(by: i64, player: State<'_, SharedPlayer>) -> Result<(), String> {
    player.with_player(|p| p.skip(by))
}
