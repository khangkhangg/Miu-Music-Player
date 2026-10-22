//! Second output (F7.9, experimental): the same decoded stream on two devices at once, either the
//! same sound on both or stereo split (left on the first, right on the second). Each output has
//! its own delay; the second one is resampled with a ratio nudged by a drift controller so the two
//! device clocks stay together.

use rubato::{Resampler as _, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};
use serde::{Deserialize, Serialize};

use crate::audio::decode::AudioError;
use crate::audio::pipeline::remix;

/// What the two outputs play.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DualMode {
    /// Both play the whole mix (multi-room).
    #[default]
    Same,
    /// Left channel on the first output, right on the second.
    Split,
}

/// Which part of the source one output plays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Both,
    Left,
    Right,
}

impl DualMode {
    /// (first output, second output).
    pub fn sides(self) -> (Side, Side) {
        match self {
            DualMode::Same => (Side::Both, Side::Both),
            DualMode::Split => (Side::Left, Side::Right),
        }
    }
}

/// Interleaved `samples` with `from` channels → `to` channels carrying `side` of the source. One
/// side of a stereo source goes to every channel of the output (one speaker plays all of "left").
pub fn route(samples: &[f32], from: usize, to: usize, side: Side) -> Vec<f32> {
    let pick = match side {
        Side::Both => return remix(samples, from, to),
        _ if from < 2 => return remix(samples, from, to),
        Side::Left => 0,
        Side::Right => 1,
    };
    samples.chunks_exact(from).flat_map(|f| std::iter::repeat(f[pick]).take(to)).collect()
}

/// How far the balance can turn one output down, dB.
pub const MAX_BALANCE_DB: f32 = 24.0;

/// Level of the first and second output for a balance in dB: positive turns the second output
/// down by that much, negative the first. The louder one always stays at full level.
pub fn balance_gains(balance_db: f32) -> [f32; 2] {
    if balance_db.is_nan() {
        return [1.0, 1.0];
    }
    let db = balance_db.clamp(-MAX_BALANCE_DB, MAX_BALANCE_DB);
    let down = 10f32.powf(-db.abs() / 20.0);
    if db > 0.0 { [1.0, down] } else if db < 0.0 { [down, 1.0] } else { [1.0, 1.0] }
}

/// Delays a stream by a number of frames that can change while it plays: a longer delay inserts
/// silence, a shorter one skips audio. `reset` (after a seek) starts over with the full delay.
pub struct DelayLine {
    channels: usize,
    frames: usize,
    insert: usize,
    skip: usize,
    /// Silence inserted minus audio skipped since the last reset, in frames.
    net: i64,
}

impl DelayLine {
    pub fn new(channels: usize, frames: usize) -> Self {
        Self { channels, frames, insert: frames, skip: 0, net: 0 }
    }

    pub fn frames(&self) -> usize {
        self.frames
    }

    pub fn set_frames(&mut self, frames: usize) {
        if frames > self.frames {
            let more = frames - self.frames;
            let cancel = more.min(self.skip);
            self.skip -= cancel;
            self.insert += more - cancel;
        } else {
            let less = self.frames - frames;
            let cancel = less.min(self.insert);
            self.insert -= cancel;
            self.skip += less - cancel;
        }
        self.frames = frames;
    }

    pub fn reset(&mut self) {
        self.insert = self.frames;
        self.skip = 0;
        self.net = 0;
    }

    /// Silence inserted minus audio skipped since the last reset, in frames.
    pub fn net_frames(&self) -> i64 {
        self.net
    }

    pub fn process(&mut self, samples: &[f32]) -> Vec<f32> {
        let frames = samples.len() / self.channels;
        let skip = self.skip.min(frames);
        self.skip -= skip;
        let insert = std::mem::take(&mut self.insert);
        self.net += insert as i64 - skip as i64;
        let mut out = vec![0.0; insert * self.channels];
        out.extend_from_slice(&samples[skip * self.channels..]);
        out
    }
}

/// Largest correction the drift controller applies (0.5 %, well under a semitone's 6 %).
pub const MAX_CORRECTION: f64 = 0.005;
/// Further apart than this and the controller gives up: the engine realigns by restarting.
pub const LOST_SYNC_S: f64 = 0.15;

