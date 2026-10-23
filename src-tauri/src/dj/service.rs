//! DJ mode for the app: the background analyser (D.1), which works through the library one song
//! at a time at background priority so DJ mode knows every song's tempo, key and energy (progress
//! goes out as `dj-analysis-progress` { done, total }), and the commands that run the DJ engine (D.2).

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::{AppHandle, Emitter, State};

use super::actions::{Action, Deck as Side};
use super::analysis::{self, Analysis};
use super::autodj::{self, EnergyCurve};
use super::engine::{DjEngine, EngineStatus};
use super::store;
use crate::audio::service::SharedPlayer;
use crate::library::service::SharedDb;
use crate::settings;

static RUNNING: AtomicBool = AtomicBool::new(false);

/// Analyses the next waiting song with `analyse`. Returns false when nothing is left.
/// A file that can't be decoded is recorded as failed so it isn't tried again until it changes.
pub fn step(db: &SharedDb, analyse: impl Fn(&str) -> Result<Analysis, String>, now_unix: i64) -> bool {
    let next = match db.lock() {
        Ok(d) => store::pending(&d, 1).unwrap_or_default().pop(),
        Err(_) => return false,
    };
    let Some(song) = next else { return false };
    let result = analyse(&song.path); // without the database lock: this takes a while
    let Ok(d) = db.lock() else { return false };
    match result {
        Ok(a) => store::save(&d, song.song_id, song.size, song.mtime, &a, now_unix).is_ok(),
        Err(e) => {
            log::warn!("DJ analysis of {} failed: {e}", song.path);
            store::save_failed(&d, song.song_id, song.size, song.mtime, now_unix).is_ok()
        }
    }
}

fn analyse_file(path: &str) -> Result<Analysis, String> {
    let (mono, rate) = analysis::decode_mono(std::path::Path::new(path))?;
    Ok(analysis::analyze(&mono, rate))
}

#[cfg(windows)]
fn background_priority() {
    use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN};
    // SAFETY: the pseudo handle of the calling thread is always valid.
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) };
}

#[cfg(not(windows))]
fn background_priority() {}

fn emit_progress(app: &AppHandle, db: &SharedDb) {
    if let Some((done, total)) = db.lock().ok().and_then(|d| store::progress(&d).ok()) {
        let _ = app.emit("dj-analysis-progress", serde_json::json!({ "done": done, "total": total }));
    }
}

/// Starts a pass over the library unless one is running (that one picks up new songs too).
pub fn analyse_library(app: &AppHandle, db: &SharedDb) {
    if RUNNING.swap(true, Ordering::AcqRel) {
        return;
    }
    let (app, db) = (app.clone(), db.clone());
    std::thread::spawn(move || {
        background_priority();
        let mut n = 0;
        loop {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
            if !step(&db, analyse_file, now) {
                break;
            }
            n += 1;
            if n % 5 == 0 {
                emit_progress(&app, &db);
            }
        }
        RUNNING.store(false, Ordering::Release);
        emit_progress(&app, &db);
        if n > 0 {
            log::info!("DJ analysis: {n} songs analysed");
        }
    });
}

/// Starts the first pass a minute after launch, when the library scan has settled, so DJ mode is
/// ready by the time someone opens it.
pub fn start(app: AppHandle, db: SharedDb) {
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(60));
        analyse_library(&app, &db);
    });
}

#[derive(serde::Serialize)]
pub struct Progress {
    pub done: u32,
    pub total: u32,
}

