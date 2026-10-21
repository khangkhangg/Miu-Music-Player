//! The player (F7.1): an engine thread that owns the output device — WASAPI shared mode via cpal,
//! or exclusive (bit-perfect) mode at each file's own rate — decodes ahead into a lock-free ring
//! buffer, and reports what the listener actually hears.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::Serialize;

use crate::audio::decode::{AudioError, SourceFormat};
use crate::audio::dual::{route, ClickTrack, DelayLine, DriftController, DualMode, Side, VarResampler};
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
    /// The second output while it plays (F7.9).
    pub second: Option<SecondStatus>,
    /// A second output was asked for but could not be opened, or went away: one output plays.
    pub second_lost: bool,
    /// The alignment clicks are playing instead of the music.
    pub aligning: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SecondStatus {
    pub device_name: String,
    pub device_rate: u32,
    pub mode: DualMode,
    /// How much faster the second device's clock runs (ppm), as learnt so far.
    pub drift_ppm: f64,
    /// How far ahead (ms; negative: behind) the second output plays, after the delays.
    pub offset_ms: f64,
}

/// A second output device and how it shares the music with the first (F7.9, shared mode only).
#[derive(Debug, Clone, PartialEq)]
pub struct SecondOutput {
    pub device: String,
    pub mode: DualMode,
    /// Delay of the first and of the second output, ms (each 0..=MAX_DELAY_MS).
    pub delays_ms: [u32; 2],
}

pub const MAX_DELAY_MS: u32 = 500;

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
    /// `in_order`: a book's chapters, played in list order whatever shuffle and repeat say.
    Play { tracks: Vec<PathBuf>, start: usize, in_order: bool },
    Pause,
    Resume,
    Seek(u64),
    Volume(f32),
    Skip(i64),
    Shuffle(bool),
    Repeat(Repeat),
    /// Delays of the first and second output, ms.
    Delays([u32; 2]),
    Mode(DualMode),
    /// Play the alignment clicks (true) or go back to the music (false).
    Align(bool),
    Stop,
}

/// State shared with one device callback (each output has its own).
struct Shared {
    paused: AtomicBool,
    flush: AtomicBool,
    volume: AtomicU32,
    played: AtomicU64,
    /// When the callback last took audio (ns since `epoch`) and how many frames it took: lets the
    /// drift controller tell where the device is between callbacks.
    epoch: Instant,
    stamp_ns: AtomicU64,
    last_frames: AtomicU64,
    /// Flushes handled so far (the engine waits for them after a seek before measuring drift).
    flushes: AtomicU64,
    /// The device reported an error (unplugged, switched off).
    lost: AtomicBool,
}

impl Shared {
    fn new(epoch: Instant) -> Self {
        Self {
            paused: AtomicBool::new(true),
            flush: AtomicBool::new(false),
            volume: AtomicU32::new(1.0f32.to_bits()),
            played: AtomicU64::new(0),
            epoch,
            stamp_ns: AtomicU64::new(0),
            last_frames: AtomicU64::new(0),
            flushes: AtomicU64::new(0),
            lost: AtomicBool::new(false),
        }
    }

    /// Frames played as of `now`: the counter, less the last buffer, plus how far into it the
    /// device should be (see `DriftController::update`).
    fn position(&self, now: Instant, rate: u32) -> f64 {
        let played = self.played.load(Ordering::Acquire) as f64;
        let last = self.last_frames.load(Ordering::Relaxed) as f64;
        let stamp = self.epoch + Duration::from_nanos(self.stamp_ns.load(Ordering::Relaxed));
        let into = now.saturating_duration_since(stamp).as_secs_f64() * rate as f64;
        (played - last + into.min(last)).max(0.0)
    }
}

/// The most recent samples the device played (mono, before volume), for the visualizer.
type Tap = Arc<Mutex<VecDeque<f32>>>;
const TAP_LEN: usize = 2048;
/// How much of the queue the "Up next" list shows (it scrolls).
const UP_NEXT: usize = 100;

/// Equalizer settings and a generation number that goes up with every change, so a renderer
/// created later (after the device is reopened) still picks up the current settings.
pub type EqShared = Arc<Mutex<(u64, EqSettings)>>;

