//! The DJ engine (D.2): runs the mixer on its own thread and feeds the speakers (master) and the
//! headphones (a second device, optional). Blocks are small and the buffers short (~40 ms), so
//! a knob turned on the screen or a controller is heard at once.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use serde::Serialize;

use super::actions::{Action, Deck as Side};
use super::analysis::Analysis;
use super::deck::Track;
use super::automix::{AutoMix, Event, Next};
use super::mixer::Mixer;
use crate::audio::dual::{VarResampler, MAX_CORRECTION};

/// Frames rendered at a time.
const BLOCK: usize = 256;
/// How much audio waits in each device's buffer.
const TARGET_S: f64 = 0.04;
/// How often the status (positions, meters) is refreshed for the screen.
const STATUS_EVERY: Duration = Duration::from_millis(33);

/// Stereo frames laid out for a device with `channels` channels: mono devices get the average,
/// extra channels stay silent.
pub fn spread(stereo: &[f32], channels: usize) -> Vec<f32> {
    match channels {
        2 => stereo.to_vec(),
        1 => stereo.chunks_exact(2).map(|f| (f[0] + f[1]) / 2.0).collect(),
        n => stereo.chunks_exact(2).flat_map(|f| (0..n).map(move |c| if c < 2 { f[c] } else { 0.0 })).collect(),
    }
}