/// Keeps the second output's playback in step with the first. Both report how many frames their
/// device consumed; the first one's clock is the reference. The second one's frames are turned
/// back into source time using the resampling correction in force, so `error` is how far ahead
/// (positive) or behind the second output plays the music. A PI loop turns that into a small
/// resampling correction: positive = stretch (the second output plays the music slower).
pub struct DriftController {
    rate1: f64,
    rate2: f64,
    last: Option<(f64, f64)>,
    played1: f64,
    played2: f64,
    smoothed: f64,
    integral: f64,
    correction: f64,
    /// A jump for the second output, waiting to be taken (seconds; + skip ahead, - hold).
    step: Option<f64>,
    /// No new step before the first output has played this far (seconds since the restart).
    step_ok_at: f64,
    /// Recent (time, gap) readings: a step is sized from their median, which ignores the bursts a
    /// Bluetooth device takes audio in.
    window: std::collections::VecDeque<(f64, f64)>,
}

const KP: f64 = 0.1;
const KI: f64 = 0.002;
/// Smoothing of the measured error (seconds): the counters move in whole device buffers.
const SMOOTH_S: f64 = 1.0;
/// Gaps above this are closed by a step; after one, wait this long (seconds) before the next.
const STEP_S: f64 = 0.005;
/// How long a gap must be seen (seconds) before it is stepped away.
const PHASE_WINDOW_S: f64 = 2.0;
/// The quarter-second medians in that window must lie within this of each other (seconds).
const SETTLED_SPREAD_S: f64 = 0.004;
const STEP_GAP_S: f64 = 1.0;
/// After a start or seek, devices (Bluetooth above all) take audio in bursts while they fill their
/// own buffers: nothing is judged until both have played this long (seconds).
const WARMUP_S: f64 = 3.0;

impl DriftController {
    pub fn new(rate1: u32, rate2: u32) -> Self {
        Self { rate1: rate1 as f64, rate2: rate2 as f64, last: Some((0.0, 0.0)), played1: 0.0, played2: 0.0, smoothed: 0.0, integral: 0.0, correction: 0.0, step: None, step_ok_at: WARMUP_S, window: Default::default() }
    }

    /// Both outputs start the music again together (play, seek): measure from here. The clock
    /// difference learnt so far is kept.
    /// (Both devices count from zero again after a flush.)
    pub fn restart(&mut self) {
        self.last = Some((0.0, 0.0));
        self.played1 = 0.0;
        self.played2 = 0.0;
        self.smoothed = 0.0;
        self.step = None;
        self.step_ok_at = WARMUP_S;
        self.window.clear();
    }

