//! The player (F7.1): an engine thread that owns the output device — WASAPI shared mode via cpal,
//! or exclusive (bit-perfect) mode at each file's own rate — decodes ahead into a lock-free ring
//! buffer, and reports what the listener actually hears.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::Serialize;

use crate::audio::decode::{AudioError, SourceFormat};
use crate::audio::eq::{EqSettings, Equalizer};
use crate::audio::order::{PlayOrder, Repeat};
use crate::audio::pipeline::Feeder;
use crate::audio::timeline::Timeline;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub playing: bool,
    /// Index into the queue given to `play`.
    pub track: Option<usize>,
    pub position_ms: u64,
    pub volume: f32,
    /// Format of the file being heard.
    pub source: Option<SourceFormat>,
    pub device_name: String,
    pub device_rate: u32,
    /// True while the device is held in exclusive mode.
    pub exclusive: bool,
    /// Valid bits per sample sent to the device in exclusive mode (0 in shared mode: 32-bit float).
    pub device_bits: u32,
    pub shuffle: bool,
    pub repeat: Repeat,
    /// Queue indices that play next, in order (for "Up next").
    pub up_next: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputDevice {
    pub name: String,
    pub is_default: bool,
}

/// Output devices Windows offers, the default one marked.
pub fn output_devices() -> Vec<OutputDevice> {
    let host = cpal::default_host();
    let default = host.default_output_device().and_then(|d| d.name().ok());
    host.output_devices()
        .map(|it| {
            it.filter_map(|d| d.name().ok())
                .map(|name| OutputDevice { is_default: Some(&name) == default.as_ref(), name })
                .collect()
        })
        .unwrap_or_default()
}

enum Cmd {
    Play { tracks: Vec<PathBuf>, start: usize },
    Pause,
    Resume,
    Seek(u64),
    Volume(f32),
    Skip(i64),
    Shuffle(bool),
    Repeat(Repeat),
    Stop,
}

/// State shared with the device callback.
struct Shared {
    paused: AtomicBool,
    flush: AtomicBool,
    volume: AtomicU32,
    played: AtomicU64,
}

impl Shared {
    fn new() -> Self {
        Self {
            paused: AtomicBool::new(true),
            flush: AtomicBool::new(false),
            volume: AtomicU32::new(1.0f32.to_bits()),
            played: AtomicU64::new(0),
        }
    }
}

/// The most recent samples the device played (mono, before volume), for the visualizer.
type Tap = Arc<Mutex<VecDeque<f32>>>;
const TAP_LEN: usize = 2048;

/// Equalizer settings and a generation number that goes up with every change, so a renderer
/// created later (after the device is reopened) still picks up the current settings.
pub type EqShared = Arc<Mutex<(u64, EqSettings)>>;

/// What runs in the device callback: ring buffer → volume → EQ → device, plus the visualizer tap.
/// At full volume with the EQ off or flat, samples pass through untouched.
struct Renderer {
    consumer: rtrb::Consumer<f32>,
    channels: usize,
    shared: Arc<Shared>,
    tap: Tap,
    eq_shared: EqShared,
    eq: Equalizer,
    eq_seen: u64,
}

impl Renderer {
    fn new(consumer: rtrb::Consumer<f32>, rate: u32, channels: usize, shared: Arc<Shared>, tap: Tap, eq_shared: EqShared) -> Self {
        Self { consumer, channels, shared, tap, eq_shared, eq: Equalizer::new(rate, channels), eq_seen: 0 }
    }

