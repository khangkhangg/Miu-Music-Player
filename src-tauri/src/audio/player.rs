//! The player (F7.1): an engine thread that owns the output device (WASAPI shared mode via cpal),
//! decodes ahead into a lock-free ring buffer, and reports what the listener actually hears.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::collections::VecDeque;
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::Serialize;

use crate::audio::decode::{AudioError, SourceFormat};
use crate::audio::eq::{EqSettings, Equalizer};
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
}

/// State shared with the device callback.
struct Shared {
    paused: AtomicBool,
    flush: AtomicBool,
    volume: AtomicU32,
    played: AtomicU64,
}

/// The most recent samples the device played (mono, before volume), for the visualizer.
type Tap = Arc<Mutex<VecDeque<f32>>>;
const TAP_LEN: usize = 2048;

pub struct Player {
    tx: Sender<Cmd>,
    status: Arc<Mutex<Status>>,
    tap: Tap,
    rate: u32,
    eq: Arc<Mutex<Option<EqSettings>>>,
}

impl Player {
    /// Opens the default output device on its own thread.
    pub fn start() -> Result<Self, AudioError> {
        Self::start_on(None)
    }

    /// Opens the named output device (or the default if it is gone) on its own thread.
    pub fn start_on(device: Option<&str>) -> Result<Self, AudioError> {
        let device = device.map(str::to_string);
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(Status { volume: 1.0, ..Default::default() }));
        let (ready_tx, ready_rx) = mpsc::channel();
        let engine_status = status.clone();
        let tap: Tap = Arc::new(Mutex::new(VecDeque::with_capacity(TAP_LEN * 2)));
        let engine_tap = tap.clone();
        let eq: Arc<Mutex<Option<EqSettings>>> = Arc::new(Mutex::new(None));
        let engine_eq = eq.clone();
        std::thread::Builder::new()
            .name("audio-engine".into())
            .spawn(move || engine(rx, engine_status, engine_tap, engine_eq, device, ready_tx))
            .map_err(|e| AudioError(e.to_string()))?;
        let rate = ready_rx.recv().map_err(|_| AudioError("audio engine stopped".into()))??;
        Ok(Self { tx, status, tap, rate, eq })
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
    /// +1 next track, -1 previous.
    pub fn skip(&self, by: i64) {
        let _ = self.tx.send(Cmd::Skip(by));
    }
    pub fn status(&self) -> Status {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }
    /// Apply new equalizer settings (takes effect within one device buffer).
    pub fn set_eq(&self, settings: EqSettings) {
        if let Ok(mut e) = self.eq.lock() {
            *e = Some(settings);
        }
    }
    /// Output sample rate of the device.
    pub fn rate(&self) -> u32 {
        self.rate
    }
    /// The last ~2048 samples actually played, mono and before volume.
    pub fn recent_mono(&self) -> Vec<f32> {
        self.tap.lock().map(|t| t.iter().copied().collect()).unwrap_or_default()
    }
}