/// How far the analysis is; opening DJ mode calls this, which also starts a pass.
#[tauri::command]
pub fn dj_analysis_progress(app: AppHandle, db: State<'_, SharedDb>) -> Result<Progress, String> {
    let (done, total) = store::progress(&*db.lock().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if done < total {
        analyse_library(&app, &db);
    }
    Ok(Progress { done, total })
}

#[tauri::command]
pub fn dj_song_analysis(song_id: i64, db: State<'_, SharedDb>) -> Result<Option<Analysis>, String> {
    store::get(&*db.lock().map_err(|e| e.to_string())?, song_id).map_err(|e| e.to_string())
}


/// Auto DJ's choices for this session: the energy curve, songs queued by hand, what has played.
#[derive(Default)]
pub struct AutoPrefs {
    pub curve: EnergyCurve,
    pub queue: Vec<i64>,
    pub played: HashSet<i64>,
    pub seed: u64,
}

/// DJ mode while it is open: the engine and Auto DJ's choices.
pub struct DjSession {
    pub engine: DjEngine,
    pub auto: AutoPrefs,
    pub midi: Arc<super::controllers::MidiShared>,
    /// Cleared when DJ mode closes, which stops the Auto DJ controller and the DJ controllers.
    alive: Arc<AtomicBool>,
}

impl Drop for DjSession {
    fn drop(&mut self) {
        self.alive.store(false, Ordering::Release);
    }
}

pub type SharedDj = Arc<Mutex<Option<DjSession>>>;

fn engine_do<T>(dj: &SharedDj, f: impl FnOnce(&DjEngine) -> T) -> Result<T, String> {
    let g = dj.lock().map_err(|e| e.to_string())?;
    g.as_ref().map(|s| f(&s.engine)).ok_or_else(|| "DJ mode is not open".to_string())
}

/// With Auto DJ on: the deck that needs a song and the song on air (if any) to choose after.
/// A first song goes on deck A; after that the free deck beside the one on air.
pub fn needs_song(s: &EngineStatus) -> Option<(Side, Option<i64>)> {
    if !s.auto_dj || s.decks.iter().any(|d| d.loading) {
        return None;
    }
    let free = |i: usize| s.decks[i].song_id.is_none();
    match s.on_air {
        None if free(0) && free(1) => Some((Side::A, None)),
        None => None,
        Some(live) => {
            let other = 1 - live as usize;
            free(other).then(|| (if other == 0 { Side::A } else { Side::B }, s.decks[live as usize].song_id))
        }
    }
}

/// What Auto DJ does about the party queue's top entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartyStep {
    /// Put this library song on this (free) deck; it is mixed in like any other.
    Load(Side, i64),
    /// The song on air is (nearly) over: hand over to this video on the party screen.
    Video(String, String),
    /// Nothing queued: Auto DJ picks the next song itself.
    Pick,
    Wait,
}

/// A queued video takes over when the song on air has this little left (or nothing plays).
const VIDEO_LEAD_S: f64 = 8.0;

pub fn party_step(s: &EngineStatus, top: Option<&crate::party::queue::Item>, video_playing: bool) -> PartyStep {
    use crate::party::queue::Item;
    if video_playing || !s.auto_dj {
        return PartyStep::Wait;
    }
    match top {
        None => PartyStep::Pick,
        Some(Item::Video { video_id, title }) => {
            let on_air = s.on_air.map(|d| &s.decks[d as usize]).or_else(|| s.decks.iter().find(|d| d.playing));
            let over = on_air.is_none_or(|d| !d.playing || d.ended || d.duration_s - d.position_s <= VIDEO_LEAD_S);
            if over { PartyStep::Video(video_id.clone(), title.clone()) } else { PartyStep::Wait }
        }
        Some(Item::Song { song_id, .. }) => {
            if s.decks.iter().any(|d| d.song_id == Some(*song_id)) {
                return PartyStep::Wait;
            }
            match needs_song(s) {
                Some((deck, _)) => PartyStep::Load(deck, *song_id),
                None => PartyStep::Wait,
            }
        }
    }
}

fn song_path(db: &SharedDb, song_id: i64) -> Option<std::path::PathBuf> {
    let d = db.lock().ok()?;
    crate::library::songs::paths_for(&d, &[song_id]).ok()?.pop().map(|(_, p)| p)
}

