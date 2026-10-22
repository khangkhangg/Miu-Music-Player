//! Windows side of `media_controls`: Windows' System Media Transport Controls for the main window.
//! Uses the `windows` crate directly (already a dependency; `souvlaki` would add a crate for the
//! same few calls). Miu is an unpackaged Win32 app, so the controls come from
//! `ISystemMediaTransportControlsInterop::GetForWindow` on the main window, not `GetForCurrentView`.
//! The controls are agile: `show` runs on the player thread, button presses arrive on a thread pool.

use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use ::windows::core::{factory, Ref, HSTRING};
use ::windows::Foundation::{TimeSpan, TypedEventHandler, Uri};
use ::windows::Media::{
    MediaPlaybackStatus, MediaPlaybackType, PlaybackPositionChangeRequestedEventArgs, SystemMediaTransportControls, SystemMediaTransportControlsButton,
    SystemMediaTransportControlsButtonPressedEventArgs, SystemMediaTransportControlsTimelineProperties,
};
use ::windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream, RandomAccessStreamReference};
use ::windows::Win32::Foundation::HWND;
use ::windows::Win32::System::WinRT::ISystemMediaTransportControlsInterop;
use tauri::{AppHandle, Manager};

use crate::audio::service::{PlayerEvent, SharedPlayer};
use crate::library::service::SharedDb;
use crate::media_controls::{command, describe, status, track, Button, Command, Status, TimelineClock, Track};

/// Cover size handed to Windows (the lock screen shows it large).
const COVER_SIZE: u32 = 300;

