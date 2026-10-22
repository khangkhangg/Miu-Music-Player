//! Auto-calibration of two outputs with a microphone (F7.9). Miu plays a short test sound, the
//! microphone at the listening spot records it, and the recording says how far apart and how much
//! louder one output sounds than the other; delay and balance are then set so both arrive
//! together and evenly at that spot.
//!
//! The sound (channel 0 = first output, channel 1 = second, played in stereo split): a moment of
//! silence, bars of the alignment clicks (low on the first, high on the second, then both
//! together) for the timing, then pairs of short noise bursts, the same on each output, for the
//! loudness. Everything is short: headset microphones gate out steady sound within a fraction
//! of a second, but let transients through.

use crate::audio::dual::{ClickTrack, BEAT_MS};

pub const SILENCE_S: f64 = 0.5;
pub const BARS: u32 = 5;
pub const GAP_S: f64 = 0.5;
pub const PAIRS: u32 = 8;
/// A pair every half second: the first output's burst, then the second's a quarter later.
pub const PAIR_S: f64 = 0.5;
pub const BURST_S: f64 = 0.03;

const BAR_S: f64 = 4.0 * BEAT_MS as f64 / 1000.0;

fn clicks_at() -> f64 {
    SILENCE_S
}

fn bursts_at() -> f64 {
    SILENCE_S + BARS as f64 * BAR_S + GAP_S
}

/// Length of the whole calibration sound, seconds.
pub fn duration_s() -> f64 {
    bursts_at() + PAIRS as f64 * PAIR_S
}

pub struct CalibrationSignal {
    rate: u32,
    channels: usize,
    pos: u64,
    seed: u64,
    clicks: ClickTrack,
}

impl CalibrationSignal {
    pub fn new(rate: u32, channels: usize) -> Self {
        Self { rate, channels, pos: 0, seed: 0x9e37_79b9_7f4a_7c15, clicks: ClickTrack::new(rate, channels) }
    }

    /// The next `frames` of the sound (silence once it is over).
    pub fn next(&mut self, frames: usize) -> Vec<f32> {
        let rate = self.rate as f64;
        let clicks_end = clicks_at() + BARS as f64 * BAR_S;
        let fade = 0.002;
        let mut out = Vec::with_capacity(frames * self.channels);
        for _ in 0..frames {
            let t = self.pos as f64 / rate;
            let mut frame = [0.0f32; 2];
            if (clicks_at()..clicks_end).contains(&t) {
                let c = self.clicks.next(1);
                frame = [c[0], *c.get(1).unwrap_or(&c[0])];
            } else if (bursts_at()..duration_s()).contains(&t) {
                let half = PAIR_S / 2.0;
                let k = ((t - bursts_at()) / half) as usize;
                let into = t - bursts_at() - k as f64 * half;
                if into < BURST_S {
                    let gain = (into / fade).min((BURST_S - into) / fade).min(1.0) as f32;
                    self.seed = self.seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    frame[k % 2] = ((self.seed >> 33) as f32 / (1u64 << 31) as f32 - 0.5) * 0.6 * gain;
                }
            }
            match self.channels {
                1 => out.push((frame[0] + frame[1]) / 2.0),
                n => out.extend((0..n).map(|c| if c < 2 { frame[c] } else { 0.0 })),
            }
            self.pos += 1;
        }
        out
    }
}

/// What the microphone heard.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measurement {
    /// How far the second output sounds after the first, seconds (negative: before).
    pub offset_s: f64,
    /// How much louder the second output sounds than the first, dB (negative: quieter).
    pub louder_db: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unheard {
    /// The clicks could not be timed: microphone too far, muted or too quiet.
    Clicks,
    /// The clicks were timed but the loudness bursts were not heard.
    Bursts,
}