    /// Frames each device has consumed (since their last reset), as of now: the counter plus the
    /// time since its last callback times the rate, so positions do not jump a buffer at a time.
    /// Counters that went backwards (a device was flushed) start a new measurement.
    pub fn update(&mut self, frames1: f64, frames2: f64) -> f64 {
        let Some((l1, l2)) = self.last.replace((frames1, frames2)) else { return self.correction };
        if frames1 < l1 || frames2 < l2 {
            // Flushed: both counted again from zero.
            self.restart();
            return self.update(frames1, frames2);
        }
        let dt = (frames1 - l1) / self.rate1;
        self.played1 += dt;
        self.played2 += (frames2 - l2) / (self.rate2 * (1.0 + self.correction));
        if dt <= 0.0 {
            return self.correction;
        }
        let raw = self.played2 - self.played1;
        let a = (dt / SMOOTH_S).min(1.0);
        self.smoothed += (raw - self.smoothed) * a;
        if self.played1 < WARMUP_S {
            // Still settling: keep to the clock difference learnt so far, judge nothing yet.
            self.correction = (KI * self.integral).clamp(-MAX_CORRECTION, MAX_CORRECTION);
            return self.correction;
        }
        // A gap bigger than drift could make (a speaker that started late, a dropout) is closed in
        // one step, not learnt as a clock difference.
        self.window.push_back((self.played1, raw));
        while self.window.front().is_some_and(|&(t, _)| self.played1 - t > PHASE_WINDOW_S) {
            self.window.pop_front();
        }
        let seen_long_enough = self.window.front().is_some_and(|&(t, _)| self.played1 - t >= PHASE_WINDOW_S * 0.95);
        if seen_long_enough && self.played1 >= self.step_ok_at {
            let mut gaps: Vec<f64> = self.window.iter().map(|&(_, g)| g).collect();
            gaps.sort_by(f64::total_cmp);
            let median = gaps[gaps.len() / 2];
            // Only once the gap has stopped moving: a speaker still settling after a start is left alone.
            // Judged on quarter-second medians, so callback jitter does not count as movement.
            let start = self.window.front().map_or(0.0, |&(t, _)| t);
            let mut bins: Vec<Vec<f64>> = vec![Vec::new(); 8];
            for &(t, g) in &self.window {
                bins[(((t - start) / (PHASE_WINDOW_S / 8.0)) as usize).min(7)].push(g);
            }
            let medians: Vec<f64> = bins.iter_mut().filter(|b| !b.is_empty()).map(|b| {
                b.sort_by(f64::total_cmp);
                b[b.len() / 2]
            }).collect();
            let spread = medians.iter().copied().fold(f64::MIN, f64::max) - medians.iter().copied().fold(f64::MAX, f64::min);
            if spread <= SETTLED_SPREAD_S && median.abs() > STEP_S && gaps.iter().filter(|g| g.signum() == median.signum() && g.abs() > STEP_S / 2.0).count() * 4 >= gaps.len() * 3 {
                // Sized from the latest half second: the speed correction has closed some of it meanwhile.
                let mut recent: Vec<f64> = self.window.iter().filter(|&&(t, _)| self.played1 - t <= 0.5).map(|&(_, g)| g).collect();
                recent.sort_by(f64::total_cmp);
                let size = recent.get(recent.len() / 2).copied().unwrap_or(median);
                self.step = Some(-size);
                self.smoothed -= size;
                self.window.clear();
                self.step_ok_at = self.played1 + STEP_GAP_S;
                return self.correction;
            }
        }
        // A big gap is left alone until it stops moving and is then closed by a step; nudging the
        // speed meanwhile would keep it moving. Small gaps and the clock difference are followed here.
        if self.smoothed.abs() > STEP_S {
            self.correction = (KI * self.integral).clamp(-MAX_CORRECTION, MAX_CORRECTION);
            return self.correction;
        }
        // Integrate only while the correction is not pinned at its limit (no wind-up).
        let next_integral = self.integral + self.smoothed * dt;
        let c = KP * self.smoothed + KI * next_integral;
        if c.abs() < MAX_CORRECTION {
            self.integral = next_integral;
        }
        self.correction = c.clamp(-MAX_CORRECTION, MAX_CORRECTION);
        self.correction
    }

    /// Seconds the second output is ahead of the first (negative: behind), smoothed.
    pub fn error_s(&self) -> f64 {
        self.smoothed
    }

    /// Current resampling correction (fraction: +0.001 = 0.1 % more output frames).
    pub fn correction(&self) -> f64 {
        self.correction
    }

    /// Current correction in parts per million.
    pub fn ppm(&self) -> f64 {
        self.correction * 1e6
    }

    /// How much faster the second clock runs than the first, as learnt so far (ppm; steadier
    /// than `ppm`, which also carries the moment-to-moment phase correction).
    pub fn clock_ppm(&self) -> f64 {
        (KI * self.integral).clamp(-MAX_CORRECTION, MAX_CORRECTION) * 1e6
    }

    /// A jump the second output should make now, if any (seconds): positive = skip that much
    /// audio, negative = play that much silence first.
    pub fn take_step(&mut self) -> Option<f64> {
        self.step.take()
    }

    pub fn lost_sync(&self) -> bool {
        self.smoothed.abs() > LOST_SYNC_S
    }
}

/// Resampler for the second output: always running (even at equal rates) so its ratio can be
/// nudged a little while it plays.
pub struct VarResampler {
    channels: usize,
    inner: SincFixedIn<f32>,
    pending: Vec<Vec<f32>>,
}

impl VarResampler {
    pub fn new(from_rate: u32, to_rate: u32, channels: usize) -> Result<Self, AudioError> {
        let params = SincInterpolationParameters {
            sinc_len: 128,
            f_cutoff: 0.95,
            interpolation: SincInterpolationType::Cubic,
            oversampling_factor: 256,
            window: WindowFunction::BlackmanHarris2,
        };
        let ratio = to_rate as f64 / from_rate as f64;
        let inner = SincFixedIn::<f32>::new(ratio, 1.0 + 2.0 * MAX_CORRECTION, params, 512, channels).map_err(|e| AudioError(e.to_string()))?;
        Ok(Self { channels, inner, pending: vec![Vec::new(); channels] })
    }