struct State {
    smtc: SystemMediaTransportControls,
    status: Option<Status>,
    shown: Option<Track>,
    duration_ms: Option<u64>,
    clock: TimelineClock,
    started: Instant,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

/// Attach the controls to the main window; presses go straight to `player`. Call once at setup.
pub fn start(app: &AppHandle, player: SharedPlayer) {
    let Some(hwnd) = app.get_webview_window("main").and_then(|w| w.hwnd().ok()) else {
        log::warn!("media controls: no main window");
        return;
    };
    let hwnd = hwnd.0 as isize;
    let made = app.run_on_main_thread(move || match attach(HWND(hwnd as *mut _), player) {
        Ok(smtc) => {
            let _ = STATE.set(Mutex::new(State { smtc, status: None, shown: None, duration_ms: None, clock: TimelineClock::default(), started: Instant::now() }));
            log::info!("media controls ready");
        }
        Err(e) => log::warn!("media controls: {e}"),
    });
    if let Err(e) = made {
        log::warn!("media controls: {e}");
    }
}

fn attach(hwnd: HWND, player: SharedPlayer) -> ::windows::core::Result<SystemMediaTransportControls> {
    let interop = factory::<SystemMediaTransportControls, ISystemMediaTransportControlsInterop>()?;
    let smtc: SystemMediaTransportControls = unsafe { interop.GetForWindow(hwnd)? };
    smtc.SetIsPlayEnabled(true)?;
    smtc.SetIsPauseEnabled(true)?;
    smtc.SetIsStopEnabled(true)?;
    smtc.SetIsNextEnabled(true)?;
    smtc.SetIsPreviousEnabled(true)?;
    smtc.SetIsEnabled(false)?;
    let p = player.clone();
    smtc.ButtonPressed(&TypedEventHandler::new(move |_, args: Ref<SystemMediaTransportControlsButtonPressedEventArgs>| {
        let button = match args.ok()?.Button()? {
            SystemMediaTransportControlsButton::Play => Button::Play,
            SystemMediaTransportControlsButton::Pause => Button::Pause,
            SystemMediaTransportControlsButton::Stop => Button::Stop,
            SystemMediaTransportControlsButton::Next => Button::Next,
            SystemMediaTransportControlsButton::Previous => Button::Previous,
            _ => return Ok(()),
        };
        if let Err(e) = p.run(command(button)) {
            log::warn!("media controls: {button:?}: {e}");
        }
        Ok(())
    }))?;
    smtc.PlaybackPositionChangeRequested(&TypedEventHandler::new(move |_, args: Ref<PlaybackPositionChangeRequestedEventArgs>| {
        let ms = (args.ok()?.RequestedPlaybackPosition()?.Duration.max(0) / 10_000) as u64;
        if let Err(e) = player.run(Command::Seek(ms)) {
            log::warn!("media controls: seek: {e}");
        }
        Ok(())
    }))?;
    Ok(smtc)
}

/// Bring the controls up to date with the player (called on every `player` event).
pub fn show(ev: &PlayerEvent, db: &SharedDb) {
    let Some(state) = STATE.get() else { return };
    let Ok(mut st) = state.lock() else { return };
    if let Err(e) = update(&mut st, ev, db) {
        log::warn!("media controls: {e}");
    }
}

fn update(st: &mut State, ev: &PlayerEvent, db: &SharedDb) -> ::windows::core::Result<()> {
    let now = track(ev.song_id, ev.episode_id);
    let s = status(ev.available, ev.playing, now);
    if st.shown != now {
        st.shown = now;
        st.duration_ms = None;
        if let Some(t) = now {
            st.duration_ms = show_track(&st.smtc, t, db)?;
        }
    }
    if st.status != Some(s) {
        st.status = Some(s);
        st.smtc.SetIsEnabled(!matches!(s, Status::Closed | Status::Stopped))?;
        st.smtc.SetPlaybackStatus(match s {
            Status::Closed => MediaPlaybackStatus::Closed,
            Status::Stopped => MediaPlaybackStatus::Stopped,
            Status::Playing => MediaPlaybackStatus::Playing,
            Status::Paused => MediaPlaybackStatus::Paused,
        })?;
    }
    let at = st.started.elapsed().as_millis() as u64;
    if let (Some(end), true) = (st.duration_ms, st.clock.due(now, ev.playing, ev.position_ms, at)) {
        let span = |ms: u64| TimeSpan { Duration: ms as i64 * 10_000 };
        let t = SystemMediaTransportControlsTimelineProperties::new()?;
        t.SetStartTime(span(0))?;
        t.SetMinSeekTime(span(0))?;
        t.SetEndTime(span(end))?;
        t.SetMaxSeekTime(span(end))?;
        t.SetPosition(span(ev.position_ms.min(end)))?;
        st.smtc.UpdateTimelineProperties(&t)?;
    }
    Ok(())
}

/// Title, artist, album and cover of `track`; returns its length if known.
fn show_track(smtc: &SystemMediaTransportControls, track: Track, db: &SharedDb) -> ::windows::core::Result<Option<u64>> {
    let info = db.lock().ok().and_then(|d| describe(&d, track));
    let u = smtc.DisplayUpdater()?;
    u.ClearAll()?;
    u.SetType(MediaPlaybackType::Music)?;
    let Some(info) = info else {
        u.Update()?;
        return Ok(None);
    };
    let m = u.MusicProperties()?;
    m.SetTitle(&HSTRING::from(&info.display.title))?;
    m.SetArtist(&HSTRING::from(&info.display.artist))?;
    m.SetAlbumTitle(&HSTRING::from(&info.display.album))?;
    let cover = match track {
        Track::Song(id) => crate::library::service::cover_jpeg(db, id, COVER_SIZE).map(|jpg| jpeg_stream(&jpg)).transpose()?,
        Track::Episode(_) => info.artwork_url.filter(|u| u.starts_with("https://")).map(|url| RandomAccessStreamReference::CreateFromUri(&Uri::CreateUri(&HSTRING::from(url))?)).transpose()?,
    };
    if let Some(cover) = cover {
        u.SetThumbnail(&cover)?;
    }
    u.Update()?;
    Ok(info.duration_ms.filter(|&d| d > 0))
}

fn jpeg_stream(jpg: &[u8]) -> ::windows::core::Result<RandomAccessStreamReference> {
    let stream = InMemoryRandomAccessStream::new()?;
    let w = DataWriter::CreateDataWriter(&stream)?;
    w.WriteBytes(jpg)?;
    w.StoreAsync()?.join()?;
    w.DetachStream()?;
    stream.Seek(0)?;
    RandomAccessStreamReference::CreateFromStream(&stream)
}