/// Keeps Auto DJ supplied: whenever a deck is free, picks the next song and loads it there.
fn auto_controller(app: tauri::AppHandle, dj: SharedDj, db: SharedDb, party: crate::party::service::SharedParty, alive: Arc<AtomicBool>) {
    use crate::party::queue::Item;
    use tauri::Emitter;
    std::thread::spawn(move || {
        let mut last_on_air: Option<i64> = None;
        while alive.load(Ordering::Acquire) {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let (status, curve, queue, mut played, seed) = {
                let Ok(g) = dj.lock() else { return };
                let Some(s) = g.as_ref() else { return };
                (s.engine.status(), s.auto.curve, s.auto.queue.clone(), s.auto.played.clone(), s.auto.seed)
            };
            // The party queue: what went on air leaves it; its top entry comes before Auto DJ's own pick.
            let on_air_song = status.on_air.and_then(|d| status.decks[d as usize].song_id);
            if on_air_song != last_on_air {
                last_on_air = on_air_song;
                if let Some(id) = on_air_song {
                    // With its title, so guests see what plays even when Auto DJ picked it.
                    let item = party.party.library_song(id).or_else(|| {
                        let rows = db.lock().ok().and_then(|d| crate::library::songs::list_songs(&d).ok())?;
                        let r = rows.into_iter().find(|r| r.id == id)?;
                        Some(Item::Song { song_id: id, title: r.title, artist: r.artist.unwrap_or_default() })
                    });
                    let item = item.unwrap_or(Item::Song { song_id: id, title: String::new(), artist: String::new() });
                    party.party.change(|q| q.start_item(&item));
                }
            }
            let top = party.party.queue().peek().map(|e| e.item.clone());
            match party_step(&status, top.as_ref(), party.video().is_some()) {
                PartyStep::Load(deck, id) => {
                    let analysis = db.lock().ok().and_then(|d| store::get(&d, id).ok().flatten());
                    if let (Some(path), Ok(g)) = (song_path(&db, id), dj.lock()) {
                        if let Some(s) = g.as_ref() {
                            s.engine.load(deck, path, id, analysis);
                        }
                    }
                    continue;
                }
                PartyStep::Video(video_id, title) => {
                    let item = Item::Video { video_id: video_id.clone(), title: title.clone() };
                    party.party.change(|q| q.start_item(&item));
                    party.set_video(Some((video_id.clone(), title.clone())));
                    if let Ok(g) = dj.lock() {
                        if let Some(s) = g.as_ref() {
                            s.engine.set_hold(true);
                        }
                    }
                    let _ = app.emit("party-video", serde_json::json!({ "videoId": video_id, "title": title }));
                    continue;
                }
                PartyStep::Wait if top.is_some() || party.video().is_some() => continue,
                PartyStep::Wait | PartyStep::Pick => {}
            }
            // Whatever has been on a deck counts as played, by hand or by Auto DJ.
            played.extend(status.decks.iter().filter_map(|d| d.song_id));
            let pick = needs_song(&status).and_then(|(deck, current)| {
                let candidates = db.lock().ok().and_then(|d| store::candidates(&d).ok())?;
                let now = current
                    .and_then(|id| candidates.iter().find(|c| c.song_id == id).map(|c| c.analysis.clone()))
                    .unwrap_or(Analysis { bpm: 0.0, beat_confidence: 0.0, first_beat_s: 0.0, downbeat_s: 0.0, key: None, energy: 5, mix_in_s: 0.0, mix_out_s: 0.0, duration_s: 0.0, ..Default::default() });
                let next = autodj::pick(&now, &candidates, &played, &queue, curve, seed)?;
                let analysis = candidates.iter().find(|c| c.song_id == next).map(|c| c.analysis.clone());
                Some((deck, next, song_path(&db, next)?, analysis))
            });
            let Ok(mut g) = dj.lock() else { return };
            let Some(s) = g.as_mut() else { return };
            s.auto.played.extend(played);
            if let Some((deck, next, path, analysis)) = pick {
                s.engine.load(deck, path, next, analysis);
                s.auto.played.insert(next);
                s.auto.queue.retain(|id| *id != next);
            }
        }
    });
}

