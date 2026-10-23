//! App glue for playback (F7.1, F7.8): Tauri commands and a `player` event with what is playing.
//! The engine is `audio::player`; songs are looked up in the library database.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

use crate::audio::decode::SourceFormat;
use crate::audio::order::Repeat;
use crate::audio::dual::DualMode;
use crate::audio::player::{output_devices, OutputDevice, Player, SecondOutput, SecondStatus};
use crate::library::service::SharedDb;
use crate::library::songs::paths_for;

pub struct PlayerHandle {
    /// Replaced when the output device changes.
    player: Mutex<Option<Player>>,
    /// Song ids and paths of the current queue, by queue index.
    queue: Mutex<Vec<i64>>,
    paths: Mutex<Vec<PathBuf>>,
    /// Podcast episodes or audiobook chapters played instead of songs (P.5, A.3), by queue index;
    /// empty while songs play.
    episodes: Mutex<Vec<i64>>,
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
    pub source: Option<SourceFormat>,
    pub device_name: String,
    pub device_rate: u32,
    pub exclusive: bool,
    pub device_bits: u32,
    pub shuffle: bool,
    pub repeat: Repeat,
    /// Song ids that play next, in order.
    pub up_next: Vec<i64>,
    /// Set while a podcast episode plays (then `song_id` is empty).
    pub episode_id: Option<i64>,
    /// The second output while it plays (F7.9).
    pub second: Option<SecondStatus>,
    /// A second output is set but could not be opened or went away: one output plays.
    pub second_lost: bool,
    /// The alignment clicks are playing.
    pub aligning: bool,
    /// How steady the first output has been over the last minute.
    pub health: crate::audio::health::Health,
}

fn open(device: Option<&str>, exclusive: bool, second: Option<SecondOutput>) -> Option<Player> {
    match Player::start_with(device, exclusive, second) {
        Ok(p) => Some(p),
        Err(e) => {
            log::error!("no audio output: {e}");
            None
        }
    }
}

impl PlayerHandle {
    /// `device`: the saved output device name, or `None` for the Windows default.
    pub fn new(device: Option<&str>, exclusive: bool) -> Self {
        Self { player: Mutex::new(open(device, exclusive, None)), queue: Mutex::new(Vec::new()), paths: Mutex::new(Vec::new()), episodes: Mutex::new(Vec::new()) }
    }

    fn with<T>(&self, f: impl FnOnce(&Player) -> T) -> Option<T> {
        self.player.lock().ok()?.as_ref().map(f)
    }

    fn event(&self) -> PlayerEvent {
        let Some(s) = self.with(Player::status) else {
            return PlayerEvent {
                available: false,
                playing: false,
                song_id: None,
                position_ms: 0,
                volume: 0.0,
                source: None,
                device_name: String::new(),
                device_rate: 0,
                exclusive: false,
                device_bits: 0,
                shuffle: false,
                repeat: Repeat::Off,
                up_next: Vec::new(),
                episode_id: None,
                second: None,
                second_lost: false,
                aligning: false,
                health: Default::default(),
            };
        };
        let queue = self.queue.lock().map(|q| q.clone()).unwrap_or_default();
        let episode_id = s.track.and_then(|t| self.episodes.lock().ok()?.get(t).copied());
        let song_id = if episode_id.is_some() { None } else { s.track.and_then(|t| queue.get(t).copied()) };
        let up_next = s.up_next.iter().filter_map(|&t| queue.get(t).copied()).collect();
        PlayerEvent {
            available: true,
            playing: s.playing,
            song_id,
            position_ms: s.position_ms,
            volume: s.volume,
            source: s.source,
            device_name: s.device_name,
            device_rate: s.device_rate,
            exclusive: s.exclusive,
            device_bits: s.device_bits,
            shuffle: s.shuffle,
            repeat: s.repeat,
            up_next,
            episode_id,
            second: s.second,
            second_lost: s.second_lost,
            aligning: s.aligning,
            health: s.health,
        }
    }