/// Decodes a whole song for a deck (stereo, 16-bit; mono is put on both sides).
pub fn decode_track(path: &Path, song_id: i64, analysis: Option<Analysis>) -> Result<Track, String> {
    let mut dec = crate::audio::decode::Decoder::open(path).map_err(|e| e.to_string())?;
    let (rate, ch) = (dec.sample_rate, dec.channels.max(1));
    let to_i16 = |s: f32| (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
    let mut pcm = Vec::new();
    while let Some(chunk) = dec.next_chunk().map_err(|e| e.to_string())? {
        for f in chunk.chunks_exact(ch) {
            let (l, r) = if ch == 1 { (f[0], f[0]) } else { (f[0], f[1]) };
            pcm.push(to_i16(l));
            pcm.push(to_i16(r));
        }
    }
    Ok(Track { song_id, rate, pcm, analysis })
}

/// Rate correction for the headphone resampler from how full its buffer is: the headphones run
/// on their own clock, so their buffer is kept near `target` by making a little more or less audio.
pub fn phones_correction(buffered: usize, target: usize) -> f64 {
    let error = (target as f64 - buffered as f64) / target.max(1) as f64;
    (error * MAX_CORRECTION * 2.0).clamp(-MAX_CORRECTION, MAX_CORRECTION)
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckStatus {
    pub song_id: Option<i64>,
    pub playing: bool,
    pub ended: bool,
    pub position_s: f64,
    pub duration_s: f64,
    /// Tempo heard now (None: the song has no beat grid).
    pub bpm: Option<f64>,
    pub tempo: f64,
    pub level: f32,
    pub loading: bool,
    pub hot_cues: [Option<f64>; 4],
    pub loop_s: Option<(f64, f64)>,
    pub echo: bool,
    pub sweep: bool,
    /// The key heard now (Camelot), key lock and the pitch shift in semitones.
    pub key: Option<String>,
    pub key_lock: bool,
    pub key_shift: i8,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EngineStatus {
    pub decks: [DeckStatus; 2],
    pub crossfader: f32,
    pub master: String,
    /// The headphone device, when one is open.
    pub phones: Option<String>,
    /// What went wrong with a device (unplugged, refused).
    pub problem: Option<String>,
    pub auto_dj: bool,
    /// The deck the speakers play (Auto DJ's view).
    pub on_air: Option<Side>,
    pub next: Option<Next>,
    pub mixing: bool,
    pub style: super::autodj::Style,
}

enum Cmd {
    Action(Action),
    Loaded(Side, Arc<Track>),
    Phones(Option<String>),
    Style(super::autodj::Style),
    Stop,
}

/// An open device and the ring buffer feeding it.
struct DeviceOut {
    _stream: cpal::Stream,
    producer: rtrb::Producer<f32>,
    rate: u32,
    channels: usize,
    name: String,
    lost: Arc<AtomicBool>,
}

impl DeviceOut {
    fn buffered_frames(&self) -> usize {
        (self.producer.buffer().capacity() - self.producer.slots()) / self.channels
    }

    /// Writes what fits; the rest is dropped (only ever the headphones, when they fall behind).
    fn write(&mut self, samples: &[f32]) {
        let n = self.producer.slots().min(samples.len());
        if let Ok(mut chunk) = self.producer.write_chunk(n) {
            let (a, b) = chunk.as_mut_slices();
            a.copy_from_slice(&samples[..a.len()]);
            b.copy_from_slice(&samples[a.len()..n]);
            chunk.commit_all();
        }
    }
}

/// Opens a device by name in shared mode (`None`: the Windows default).
fn open(name: Option<&str>) -> Result<DeviceOut, String> {
    let host = cpal::default_host();
    let device = match name {
        Some(n) => host
            .output_devices()
            .map_err(|e| e.to_string())?
            .find(|d| d.name().ok().as_deref() == Some(n))
            .ok_or_else(|| format!("output device {n:?} not found"))?,
        None => host.default_output_device().ok_or("no audio output device")?,
    };
    let config = device.default_output_config().map_err(|e| e.to_string())?;
    let (rate, channels) = (config.sample_rate().0, config.channels() as usize);
    let (producer, mut consumer) = rtrb::RingBuffer::<f32>::new(rate as usize * channels / 4);
    let lost = Arc::new(AtomicBool::new(false));
    let watch = lost.clone();
    let stream = device
        .build_output_stream(
            &config.config(),
            move |out: &mut [f32], _| {
                let n = consumer.slots().min(out.len());
                if let Ok(chunk) = consumer.read_chunk(n) {
                    let (a, b) = chunk.as_slices();
                    out[..a.len()].copy_from_slice(a);
                    out[a.len()..n].copy_from_slice(b);
                    chunk.commit_all();
                }
                out[n..].fill(0.0);
            },
            move |e| {
                log::warn!("DJ output error: {e}");
                if matches!(e, cpal::StreamError::DeviceNotAvailable) {
                    watch.store(true, Ordering::Release);
                }
            },
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    let name = device.name().unwrap_or_default();
    log::info!("DJ output {name:?}: {rate} Hz, {channels} ch");
    Ok(DeviceOut { _stream: stream, producer, rate, channels, name, lost })
}

/// Peaks across the whole song on a deck, for the waveform overview.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub song_id: i64,
    pub peaks: Vec<u8>,
}

const OVERVIEW_BINS: usize = 600;

pub struct DjEngine {
    tx: Sender<Cmd>,
    status: Arc<Mutex<EngineStatus>>,
    overviews: Arc<Mutex<[Option<Overview>; 2]>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for DjEngine {
    fn drop(&mut self) {
        let _ = self.tx.send(Cmd::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl DjEngine {
    /// Opens the speakers (`master`, `None` = default) and, if given, the headphones. A
    /// headphone device that cannot be opened leaves the engine running without one.
    pub fn start(master: Option<String>, phones: Option<String>) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::channel();
        let status = Arc::new(Mutex::new(EngineStatus::default()));
        let st = status.clone();
        let thread = std::thread::Builder::new()
            .name("dj-engine".into())
            .spawn(move || run(rx, st, master, phones, ready_tx))
            .map_err(|e| e.to_string())?;
        ready_rx.recv().map_err(|_| "DJ engine stopped".to_string())??;
        Ok(Self { tx, status, overviews: Arc::default(), thread: Some(thread) })
    }

    pub fn apply(&self, action: Action) {
        let _ = self.tx.send(Cmd::Action(action));
    }

    /// Decodes the song on a worker thread and puts it on the deck when it is ready.
    pub fn load(&self, side: Side, path: PathBuf, song_id: i64, analysis: Option<Analysis>) {
        if let Ok(mut s) = self.status.lock() {
            s.decks[side as usize].loading = true;
        }
        let (tx, status, overviews) = (self.tx.clone(), self.status.clone(), self.overviews.clone());
        std::thread::spawn(move || match decode_track(&path, song_id, analysis) {
            Ok(t) => {
                if let Ok(mut o) = overviews.lock() {
                    o[side as usize] = Some(Overview { song_id, peaks: t.overview(OVERVIEW_BINS) });
                }
                let _ = tx.send(Cmd::Loaded(side, Arc::new(t)));
            }
            Err(e) => {
                log::warn!("DJ deck: cannot load {}: {e}", path.display());
                if let Ok(mut s) = status.lock() {
                    s.decks[side as usize].loading = false;
                    s.problem = Some(format!("cannot load {}: {e}", path.display()));
                }
            }
        });
    }

    /// The waveform overview of the song last loaded on `side`.
    pub fn overview(&self, side: Side) -> Option<Overview> {
        self.overviews.lock().ok()?[side as usize].clone()
    }

    /// How Auto DJ mixes from now on.
    pub fn set_style(&self, style: super::autodj::Style) {
        let _ = self.tx.send(Cmd::Style(style));
    }

    /// Switch the headphones to another device (`None`: no headphone cue) while playing.
    pub fn set_phones(&self, name: Option<String>) {
        let _ = self.tx.send(Cmd::Phones(name));
    }

    pub fn status(&self) -> EngineStatus {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

fn run(rx: Receiver<Cmd>, status: Arc<Mutex<EngineStatus>>, master: Option<String>, phones: Option<String>, ready: Sender<Result<(), String>>) {
    let mut out = match open(master.as_deref()) {
        Ok(o) => o,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let open_phones = |name: Option<&str>, master: &DeviceOut| {
        let p = name.and_then(|n| match open(Some(n)) {
            Ok(o) if o.name != master.name => Some(o),
            Ok(_) => None, // the speakers themselves: no separate headphones
            Err(e) => {
                log::warn!("DJ headphones: {e}");
                None
            }
        });
        let r = p.as_ref().and_then(|p| VarResampler::new(master.rate, p.rate, 2).ok());
        (p, r)
    };
    let (mut phones_out, mut resampler) = open_phones(phones.as_deref(), &out);
    let _ = ready.send(Ok(()));
    let mut mixer = Mixer::new(out.rate);
    let mut auto = AutoMix::default();
    let target = (out.rate as f64 * TARGET_S) as usize;
    let (mut master_buf, mut phones_buf) = (vec![0.0; BLOCK * 2], vec![0.0; BLOCK * 2]);
    let mut last_status = Instant::now() - STATUS_EVERY;
    let mut problem: Option<String> = None;
    loop {
        match rx.recv_timeout(Duration::from_millis(2)) {
            Ok(Cmd::Stop) | Err(RecvTimeoutError::Disconnected) => break,
            Ok(Cmd::Action(Action::AutoDj { on })) => auto.set_on(on),
            Ok(Cmd::Action(a)) => {
                mixer.apply(&a);
            }
            Ok(Cmd::Loaded(side, t)) => {
                mixer.deck_mut(side).load(t);
                if let Ok(mut s) = status.lock() {
                    s.decks[side as usize].loading = false;
                }
            }
            Ok(Cmd::Style(style)) => {
                auto.set_style(style);
                if let Ok(mut s) = status.lock() {
                    s.style = style;
                }
            }
            Ok(Cmd::Phones(name)) => {
                drop(phones_out.take()); // closes the old device first
                (phones_out, resampler) = open_phones(name.as_deref(), &out);
                if name.is_some() && phones_out.is_none() {
                    problem = Some(format!("cannot open {}", name.unwrap_or_default()));
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        while out.buffered_frames() < target {
            if let Some(Event::OnAir { deck, song_id }) = auto.tick(&mut mixer, BLOCK) {
                log::info!("Auto DJ: song {song_id} on air on deck {deck:?}");
            }
            mixer.render(&mut master_buf, &mut phones_buf);
            out.write(&spread(&master_buf, out.channels));
            if let (Some(p), Some(r)) = (phones_out.as_mut(), resampler.as_mut()) {
                let p_target = (p.rate as f64 * TARGET_S) as usize;
                r.set_correction(phones_correction(p.buffered_frames(), p_target));
                if let Ok(res) = r.process(&phones_buf) {
                    p.write(&spread(&res, p.channels));
                }
            }
        }
        if out.lost.load(Ordering::Acquire) && problem.is_none() {
            problem = Some(format!("{} was disconnected", out.name));
        }
        if phones_out.as_ref().is_some_and(|p| p.lost.load(Ordering::Acquire)) {
            let name = phones_out.take().map(|p| p.name).unwrap_or_default();
            resampler = None;
            problem = Some(format!("{name} was disconnected"));
        }
        if last_status.elapsed() >= STATUS_EVERY {
            last_status = Instant::now();
            if let Ok(mut s) = status.lock() {
                let levels = mixer.levels();
                for (i, side) in [Side::A, Side::B].into_iter().enumerate() {
                    let d = mixer.deck(side);
                    let loading = s.decks[i].loading;
                    s.decks[i] = DeckStatus {
                        song_id: d.track().map(|t| t.song_id),
                        playing: d.is_playing(),
                        ended: d.ended(),
                        position_s: d.position_s(),
                        duration_s: d.track().map_or(0.0, |t| t.duration_s()),
                        bpm: d.bpm(),
                        tempo: d.tempo(),
                        level: levels[i],
                        loading,
                        hot_cues: d.hot_cues(),
                        loop_s: d.loop_s(),
                        echo: mixer.effects(side).0,
                        sweep: mixer.effects(side).1,
                        key: d.key(),
                        key_lock: d.key_lock(),
                        key_shift: d.key_shift(),
                    };
                }
                s.crossfader = mixer.crossfader();
                s.auto_dj = auto.is_on();
                s.on_air = auto.live().filter(|_| auto.is_on());
                s.next = auto.next();
                s.mixing = auto.is_mixing();
                s.master = out.name.clone();
                s.phones = phones_out.as_ref().map(|p| p.name.clone());
                if problem.is_some() {
                    s.problem = problem.take();
                }
            }
        }
    }
    log::info!("DJ engine stopped");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_reaches_devices_with_other_channel_counts() {
        let stereo = [0.5, -0.5, 0.25, 0.75];
        assert_eq!(spread(&stereo, 2), stereo.to_vec());
        assert_eq!(spread(&stereo, 1), vec![0.0, 0.5]);
        assert_eq!(spread(&stereo, 4), vec![0.5, -0.5, 0.0, 0.0, 0.25, 0.75, 0.0, 0.0]);
    }

    #[test]
    fn songs_are_decoded_whole_into_stereo_16_bit() {
        let dir = tempfile::tempdir().unwrap();
        let mono = crate::audio::fixtures::wav(dir.path(), "m.wav", 44_100, 1, 16, 4410);
        let t = decode_track(&mono, 7, None).unwrap();
        assert_eq!((t.song_id, t.rate, t.frames()), (7, 44_100, 4410));
        assert_eq!(t.pcm[200], t.pcm[201], "mono is played on both sides");
        let stereo = crate::audio::fixtures::wav(dir.path(), "s.wav", 48_000, 2, 24, 960);
        let t = decode_track(&stereo, 8, None).unwrap();
        assert_eq!((t.rate, t.frames()), (48_000, 960));
        assert!(decode_track(&dir.path().join("missing.mp3"), 9, None).is_err());
    }

    #[test]
    fn the_headphone_buffer_is_kept_near_its_target() {
        assert_eq!(phones_correction(1000, 1000), 0.0);
        assert!(phones_correction(500, 1000) > 0.0, "running low: make a little more");
        assert!(phones_correction(1500, 1000) < 0.0, "running high: make a little less");
        let max = crate::audio::dual::MAX_CORRECTION;
        assert_eq!(phones_correction(0, 1000), max);
        assert_eq!(phones_correction(100_000, 1000), -max);
    }
}

// Machine test: runs the engine on this PC's default output (master volume 0, so nothing is
// heard) and checks a deck moves with the device clock.
#[cfg(test)]
mod machine_tests {
    use super::*;

    #[test]
    #[ignore]
    fn plays_on_the_default_device() {
        let dir = tempfile::tempdir().unwrap();
        let wav = crate::audio::fixtures::wav(dir.path(), "t.wav", 44_100, 2, 16, 44_100 * 5);
        let engine = DjEngine::start(None, None).unwrap();
        engine.apply(Action::MasterVolume { value: 0.0 });
        engine.load(Side::A, wav, 1, None);
        let t = Instant::now();
        while engine.status().decks[0].song_id.is_none() {
            assert!(t.elapsed() < Duration::from_secs(5), "loaded");
            std::thread::sleep(Duration::from_millis(20));
        }
        engine.apply(Action::PlayPause { deck: Side::A });
        std::thread::sleep(Duration::from_millis(1000));
        let s = engine.status();
        println!("{s:?}");
        assert!(s.decks[0].playing);
        assert!((s.decks[0].position_s - 1.0).abs() < 0.15, "{}", s.decks[0].position_s);
    }
}