    /// `correction` from the drift controller: +0.001 makes 0.1 % more output frames.
    pub fn set_correction(&mut self, correction: f64) {
        let _ = self.inner.set_resample_ratio_relative(1.0 + correction, true);
    }

    pub fn process(&mut self, interleaved: &[f32]) -> Result<Vec<f32>, AudioError> {
        for frame in interleaved.chunks_exact(self.channels) {
            for (c, s) in frame.iter().enumerate() {
                self.pending[c].push(*s);
            }
        }
        let mut out = Vec::new();
        while self.pending[0].len() >= self.inner.input_frames_next() {
            let n = self.inner.input_frames_next();
            let chunk: Vec<Vec<f32>> = self.pending.iter_mut().map(|ch| ch.drain(..n).collect()).collect();
            let res = self.inner.process(&chunk, None).map_err(|e| AudioError(e.to_string()))?;
            for i in 0..res[0].len() {
                for ch in &res {
                    out.push(ch[i]);
                }
            }
        }
        Ok(out)
    }
}

/// Alignment aid: a click on the first output, then on the second, then on both together, then a
/// rest; each beat `BEAT_MS` long. Channel 0 carries the first output's clicks and channel 1 the
/// second's, to be played in stereo split. The first output clicks low and the second an octave
/// higher, so you can tell which speaker is early. When the delays are right the clicks are evenly
/// spaced and the two on the third beat land as one.
pub struct ClickTrack {
    rate: u32,
    channels: usize,
    pos: u64,
}

pub const BEAT_MS: u64 = 400;
const CLICK_MS: f32 = 8.0;
/// Pitch of the first and second output's clicks.
const CLICK_HZ: [f32; 2] = [1_200.0, 2_400.0];

impl ClickTrack {
    pub fn new(rate: u32, channels: usize) -> Self {
        Self { rate, channels, pos: 0 }
    }

    /// Which outputs click on beat `n` (first, second).
    fn beat(n: u64) -> (bool, bool) {
        match n % 4 {
            0 => (true, false),
            1 => (false, true),
            2 => (true, true),
            _ => (false, false),
        }
    }