#[tauri::command]
pub fn dj_overview(deck: Side, dj: State<'_, SharedDj>) -> Result<Option<super::engine::Overview>, String> {
    engine_do(&dj, |e| e.overview(deck))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DjSong {
    pub song_id: i64,
    #[serde(flatten)]
    pub analysis: Analysis,
}

/// Tempo, key and energy of every analysed song, for the DJ song list.
#[tauri::command]
pub fn dj_library(db: State<'_, SharedDb>) -> Result<Vec<DjSong>, String> {
    let c = store::candidates(&*db.lock().map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(c.into_iter().map(|c| DjSong { song_id: c.song_id, analysis: c.analysis }).collect())
}

/// The kinds of transition Auto DJ may use (none checked: any), kept for next time.
#[tauri::command]
pub fn dj_set_transitions(transitions: settings::DjTransitions, db: State<'_, SharedDb>, dj: State<'_, SharedDj>) -> Result<(), String> {
    settings::set_dj_transitions(&*db.lock().map_err(|e| e.to_string())?, &transitions).map_err(|e| e.to_string())?;
    engine_do(&dj, |e| e.set_transitions(transitions)).or(Ok(()))
}

/// Pick the next transition by hand (`None`: Auto DJ picks the best fit).
#[tauri::command]
pub fn dj_choose_transition(transition: Option<super::autodj::Transition>, dj: State<'_, SharedDj>) -> Result<(), String> {
    engine_do(&dj, |e| e.choose_transition(transition))
}

/// How Auto DJ mixes: smooth, party or chill (kept for next time).
#[tauri::command]
pub fn dj_set_style(style: super::autodj::Style, db: State<'_, SharedDb>, dj: State<'_, SharedDj>) -> Result<(), String> {
    settings::set_dj_style(&*db.lock().map_err(|e| e.to_string())?, style).map_err(|e| e.to_string())?;
    engine_do(&dj, |e| e.set_style(style)).or(Ok(()))
}

/// MIDI devices seen while DJ mode is open, with the mapping each uses.
#[tauri::command]
pub fn dj_controllers(dj: State<'_, SharedDj>) -> Result<Vec<super::controllers::Device>, String> {
    let g = dj.lock().map_err(|e| e.to_string())?;
    Ok(g.as_ref().and_then(|s| s.midi.devices.lock().ok().map(|d| d.clone())).unwrap_or_default())
}

/// MIDI learn: the next control moved on any device is taught as `control` (for `deck`);
/// `None` stops learning.
#[tauri::command]
pub fn dj_midi_learn(control: Option<super::midi::Control>, deck: Option<Side>, dj: State<'_, SharedDj>) -> Result<(), String> {
    let g = dj.lock().map_err(|e| e.to_string())?;
    let s = g.as_ref().ok_or("DJ mode is not open")?;
    *s.midi.learn.lock().map_err(|e| e.to_string())? = control.map(|c| (c, deck));
    Ok(())
}

/// Forgets everything learnt for a device (it goes back to its built-in mapping, if any).
#[tauri::command]
pub fn dj_midi_forget(device: String, db: State<'_, SharedDb>, dj: State<'_, SharedDj>) -> Result<(), String> {
    settings::clear_dj_learnt(&*db.lock().map_err(|e| e.to_string())?, &device).map_err(|e| e.to_string())?;
    if let Some(s) = dj.lock().map_err(|e| e.to_string())?.as_ref() {
        s.midi.reload.store(true, Ordering::Release);
    }
    Ok(())
}

/// Auto DJ's energy curve and the songs queued by hand (played first, in order).
#[tauri::command]
pub fn dj_set_auto(curve: EnergyCurve, queue: Vec<i64>, dj: State<'_, SharedDj>) -> Result<(), String> {
    let mut g = dj.lock().map_err(|e| e.to_string())?;
    let s = g.as_mut().ok_or("DJ mode is not open")?;
    s.auto.curve = curve;
    s.auto.queue = queue;
    Ok(())
}

/// Opens DJ mode: the normal player pauses and the DJ engine takes the speakers (the player's
/// output) and the chosen headphones.
#[tauri::command]
pub fn dj_enter(app: tauri::AppHandle, db: State<'_, SharedDb>, player: State<'_, SharedPlayer>, dj: State<'_, SharedDj>, party: State<'_, crate::party::service::SharedParty>) -> Result<EngineStatus, String> {
    let mut g = dj.lock().map_err(|e| e.to_string())?;
    if let Some(s) = g.as_ref() {
        return Ok(s.engine.status());
    }
    let _ = player.run(crate::media_controls::Command::Pause);
    let (master, phones, exclusive, style, transitions) = {
        let d = db.lock().map_err(|e| e.to_string())?;
        (settings::output_device(&d), settings::dj_phones(&d), settings::exclusive_output(&d), settings::dj_style(&d), settings::dj_transitions(&d))
    };
    let engine = DjEngine::start(master, phones).map_err(|e| {
        if exclusive { format!("{e} (exclusive mode holds the speakers: turn it off for DJ mode)") } else { e }
    })?;
    engine.set_style(style);
    engine.set_transitions(transitions);
    let status = engine.status();
    let alive = Arc::new(AtomicBool::new(true));
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(0);
    let midi = Arc::new(super::controllers::MidiShared::default());
    *g = Some(DjSession { engine, auto: AutoPrefs { seed, ..Default::default() }, midi: midi.clone(), alive: alive.clone() });
    auto_controller(app.clone(), dj.inner().clone(), db.inner().clone(), party.inner().clone(), alive.clone());
    let (to_engine, from_engine) = (dj.inner().clone(), dj.inner().clone());
    super::controllers::start(
        app,
        db.inner().clone(),
        midi,
        alive,
        move |a| {
            if let Some(s) = to_engine.lock().ok().as_ref().and_then(|g| g.as_ref()) {
                s.engine.apply(a);
            }
        },
        move || from_engine.lock().ok().and_then(|g| g.as_ref().map(|s| s.engine.status())),
    );
    Ok(status)
}

/// Closes DJ mode and gives the speakers back to the normal player.
#[tauri::command]
pub fn dj_leave(dj: State<'_, SharedDj>) -> Result<(), String> {
    let old = dj.lock().map_err(|e| e.to_string())?.take();
    drop(old); // waits for the engine to close its devices
    Ok(())
}

#[tauri::command]
pub fn dj_action(action: Action, dj: State<'_, SharedDj>) -> Result<(), String> {
    engine_do(&dj, |e| e.apply(action))
}

/// Puts a library song on a deck; it plays from its first beat when started.
#[tauri::command]
pub fn dj_load(deck: Side, song_id: i64, db: State<'_, SharedDb>, dj: State<'_, SharedDj>) -> Result<(), String> {
    let (path, analysis) = {
        let d = db.lock().map_err(|e| e.to_string())?;
        let path = crate::library::songs::paths_for(&d, &[song_id]).map_err(|e| e.to_string())?.pop().map(|(_, p)| p);
        (path.ok_or(format!("song {song_id} is not in the library"))?, store::get(&d, song_id).map_err(|e| e.to_string())?)
    };
    engine_do(&dj, |e| e.load(deck, path, song_id, analysis))
}

/// Decks, meters and devices for the DJ screen (`None` when DJ mode is closed).
#[tauri::command]
pub fn dj_state(dj: State<'_, SharedDj>) -> Result<Option<EngineStatus>, String> {
    Ok(dj.lock().map_err(|e| e.to_string())?.as_ref().map(|s| s.engine.status()))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DjOutputs {
    pub devices: Vec<crate::audio::player::OutputDevice>,
    pub phones: Option<String>,
}

#[tauri::command]
pub fn dj_outputs(db: State<'_, SharedDb>) -> Result<DjOutputs, String> {
    let phones = settings::dj_phones(&*db.lock().map_err(|e| e.to_string())?);
    Ok(DjOutputs { devices: crate::audio::player::output_devices(), phones })
}

/// Chooses the headphones (`None`: no headphone cue); switches at once when DJ mode is open.
#[tauri::command]
pub fn dj_set_phones(name: Option<String>, db: State<'_, SharedDb>, dj: State<'_, SharedDj>) -> Result<(), String> {
    settings::set_dj_phones(&*db.lock().map_err(|e| e.to_string())?, name.as_deref()).map_err(|e| e.to_string())?;
    if let Some(s) = dj.lock().map_err(|e| e.to_string())?.as_ref() {
        s.engine.set_phones(name);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dj::engine::DeckStatus;
    use crate::library::db::Db;

    fn deck(song_id: Option<i64>, loading: bool) -> DeckStatus {
        DeckStatus { song_id, loading, ..Default::default() }
    }

    fn status(a: DeckStatus, b: DeckStatus, on_air: Option<Side>) -> EngineStatus {
        EngineStatus { decks: [a, b], auto_dj: true, on_air, ..Default::default() }
    }

    #[test]
    fn the_party_queue_comes_first() {
        use crate::party::queue::Item;
        let song = |id: i64| Item::Song { song_id: id, title: String::new(), artist: String::new() };
        let video = Item::Video { video_id: "abc".into(), title: "Sing".into() };
        let mut playing = deck(Some(4), false);
        playing.playing = true;
        playing.duration_s = 200.0;
        playing.position_s = 100.0;
        let s = status(playing.clone(), deck(None, false), Some(Side::A));
        assert_eq!(party_step(&s, Some(&song(9)), false), PartyStep::Load(Side::B, 9), "the top of the queue on the free deck");
        assert_eq!(party_step(&s, None, false), PartyStep::Pick, "nothing queued: Auto DJ picks");
        let loaded = status(playing.clone(), deck(Some(9), false), Some(Side::A));
        assert_eq!(party_step(&loaded, Some(&song(9)), false), PartyStep::Wait, "already on a deck");
        assert_eq!(party_step(&s, Some(&video), false), PartyStep::Wait, "a video waits for the song to end");
        let mut ending = playing.clone();
        ending.position_s = 195.0;
        let s2 = status(ending, deck(None, false), Some(Side::A));
        assert_eq!(party_step(&s2, Some(&video), false), PartyStep::Video("abc".into(), "Sing".into()), "near the end: over to the video");
        let quiet = status(deck(None, false), deck(None, false), None);
        assert_eq!(party_step(&quiet, Some(&video), false), PartyStep::Video("abc".into(), "Sing".into()), "nothing playing: at once");
        assert_eq!(party_step(&quiet, Some(&song(9)), true), PartyStep::Wait, "a video is playing");
    }

    #[test]
    fn auto_dj_keeps_the_free_deck_loaded() {
        let empty = || deck(None, false);
        assert_eq!(needs_song(&status(empty(), empty(), None)), Some((Side::A, None)), "a first song");
        assert_eq!(needs_song(&status(deck(Some(4), false), empty(), Some(Side::A))), Some((Side::B, Some(4))));
        assert_eq!(needs_song(&status(empty(), deck(Some(5), false), Some(Side::B))), Some((Side::A, Some(5))));
        assert_eq!(needs_song(&status(deck(Some(4), false), deck(None, true), Some(Side::A))), None, "already loading");
        assert_eq!(needs_song(&status(deck(Some(4), false), deck(Some(6), false), Some(Side::A))), None, "both full");
        assert_eq!(needs_song(&status(deck(Some(4), false), empty(), None)), None, "waiting for the first to start");
        let mut off = status(empty(), empty(), None);
        off.auto_dj = false;
        assert_eq!(needs_song(&off), None);
    }

    fn library(paths: &[&str]) -> SharedDb {
        let db = Db::open_in_memory().unwrap();
        db.conn().execute("INSERT INTO watched_folders (id, path, added_at) VALUES (1, 'C:/M', 1)", []).unwrap();
        for p in paths {
            db.conn()
                .execute("INSERT INTO songs (folder_id, path, size, mtime, added_at) VALUES (1, ?1, 1, 1, 1)", [p])
                .unwrap();
        }
        Arc::new(Mutex::new(db))
    }

    fn fake(path: &str) -> Result<Analysis, String> {
        if path.ends_with(".bad") {
            return Err("cannot decode".into());
        }
        Ok(analysis::analyze(&vec![0.0; 11025], 11025))
    }

    #[test]
    fn works_through_every_song_once_and_records_failures() {
        let db = library(&["C:/M/a.mp3", "C:/M/b.bad", "C:/M/c.flac"]);
        let mut steps = 0;
        while step(&db, fake, 7) {
            steps += 1;
            assert!(steps <= 3, "each song is analysed once");
        }
        assert_eq!(steps, 3);
        let d = db.lock().unwrap();
        assert_eq!(store::progress(&d).unwrap(), (3, 3));
        assert!(store::get(&d, 1).unwrap().is_some());
        assert_eq!(store::get(&d, 2).unwrap(), None, "a failed file has no analysis");
    }
}