    fn render(&mut self, out: &mut [f32]) {
        let cb = &self.shared;
        if cb.flush.swap(false, Ordering::AcqRel) {
            let n = self.consumer.slots();
            if let Ok(chunk) = self.consumer.read_chunk(n) {
                chunk.commit_all();
            }
            cb.played.store(0, Ordering::Release);
        }
        if cb.paused.load(Ordering::Acquire) {
            out.fill(0.0);
            return;
        }
        // Never wait in the audio callback: new settings are picked up on a later call if busy.
        if let Ok(g) = self.eq_shared.try_lock() {
            if g.0 != self.eq_seen {
                self.eq.set(&g.1);
                self.eq_seen = g.0;
            }
        }
        let vol = f32::from_bits(cb.volume.load(Ordering::Relaxed));
        let n = out.len().min(self.consumer.slots());
        if let Ok(chunk) = self.consumer.read_chunk(n) {
            let (a, b) = chunk.as_slices();
            for (o, s) in out.iter_mut().zip(a.iter().chain(b)) {
                *o = s * vol;
            }
            chunk.commit_all();
        }
        self.eq.process(&mut out[..n]);
        out[n..].fill(0.0);
        cb.played.fetch_add((n / self.channels) as u64, Ordering::AcqRel);
        if let Ok(mut t) = self.tap.try_lock() {
            let undo = if vol > 1e-3 { 1.0 / vol } else { 0.0 };
            for frame in out[..n].chunks_exact(self.channels) {
                t.push_back(frame.iter().sum::<f32>() / self.channels as f32 * undo);
            }
            let extra = t.len().saturating_sub(TAP_LEN);
            t.drain(..extra);
        }
    }
}

pub struct Player {
    tx: Sender<Cmd>,
    status: Arc<Mutex<Status>>,
    tap: Tap,
    eq: EqShared,
    engine: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Player {
    /// Waits until the engine has closed the device, so it can be opened again straight away
    /// (an exclusive-mode device stays locked until then).
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Stop);
        if let Some(t) = self.engine.take() {
            let _ = t.join();
        }
    }
}

impl Player {
    /// Opens the default output device (shared mode) on its own thread.
    pub fn start() -> Result<Self, AudioError> {
        Self::start_on(None, false)
    }

    /// Opens the named output device (or the default if it is gone) on its own thread. With
    /// `exclusive`, the device is taken over at each file's own rate when a song starts; if that
    /// is refused, playback continues in shared mode.
    pub fn start_on(device: Option<&str>, exclusive: bool) -> Result<Self, AudioError> {
        let device = device.map(str::to_string);
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(Status { volume: 1.0, ..Default::default() }));
        let (ready_tx, ready_rx) = mpsc::channel();
        let tap: Tap = Arc::new(Mutex::new(VecDeque::with_capacity(TAP_LEN * 2)));
        let eq: EqShared = Arc::new(Mutex::new((0, EqSettings::default())));
        let (engine_status, engine_tap, engine_eq) = (status.clone(), tap.clone(), eq.clone());
        let engine_thread = std::thread::Builder::new()
            .name("audio-engine".into())
            .spawn(move || engine(rx, engine_status, engine_tap, engine_eq, device, exclusive, ready_tx))
            .map_err(|e| AudioError(e.to_string()))?;
        ready_rx.recv().map_err(|_| AudioError("audio engine stopped".into()))??;
        Ok(Self { tx, status, tap, eq, engine: Some(engine_thread) })
    }

    pub fn play(&self, tracks: Vec<PathBuf>, start: usize) {
        let _ = self.tx.send(Cmd::Play { tracks, start });
    }
    pub fn pause(&self) {
        let _ = self.tx.send(Cmd::Pause);
    }
    pub fn resume(&self) {
        let _ = self.tx.send(Cmd::Resume);
    }
    pub fn seek(&self, ms: u64) {
        let _ = self.tx.send(Cmd::Seek(ms));
    }
    pub fn set_volume(&self, v: f32) {
        let _ = self.tx.send(Cmd::Volume(v.clamp(0.0, 1.0)));
    }
    /// +1 next track, -1 previous, following the play order.
    pub fn skip(&self, by: i64) {
        let _ = self.tx.send(Cmd::Skip(by));
    }
    /// Shuffle the rest of the queue (the current song keeps playing), or go back to list order.
    pub fn set_shuffle(&self, on: bool) {
        let _ = self.tx.send(Cmd::Shuffle(on));
    }
    pub fn set_repeat(&self, repeat: Repeat) {
        let _ = self.tx.send(Cmd::Repeat(repeat));
    }
    pub fn status(&self) -> Status {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }
    /// Apply new equalizer settings (takes effect within one device buffer).
    pub fn set_eq(&self, settings: EqSettings) {
        if let Ok(mut e) = self.eq.lock() {
            *e = (e.0 + 1, settings);
        }
    }
    /// Output sample rate of the device right now (0 before an exclusive device is opened).
    pub fn rate(&self) -> u32 {
        self.status().device_rate
    }
    /// The last ~2048 samples actually played, mono and before volume.
    pub fn recent_mono(&self) -> Vec<f32> {
        self.tap.lock().map(|t| t.iter().copied().collect()).unwrap_or_default()
    }
}