/// Read a mono recording of the calibration sound (it may start up to a second before the sound).
pub fn analyze(samples: &[f32], rate: u32) -> Result<Measurement, Unheard> {
    use crate::audio::listen::{offset, onsets, PITCH_HZ};
    // Timing from the clicks (the bursts come later: cut them off so they do not look like clicks).
    let clicks = &samples[..((bursts_at() * rate as f64) as usize).min(samples.len())];
    let offset_s = offset(clicks, rate, 0.2).ok_or(Unheard::Clicks)?;
    // Where the sound starts in the recording, from the first output's clicks on the "both" beats
    // (0.8 s into every bar): every other burst is placed from there.
    let low = onsets(clicks, rate, PITCH_HZ[0]);
    let high = onsets(clicks, rate, PITCH_HZ[1]);
    let both_beat = clicks_at() + 2.0 * BEAT_MS as f64 / 1000.0;
    let mut lags: Vec<f64> = low
        .iter()
        .filter(|&&l| high.iter().any(|h| (h - l).abs() <= 0.2))
        .map(|&l| (l - both_beat).rem_euclid(BAR_S))
        .collect();
    if lags.is_empty() {
        return Err(Unheard::Clicks);
    }
    lags.sort_by(f64::total_cmp);
    let lag = lags[lags.len() / 2];
    // Loudness: the loudest 5 ms of each burst (and its echo), against the room.
    let win = (rate as usize / 200).max(1);
    let level_at = |t: f64| -> f64 {
        let from = ((t - 0.005).max(0.0) * rate as f64) as usize;
        let to = (((t + 0.045) * rate as f64) as usize).min(samples.len());
        samples.get(from..to).map_or(0.0, |s| s.chunks(win).map(|c| c.iter().map(|x| (x * x) as f64).sum::<f64>() / c.len() as f64).fold(0.0, f64::max))
    };
    let mut room: Vec<f64> = samples.chunks(win).map(|c| c.iter().map(|x| (x * x) as f64).sum::<f64>() / c.len() as f64).collect();
    room.sort_by(f64::total_cmp);
    let room = room[room.len() / 2].max(1e-12);
    let mut diffs: Vec<f64> = (0..PAIRS)
        .filter_map(|k| {
            let t1 = bursts_at() + k as f64 * PAIR_S + lag;
            let (p1, p2) = (level_at(t1), level_at(t1 + PAIR_S / 2.0 + offset_s));
            (p1 > room * HEARD_OVER_ROOM && p2 > room * HEARD_OVER_ROOM).then(|| 10.0 * (p2 / p1).log10())
        })
        .collect();
    if diffs.is_empty() {
        return Err(Unheard::Bursts);
    }
    diffs.sort_by(f64::total_cmp);
    Ok(Measurement { offset_s, louder_db: diffs[diffs.len() / 2] })
}

/// A burst counts as heard this many times above the room's power (about 10 dB).
const HEARD_OVER_ROOM: f64 = 10.0;

