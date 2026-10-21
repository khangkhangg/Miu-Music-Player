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
}

const KP: f64 = 0.1;
const KI: f64 = 0.002;
/// Smoothing of the measured error (seconds): the counters move in whole device buffers.
const SMOOTH_S: f64 = 1.0;

impl DriftController {
    pub fn new(rate1: u32, rate2: u32) -> Self {
        Self { rate1: rate1 as f64, rate2: rate2 as f64, last: None, played1: 0.0, played2: 0.0, smoothed: 0.0, integral: 0.0, correction: 0.0 }
    }

    /// Both outputs start the music again together (play, seek): measure from here. The clock
    /// difference learnt so far is kept.
    pub fn restart(&mut self) {
        self.last = None;
        self.played1 = 0.0;
        self.played2 = 0.0;
        self.smoothed = 0.0;
    }

    /// Frames each device has consumed (since their last reset), as of now: the counter plus the
    /// time since its last callback times the rate, so positions do not jump a buffer at a time.
    /// Counters that went backwards (a device was flushed) start a new measurement.
    pub fn update(&mut self, frames1: f64, frames2: f64) -> f64 {
        let Some((l1, l2)) = self.last.replace((frames1, frames2)) else { return self.correction };
        if frames1 < l1 || frames2 < l2 {
            self.restart();
            self.last = Some((frames1, frames2));
            return self.correction;
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

    /// Current correction in parts per million.
    pub fn ppm(&self) -> f64 {
        self.correction * 1e6
    }

    /// How much faster the second clock runs than the first, as learnt so far (ppm; steadier
    /// than `ppm`, which also carries the moment-to-moment phase correction).
    pub fn clock_ppm(&self) -> f64 {
        (KI * self.integral).clamp(-MAX_CORRECTION, MAX_CORRECTION) * 1e6
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
/// second's, to be played in stereo split. When the delays are right the clicks are evenly spaced
/// and the third one sounds like a single click.
pub struct ClickTrack {
    rate: u32,
    channels: usize,
    pos: u64,
}

pub const BEAT_MS: u64 = 400;
const CLICK_MS: f32 = 6.0;
const CLICK_HZ: f32 = 2_000.0;

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
            let v = if into < click_len {
                let t = into as f32 / self.rate as f32;
                (2.0 * std::f32::consts::PI * CLICK_HZ * t).sin() * 0.5 * (1.0 - into as f32 / click_len as f32)
            } else {
                0.0
            };
            let (a, b) = Self::beat(n);
            for c in 0..self.channels {
                let on = if c == 1 { b } else if c == 0 { a } else { false };
                out.push(if on || (self.channels == 1 && (a || b)) { v } else { 0.0 });
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
        let mut c = DriftController::new(rate1, rate2);
        let (chunk1, chunk2) = (rate1 as u64 / 100, rate2 as u64 / 100);
        let (mut f1, mut f2) = (0u64, 0u64);
        let (mut t1, mut t2) = (0.0f64, 0.0f64); // when each device next asks for audio
        let (mut seen1, mut seen2) = (0.0f64, 0.0f64); // when the last callback was noted
        let mut seed = 12345u64;
        let mut wobble = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            ((seed >> 33) as f64 / (1u64 << 31) as f64 - 0.5) * 0.002
        };
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
            while t2 <= now {
                f2 += chunk2;
                seen2 = t2 + wobble().abs();
                t2 += chunk2 as f64 / (rate2 as f64 * (1.0 + ppm * 1e-6));
            }
            let at = |frames: u64, seen: f64, rate: u32, chunk: u64| frames as f64 - chunk as f64 + ((now - seen).max(0.0) * rate as f64).min(chunk as f64);
            c.update(at(f1, seen1, rate1, chunk1), at(f2, seen2, rate2, chunk2));
            if now > total - 60.0 {
                worst = worst.max(c.error_s().abs());
            }
        }
        (c, worst)
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
        c.update(48_000.0, 0.0); // second device silent for a second
        assert!(c.error_s() < -0.5);
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