    /// Like `new`, but the device is opened on another thread: the app window does not wait for
    /// it. Until then the player reports "unavailable" and commands answer "no audio output device".
    pub fn new_deferred(device: Option<String>, exclusive: bool, second: Option<SecondOutput>, shuffle: bool, repeat: Repeat) -> Arc<Self> {
        let handle = Arc::new(Self { player: Mutex::new(None), queue: Mutex::new(Vec::new()), paths: Mutex::new(Vec::new()), episodes: Mutex::new(Vec::new()) });
        let h = handle.clone();
        std::thread::spawn(move || {
            if let Some(p) = open(device.as_deref(), exclusive, second) {
                p.set_shuffle(shuffle);
                p.set_repeat(repeat);
                if let Ok(mut slot) = h.player.lock() {
                    if slot.is_none() {
                        *slot = Some(p);
                    }
                }
            }
        });
        handle
    }

    pub fn set_eq(&self, settings: crate::audio::eq::EqSettings) {
        self.with(|p| p.set_eq(settings));
    }

    pub fn rate(&self) -> Option<u32> {
        self.with(Player::rate)
    }

    pub fn is_playing(&self) -> bool {
        self.with(|p| p.status().playing).unwrap_or(false)
    }

    pub fn recent_mono(&self) -> Vec<f32> {
        self.with(Player::recent_mono).unwrap_or_default()
    }

    fn act(&self, f: impl FnOnce(&Player)) -> Result<(), String> {
        self.with(f).ok_or_else(|| "no audio output device".to_string())
    }

    /// A press on Windows' media controls: the same player calls as the UI's buttons.
    pub fn run(&self, command: crate::media_controls::Command) -> Result<(), String> {
        use crate::media_controls::Command;
        match command {
            Command::Resume => self.act(Player::resume),
            Command::Pause => self.act(Player::pause),
            Command::Skip(by) => self.act(|p| p.skip(by)),
            Command::Seek(ms) => self.act(|p| p.seek(ms)),
        }
    }

    /// Move playback to another output device or mode, keeping queue, song, position, volume, EQ and pause.
    pub fn switch_output(&self, device: Option<&str>, exclusive: bool, second: Option<SecondOutput>, eq: crate::audio::eq::EqSettings) -> Result<(), String> {
        let mut slot = self.player.lock().map_err(|e| e.to_string())?;
        let before = slot.as_ref().map(Player::status);
        *slot = None; // stops the old engine and closes its device
        let player = open(device, exclusive, second).ok_or("could not open that output device")?;
        player.set_eq(eq);
        if let Some(s) = before {
            player.set_shuffle(s.shuffle);
            player.set_repeat(s.repeat);
            player.set_volume(s.volume);
            let paths = self.paths.lock().map_err(|e| e.to_string())?.clone();
            if let (Some(track), false) = (s.track, paths.is_empty()) {
                if self.episodes.lock().map(|e| e.len() > 1).unwrap_or(false) {
                    player.play_in_order(paths, track);
                } else {
                    player.play(paths, track);
                }
                player.seek(s.position_ms);
                if !s.playing {
                    player.pause();
                }
            }
        }
        *slot = Some(player);
        Ok(())
    }
}