/// What runs in the device callback: ring buffer → volume → EQ → device, plus the visualizer tap.
/// At full volume with the EQ off or flat, samples pass through untouched.
struct Renderer {
    consumer: rtrb::Consumer<f32>,
    channels: usize,
    shared: Arc<Shared>,
    /// Only the first output feeds the visualizer.
    tap: Option<Tap>,
    eq_shared: EqShared,
    eq: Equalizer,
    eq_seen: u64,
}

impl Renderer {
    fn new(consumer: rtrb::Consumer<f32>, rate: u32, channels: usize, shared: Arc<Shared>, tap: Option<Tap>, eq_shared: EqShared) -> Self {
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
            cb.last_frames.store(0, Ordering::Relaxed);
            cb.flushes.fetch_add(1, Ordering::AcqRel);
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
        cb.stamp_ns.store(cb.epoch.elapsed().as_nanos() as u64, Ordering::Relaxed);
        cb.last_frames.store((n / self.channels) as u64, Ordering::Relaxed);
        cb.played.fetch_add((n / self.channels) as u64, Ordering::AcqRel);
        if let Some(Ok(mut t)) = self.tap.as_ref().map(|t| t.try_lock()) {
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
        Self::start_with(device, exclusive, None)
    }

    /// Like `start_on`, also playing on a `second` device (shared mode only; ignored in exclusive
    /// mode). If the second device cannot be opened, the first one plays alone.
    pub fn start_with(device: Option<&str>, exclusive: bool, second: Option<SecondOutput>) -> Result<Self, AudioError> {
        let device = device.map(str::to_string);
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(Status { volume: 1.0, ..Default::default() }));
        let (ready_tx, ready_rx) = mpsc::channel();
        let tap: Tap = Arc::new(Mutex::new(VecDeque::with_capacity(TAP_LEN * 2)));
        let eq: EqShared = Arc::new(Mutex::new((0, EqSettings::default())));
        let (engine_status, engine_tap, engine_eq) = (status.clone(), tap.clone(), eq.clone());
        let engine_thread = std::thread::Builder::new()
            .name("audio-engine".into())
            .spawn(move || engine(rx, engine_status, engine_tap, engine_eq, device, exclusive, second, ready_tx))
            .map_err(|e| AudioError(e.to_string()))?;
        ready_rx.recv().map_err(|_| AudioError("audio engine stopped".into()))??;
        Ok(Self { tx, status, tap, eq, engine: Some(engine_thread) })
    }