/// New delays (ms, first and second) and balance (dB, + = second turned down) from what was
/// measured while `delays` and `balance_db` were in force.
pub fn corrected(delays: [u32; 2], balance_db: f32, m: Measurement) -> ([u32; 2], f32) {
    use crate::audio::{dual::MAX_BALANCE_DB, player::MAX_DELAY_MS};
    // Offset = how much later the second output is held back than the first.
    let offset_ms = delays[1] as f64 - delays[0] as f64 - m.offset_s * 1000.0;
    let ms = (offset_ms.abs().round() as u32).min(MAX_DELAY_MS);
    let delays = if offset_ms < 0.0 { [ms, 0] } else { [0, ms] };
    let balance = ((balance_db as f64 + m.louder_db) as f32).clamp(-MAX_BALANCE_DB, MAX_BALANCE_DB);
    (delays, (balance * 2.0).round() / 2.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::player::MAX_DELAY_MS;

    const RATE: u32 = 48_000;

    fn rms(all: &[f32], from: f64, to: f64, ch: usize) -> f32 {
        let frames = &all[(from * RATE as f64) as usize * 2..(to * RATE as f64) as usize * 2];
        (frames.chunks_exact(2).map(|f| f[ch] * f[ch]).sum::<f32>() / (frames.len() / 2).max(1) as f32).sqrt()
    }

    #[test]
    fn the_sound_is_silence_then_clicks_then_short_bursts_on_each_output_in_turn() {
        let mut s = CalibrationSignal::new(RATE, 2);
        let all = s.next((duration_s() * RATE as f64) as usize + RATE as usize);
        assert_eq!(rms(&all, 0.0, SILENCE_S, 0) + rms(&all, 0.0, SILENCE_S, 1), 0.0);
        assert!(rms(&all, clicks_at(), clicks_at() + 0.01, 0) > 0.05, "a low click on the first output");
        let b = bursts_at();
        let first = rms(&all, b, b + BURST_S, 0);
        let second = rms(&all, b + PAIR_S / 2.0, b + PAIR_S / 2.0 + BURST_S, 1);
        assert!(first > 0.05 && second > 0.05);
        assert!((first / second - 1.0).abs() < 0.1, "the same level on both: {first} {second}");
        assert_eq!(rms(&all, b, b + BURST_S, 1), 0.0, "each burst on one output only");
        assert_eq!(rms(&all, b + BURST_S + 0.01, b + PAIR_S / 2.0, 0), 0.0, "short, then silence");
        assert_eq!(rms(&all, duration_s(), duration_s() + 0.5, 0), 0.0, "silent once over");
    }

    /// A recording of the sound in a room: the first output arrives 0.43 s after the recording
    /// starts at `level1`, the second `apart` later than the first at `level2`; room noise and
    /// an echo. `gated`: every third burst is lost, as a gating headset microphone does.
    fn room(apart: f64, level1: f32, level2: f32, gated: bool) -> Vec<f32> {
        let lag1 = 0.430;
        let mut sound = CalibrationSignal::new(RATE, 2);
        let mut played = sound.next((duration_s() * RATE as f64) as usize);
        if gated {
            for k in (0..PAIRS * 2).step_by(3) {
                let at = bursts_at() + k as f64 * PAIR_S / 2.0;
                let (from, to) = ((at * RATE as f64) as usize, ((at + BURST_S) * RATE as f64) as usize);
                for f in played[from * 2..to * 2].iter_mut() {
                    *f = 0.0;
                }
            }
        }
        let len = ((duration_s() + 1.5) * RATE as f64) as usize;
        let mut seed = 5u64;
        let mut out: Vec<f32> = (0..len)
            .map(|_| {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                ((seed >> 33) as f32 / (1u64 << 31) as f32 - 0.5) * 0.004
            })
            .collect();
        for (ch, at, level) in [(0usize, lag1, level1), (1, lag1 + apart, level2)] {
            for (delay, gain) in [(0.0, 1.0), (0.015, 0.3)] {
                let start = ((at + delay) * RATE as f64) as usize;
                for (i, f) in played.chunks_exact(2).enumerate() {
                    if let Some(s) = out.get_mut(start + i) {
                        *s += f[ch] * level * gain;
                    }
                }
            }
        }
        out
    }

    #[test]
    fn hears_how_far_apart_and_how_much_louder_the_outputs_are() {
        for (apart, l1, l2) in [(0.0, 0.05, 0.05), (0.043, 0.05, 0.1), (-0.120, 0.08, 0.02)] {
            let m = analyze(&room(apart, l1, l2, false), RATE).expect("heard");
            let want_db = 20.0 * (l2 as f64 / l1 as f64).log10();
            assert!((m.offset_s - apart).abs() < 0.002, "offset: want {apart}, got {}", m.offset_s);
            assert!((m.louder_db - want_db).abs() < 1.0, "level: want {want_db:.1} dB, got {:.1}", m.louder_db);
        }
    }

    #[test]
    fn bursts_the_microphone_gated_out_do_not_spoil_the_loudness() {
        let m = analyze(&room(0.030, 0.05, 0.1, true), RATE).expect("heard");
        assert!((m.louder_db - 6.0).abs() < 1.0, "{}", m.louder_db);
    }

    #[test]
    fn a_microphone_that_hears_nothing_says_so() {
        assert_eq!(analyze(&room(0.0, 0.0, 0.0, false), RATE), Err(Unheard::Clicks));
    }

    #[test]
    fn the_late_output_gets_less_delay_or_the_early_one_more() {
        let m = |offset_s, louder_db| Measurement { offset_s, louder_db };
        // The second sounds 40 ms late with 10 ms of its own delay: take that off, then delay the first.
        assert_eq!(corrected([0, 10], 0.0, m(0.040, 0.0)).0, [30, 0]);
        // The second sounds 25 ms early: it waits 25 ms more.
        assert_eq!(corrected([0, 10], 0.0, m(-0.025, 0.0)).0, [0, 35]);
        assert_eq!(corrected([0, 0], 0.0, m(-2.0, 0.0)).0, [0, MAX_DELAY_MS], "kept within range");
    }

    #[test]
    fn the_louder_output_is_turned_down_by_the_difference() {
        let m = |louder_db| Measurement { offset_s: 0.0, louder_db };
        assert_eq!(corrected([0, 0], 0.0, m(4.0)).1, 4.0, "second louder: turn it down");
        assert_eq!(corrected([0, 0], 4.0, m(-1.5)).1, 2.5, "second now quieter: turn it back up a bit");
        assert_eq!(corrected([0, 0], 0.0, m(-6.0)).1, -6.0, "first louder: turn the first down");
        assert_eq!(corrected([0, 0], 20.0, m(10.0)).1, crate::audio::dual::MAX_BALANCE_DB);
    }
}