fn engine(
    rx: Receiver<Cmd>,
    status: Arc<Mutex<Status>>,
    tap: Tap,
    eq_update: Arc<Mutex<Option<EqSettings>>>,
    wanted: Option<String>,
    ready: Sender<Result<u32, AudioError>>,
) {
    let fail = |e: String| AudioError(e);
    let host = cpal::default_host();
    let named = wanted.as_deref().and_then(|w| {
        let found = host.output_devices().ok()?.find(|d| d.name().ok().as_deref() == Some(w));
        if found.is_none() {
            log::warn!("output device {w:?} not found; using the default");
        }
        found
    });
    let Some(device) = named.or_else(|| host.default_output_device()) else {
        let _ = ready.send(Err(fail("no audio output device".into())));
        return;
    };
    let config = match device.default_output_config() {
        Ok(c) => c,
        Err(e) => {
            let _ = ready.send(Err(fail(e.to_string())));
            return;
        }
    };
    let rate = config.sample_rate().0;
    let channels = config.channels() as usize;
    let device_name = device.name().unwrap_or_default();
    let mut formats: std::collections::HashMap<usize, SourceFormat> = std::collections::HashMap::new();
    let (mut producer, mut consumer) = rtrb::RingBuffer::<f32>::new(rate as usize * channels / 2); // ~0.5 s
    let shared = Arc::new(Shared {
        paused: AtomicBool::new(true),
        flush: AtomicBool::new(false),
        volume: AtomicU32::new(1.0f32.to_bits()),
        played: AtomicU64::new(0),
    });
    // New EQ settings arrive through `eq_update`; the callback picks them up without waiting.
    let mut eq = Equalizer::new(rate, channels);
    let cb = shared.clone();
    let stream = device.build_output_stream(
        &config.config(),
        move |out: &mut [f32], _| {
            if cb.flush.swap(false, Ordering::AcqRel) {
                let n = consumer.slots();
                if let Ok(chunk) = consumer.read_chunk(n) {
                    chunk.commit_all();
                }
                cb.played.store(0, Ordering::Release);
            }
            if cb.paused.load(Ordering::Acquire) {
                out.fill(0.0);
                return;
            }
            if let Ok(mut pending) = eq_update.try_lock() {
                if let Some(s) = pending.take() {
                    eq.set(&s);
                }
            }
            let vol = f32::from_bits(cb.volume.load(Ordering::Relaxed));
            let n = out.len().min(consumer.slots());
            if let Ok(chunk) = consumer.read_chunk(n) {
                let (a, b) = chunk.as_slices();
                for (o, s) in out.iter_mut().zip(a.iter().chain(b)) {
                    *o = s * vol;
                }
                chunk.commit_all();
            }
            eq.process(&mut out[..n]);
            out[n..].fill(0.0);
            cb.played.fetch_add((n / channels) as u64, Ordering::AcqRel);
            // Never wait in the audio callback: skip the tap if the visualizer is reading it.
            if let Ok(mut t) = tap.try_lock() {
                let undo = if vol > 1e-3 { 1.0 / vol } else { 0.0 };
                for frame in out[..n].chunks_exact(channels) {
                    t.push_back(frame.iter().sum::<f32>() / channels as f32 * undo);
                }
                let extra = t.len().saturating_sub(TAP_LEN);
                t.drain(..extra);
            }
        },
        |e| log::warn!("audio output error: {e}"),
        None,
    );
    let stream = match stream {
        Ok(s) => s,
        Err(e) => {
            let _ = ready.send(Err(fail(e.to_string())));
            return;
        }
    };
    if let Err(e) = stream.play() {
        let _ = ready.send(Err(fail(e.to_string())));
        return;
    }
    log::info!("audio output: {} Hz, {channels} ch ({:?})", rate, config.sample_format());
    let _ = ready.send(Ok(rate));

    let mut feeder: Option<Feeder> = None;
    let mut tracks: Vec<PathBuf> = Vec::new();
    let mut timeline = Timeline::default();
    let mut pending: Vec<f32> = Vec::new();
    let mut pending_at = 0;
    let mut finished = false;
    let restart = |feeder: &mut Option<Feeder>, timeline: &mut Timeline, tracks: &[PathBuf], start: usize| {
        *feeder = Some(Feeder::new(tracks.to_vec(), start, rate, channels));
        *timeline = Timeline::default();
        timeline.start_track(start, 0);
        shared.flush.store(true, Ordering::Release);
    };

    loop {
        loop {
            let cmd = match rx.try_recv() {
                Ok(c) => c,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return,
            };
            match cmd {
                Cmd::Play { tracks: t, start } => {
                    tracks = t;
                    formats.clear();
                    restart(&mut feeder, &mut timeline, &tracks, start);
                    pending.clear();
                    finished = false;
                    shared.paused.store(false, Ordering::Release);
                }
                Cmd::Pause => shared.paused.store(true, Ordering::Release),
                Cmd::Resume => shared.paused.store(false, Ordering::Release),
                Cmd::Volume(v) => shared.volume.store(v.to_bits(), Ordering::Relaxed),
                Cmd::Seek(ms) => {
                    let played = shared.played.load(Ordering::Acquire);
                    if let (Some(f), Some((track, _))) = (feeder.as_mut(), timeline.at(played)) {
                        let mut fresh = Feeder::new(tracks.clone(), track, rate, channels);
                        if fresh.seek_ms(ms).is_ok() {
                            *f = fresh;
                            timeline = Timeline::default();
                            timeline.start_track(track, ms * rate as u64 / 1000);
                            pending.clear();
                            finished = false;
                            shared.flush.store(true, Ordering::Release);
                        }
                    }
                }
                Cmd::Skip(by) => {
                    let played = shared.played.load(Ordering::Acquire);
                    if let Some((track, _)) = timeline.at(played) {
                        let next = track as i64 + by;
                        if next >= 0 && (next as usize) < tracks.len() {
                            restart(&mut feeder, &mut timeline, &tracks, next as usize);
                            pending.clear();
                            finished = false;
                        }
                    }
                }
            }
        }

        // Fill the ring buffer, a few blocks per turn so commands and status stay responsive
        // even when decoding is slow (e.g. debug builds).
        let mut budget = 8;
        while let Some(f) = feeder.as_mut() {
            if budget == 0 {
                break;
            }
            budget -= 1;
            if pending_at >= pending.len() {
                match f.next_block() {
                    Ok(Some(block)) => {
                        if let (false, Some(fmt)) = (formats.contains_key(&block.track), f.current_format()) {
                            formats.insert(block.track, fmt);
                        }
                        timeline.add(block.track, (block.samples.len() / channels) as u64);
                        pending = block.samples;
                        pending_at = 0;
                    }
                    Ok(None) | Err(_) => {
                        feeder = None;
                        finished = true;
                        break;
                    }
                }
            }
            let free = producer.slots().min(pending.len() - pending_at);
            if free == 0 {
                break;
            }
            if let Ok(mut chunk) = producer.write_chunk(free) {
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
        let drained = finished && pending_at >= pending.len() && producer.slots() == producer.buffer().capacity();
        if let Ok(mut s) = status.lock() {
            let here = timeline.at(played);
            s.track = here.map(|(t, _)| t);
            s.position_ms = here.map_or(0, |(_, f)| f * 1000 / rate as u64);
            s.playing = !shared.paused.load(Ordering::Acquire) && here.is_some() && !drained;
            s.volume = f32::from_bits(shared.volume.load(Ordering::Relaxed));
            s.source = here.and_then(|(t, _)| formats.get(&t).copied());
            s.device_rate = rate;
            if s.device_name != device_name {
                s.device_name = device_name.clone();
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
        let player = Player::start_on(Some("No such device")).expect("falls back to default");
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