    pub fn next(&mut self, frames: usize) -> Vec<f32> {
        let beat_len = self.rate as u64 * BEAT_MS / 1000;
        let click_len = (self.rate as f32 * CLICK_MS / 1000.0) as u64;
        let mut out = Vec::with_capacity(frames * self.channels);
        for _ in 0..frames {
            let (n, into) = (self.pos / beat_len, self.pos % beat_len);
            let (a, b) = Self::beat(n);
            let tone = |hz: f32| {
                if into >= click_len {
                    return 0.0;
                }
                let t = into as f32 / self.rate as f32;
                (2.0 * std::f32::consts::PI * hz * t).sin() * 0.5 * (1.0 - into as f32 / click_len as f32)
            };
            let first = if a { tone(CLICK_HZ[0]) } else { 0.0 };
            let second = if b { tone(CLICK_HZ[1]) } else { 0.0 };
            for c in 0..self.channels {
                out.push(match (self.channels, c) {
                    (1, _) => (first + second) / 2.0,
                    (_, 0) => first,
                    (_, 1) => second,
                    _ => 0.0,
                });
            }
            self.pos += 1;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_sound_sends_the_whole_mix_to_both() {
        assert_eq!(DualMode::Same.sides(), (Side::Both, Side::Both));
        assert_eq!(route(&[0.1, 0.2, 0.3, 0.4], 2, 2, Side::Both), vec![0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn stereo_split_puts_left_on_the_first_and_right_on_the_second() {
        assert_eq!(DualMode::Split.sides(), (Side::Left, Side::Right));
        let s = [0.1, 0.2, 0.3, 0.4];
        assert_eq!(route(&s, 2, 2, Side::Left), vec![0.1, 0.1, 0.3, 0.3], "left on both speaker channels");
        assert_eq!(route(&s, 2, 2, Side::Right), vec![0.2, 0.2, 0.4, 0.4]);
        assert_eq!(route(&s, 2, 1, Side::Right), vec![0.2, 0.4], "a mono device gets its side");
        assert_eq!(route(&s, 2, 4, Side::Left), vec![0.1; 4].into_iter().chain([0.3; 4]).collect::<Vec<_>>());
    }

    #[test]
    fn a_mono_source_plays_on_both_sides_of_a_split() {
        assert_eq!(route(&[0.5, 0.25], 1, 2, Side::Right), vec![0.5, 0.5, 0.25, 0.25]);
    }

    #[test]
    fn the_delay_line_starts_with_its_delay_in_silence() {
        let mut d = DelayLine::new(2, 3);
        assert_eq!(d.process(&[1.0, 1.0, 2.0, 2.0]), vec![0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0, 2.0, 2.0]);
        assert_eq!(d.process(&[3.0, 3.0]), vec![3.0, 3.0], "then audio flows straight through");
        assert_eq!(d.net_frames(), 3);
    }

    #[test]
    fn a_longer_delay_inserts_silence_and_a_shorter_one_skips_audio() {
        let mut d = DelayLine::new(1, 0);
        assert_eq!(d.process(&[1.0]), vec![1.0]);
        d.set_frames(2);
        assert_eq!(d.process(&[2.0]), vec![0.0, 0.0, 2.0]);
        d.set_frames(1);
        assert_eq!(d.process(&[3.0, 4.0]), vec![4.0]);
        assert_eq!(d.net_frames(), 1);
        d.set_frames(4);
        d.set_frames(1);
        assert_eq!(d.process(&[5.0]), vec![5.0], "changes that cancel out do nothing");
    }

    #[test]
    fn skipping_more_than_one_block_carries_over() {
        let mut d = DelayLine::new(1, 5);
        d.process(&[]);
        d.set_frames(0);
        assert_eq!(d.process(&[1.0, 2.0, 3.0]), Vec::<f32>::new());
        assert_eq!(d.process(&[4.0, 5.0, 6.0]), vec![6.0]);
        assert_eq!(d.net_frames(), 0);
    }

    #[test]
    fn reset_brings_back_the_full_delay() {
        let mut d = DelayLine::new(1, 2);
        d.process(&[1.0]);
        d.reset();
        assert_eq!(d.process(&[7.0]), vec![0.0, 0.0, 7.0]);
    }

    /// Two device clocks: the second runs `ppm` fast. Each pulls 10 ms per callback, and the time a
    /// callback is noted wobbles by up to a millisecond. The engine looks every 50 ms and, like
    /// the real one, extrapolates each counter from its last callback at the nominal rate.
    /// Returns (controller, worst error in the last minute, seconds).
    fn simulate(rate1: u32, rate2: u32, ppm: f64, minutes: f64) -> (DriftController, f64) {
        let (c, worst, _) = simulate_with(rate1, rate2, ppm, minutes, 0.0, 0.0, 0.0);
        (c, worst)
    }

    /// Like `simulate`, with the second device starting `late_s` after the first, and the
    /// controller's phase steps applied the way the renderer does: skipping ahead counts the
    /// skipped frames as played, holding plays silence that is not counted.
    pub(super) fn simulate_with(rate1: u32, rate2: u32, ppm: f64, minutes: f64, late_s: f64, burst_s: f64, jitter_s: f64) -> (DriftController, f64, usize) {
        let mut steps = 0;
        let mut burst = (burst_s * rate2 as f64) as u64; // taken all at once by the first callback, like a Bluetooth prebuffer
        let mut c = DriftController::new(rate1, rate2);
        let (chunk1, chunk2) = (rate1 as u64 / 100, rate2 as u64 / 100);
        let (mut f1, mut f2) = (0u64, 0u64);
        let (mut t1, mut t2) = (0.0f64, late_s); // when each device next asks for audio
        let (mut seen1, mut seen2) = (0.0f64, 0.0f64); // when the last callback was noted
        let mut nudge = 0i64; // frames for the second device: + skip ahead, - hold
        let mut seed = 12345u64;
        let mut wobble = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) as f64 / (1u64 << 31) as f64 - 0.5) * 0.002
        };
        // Bluetooth: each of the second device's callbacks comes up to `jitter_s` / 2 early or late, so they bunch up.
        let mut jseed = 777u64;
        let mut jitter = move || {
            jseed = jseed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((jseed >> 33) as f64 / (1u64 << 31) as f64 - 0.5) * jitter_s
        };
        let mut late2 = jitter();
        let step = 0.05;
        let total = minutes * 60.0;
        let mut worst = 0.0f64;
        let mut now = 0.0;
        while now < total {
            now += step;
            while t1 <= now {
                f1 += chunk1;
                seen1 = t1 + wobble().abs();
                t1 += chunk1 as f64 / rate1 as f64;
            }
            while t2 + late2 <= now {
                let held = if nudge < 0 { (-nudge).min(chunk2 as i64) } else { 0 };
                nudge += held;
                let skipped = nudge.max(0);
                nudge -= skipped;
                f2 += chunk2 - held as u64 + skipped as u64;
                f2 += std::mem::take(&mut burst);
                seen2 = t2 + late2 + wobble().abs();
                late2 = jitter();
                t2 += chunk2 as f64 / (rate2 as f64 * (1.0 + ppm * 1e-6));
            }
            let at = |frames: u64, seen: f64, rate: u32, chunk: u64| frames as f64 - chunk as f64 + ((now - seen).max(0.0) * rate as f64).min(chunk as f64);
            c.update(at(f1, seen1, rate1, chunk1), at(f2, seen2, rate2, chunk2));
            if let Some(s) = c.take_step() {
                nudge += (s * rate2 as f64).round() as i64;
                steps += 1;
            }
            if now > total - 60.0 {
                worst = worst.max(c.error_s().abs());
            }
        }
        (c, worst, steps)
    }

    #[test]
    fn a_second_device_that_starts_late_is_stepped_into_line_not_mistaken_for_drift() {
        let (c, worst, steps) = simulate_with(44_100, 44_100, 0.0, 1.5, 0.025, 0.0, 0.0);
        assert!(worst < 0.002, "worst {worst} s");
        assert_eq!(steps, 1);
        assert!(c.clock_ppm().abs() < 20.0, "a 25 ms late start is not a clock difference: {} ppm", c.clock_ppm());
    }

    #[test]
    fn a_second_device_that_starts_early_waits_for_the_first() {
        let mut c = DriftController::new(48_000, 48_000);
        c.update(0.0, 0.0);
        for i in 1..=120 {
            c.update(i as f64 * 2_400.0, i as f64 * 2_400.0 + 960.0); // 20 ms ahead, for 6 s
        }
        let s = c.take_step().expect("a step");
        // About 20 ms, less what the speed correction closed while the gap was being confirmed.
        assert!((-0.021..-0.014).contains(&s), "hold the second output about 20 ms: {s}");
        assert!(c.take_step().is_none(), "one step at a time");
    }

    #[test]
    fn matched_clocks_need_no_correction() {
        let (c, worst) = simulate(48_000, 48_000, 0.0, 3.0);
        assert!(c.clock_ppm().abs() < 5.0, "{} ppm", c.clock_ppm());
        assert!(worst < 0.002, "{worst}");
    }

    #[test]
    fn a_fast_second_clock_is_slowed_down_and_kept_within_a_millisecond_or_two() {
        let (c, worst) = simulate(48_000, 48_000, 300.0, 5.0);
        assert!((c.clock_ppm() - 300.0).abs() < 30.0, "learnt {} ppm", c.clock_ppm());
        assert!(worst < 0.002, "worst {worst} s");
    }

    #[test]
    fn a_slow_second_clock_at_another_rate_is_sped_up() {
        let (c, worst) = simulate(48_000, 44_100, -150.0, 5.0);
        assert!((c.clock_ppm() + 150.0).abs() < 30.0, "learnt {} ppm", c.clock_ppm());
        assert!(worst < 0.002, "worst {worst} s");
    }

    #[test]
    fn the_correction_never_exceeds_its_limit() {
        let (c, _) = simulate(48_000, 48_000, 20_000.0, 2.0);
        assert!(c.ppm() <= MAX_CORRECTION * 1e6 + 1e-6);
        assert!(c.lost_sync(), "2 % off cannot be followed; the engine must realign");
    }

    #[test]
    fn counters_that_restart_begin_a_new_measurement() {
        let mut c = DriftController::new(48_000, 48_000);
        c.update(0.0, 0.0);
        for i in 1..=120 {
            // The second device takes nothing for the first second, then plays along.
            c.update(i as f64 * 2_400.0, (i.max(20) - 20) as f64 * 2_400.0);
        }
        assert!(c.take_step().is_some_and(|s| s > 0.9), "skip the second output ahead");
        c.update(0.0, 0.0); // both flushed (seek)
        c.update(4_800.0, 4_800.0);
        assert!(c.error_s().abs() < 1e-3, "fresh start, no error: {}", c.error_s());
    }

    #[test]
    fn the_variable_resampler_follows_its_correction() {
        let input = vec![0.25f32; 48_000 * 2];
        let mut r = VarResampler::new(48_000, 48_000, 2).unwrap();
        let plain = r.process(&input).unwrap().len() / 2;
        let mut r = VarResampler::new(48_000, 48_000, 2).unwrap();
        r.set_correction(0.004);
        let stretched = r.process(&input).unwrap().len() / 2;
        assert!((plain as i64 - 48_000).abs() < 1_000, "{plain}");
        let gained = stretched as f64 / plain as f64 - 1.0;
        assert!((gained - 0.004).abs() < 0.0015, "gained {gained}");
    }

    #[test]
    fn the_variable_resampler_converts_rates() {
        let mut r = VarResampler::new(44_100, 48_000, 1).unwrap();
        let n = r.process(&vec![0.1; 44_100]).unwrap().len() as i64;
        assert!((n - 48_000).abs() < 1_000, "{n}");
    }

    fn onsets(samples: &[f32], channels: usize, ch: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut quiet = true;
        for (i, f) in samples.chunks_exact(channels).enumerate() {
            let loud = f[ch].abs() > 0.05;
            if loud && quiet {
                out.push(i);
            }
            quiet = !loud && (out.last().is_none_or(|&o| i - o > 100));
        }
        out
    }

    #[test]
    fn the_click_track_goes_first_second_both_rest() {
        let rate = 10_000;
        let mut t = ClickTrack::new(rate, 2);
        let s = t.next(rate as usize * 4 * BEAT_MS as usize / 1000 * 2); // two bars
        let beat = rate as usize * BEAT_MS as usize / 1000;
        let near = |v: Vec<usize>, want: Vec<usize>| {
            assert_eq!(v.len(), want.len(), "{v:?} vs {want:?}");
            for (a, b) in v.iter().zip(&want) {
                assert!(a.abs_diff(*b) <= 2, "{v:?} vs {want:?}");
            }
        };
        near(onsets(&s, 2, 0), vec![0, 2 * beat, 4 * beat, 6 * beat]);
        near(onsets(&s, 2, 1), vec![beat, 2 * beat, 5 * beat, 6 * beat]);
    }

    #[test]
    fn the_click_track_continues_across_calls() {
        let mut a = ClickTrack::new(8_000, 2);
        let mut b = ClickTrack::new(8_000, 2);
        let whole = a.next(10_000);
        let mut parts = b.next(3_333);
        parts.extend(b.next(6_667));
        assert_eq!(whole, parts);
    }
}

#[cfg(test)]
mod click_pitch_tests {
    use super::*;

    /// Upward zero crossings in the first `CLICK_MS` after `start`, scaled to Hz.
    fn pitch(samples: &[f32], channels: usize, ch: usize, start: usize, rate: u32) -> f32 {
        let len = (rate as f32 * CLICK_MS / 1000.0) as usize;
        let s: Vec<f32> = samples.chunks_exact(channels).skip(start).take(len).map(|f| f[ch]).collect();
        let ups = s.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count() as f32;
        ups / (len as f32 / rate as f32)
    }

    #[test]
    fn each_output_clicks_at_its_own_pitch_so_you_can_tell_them_apart() {
        let rate = 48_000;
        let beat = (rate as u64 * BEAT_MS / 1000) as usize;
        let s = ClickTrack::new(rate, 2).next(beat * 3);
        let (first, second) = (pitch(&s, 2, 0, 0, rate), pitch(&s, 2, 1, beat, rate));
        assert!(second > first * 1.4, "second {second} Hz should be clearly higher than first {first} Hz");
        // On the "both" beat each output keeps its own pitch.
        assert!((pitch(&s, 2, 0, 2 * beat, rate) - first).abs() < 200.0);
        assert!((pitch(&s, 2, 1, 2 * beat, rate) - second).abs() < 200.0);
    }
}

#[cfg(test)]
mod balance_tests {
    use super::*;

    #[test]
    fn centre_balance_leaves_both_at_full_level() {
        assert_eq!(balance_gains(0.0), [1.0, 1.0]);
    }

    #[test]
    fn positive_balance_turns_the_second_output_down_negative_the_first() {
        let [a, b] = balance_gains(6.0);
        assert_eq!(a, 1.0);
        assert!((b - 0.501).abs() < 0.001, "-6 dB: {b}");
        let [a, b] = balance_gains(-20.0);
        assert!((a - 0.1).abs() < 0.001, "-20 dB: {a}");
        assert_eq!(b, 1.0);
    }

    #[test]
    fn balance_is_kept_within_its_range() {
        assert_eq!(balance_gains(99.0), balance_gains(MAX_BALANCE_DB));
        assert_eq!(balance_gains(f32::NAN), [1.0, 1.0]);
    }
}

#[cfg(test)]
mod warm_up_tests {
    use super::*;

    #[test]
    fn a_bluetooth_start_burst_gets_one_correction_after_settling_not_a_back_and_forth() {
        // The second speaker starts 20 ms late and its first callback takes 60 ms in one go.
        let (_, worst, steps) = super::tests::simulate_with(44_100, 44_100, 0.0, 1.5, 0.020, 0.060, 0.0);
        assert_eq!(steps, 1, "one step once both have settled");
        assert!(worst < 0.002, "worst {worst} s");
    }

    #[test]
    fn nothing_is_judged_in_the_first_seconds() {
        let mut c = DriftController::new(48_000, 48_000);
        c.update(0.0, 0.0);
        for i in 1..=50 {
            c.update(i as f64 * 2_400.0, i as f64 * 2_400.0 + 4_800.0); // 100 ms ahead for 2.5 s
            assert!(c.take_step().is_none(), "no step during warm-up ({} s)", i as f64 * 0.05);
        }
        for i in 51..=120 {
            c.update(i as f64 * 2_400.0, i as f64 * 2_400.0 + 4_800.0);
        }
        assert!(c.take_step().is_some_and(|s| (s + 0.1).abs() < 0.01), "then one step back by 100 ms");
    }
}

#[cfg(test)]
mod jitter_tests {
    #[test]
    fn bursty_bluetooth_timing_with_no_real_offset_never_causes_a_jump() {
        // Callbacks up to 20 ms early or late and bunching up, as measured on a Bose SoundLink Flex.
        let (_, _, steps) = super::tests::simulate_with(44_100, 44_100, 0.0, 2.0, 0.0, 0.0, 0.040);
        assert_eq!(steps, 0, "every jump here is an audible tick for nothing");
    }

    #[test]
    fn a_late_start_is_still_corrected_once_despite_bursty_timing() {
        let (_, _, steps) = super::tests::simulate_with(44_100, 44_100, 0.0, 1.0, 0.030, 0.060, 0.040);
        assert_eq!(steps, 1);
    }
}

#[cfg(test)]
mod dropout_tests {
    use super::*;

    #[test]
    fn a_dropout_mid_song_is_closed_in_one_jump_of_its_full_size() {
        let mut c = DriftController::new(48_000, 48_000);
        let mut lag = 0.0; // frames the second device is behind
        let mut steps = Vec::new();
        for i in 1..=400 {
            if i == 160 {
                lag = 3_840.0; // 80 ms dropout at 8 s
            }
            let f = i as f64 * 2_400.0;
            c.update(f, f - lag);
            if let Some(s) = c.take_step() {
                steps.push(s);
                lag -= s * 48_000.0; // the skip counts as played
            }
        }
        assert_eq!(steps.len(), 1, "{steps:?}");
        // The speed correction closes a little of it while the gap is being confirmed; the jump does the rest.
        assert!((0.065..0.085).contains(&steps[0]), "{steps:?}");
        assert!(c.error_s().abs() < 0.003, "together afterwards: {} s", c.error_s());
    }
}

#[cfg(test)]
mod settling_tests {
    use super::*;

    #[test]
    fn a_gap_that_is_still_moving_is_left_to_settle_not_jumped_on() {
        // After the warm-up the second device's gap swings to -25 ms and back over 4 s (a slow start burst).
        let mut c = DriftController::new(48_000, 48_000);
        let mut steps = 0;
        for i in 1..=200 {
            let t = i as f64 * 0.05;
            let gap = if (3.0..7.0).contains(&t) { -0.025 * (std::f64::consts::PI * (t - 3.0) / 4.0).sin() } else { 0.0 };
            let f = i as f64 * 2_400.0;
            c.update(f, f + gap * 48_000.0);
            if c.take_step().is_some() {
                steps += 1;
            }
        }
        assert_eq!(steps, 0, "the swing settles by itself");
    }
}