enum Sink {
    Shared(#[allow(dead_code)] cpal::Stream), // kept alive; dropping it closes the device
    #[cfg(windows)]
    Exclusive(#[allow(dead_code)] crate::audio::exclusive::device::ExclusiveOut),
}

/// An open output device and the ring buffer that feeds it.
struct Out {
    _sink: Sink,
    producer: rtrb::Producer<f32>,
    rate: u32,
    channels: usize,
    name: String,
    exclusive: bool,
    bits: u32,
}

impl Out {
    fn is_empty(&self) -> bool {
        self.producer.slots() == self.producer.buffer().capacity()
    }
}

fn ring(rate: u32, channels: usize) -> (rtrb::Producer<f32>, rtrb::Consumer<f32>) {
    rtrb::RingBuffer::new(rate as usize * channels / 2) // ~0.5 s
}

fn open_shared(wanted: Option<&str>, shared: &Arc<Shared>, tap: &Tap, eq: &EqShared) -> Result<Out, AudioError> {
    let fail = |e: String| AudioError(e);
    let host = cpal::default_host();
    let named = wanted.and_then(|w| {
        let found = host.output_devices().ok()?.find(|d| d.name().ok().as_deref() == Some(w));
        if found.is_none() {
            log::warn!("output device {w:?} not found; using the default");
        }
        found
    });
    let device = named.or_else(|| host.default_output_device()).ok_or_else(|| fail("no audio output device".into()))?;
    let config = device.default_output_config().map_err(|e| fail(e.to_string()))?;
    let rate = config.sample_rate().0;
    let channels = config.channels() as usize;
    let (producer, consumer) = ring(rate, channels);
    let mut renderer = Renderer::new(consumer, rate, channels, shared.clone(), tap.clone(), eq.clone());
    let stream = device
        .build_output_stream(&config.config(), move |out: &mut [f32], _| renderer.render(out), |e| log::warn!("audio output error: {e}"), None)
        .map_err(|e| fail(e.to_string()))?;
    stream.play().map_err(|e| fail(e.to_string()))?;
    log::info!("audio output (shared): {rate} Hz, {channels} ch ({:?})", config.sample_format());
    Ok(Out {
        _sink: Sink::Shared(stream),
        producer,
        rate,
        channels,
        name: device.name().unwrap_or_default(),
        exclusive: false,
        bits: 0,
    })
}

#[cfg(windows)]
fn open_exclusive(wanted: Option<&str>, source_rate: u32, shared: &Arc<Shared>, tap: &Tap, eq: &EqShared) -> Result<Out, AudioError> {
    use crate::audio::exclusive::device;
    let (sink, opened, give) = device::open(wanted.map(str::to_string), source_rate, 2).map_err(AudioError)?;
    let (producer, consumer) = ring(opened.rate, opened.channels);
    let mut renderer = Renderer::new(consumer, opened.rate, opened.channels, shared.clone(), tap.clone(), eq.clone());
    give.send(Box::new(move |out: &mut [f32]| renderer.render(out))).map_err(|_| AudioError("exclusive output stopped".into()))?;
    log::info!("audio output (exclusive): {} Hz, {:?}, {} ch on {}", opened.rate, opened.kind, opened.channels, opened.name);
    Ok(Out {
        _sink: Sink::Exclusive(sink),
        producer,
        rate: opened.rate,
        channels: opened.channels,
        name: opened.name,
        exclusive: true,
        bits: opened.kind.valid_bits(),
    })
}

#[cfg(not(windows))]
fn open_exclusive(_: Option<&str>, _: u32, _: &Arc<Shared>, _: &Tap, _: &EqShared) -> Result<Out, AudioError> {
    Err(AudioError("exclusive mode needs Windows".into()))
}

fn engine(
    rx: Receiver<Cmd>,
    status: Arc<Mutex<Status>>,
    tap: Tap,
    eq: EqShared,
    wanted: Option<String>,
    mut exclusive: bool,
    ready: Sender<Result<(), AudioError>>,
) {
    let shared = Arc::new(Shared::new());
    let wanted = wanted.as_deref();
    // Exclusive mode opens the device when a song starts, at that song's rate.
    let mut out = if exclusive {
        None
    } else {
        match open_shared(wanted, &shared, &tap, &eq) {
            Ok(o) => Some(o),
            Err(e) => {
                let _ = ready.send(Err(e));
                return;
            }
        }
    };
    let _ = ready.send(Ok(()));

    let mut formats: std::collections::HashMap<usize, SourceFormat> = std::collections::HashMap::new();
    let mut feeder: Option<Feeder> = None;
    let mut tracks: Vec<PathBuf> = Vec::new();
    let mut timeline = Timeline::default();
    let mut pending: Vec<f32> = Vec::new();
    let mut pending_at = 0;
    let mut finished = false;
    // A rate the feeder asked for (exclusive mode), handled once the device has played everything before it.
    let mut want: Option<u32> = None;
    // Where in its track the feeder started, for the timeline after the device is reopened.
    let mut start_ms = 0u64;
    // Shuffle and repeat, and the order they make for the current queue.
    let (mut shuffle, mut repeat) = (false, Repeat::Off);
    let mut order = PlayOrder::new(0, 0, false, Repeat::Off, 1);
    let seed = || std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);

    loop {
        // Rate 0 until an exclusive device is open: the feeder then asks for the first file's rate.
        let (rate, channels) = out.as_ref().map_or((0, 2), |o| (o.rate, o.channels));
        let fresh = |tracks: &[PathBuf], start: usize, exclusive: bool| {
            if exclusive {
                Feeder::following_source(tracks.to_vec(), start, rate, channels)
            } else {
                Feeder::new(tracks.to_vec(), start, rate, channels)
            }
        };
        loop {
            let cmd = match rx.try_recv() {
                Ok(c) => c,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return,
            };
            let mut restart = |tracks: &[PathBuf], track: usize, offset_ms: u64, order: &PlayOrder, feeder: &mut Option<Feeder>, timeline: &mut Timeline| {
                let mut f = fresh(tracks, track, exclusive).with_order(order.clone());
                if offset_ms > 0 && f.seek_ms(offset_ms).is_err() {
                    return;
                }
                *feeder = Some(f);
                *timeline = Timeline::default();
                timeline.start_track(track, offset_ms * rate as u64 / 1000);
                start_ms = offset_ms;
                want = None;
                pending.clear();
                finished = false;
                shared.flush.store(true, Ordering::Release);
            };
            match cmd {
                Cmd::Play { tracks: t, start } => {
                    tracks = t;
                    formats.clear();
                    order = PlayOrder::new(tracks.len(), start, shuffle, repeat, seed());
                    restart(&tracks, start, 0, &order, &mut feeder, &mut timeline);
                    shared.paused.store(false, Ordering::Release);
                }
                Cmd::Stop => return,
                Cmd::Pause => shared.paused.store(true, Ordering::Release),
                Cmd::Resume => shared.paused.store(false, Ordering::Release),
                Cmd::Volume(v) => shared.volume.store(v.to_bits(), Ordering::Relaxed),
                Cmd::Seek(ms) => {
                    if let (true, Some((track, _))) = (feeder.is_some(), timeline.at(shared.played.load(Ordering::Acquire))) {
                        restart(&tracks, track, ms, &order, &mut feeder, &mut timeline);
                    }
                }
                Cmd::Skip(by) => {
                    if let Some((track, _)) = timeline.at(shared.played.load(Ordering::Acquire)) {
                        match order.skip(track, by) {
                            Some(next) => restart(&tracks, next, 0, &order, &mut feeder, &mut timeline),
                            // Previous on the first song: start it again.
                            None if by < 0 => restart(&tracks, track, 0, &order, &mut feeder, &mut timeline),
                            None => {}
                        }
                    }
                }
                Cmd::Shuffle(on) => {
                    shuffle = on;
                    let current = timeline.at(shared.played.load(Ordering::Acquire)).map_or(0, |(t, _)| t);
                    order.set_shuffle(on, current);
                    if let Some(f) = feeder.as_mut() {
                        *f.order_mut() = order.clone();
                    }
                }
                Cmd::Repeat(r) => {
                    repeat = r;
                    order.set_repeat(r);
                    if let Some(f) = feeder.as_mut() {
                        f.order_mut().set_repeat(r);
                    }
                }
            }
        }

        // The next file needs another rate: in exclusive mode, reopen the device at it once
        // everything before it has been heard; in shared mode, just resample.
        if let (Some(r), Some(f)) = (want, feeder.as_mut()) {
            if !exclusive {
                if f.set_out_rate(rate).is_err() {
                    feeder = None;
                }
                want = None;
            } else if out.as_ref().is_none_or(Out::is_empty) {
                drop(out.take()); // release the device before taking it again
                out = match open_exclusive(wanted, r, &shared, &tap, &eq) {
                    Ok(o) => Some(o),
                    Err(e) => {
                        log::warn!("exclusive mode unavailable ({e}); playing in shared mode");
                        exclusive = false;
                        open_shared(wanted, &shared, &tap, &eq).map_err(|e| log::error!("no audio output: {e}")).ok()
                    }
                };
                match out.as_ref() {
                    Some(o) if f.set_out_rate(o.rate).is_ok() => {
                        shared.played.store(0, Ordering::Release);
                        timeline = Timeline::default();
                        timeline.start_track(f.current_track(), start_ms * o.rate as u64 / 1000);
                    }
                    _ => feeder = None,
                }
                start_ms = 0;
                want = None;
            }
        }

        // Fill the ring buffer, a few blocks per turn so commands and status stay responsive
        // even when decoding is slow (e.g. debug builds).
        let mut budget = 8;
        while let Some(f) = feeder.as_mut() {
            if budget == 0 || want.is_some() {
                break;
            }
            budget -= 1;
            if pending_at >= pending.len() {
                match f.next_block() {
                    Ok(Some(block)) => {
                        if let (false, Some(fmt)) = (formats.contains_key(&block.track), f.current_format()) {
                            formats.insert(block.track, fmt);
                        }
                        if block.starts_track {
                            // A new pass (next song, or the same one again with repeat-one) starts at 0.
                            timeline.start_track(block.track, 0);
                        }
                        timeline.add(block.track, (block.samples.len() / channels) as u64);
                        pending = block.samples;
                        pending_at = 0;
                    }
                    Ok(None) if f.wants_rate().is_some() => {
                        want = f.wants_rate();
                        break;
                    }
                    Ok(None) | Err(_) => {
                        feeder = None;
                        finished = true;
                        break;
                    }
                }
            }
            let Some(o) = out.as_mut() else { break };
            let free = o.producer.slots().min(pending.len() - pending_at);
            if free == 0 {
                break;
            }
            if let Ok(mut chunk) = o.producer.write_chunk(free) {
                let (a, b) = chunk.as_mut_slices();
                let src = &pending[pending_at..pending_at + free];
                a.copy_from_slice(&src[..a.len()]);
                b.copy_from_slice(&src[a.len()..]);
                chunk.commit_all();
                pending_at += free;
            }
        }

        // Publish what the listener hears.
        let played = shared.played.load(Ordering::Acquire);
        let drained = finished && pending_at >= pending.len() && out.as_ref().is_none_or(Out::is_empty);
        if let Ok(mut s) = status.lock() {
            let here = timeline.at(played);
            s.track = here.map(|(t, _)| t);
            s.position_ms = match (here, out.as_ref()) {
                (Some((_, f)), Some(o)) => f * 1000 / o.rate as u64,
                (Some(_), None) => start_ms,
                _ => 0,
            };
            s.playing = !shared.paused.load(Ordering::Acquire) && here.is_some() && !drained;
            s.volume = f32::from_bits(shared.volume.load(Ordering::Relaxed));
            s.source = here.and_then(|(t, _)| formats.get(&t).copied());
            s.device_rate = out.as_ref().map_or(0, |o| o.rate);
            s.exclusive = out.as_ref().is_some_and(|o| o.exclusive);
            s.device_bits = out.as_ref().map_or(0, |o| o.bits);
            s.shuffle = shuffle;
            s.repeat = repeat;
            s.up_next = here.map(|(t, _)| order.upcoming(t, 4)).unwrap_or_default();
            let name = out.as_ref().map_or(wanted.unwrap_or(""), |o| o.name.as_str());
            if s.device_name != name {
                s.device_name = name.to_string();
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

// Machine test: plays through the real default output device (quietly).
#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::fixtures::wav;

    #[test]
    #[ignore]
    fn plays_pauses_and_seeks_on_the_default_device() {
        let d = tempfile::tempdir().unwrap();
        // Mono: the stereo fixture is +v / -v, which cancels out when the tap mixes to mono.
        let p = wav(d.path(), "t.wav", 44_100, 1, 16, 44_100 * 3);
        let player = Player::start().expect("audio device");
        player.set_volume(0.05);
        player.play(vec![p], 0);
        std::thread::sleep(Duration::from_millis(800));
        let s = player.status();
        println!("{s:?}");
        assert!(s.playing);
        assert_eq!(s.track, Some(0));
        assert!((300..1500).contains(&s.position_ms), "position {}", s.position_ms);
        let tap = player.recent_mono();
        assert_eq!(tap.len(), 2048, "visualizer tap holds the last 2048 samples");
        assert!(tap.iter().any(|v| v.abs() > 0.05), "tap carries the played signal (before volume)");

        // Changing the EQ mid-song must not interrupt playback.
        player.set_eq(crate::audio::eq::EqSettings { enabled: true, preamp_db: -6.0, gains_db: [6.0, 4.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0] });
        std::thread::sleep(Duration::from_millis(200));
        assert!(player.status().playing, "still playing after an EQ change");

        player.pause();
        std::thread::sleep(Duration::from_millis(200));
        let frozen = player.status().position_ms;
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(player.status().position_ms, frozen, "paused position must not move");
        assert!(!player.status().playing);

        player.seek(2_000);
        player.resume();
        std::thread::sleep(Duration::from_millis(300));
        let s = player.status();
        assert!(s.position_ms >= 2_000, "after seek: {}", s.position_ms);
    }
}

#[cfg(test)]
mod output_tests {
    use super::*;
    use crate::audio::fixtures::wav;

    #[test]
    #[ignore]
    fn lists_output_devices_with_one_default() {
        let devices = output_devices();
        println!("{devices:?}");
        assert!(!devices.is_empty());
        assert_eq!(devices.iter().filter(|d| d.is_default).count(), 1);
    }

    #[test]
    #[ignore]
    fn an_unknown_device_falls_back_to_the_default_and_reports_the_source_format() {
        let d = tempfile::tempdir().unwrap();
        let p = wav(d.path(), "t.wav", 44_100, 1, 16, 44_100);
        let player = Player::start_on(Some("No such device"), false).expect("falls back to default");
        player.set_volume(0.05);
        player.play(vec![p], 0);
        std::thread::sleep(Duration::from_millis(500));
        let s = player.status();
        println!("{s:?}");
        assert_eq!(s.source, Some(SourceFormat { rate: 44_100, bits: Some(16), channels: 1 }));
        assert!(s.device_rate > 0);
        assert!(!s.device_name.is_empty());
    }
}

#[cfg(test)]
mod renderer_tests {
    use super::*;

    fn setup(samples: &[f32], channels: usize) -> (Renderer, Arc<Shared>, EqShared) {
        let (mut p, c) = rtrb::RingBuffer::<f32>::new(1024);
        for s in samples {
            p.push(*s).unwrap();
        }
        let shared = Arc::new(Shared::new());
        shared.paused.store(false, Ordering::Release);
        let eq: EqShared = Arc::new(Mutex::new((0, EqSettings::default())));
        let tap: Tap = Arc::new(Mutex::new(VecDeque::new()));
        (Renderer::new(c, 48_000, channels, shared.clone(), tap, eq.clone()), shared, eq)
    }

    #[test]
    fn plays_buffered_samples_at_the_volume() {
        let (mut r, shared, _) = setup(&[0.5, -0.5, 0.25, -0.25], 2);
        shared.volume.store(0.5f32.to_bits(), Ordering::Relaxed);
        let mut out = [9.0; 4];
        r.render(&mut out);
        assert_eq!(out, [0.25, -0.25, 0.125, -0.125]);
        assert_eq!(shared.played.load(Ordering::Acquire), 2);
    }

    #[test]
    fn full_volume_and_flat_eq_pass_samples_through_untouched() {
        let input = [0.123_456_79, -0.987_654_3, 1.0 / 8_388_608.0, -1.0];
        let (mut r, _, _) = setup(&input, 2);
        let mut out = [0.0; 4];
        r.render(&mut out);
        assert_eq!(out, input, "bit-perfect needs an identity path");
    }

    #[test]
    fn runs_dry_into_silence_and_counts_only_real_frames() {
        let (mut r, shared, _) = setup(&[0.5, 0.5], 2);
        let mut out = [9.0; 6];
        r.render(&mut out);
        assert_eq!(out, [0.5, 0.5, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!(shared.played.load(Ordering::Acquire), 1);
    }

    #[test]
    fn paused_is_silent_and_keeps_the_buffer() {
        let (mut r, shared, _) = setup(&[0.5, 0.5], 1);
        shared.paused.store(true, Ordering::Release);
        let mut out = [9.0; 2];
        r.render(&mut out);
        assert_eq!(out, [0.0, 0.0]);
        shared.paused.store(false, Ordering::Release);
        r.render(&mut out);
        assert_eq!(out, [0.5, 0.5]);
    }

    #[test]
    fn flush_empties_the_buffer_and_restarts_the_count() {
        let (mut r, shared, _) = setup(&[0.5; 8], 1);
        shared.played.store(99, Ordering::Release);
        shared.flush.store(true, Ordering::Release);
        let mut out = [9.0; 2];
        r.render(&mut out);
        assert_eq!(out, [0.0, 0.0]);
        assert_eq!(shared.played.load(Ordering::Acquire), 0);
    }

    #[test]
    fn eq_settings_made_before_the_renderer_existed_still_apply() {
        let (mut p, c) = rtrb::RingBuffer::<f32>::new(16);
        p.push(0.5).unwrap();
        let shared = Arc::new(Shared::new());
        shared.paused.store(false, Ordering::Release);
        let quieter = EqSettings { preamp_db: -6.0, ..EqSettings::default() };
        let eq: EqShared = Arc::new(Mutex::new((3, quieter)));
        let mut r = Renderer::new(c, 48_000, 1, shared, Arc::new(Mutex::new(VecDeque::new())), eq);
        let mut out = [0.0; 1];
        r.render(&mut out);
        assert!((out[0] - 0.5 * 0.501).abs() < 0.01, "preamp applied: {}", out[0]);
    }
}

// Machine test: takes the default device in exclusive mode (quietly).
#[cfg(test)]
mod exclusive_machine_tests {
    use super::*;
    use crate::audio::fixtures::wav;

    #[test]
    #[ignore]
    fn exclusive_mode_runs_the_device_at_each_files_own_rate() {
        use crate::audio::exclusive::device::accepted_format;
        let d = tempfile::tempdir().unwrap();
        let a = wav(d.path(), "a.wav", 44_100, 2, 16, 44_100 * 2);
        let b = wav(d.path(), "b.wav", 48_000, 2, 24, 48_000 * 2);
        let player = Player::start_on(None, true).expect("engine");
        player.set_volume(0.05);
        player.play(vec![a, b], 0);
        for (track, source_rate) in [(0, 44_100), (1, 48_000)] {
            if track == 1 {
                player.skip(1);
            }
            std::thread::sleep(Duration::from_millis(1_000));
            let s = player.status();
            let (rate, kind) = accepted_format(None, source_rate, 2).expect("device accepts some exclusive format");
            println!("{s:?}
  device accepts {rate} Hz {kind:?} for {source_rate} Hz");
            assert!(s.exclusive, "device taken exclusively");
            assert_eq!(s.track, Some(track));
            assert!(s.playing && s.position_ms > 300, "plays (after the reopen): {s:?}");
            assert_eq!((s.device_rate, s.device_bits), (rate, kind.valid_bits()), "the best format the device takes");
        }
        player.pause();
    }

    #[test]
    #[ignore]
    fn leaving_exclusive_mode_frees_the_device_at_once() {
        let d = tempfile::tempdir().unwrap();
        let a = wav(d.path(), "a.wav", 48_000, 2, 16, 48_000);
        let player = Player::start_on(None, true).expect("engine");
        player.set_volume(0.05);
        player.play(vec![a.clone()], 0);
        std::thread::sleep(Duration::from_millis(500));
        assert!(player.status().exclusive);
        drop(player);
        let shared = Player::start_on(None, false).expect("the device is free again for shared mode");
        shared.set_volume(0.05);
        shared.play(vec![a], 0);
        std::thread::sleep(Duration::from_millis(400));
        assert!(shared.status().playing);
    }
}

// Machine test: shuffle and repeat on the default device (quietly).
#[cfg(test)]
mod order_machine_tests {
    use super::*;
    use crate::audio::fixtures::wav;

    #[test]
    #[ignore]
    fn repeat_one_keeps_the_song_and_shuffle_changes_what_comes_next() {
        let d = tempfile::tempdir().unwrap();
        let tracks: Vec<PathBuf> = (0..6).map(|i| wav(d.path(), &format!("{i}.wav"), 44_100, 1, 16, 22_050)).collect(); // 0.5 s each
        let player = Player::start().expect("engine");
        player.set_volume(0.05);
        player.set_repeat(Repeat::One);
        player.play(tracks, 0);
        std::thread::sleep(Duration::from_millis(1_400));
        let s = player.status();
        println!("{s:?}");
        assert_eq!(s.track, Some(0), "still the first song after it ended twice");
        assert!(s.position_ms < 500, "position restarts on each pass: {}", s.position_ms);
        assert_eq!(s.repeat, Repeat::One);
        player.skip(1);
        std::thread::sleep(Duration::from_millis(250));
        assert_eq!(player.status().track, Some(1), "Next moves on even with repeat one");
        assert_eq!(player.status().up_next, vec![2, 3, 4, 5]);
        player.set_shuffle(true);
        std::thread::sleep(Duration::from_millis(100));
        let s = player.status();
        println!("shuffled up next: {:?}", s.up_next);
        assert!(s.shuffle);
        assert_eq!(s.track, Some(1), "the current song keeps playing");
        assert!(!s.up_next.contains(&1));
        player.pause();
    }
}