/// Emit `player` events while something changes (every 250 ms at most).
pub fn start(app: AppHandle, handle: SharedPlayer, db: SharedDb) {
    std::thread::spawn(move || {
        let mut last: Option<PlayerEvent> = None;
        let mut saved_at = std::time::Instant::now();
        loop {
            let ev = handle.event();
            // Podcast episodes remember where the listener is (every 10 s, and on pause).
            if let Some(id) = ev.episode_id {
                let paused_now = last.as_ref().is_some_and(|l| l.playing && !ev.playing);
                if paused_now || (ev.playing && saved_at.elapsed() >= Duration::from_secs(10)) {
                    save_episode_progress(&app, &db, id, ev.position_ms);
                    saved_at = std::time::Instant::now();
                }
            }
            if last.as_ref() != Some(&ev) {
                #[cfg(windows)]
                crate::media_controls_win::show(&ev, &db);
                let _ = app.emit("player", &ev);
                last = Some(ev);
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    });
}

fn save_episode_progress(app: &AppHandle, db: &SharedDb, id: i64, position_ms: u64) {
    let Ok(d) = db.lock() else { return };
    let duration: Option<i64> = d.conn().query_row("SELECT duration_ms FROM episodes WHERE id = ?1", [id], |r| r.get(0)).ok().flatten();
    let (position, played) = crate::podcast::policy::progress(position_ms, duration.map(|v| v as u64));
    let was_played: bool = d.conn().query_row("SELECT played FROM episodes WHERE id = ?1", [id], |r| r.get(0)).unwrap_or(false);
    let _ = crate::podcast::store::set_progress(&d, id, position, played || was_played, crate::util::time::now_unix());
    if played && !was_played {
        let _ = app.emit("podcasts-changed", id);
    }
}

/// Play a downloaded podcast episode, resuming where the listener stopped.
#[tauri::command]
pub fn play_episode(id: i64, db: State<'_, SharedDb>, player: State<'_, SharedPlayer>) -> Result<(), String> {
    let (path, position, played): (Option<String>, i64, bool) = {
        let d = db.lock().map_err(|e| e.to_string())?;
        d.conn()
            .query_row("SELECT download_path, position_ms, played FROM episodes WHERE id = ?1", [id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(|e| e.to_string())?
    };
    let path = PathBuf::from(path.ok_or("download the episode first")?);
    *player.queue.lock().map_err(|e| e.to_string())? = Vec::new();
    *player.paths.lock().map_err(|e| e.to_string())? = vec![path.clone()];
    *player.episodes.lock().map_err(|e| e.to_string())? = vec![id];
    let resume = crate::podcast::policy::resume_at(position, played);
    player.act(|p| {
        p.play(vec![path], 0);
        if let Some(ms) = resume {
            p.seek(ms);
        }
    })
}

/// Play a book's downloaded chapters from `start` (A.3): in order, resuming where the listener
/// stopped in the first one, then on to the next chapter.
#[tauri::command]
pub fn play_chapters(book: i64, start: Option<i64>, db: State<'_, SharedDb>, player: State<'_, SharedPlayer>) -> Result<(), String> {
    use crate::audiobook::store;
    let chapters = store::chapters_of(&*db.lock().map_err(|e| e.to_string())?, book).map_err(|e| e.to_string())?;
    let start = start.or_else(|| store::resume_point(&chapters).map(|r| r.0)).or_else(|| chapters.first().map(|c| c.id)).ok_or("this book has no chapters")?;
    let queue = store::play_queue(&chapters, start);
    if queue.is_empty() {
        return Err("download the chapter first".into());
    }
    let first = chapters.iter().find(|c| c.id == start).ok_or("no such chapter")?;
    let resume = crate::podcast::policy::resume_at(first.position_ms, first.played);
    let (ids, paths): (Vec<i64>, Vec<PathBuf>) = queue.into_iter().map(|(id, p)| (id, PathBuf::from(p))).unzip();
    *player.queue.lock().map_err(|e| e.to_string())? = Vec::new();
    *player.paths.lock().map_err(|e| e.to_string())? = paths.clone();
    *player.episodes.lock().map_err(|e| e.to_string())? = ids;
    player.act(|p| {
        p.play_in_order(paths, 0);
        if let Some(ms) = resume {
            p.seek(ms);
        }
    })
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
    *player.paths.lock().map_err(|e| e.to_string())? = paths.clone();
    player.episodes.lock().map_err(|e| e.to_string())?.clear();
    player.act(|p| p.play(paths, start))
}

#[tauri::command]
pub fn player_pause(player: State<'_, SharedPlayer>) -> Result<(), String> {
    player.act(Player::pause)
}

#[tauri::command]
pub fn player_resume(player: State<'_, SharedPlayer>) -> Result<(), String> {
    player.act(Player::resume)
}

#[tauri::command]
pub fn player_seek(ms: u64, player: State<'_, SharedPlayer>) -> Result<(), String> {
    player.act(|p| p.seek(ms))
}

#[tauri::command]
pub fn player_volume(volume: f32, player: State<'_, SharedPlayer>) -> Result<(), String> {
    player.act(|p| p.set_volume(volume))
}

#[tauri::command]
pub fn player_skip(by: i64, player: State<'_, SharedPlayer>) -> Result<(), String> {
    player.act(|p| p.skip(by))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputState {
    devices: Vec<OutputDevice>,
    /// The saved choice; `None` = follow the Windows default.
    chosen: Option<String>,
    /// Exclusive (bit-perfect) mode is switched on.
    exclusive: bool,
    /// Second output (F7.9): the device, the mode, and the delays (ms) of the first and second
    /// output as remembered for this pair.
    second: Option<String>,
    dual_mode: DualMode,
    delays: [u32; 2],
    /// Balance between the outputs, dB: positive turns the second one down.
    balance: f32,
    /// Battery level (percent) of outputs that report one, by device name.
    batteries: std::collections::HashMap<String, u8>,
    /// Microphones that can listen for auto-calibration.
    mics: Vec<String>,
}

/// The first output's device name: the chosen one, or whichever is the Windows default now.
fn first_name(chosen: Option<String>) -> String {
    chosen.or_else(|| output_devices().into_iter().find(|d| d.is_default).map(|d| d.name)).unwrap_or_default()
}

/// The saved second output, ready for the engine.
pub fn second_config(d: &crate::library::db::Db) -> Option<SecondOutput> {
    let device = crate::settings::second_output(d)?;
    let first = first_name(crate::settings::output_device(d));
    Some(SecondOutput {
        delays_ms: crate::settings::dual_delays(d, &first, &device),
        balance_db: crate::settings::dual_balance(d, &first, &device),
        mode: crate::settings::dual_mode(d),
        device,
    })
}

#[tauri::command]
pub fn output_state(db: State<'_, SharedDb>) -> Result<OutputState, String> {
    let d = db.lock().map_err(|e| e.to_string())?;
    let second = second_config(&d);
    let devices = output_devices();
    let bluetooth = crate::audio::bluetooth::present_devices();
    let batteries = devices.iter().filter_map(|o| Some((o.name.clone(), crate::audio::bluetooth::battery_for(&o.name, &bluetooth)?))).collect();
    // Only the two speakers that play: their own microphones would switch them into call mode.
    let playing: Vec<String> = [Some(first_name(crate::settings::output_device(&d))), second.as_ref().map(|s| s.device.clone())].into_iter().flatten().collect();
    let mics = crate::audio::bluetooth::usable_mics(&crate::audio::listen::microphones(), &playing);
    Ok(OutputState {
        batteries,
        mics,
        devices,
        chosen: crate::settings::output_device(&d),
        exclusive: crate::settings::exclusive_output(&d),
        dual_mode: crate::settings::dual_mode(&d),
        delays: second.as_ref().map_or([0, 0], |s| s.delays_ms),
        balance: second.as_ref().map_or(0.0, |s| s.balance_db),
        second: second.map(|s| s.device),
    })
}

/// Reopen the output with the saved settings; playback carries on where it was.
fn reopen(db: &State<'_, SharedDb>, player: &State<'_, SharedPlayer>) -> Result<(), String> {
    let (eq, device, exclusive, second) = {
        let d = db.lock().map_err(|e| e.to_string())?;
        (crate::settings::eq_settings(&d), crate::settings::output_device(&d), crate::settings::exclusive_output(&d), second_config(&d))
    };
    player.switch_output(device.as_deref(), exclusive, second, eq)
}

/// For DJ mode: an exclusive-mode player holds the speakers even when paused, so it moves to
/// shared mode while DJ mode is open (`shared_only`) and back after. Nothing to do when exclusive
/// mode is off.
pub fn reopen_output(db: &SharedDb, player: &SharedPlayer, shared_only: bool) -> Result<(), String> {
    let (eq, device, exclusive, second) = {
        let d = db.lock().map_err(|e| e.to_string())?;
        (crate::settings::eq_settings(&d), crate::settings::output_device(&d), crate::settings::exclusive_output(&d), second_config(&d))
    };
    if !exclusive {
        return Ok(());
    }
    player.switch_output(device.as_deref(), !shared_only, second, eq)
}

/// Choose an output device (`None` = Windows default); playback moves over without losing its place.
#[tauri::command]
pub fn set_output(device: Option<String>, db: State<'_, SharedDb>, player: State<'_, SharedPlayer>) -> Result<OutputState, String> {
    {
        let d = db.lock().map_err(|e| e.to_string())?;
        crate::settings::set_output_device(&d, device.as_deref()).map_err(|e| e.to_string())?;
        // The first output cannot also be the second one.
        if device.is_some() && crate::settings::second_output(&d) == device {
            crate::settings::set_second_output(&d, None).map_err(|e| e.to_string())?;
        }
    }
    reopen(&db, &player)?;
    output_state(db)
}

/// Switch exclusive (bit-perfect) mode on or off; playback carries on where it was.
#[tauri::command]
pub fn set_exclusive(on: bool, db: State<'_, SharedDb>, player: State<'_, SharedPlayer>) -> Result<OutputState, String> {
    crate::settings::set_exclusive_output(&*db.lock().map_err(|e| e.to_string())?, on).map_err(|e| e.to_string())?;
    reopen(&db, &player)?;
    output_state(db)
}

/// Play on a second output too (F7.9, shared mode only), or on one again (`None`).
#[tauri::command]
pub fn set_second_output(device: Option<String>, db: State<'_, SharedDb>, player: State<'_, SharedPlayer>) -> Result<OutputState, String> {
    crate::settings::set_second_output(&*db.lock().map_err(|e| e.to_string())?, device.as_deref()).map_err(|e| e.to_string())?;
    reopen(&db, &player)?;
    output_state(db)
}

/// Same sound on both outputs, or stereo split; takes effect at once.
#[tauri::command]
pub fn set_dual_mode(mode: DualMode, db: State<'_, SharedDb>, player: State<'_, SharedPlayer>) -> Result<OutputState, String> {
    crate::settings::set_dual_mode(&*db.lock().map_err(|e| e.to_string())?, mode).map_err(|e| e.to_string())?;
    let _ = player.act(|p| p.set_dual_mode(mode));
    output_state(db)
}

/// Delays (ms) of the first and second output, remembered for this pair of devices.
#[tauri::command]
pub fn set_dual_delays(delays: [u32; 2], db: State<'_, SharedDb>, player: State<'_, SharedPlayer>) -> Result<(), String> {
    {
        let d = db.lock().map_err(|e| e.to_string())?;
        let second = crate::settings::second_output(&d).ok_or("choose a second output first")?;
        let first = first_name(crate::settings::output_device(&d));
        crate::settings::set_dual_delays(&d, &first, &second, delays).map_err(|e| e.to_string())?;
    }
    player.act(|p| p.set_delays(delays))
}

/// Balance (dB) between the outputs, remembered for this pair of devices; takes effect at once.
#[tauri::command]
pub fn set_dual_balance(balance: f32, db: State<'_, SharedDb>, player: State<'_, SharedPlayer>) -> Result<(), String> {
    {
        let d = db.lock().map_err(|e| e.to_string())?;
        let second = crate::settings::second_output(&d).ok_or("choose a second output first")?;
        let first = first_name(crate::settings::output_device(&d));
        crate::settings::set_dual_balance(&d, &first, &second, balance).map_err(|e| e.to_string())?;
    }
    player.act(|p| p.set_balance(balance))
}

/// Alignment clicks on (the music waits) or off (the music carries on where it was).
#[tauri::command]
pub fn output_align(on: bool, player: State<'_, SharedPlayer>) -> Result<(), String> {
    player.act(|p| p.align(on))
}

/// What an auto-calibration measured, and what it set.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CalibrationReport {
    /// How far the second output sounded after the first, and how much louder, before.
    offset_ms: f64,
    louder_db: f64,
    /// The delays and balance now in force.
    delays: [u32; 2],
    balance: f32,
}

/// Auto-calibrate the two outputs with the microphone `mic` at the listening spot: play the
/// calibration sound (the music waits), record it, then set and save delay and balance so both
/// outputs arrive there together and evenly. Errors are i18n keys.
#[tauri::command]
pub async fn calibrate_outputs(mic: String, db: State<'_, SharedDb>, player: State<'_, SharedPlayer>) -> Result<CalibrationReport, String> {
    let (db, player) = (db.inner().clone(), player.inner().clone());
    tauri::async_runtime::spawn_blocking(move || calibrate(&mic, &db, &player)).await.map_err(|e| e.to_string())?
}

fn calibrate(mic: &str, db: &SharedDb, player: &SharedPlayer) -> Result<CalibrationReport, String> {
    use crate::audio::calibrate::{analyze, corrected, duration_s, Unheard};
    let (second, first) = {
        let d = db.lock().map_err(|e| e.to_string())?;
        (second_config(&d).ok_or("output.calibrate.noSecond")?, first_name(crate::settings::output_device(&d)))
    };
    // Start listening first, then play the sound, then go back to the music.
    let name = mic.to_string();
    let recorder = std::thread::spawn(move || crate::audio::listen::record(&name, duration_s() + 1.5));
    std::thread::sleep(Duration::from_millis(300));
    player.act(Player::calibrate)?;
    let recorded = recorder.join().map_err(|_| "microphone stopped".to_string())?;
    player.act(|p| p.align(false))?;
    let (samples, rate) = recorded.map_err(|e| {
        log::warn!("calibration: {e}");
        "output.calibrate.noMic".to_string()
    })?;
    let m = analyze(&samples, rate).map_err(|u| {
        match u {
            Unheard::Clicks => "output.calibrate.unheard",
            Unheard::Bursts => "output.calibrate.noLevel",
        }
        .to_string()
    })?;
    let (delays, balance) = corrected(second.delays_ms, second.balance_db, m);
    log::info!("calibration: second output {:+.1} ms, {:+.1} dB; now delays {delays:?}, balance {balance:+.1} dB", m.offset_s * 1000.0, m.louder_db);
    {
        let d = db.lock().map_err(|e| e.to_string())?;
        crate::settings::set_dual_delays(&d, &first, &second.device, delays).map_err(|e| e.to_string())?;
        crate::settings::set_dual_balance(&d, &first, &second.device, balance).map_err(|e| e.to_string())?;
    }
    player.act(|p| {
        p.set_delays(delays);
        p.set_balance(balance);
    })?;
    Ok(CalibrationReport { offset_ms: (m.offset_s * 10_000.0).round() / 10.0, louder_db: (m.louder_db * 10.0).round() / 10.0, delays, balance })
}

// Machine test: needs a real output device (quiet test tone).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::fixtures::wav;

    #[test]
    #[ignore]
    fn switching_output_keeps_the_song_and_position() {
        let d = tempfile::tempdir().unwrap();
        let p = wav(d.path(), "t.wav", 44_100, 1, 16, 44_100 * 4);
        let h = PlayerHandle::new(None, false);
        *h.paths.lock().unwrap() = vec![p.clone()];
        *h.queue.lock().unwrap() = vec![7];
        h.act(|pl| {
            pl.set_volume(0.05);
            pl.play(vec![p], 0);
        })
        .unwrap();
        std::thread::sleep(Duration::from_millis(1_200));
        let before = h.event();
        h.switch_output(None, false, None, crate::audio::eq::EqSettings::default()).unwrap();
        std::thread::sleep(Duration::from_millis(400));
        let after = h.event();
        println!("before {before:?}\nafter {after:?}");
        assert_eq!(after.song_id, Some(7));
        assert!(after.playing);
        assert!(after.position_ms >= before.position_ms, "position kept: {} -> {}", before.position_ms, after.position_ms);
        assert!((after.volume - 0.05).abs() < 1e-6, "volume kept");
    }
}

/// Shuffle on or off (saved); the current song keeps playing.
#[tauri::command]
pub fn player_shuffle(on: bool, db: State<'_, SharedDb>, player: State<'_, SharedPlayer>) -> Result<(), String> {
    crate::settings::set_shuffle(&*db.lock().map_err(|e| e.to_string())?, on).map_err(|e| e.to_string())?;
    player.act(|p| p.set_shuffle(on))
}

/// Repeat off, all or one (saved).
#[tauri::command]
pub fn player_repeat(repeat: Repeat, db: State<'_, SharedDb>, player: State<'_, SharedPlayer>) -> Result<(), String> {
    crate::settings::set_repeat(&*db.lock().map_err(|e| e.to_string())?, repeat).map_err(|e| e.to_string())?;
    player.act(|p| p.set_repeat(repeat))
}