    pub fn play(&self, tracks: Vec<PathBuf>, start: usize) {
        let _ = self.tx.send(Cmd::Play { tracks, start, in_order: false });
    }
    /// Play `tracks` from `start` in list order, once (audiobook chapters).
    pub fn play_in_order(&self, tracks: Vec<PathBuf>, start: usize) {
        let _ = self.tx.send(Cmd::Play { tracks, start, in_order: true });
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
    /// Delays (ms) of the first and second output, to line them up.
    pub fn set_delays(&self, delays_ms: [u32; 2]) {
        let _ = self.tx.send(Cmd::Delays(delays_ms.map(|d| d.min(MAX_DELAY_MS))));
    }
    pub fn set_dual_mode(&self, mode: DualMode) {
        let _ = self.tx.send(Cmd::Mode(mode));
    }
    /// Alignment clicks on (the music pauses) or off (the music carries on where it was).
    pub fn align(&self, on: bool) {
        let _ = self.tx.send(Cmd::Align(on));
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

    /// Copy as much of `src` into the ring buffer as fits; returns how many samples went in.
    fn write(&mut self, src: &[f32]) -> usize {
        let free = self.producer.slots().min(src.len());
        if free == 0 {
            return 0;
        }
        match self.producer.write_chunk(free) {
            Ok(mut chunk) => {
                let (a, b) = chunk.as_mut_slices();
                a.copy_from_slice(&src[..a.len()]);
                b.copy_from_slice(&src[a.len()..free]);
                chunk.commit_all();
                free
            }
            Err(_) => 0,
        }
    }
}

fn ring(rate: u32, channels: usize) -> (rtrb::Producer<f32>, rtrb::Consumer<f32>) {
    rtrb::RingBuffer::new(rate as usize * channels / 2) // ~0.5 s
}

/// Opens `wanted` in shared mode. With `or_default`, a device that is gone falls back to the
/// Windows default; without, that is an error (a second output must never land on the first's).
fn open_shared(wanted: Option<&str>, or_default: bool, shared: &Arc<Shared>, tap: Option<&Tap>, eq: &EqShared) -> Result<Out, AudioError> {
    let fail = |e: String| AudioError(e);
    let host = cpal::default_host();
    let named = wanted.and_then(|w| {
        let found = host.output_devices().ok()?.find(|d| d.name().ok().as_deref() == Some(w));
        if found.is_none() && or_default {
            log::warn!("output device {w:?} not found; using the default");
        }
        found
    });
    let device = match named {
        Some(d) => d,
        None if or_default || wanted.is_none() => host.default_output_device().ok_or_else(|| fail("no audio output device".into()))?,
        None => return Err(fail(format!("output device {:?} not found", wanted.unwrap_or_default()))),
    };
    let config = device.default_output_config().map_err(|e| fail(e.to_string()))?;
    let rate = config.sample_rate().0;
    let channels = config.channels() as usize;
    let (producer, consumer) = ring(rate, channels);
    let mut renderer = Renderer::new(consumer, rate, channels, shared.clone(), tap.cloned(), eq.clone());
    let watch = shared.clone();
    let on_error = move |e: cpal::StreamError| {
        log::warn!("audio output error: {e}");
        if matches!(e, cpal::StreamError::DeviceNotAvailable) {
            watch.lost.store(true, Ordering::Release);
        }
    };
    let stream = device
        .build_output_stream(&config.config(), move |out: &mut [f32], _| renderer.render(out), on_error, None)
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
    let mut renderer = Renderer::new(consumer, opened.rate, opened.channels, shared.clone(), Some(tap.clone()), eq.clone());
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

fn delay_frames(ms: u32, rate: u32) -> usize {
    ms.min(MAX_DELAY_MS) as usize * rate as usize / 1000
}

/// How long the second device may take no audio while playing before it counts as gone.
const STALL: Duration = Duration::from_secs(2);
/// How often the drift controller looks, and how soon after one realignment another may follow.
const DRIFT_EVERY: Duration = Duration::from_millis(50);
const REALIGN_GAP: Duration = Duration::from_secs(5);

/// The second output (F7.9) and what feeds it: the music, at the first output's rate, goes through
/// its own delay and a resampler whose ratio the drift controller nudges.
struct Second {
    out: Out,
    shared: Arc<Shared>,
    in_rate: u32,
    delay: DelayLine,
    resampler: VarResampler,
    pending: Vec<f32>,
    drift: DriftController,
    /// Flush counts (first, second) to wait for after a restart before measuring again.
    wait: Option<(u64, u64)>,
    /// Frames played when last seen moving, and when: notices a device that stopped silently.
    progress: (u64, Instant),
}

impl Second {
    fn open(cfg: &SecondOutput, in_rate: u32, epoch: Instant, eq: &EqShared) -> Result<Self, AudioError> {
        let shared = Arc::new(Shared::new(epoch));
        let out = open_shared(Some(&cfg.device), false, &shared, None, eq)?;
        let resampler = VarResampler::new(in_rate, out.rate, out.channels)?;
        log::info!("second output: {} ({} Hz, {} ch), {:?}", out.name, out.rate, out.channels, cfg.mode);
        Ok(Self {
            delay: DelayLine::new(out.channels, delay_frames(cfg.delays_ms[1], in_rate)),
            resampler,
            drift: DriftController::new(in_rate, out.rate),
            pending: Vec::new(),
            wait: None,
            progress: (0, Instant::now()),
            in_rate,
            shared,
            out,
        })
    }

    /// Both outputs start over together (play, seek, new delays): empty everything, put the full
    /// delay first. Call before flushing the first output.
    fn restart(&mut self, first: &Shared) -> Result<(), AudioError> {
        self.wait = Some((first.flushes.load(Ordering::Acquire) + 1, self.shared.flushes.load(Ordering::Acquire) + 1));
        self.pending.clear();
        self.delay.reset();
        self.resampler = VarResampler::new(self.in_rate, self.out.rate, self.out.channels)?;
        self.resampler.set_correction(self.drift.clock_ppm() * 1e-6);
        self.shared.flush.store(true, Ordering::Release);
        Ok(())
    }

    /// Music for this output: at the first output's rate, already routed to this output's channels.
    fn push(&mut self, content: &[f32]) -> Result<(), AudioError> {
        let delayed = self.delay.process(content);
        self.pending.extend(self.resampler.process(&delayed)?);
        Ok(())
    }

    fn write(&mut self) {
        let n = self.out.write(&self.pending);
        self.pending.drain(..n);
        // The device stopped taking audio: do not hoard it (the stall check will notice).
        if self.pending.len() > self.out.rate as usize * self.out.channels * 2 {
            self.pending.clear();
        }
    }

    fn measure(&mut self, first: &Shared, first_rate: u32, now: Instant) {
        if let Some((a, b)) = self.wait {
            if first.flushes.load(Ordering::Acquire) < a || self.shared.flushes.load(Ordering::Acquire) < b {
                return;
            }
            self.wait = None;
            self.drift.restart();
        }
        let c = self.drift.update(first.position(now, first_rate), self.shared.position(now, self.out.rate));
        self.resampler.set_correction(c);
    }

    /// True once the device has taken no audio for `STALL` while the first one plays.
    fn stalled(&mut self, now: Instant, playing: bool) -> bool {
        let played = self.shared.played.load(Ordering::Acquire);
        if !playing || played != self.progress.0 {
            self.progress = (played, now);
            return false;
        }
        now.duration_since(self.progress.1) > STALL
    }

    fn status(&self, mode: DualMode) -> SecondStatus {
        SecondStatus {
            device_name: self.out.name.clone(),
            device_rate: self.out.rate,
            mode,
            drift_ppm: (self.drift.clock_ppm() * 10.0).round() / 10.0,
            offset_ms: (self.drift.error_s() * 10_000.0).round() / 10.0,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn engine(
    rx: Receiver<Cmd>,
    status: Arc<Mutex<Status>>,
    tap: Tap,
    eq: EqShared,
    wanted: Option<String>,
    mut exclusive: bool,
    second_cfg: Option<SecondOutput>,
    ready: Sender<Result<(), AudioError>>,
) {
    let epoch = Instant::now();
    let shared = Arc::new(Shared::new(epoch));
    let wanted = wanted.as_deref();
    // Exclusive mode opens the device when a song starts, at that song's rate.
    let mut out = if exclusive {
        None
    } else {
        match open_shared(wanted, true, &shared, Some(&tap), &eq) {
            Ok(o) => Some(o),
            Err(e) => {
                let _ = ready.send(Err(e));
                return;
            }
        }
    };
    // The second output: shared mode only, never on the first one's device.
    let mut dual_mode = second_cfg.as_ref().map_or(DualMode::Same, |c| c.mode);
    let mut second: Option<Second> = None;
    let mut second_lost = false;
    if let (Some(cfg), Some(o)) = (second_cfg.as_ref(), out.as_ref()) {
        if cfg.device == o.name {
            log::warn!("second output is the same device as the first; playing on one");
            second_lost = true;
        } else {
            match Second::open(cfg, o.rate, epoch, &eq) {
                Ok(s) => second = Some(s),
                Err(e) => {
                    log::warn!("second output unavailable ({e}); playing on one");
                    second_lost = true;
                }
            }
        }
    }
    let _ = ready.send(Ok(()));

    let first_rate = out.as_ref().map_or(0, |o| o.rate);
    let mut delay1 = DelayLine::new(out.as_ref().map_or(2, |o| o.channels), if second.is_some() { delay_frames(second_cfg.as_ref().map_or(0, |c| c.delays_ms[0]), first_rate) } else { 0 });
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
    // Alignment clicks play instead of the music; where the music was, and whether it was paused.
    let mut clicks: Option<ClickTrack> = None;
    let mut resume_after_clicks: Option<(usize, u64, bool)> = None;
    let (mut measured_at, mut realigned_at) = (epoch, epoch);

    loop {
        // Rate 0 until an exclusive device is open: the feeder then asks for the first file's rate.
        let (rate, channels) = out.as_ref().map_or((0, 2), |o| (o.rate, o.channels));
        // Frames heard so far, less the silence the first output's delay put in front.
        let heard = |played: u64, delay1: &DelayLine| (played as i64 - delay1.net_frames()).max(0) as u64;
        let now_at = |feeder: &Option<Feeder>, timeline: &Timeline, delay1: &DelayLine| {
            feeder.as_ref().and(timeline.at(heard(shared.played.load(Ordering::Acquire), delay1)))
        };
        let set_paused = |on: bool, second: &Option<Second>| {
            shared.paused.store(on, Ordering::Release);
            if let Some(s) = second {
                s.shared.paused.store(on, Ordering::Release);
            }
        };
        // Play, seek and skip all start the stream again from (track, ms); done once per turn.
        let mut restart_at: Option<(usize, u64)> = None;
        let mut flush_clicks = false;
        loop {
            let cmd = match rx.try_recv() {
                Ok(c) => c,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return,
            };
            // The track a seek or skip is relative to: one about to start, or the one heard now.
            let current = restart_at.map(|(t, _)| t).or_else(|| now_at(&feeder, &timeline, &delay1).map(|(t, _)| t));
            match cmd {
                Cmd::Play { tracks: t, start, in_order } => {
                    tracks = t;
                    formats.clear();
                    order = if in_order { PlayOrder::fixed(tracks.len(), start) } else { PlayOrder::new(tracks.len(), start, shuffle, repeat, seed()) };
                    clicks = None;
                    resume_after_clicks = None;
                    restart_at = Some((start, 0));
                    set_paused(false, &second);
                }
                Cmd::Stop => return,
                Cmd::Pause => set_paused(true, &second),
                Cmd::Resume => set_paused(false, &second),
                Cmd::Volume(v) => {
                    shared.volume.store(v.to_bits(), Ordering::Relaxed);
                    if let Some(s) = second.as_ref() {
                        s.shared.volume.store(v.to_bits(), Ordering::Relaxed);
                    }
                }
                Cmd::Seek(ms) => {
                    if let Some(track) = current {
                        restart_at = Some((track, ms));
                    }
                }
                Cmd::Skip(by) => {
                    if let Some(track) = current {
                        match order.skip(track, by) {
                            Some(next) => restart_at = Some((next, 0)),
                            // Previous on the first song: start it again.
                            None if by < 0 => restart_at = Some((track, 0)),
                            None => {}
                        }
                    }
                }
                Cmd::Shuffle(on) => {
                    shuffle = on;
                    order.set_shuffle(on, current.unwrap_or(0));
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
                Cmd::Mode(m) => dual_mode = m,
                Cmd::Delays(d) => {
                    if let Some(s) = second.as_mut() {
                        delay1.set_frames(delay_frames(d[0], rate));
                        s.delay.set_frames(delay_frames(d[1], rate));
                        // Both streams start over so the new delays apply from the same moment.
                        if clicks.is_some() {
                            flush_clicks = true;
                        } else if let Some((track, frame)) = now_at(&feeder, &timeline, &delay1) {
                            restart_at = restart_at.or(Some((track, frame * 1000 / rate.max(1) as u64)));
                        }
                    }
                }
                Cmd::Align(on) => {
                    if on && clicks.is_none() && second.is_some() {
                        let paused = shared.paused.load(Ordering::Acquire);
                        let here = restart_at.or_else(|| now_at(&feeder, &timeline, &delay1).map(|(t, f)| (t, f * 1000 / rate.max(1) as u64)));
                        resume_after_clicks = here.map(|(t, ms)| (t, ms, paused));
                        restart_at = None;
                        feeder = None;
                        timeline = Timeline::default();
                        clicks = Some(ClickTrack::new(rate, channels));
                        flush_clicks = true;
                        set_paused(false, &second);
                    } else if !on && clicks.is_some() {
                        clicks = None;
                        flush_clicks = true;
                        match resume_after_clicks.take() {
                            Some((track, ms, paused)) => {
                                restart_at = Some((track, ms));
                                set_paused(paused, &second);
                            }
                            None => set_paused(true, &second),
                        }
                    }
                }
            }
        }

        // The second output went away (error, or it stopped taking audio): carry on with one.
        let now = Instant::now();
        let playing_now = !shared.paused.load(Ordering::Acquire) && (feeder.is_some() || clicks.is_some());
        let gone = second.as_mut().is_some_and(|s| s.shared.lost.load(Ordering::Acquire) || s.stalled(now, playing_now));
        if gone {
            log::warn!("second output went away; playing on one");
            second = None;
            second_lost = true;
            delay1.set_frames(0);
            if clicks.take().is_some() {
                if let Some((track, ms, paused)) = resume_after_clicks.take() {
                    restart_at = Some((track, ms));
                    set_paused(paused, &second);
                } else {
                    set_paused(true, &second);
                }
            }
            if restart_at.is_none() {
                restart_at = now_at(&feeder, &timeline, &delay1).map(|(t, f)| (t, f * 1000 / rate.max(1) as u64));
            }
            flush_clicks = true;
        }

        // Keep the second output in step; if it is too far out, start both over where they are.
        if let Some(s) = second.as_mut() {
            if playing_now && now.duration_since(measured_at) >= DRIFT_EVERY {
                measured_at = now;
                s.measure(&shared, rate, now);
                if s.drift.lost_sync() && s.wait.is_none() && now.duration_since(realigned_at) >= REALIGN_GAP {
                    log::warn!("outputs drifted {:.0} ms apart; realigning", s.drift.error_s() * 1000.0);
                    realigned_at = now;
                    if clicks.is_some() {
                        flush_clicks = true;
                    } else if restart_at.is_none() {
                        restart_at = now_at(&feeder, &timeline, &delay1).map(|(t, f)| (t, f * 1000 / rate.max(1) as u64));
                    }
                }
            }
        }

        let flush_all = |second: &mut Option<Second>, delay1: &mut DelayLine| {
            delay1.reset();
            if let Some(s) = second.as_mut() {
                if let Err(e) = s.restart(&shared) {
                    log::warn!("second output stopped: {e}");
                    *second = None;
                }
            }
            shared.flush.store(true, Ordering::Release);
        };
        if let Some((track, ms)) = restart_at {
            let mut f = if exclusive { Feeder::following_source(tracks.clone(), track, rate, channels) } else { Feeder::new(tracks.clone(), track, rate, channels) }.with_order(order.clone());
            if ms == 0 || f.seek_ms(ms).is_ok() {
                feeder = Some(f);
                timeline = Timeline::default();
                timeline.start_track(track, ms * rate as u64 / 1000);
                start_ms = ms;
                want = None;
                pending.clear();
                finished = false;
                flush_all(&mut second, &mut delay1);
            }
        } else if flush_clicks {
            pending.clear();
            if clicks.is_some() {
                clicks = Some(ClickTrack::new(rate, channels));
            }
            flush_all(&mut second, &mut delay1);
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
                        open_shared(wanted, true, &shared, Some(&tap), &eq).map_err(|e| log::error!("no audio output: {e}")).ok()
                    }
                };
                match out.as_ref() {
                    Some(o) if f.set_out_rate(o.rate).is_ok() => {
                        shared.played.store(0, Ordering::Release);
                        timeline = Timeline::default();
                        timeline.start_track(f.current_track(), start_ms * o.rate as u64 / 1000);
                        delay1 = DelayLine::new(o.channels, 0);
                    }
                    _ => feeder = None,
                }
                start_ms = 0;
                want = None;
            }
        }

        // Fill the ring buffers, a few blocks per turn so commands and status stay responsive
        // even when decoding is slow (e.g. debug builds).
        let (side1, side2) = match (clicks.is_some(), second.is_some()) {
            (true, _) => DualMode::Split.sides(),
            (false, true) => dual_mode.sides(),
            (false, false) => (Side::Both, Side::Both),
        };
        let mut budget = 8;
        while budget > 0 && want.is_none() {
            budget -= 1;
            if pending_at >= pending.len() {
                let block = if let Some(c) = clicks.as_mut() {
                    if out.as_ref().map_or(0, |o| o.producer.slots()) < rate as usize / 100 * channels {
                        break;
                    }
                    c.next(rate as usize / 100)
                } else if let Some(f) = feeder.as_mut() {
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
                            block.samples
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
                } else {
                    break;
                };
                if let Some(s) = second.as_mut() {
                    let routed = route(&block, channels, s.out.channels, side2);
                    if let Err(e) = s.push(&routed) {
                        log::warn!("second output: {e}");
                    }
                }
                pending = delay1.process(&route(&block, channels, channels, side1));
                pending_at = 0;
            }
            let Some(o) = out.as_mut() else { break };
            let n = o.write(&pending[pending_at..]);
            pending_at += n;
            if n == 0 {
                break;
            }
        }
        if let Some(s) = second.as_mut() {
            s.write();
        }

        // Publish what the listener hears.
        let played = heard(shared.played.load(Ordering::Acquire), &delay1);
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
            s.up_next = here.map(|(t, _)| order.upcoming(t, UP_NEXT)).unwrap_or_default();
            let name = out.as_ref().map_or(wanted.unwrap_or(""), |o| o.name.as_str());
            if s.device_name != name {
                s.device_name = name.to_string();
            }
            s.second = second.as_ref().map(|sec| sec.status(dual_mode));
            s.second_lost = second_lost;
            s.aligning = clicks.is_some();
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

// Machine test (F7.9): a quiet tone on the default output and on one other output, if there is one.
#[cfg(test)]
mod dual_machine_tests {
    use super::*;
    use crate::audio::fixtures::wav_with;

    /// The two outputs to use: `MIU_TEST_OUTPUTS="first|second"` if set (e.g. two Bluetooth
    /// speakers), else the default device and another one. Hands-Free (call) endpoints are never
    /// picked: opening one switches a Bluetooth speaker into call mode.
    fn two_outputs() -> Option<(Option<String>, String)> {
        if let Ok(v) = std::env::var("MIU_TEST_OUTPUTS") {
            let (a, b) = v.split_once('|').expect("MIU_TEST_OUTPUTS=\"first|second\"");
            return Some((Some(a.to_string()), b.to_string()));
        }
        let devices = output_devices();
        println!("{devices:?}");
        let default = devices.iter().find(|d| d.is_default).map(|d| d.name.clone());
        let other = devices.iter().find(|d| Some(&d.name) != default.as_ref() && !d.name.contains("Hands-Free"))?;
        Some((None, other.name.clone()))
    }

    #[test]
    #[ignore]
    fn plays_a_tone_on_two_outputs() {
        let Some((first, other)) = two_outputs() else {
            println!("only one output device: nothing to test");
            return;
        };
        let d = tempfile::tempdir().unwrap();
        // 440 Hz on the left, 660 Hz on the right: in stereo split each output gets one of them.
        let tone = wav_with(d.path(), "tone.wav", 48_000, 2, 16, 48_000 * 30, |i, c| {
            let f = if c == 0 { 440.0 } else { 660.0 };
            (2.0 * std::f32::consts::PI * f * i as f32 / 48_000.0).sin() * 0.3
        });
        let second = SecondOutput { device: other.clone(), mode: DualMode::Split, delays_ms: [0, 0] };
        let player = Player::start_with(first.as_deref(), false, Some(second)).expect("audio device");
        player.set_volume(0.05);
        player.play(vec![tone], 0);
        // Bluetooth devices can take a moment to wake up.
        let started = Instant::now();
        while player.status().position_ms < 1_000 && started.elapsed() < Duration::from_secs(5) {
            std::thread::sleep(Duration::from_millis(50));
        }
        let s = player.status();
        println!("{s:?}");
        assert!(s.playing);
        assert!(!s.second_lost, "second output opened");
        let sec = s.second.clone().expect("second output playing");
        assert_eq!(sec.device_name, other);

        // Two device clocks over 15 s: the controller keeps them together.
        let mut worst_late = 0.0f64;
        for i in 0..15 {
            std::thread::sleep(Duration::from_secs(1));
            let sec = player.status().second.expect("still two outputs");
            println!("{:>2} s: offset {:+.1} ms, clock {:+.1} ppm", i + 1, sec.offset_ms, sec.drift_ppm);
            if i >= 5 {
                worst_late = worst_late.max(sec.offset_ms.abs());
            }
        }
        assert!(worst_late < 5.0, "outputs drifted apart: {worst_late} ms");

        player.set_delays([0, 120]);
        std::thread::sleep(Duration::from_millis(500));
        let after = player.status();
        assert!(after.playing && after.position_ms >= s.position_ms, "new delays keep the place: {} -> {}", s.position_ms, after.position_ms);

        player.align(true);
        std::thread::sleep(Duration::from_millis(1_000));
        assert!(player.status().aligning);
        player.align(false);
        std::thread::sleep(Duration::from_millis(300));
        let back = player.status();
        assert!(!back.aligning && back.playing, "music again after the clicks");
        assert!(back.position_ms >= after.position_ms, "from where it was: {} -> {}", after.position_ms, back.position_ms);
    }

    #[test]
    #[ignore]
    fn a_missing_second_output_leaves_one_playing() {
        let d = tempfile::tempdir().unwrap();
        let p = crate::audio::fixtures::wav(d.path(), "t.wav", 44_100, 1, 16, 44_100 * 2);
        let second = SecondOutput { device: "No Such Speaker".into(), mode: DualMode::Split, delays_ms: [100, 0] };
        let player = Player::start_with(None, false, Some(second)).expect("audio device");
        player.set_volume(0.05);
        player.play(vec![p], 0);
        // A Bluetooth device can take a moment to wake up: wait for it to take audio.
        let started = Instant::now();
        while player.status().position_ms < 300 && started.elapsed() < Duration::from_secs(3) {
            std::thread::sleep(Duration::from_millis(50));
        }
        let s = player.status();
        assert!(s.playing && s.second.is_none() && s.second_lost);
        assert!(s.position_ms > 200, "no delay left in front: {s:?}");
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
        let shared = Arc::new(Shared::new(Instant::now()));
        shared.paused.store(false, Ordering::Release);
        let eq: EqShared = Arc::new(Mutex::new((0, EqSettings::default())));
        let tap: Tap = Arc::new(Mutex::new(VecDeque::new()));
        (Renderer::new(c, 48_000, channels, shared.clone(), Some(tap), eq.clone()), shared, eq)
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
        let shared = Arc::new(Shared::new(Instant::now()));
        shared.paused.store(false, Ordering::Release);
        let quieter = EqSettings { preamp_db: -6.0, ..EqSettings::default() };
        let eq: EqShared = Arc::new(Mutex::new((3, quieter)));
        let mut r = Renderer::new(c, 48_000, 1, shared, Some(Arc::new(Mutex::new(VecDeque::new()))), eq);
        let mut out = [0.0; 1];
        r.render(&mut out);
        assert!((out[0] - 0.5 * 0.501).abs() < 0.01, "preamp applied: {}", out[0]);
    }

    #[test]
    fn each_handled_flush_is_counted() {
        let (mut r, shared, _) = setup(&[0.5; 4], 1);
        shared.flush.store(true, Ordering::Release);
        r.render(&mut [0.0; 2]);
        shared.flush.store(true, Ordering::Release);
        shared.paused.store(true, Ordering::Release);
        r.render(&mut [0.0; 2]);
        assert_eq!(shared.flushes.load(Ordering::Acquire), 2, "also while paused");
    }

    #[test]
    fn the_position_moves_on_between_callbacks_but_not_past_the_buffer() {
        let (mut r, shared, _) = setup(&[0.5; 960], 1);
        r.render(&mut [0.0; 480]); // 10 ms at 48 kHz
        let stamp = shared.epoch + Duration::from_nanos(shared.stamp_ns.load(Ordering::Relaxed));
        assert_eq!(shared.position(stamp, 48_000), 0.0, "the buffer just handed over starts now");
        assert!((shared.position(stamp + Duration::from_millis(5), 48_000) - 240.0).abs() < 1.0);
        assert_eq!(shared.position(stamp + Duration::from_secs(3), 48_000), 480.0, "never beyond what was handed over");
    }

    #[test]
    fn a_renderer_without_a_tap_still_plays() {
        let (mut p, c) = rtrb::RingBuffer::<f32>::new(4);
        p.push(0.5).unwrap();
        let shared = Arc::new(Shared::new(Instant::now()));
        shared.paused.store(false, Ordering::Release);
        let eq: EqShared = Arc::new(Mutex::new((0, EqSettings::default())));
        let mut r = Renderer::new(c, 48_000, 1, shared, None, eq);
        let mut out = [0.0; 1];
        r.render(&mut out);
        assert_eq!(out, [0.5]);
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
        assert_eq!(player.status().up_next, vec![2, 3, 4, 5], "the rest of the queue");
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
