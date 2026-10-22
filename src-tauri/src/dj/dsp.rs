//! The DJ mixer's sound shaping (D.2): 3-band EQ that can kill a band, the one-knob filter,
//! gain trim, channel fader and crossfader curves. Stereo interleaved, processed in place.

use std::f32::consts::{FRAC_1_SQRT_2, PI};

/// Where the EQ splits the bands (Hz).
const LOW_MID: f32 = 300.0;
const MID_HIGH: f32 = 3000.0;

/// A second-order section (RBJ cookbook), one state per stereo channel.
#[derive(Clone, Copy, Default)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    z: [[f32; 2]; 2],
}

impl Biquad {
    fn lowpass(rate: u32, hz: f32, q: f32) -> Self {
        let mut f = Self::default();
        f.set_lowpass(rate, hz, q);
        f
    }

    fn highpass(rate: u32, hz: f32, q: f32) -> Self {
        let mut f = Self::default();
        f.set_highpass(rate, hz, q);
        f
    }

    fn set_lowpass(&mut self, rate: u32, hz: f32, q: f32) {
        let (cos, alpha) = Self::omega(rate, hz, q);
        let a0 = 1.0 + alpha;
        let b1 = (1.0 - cos) / a0;
        self.b = [b1 / 2.0, b1, b1 / 2.0];
        self.a = [-2.0 * cos / a0, (1.0 - alpha) / a0];
    }

    fn set_highpass(&mut self, rate: u32, hz: f32, q: f32) {
        let (cos, alpha) = Self::omega(rate, hz, q);
        let a0 = 1.0 + alpha;
        let b1 = (1.0 + cos) / a0;
        self.b = [b1 / 2.0, -b1, b1 / 2.0];
        self.a = [-2.0 * cos / a0, (1.0 - alpha) / a0];
    }

    fn omega(rate: u32, hz: f32, q: f32) -> (f32, f32) {
        let w = 2.0 * PI * hz.min(rate as f32 * 0.45) / rate as f32;
        (w.cos(), w.sin() / (2.0 * q))
    }

    #[inline]
    fn tick(&mut self, ch: usize, x: f32) -> f32 {
        let z = &mut self.z[ch];
        let y = self.b[0] * x + z[0];
        z[0] = self.b[1] * x - self.a[0] * y + z[1];
        z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// Linkwitz-Riley 4th order: two Butterworth sections; low + high add up to an all-pass.
#[derive(Clone, Copy)]
struct Crossover {
    lp: [Biquad; 2],
    hp: [Biquad; 2],
}

impl Crossover {
    fn new(rate: u32, hz: f32) -> Self {
        let lp = Biquad::lowpass(rate, hz, FRAC_1_SQRT_2);
        let hp = Biquad::highpass(rate, hz, FRAC_1_SQRT_2);
        Self { lp: [lp, lp], hp: [hp, hp] }
    }

    #[inline]
    fn split(&mut self, ch: usize, x: f32) -> (f32, f32) {
        let l = self.lp[0].tick(ch, x);
        let low = self.lp[1].tick(ch, l);
        let h = self.hp[0].tick(ch, x);
        let high = self.hp[1].tick(ch, h);
        (low, high)
    }
}

/// -1 = kill (silent), 0 = flat, 1 = +6 dB.
fn band_gain(v: f32) -> f32 {
    let v = v.clamp(-1.0, 1.0);
    if v < 0.0 {
        (1.0 + v) * (1.0 + v)
    } else {
        10f32.powf(6.0 * v / 20.0)
    }
}

/// The DJ 3-band EQ: the bands are split with crossovers, so turning one fully down removes
/// it (a "kill"), which a shelving EQ cannot do.
pub struct ThreeBand {
    low_mid: Crossover,
    mid_high: Crossover,
    /// Passes the low band through the same phase shift the mid and high bands get at 3 kHz,
    /// so the three add back up to the original level.
    low_align: Crossover,
    gains: [f32; 3],
}

impl ThreeBand {
    pub fn new(rate: u32) -> Self {
        Self {
            low_mid: Crossover::new(rate, LOW_MID),
            mid_high: Crossover::new(rate, MID_HIGH),
            low_align: Crossover::new(rate, MID_HIGH),
            gains: [1.0; 3],
        }
    }

    /// Each -1 (kill) .. 0 (flat) .. 1 (+6 dB).
    pub fn set(&mut self, low: f32, mid: f32, high: f32) {
        self.gains = [band_gain(low), band_gain(mid), band_gain(high)];
    }

    pub fn process(&mut self, stereo: &mut [f32]) {
        let [gl, gm, gh] = self.gains;
        for frame in stereo.chunks_exact_mut(2) {
            for (ch, s) in frame.iter_mut().enumerate() {
                let (low, rest) = self.low_mid.split(ch, *s);
                let (a, b) = self.low_align.split(ch, low);
                let (mid, high) = self.mid_high.split(ch, rest);
                *s = gl * (a + b) + gm * mid + gh * high;
            }
        }
    }
}

/// The one-knob filter: left of centre a low-pass closes down to 200 Hz, right of centre a
/// high-pass opens up to 8 kHz; the centre is off.
pub struct DjFilter {
    rate: u32,
    value: f32,
    stages: [Biquad; 2],
}

/// Below this the knob counts as centred.
const FILTER_DEAD: f32 = 0.02;

impl DjFilter {
    pub fn new(rate: u32) -> Self {
        Self { rate, value: 0.0, stages: [Biquad::default(); 2] }
    }

    pub fn set(&mut self, value: f32) {
        let v = value.clamp(-1.0, 1.0);
        if (v < 0.0) != (self.value < 0.0) || self.value.abs() < FILTER_DEAD {
            self.stages = [Biquad::default(); 2]; // switching sides starts from silence
        }
        self.value = v;
        // A little resonance (Q 1) gives the sweep its DJ sound.
        for (i, s) in self.stages.iter_mut().enumerate() {
            let q = if i == 0 { 1.0 } else { FRAC_1_SQRT_2 };
            if v < 0.0 {
                s.set_lowpass(self.rate, 20_000.0 * (200.0f32 / 20_000.0).powf(-v), q);
            } else {
                s.set_highpass(self.rate, 20.0 * (8000.0f32 / 20.0).powf(v), q);
            }
        }
    }

    pub fn process(&mut self, stereo: &mut [f32]) {
        if self.value.abs() < FILTER_DEAD {
            return;
        }
        for frame in stereo.chunks_exact_mut(2) {
            for (ch, s) in frame.iter_mut().enumerate() {
                let y = self.stages[0].tick(ch, *s);
                *s = self.stages[1].tick(ch, y);
            }
        }
    }
}

/// Beat echo: repeats of what went in while it was on, each a little quieter. Turned off, the
/// repeats already in it ring on and die away (the "echo out" of a song).
pub struct Echo {
    buf: Vec<f32>,
    pos: usize,
    delay: usize,
    rate: u32,
    on: bool,
    /// Frames until the tail has died away once off (no work after that).
    ringing: usize,
}

const ECHO_FEEDBACK: f32 = 0.45;
const ECHO_MAX_S: f32 = 2.0;

impl Echo {
    pub fn new(rate: u32) -> Self {
        let frames = (rate as f32 * ECHO_MAX_S) as usize;
        Self { buf: vec![0.0; frames * 2], pos: 0, delay: rate as usize / 4, rate, on: false, ringing: 0 }
    }

    pub fn set_delay_s(&mut self, s: f32) {
        let frames = self.buf.len() / 2;
        self.delay = ((s * self.rate as f32) as usize).clamp(1, frames - 1);
    }

    pub fn set_on(&mut self, on: bool) {
        self.on = on;
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    /// Feeds `input` (while on) and adds the repeats to `out`; both stereo, same length.
    pub fn process(&mut self, input: &[f32], out: &mut [f32]) {
        let frames = self.buf.len() / 2;
        if self.on {
            self.ringing = self.delay * 12; // 0.45^12: well under hearing
        } else if self.ringing == 0 {
            return;
        }
        for (inp, o) in input.chunks_exact(2).zip(out.chunks_exact_mut(2)) {
            let read = (self.pos + frames - self.delay) % frames;
            for ch in 0..2 {
                let d = self.buf[read * 2 + ch];
                o[ch] += d;
                let feed = if self.on { inp[ch] } else { 0.0 };
                self.buf[self.pos * 2 + ch] = feed + d * ECHO_FEEDBACK;
            }
            self.pos = (self.pos + 1) % frames;
            self.ringing = self.ringing.saturating_sub(1);
        }
        if !self.on && self.ringing == 0 {
            self.buf.fill(0.0);
        }
    }
}

/// Gain trim: 0..1 with 0.5 = unity, ±12 dB at the ends.
pub fn gain(v: f32) -> f32 {
    10f32.powf((v.clamp(0.0, 1.0) - 0.5) * 24.0 / 20.0)
}

/// Channel fader 0..1 with an audio taper (squared), so the bottom half is quiet.
pub fn fader(v: f32) -> f32 {
    let v = v.clamp(0.0, 1.0);
    v * v
}

/// Gains of deck A and B for a crossfader at -1 (A) .. 1 (B): constant power, so a mix in the
/// middle is as loud as either song alone.
pub fn crossfader(x: f32) -> (f32, f32) {
    let t = (x.clamp(-1.0, 1.0) + 1.0) / 2.0 * PI / 2.0;
    if t <= 0.0 {
        return (1.0, 0.0);
    }
    (t.cos(), t.sin())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    const RATE: u32 = 48_000;

    fn tone(hz: f32, secs: f32) -> Vec<f32> {
        let n = (RATE as f32 * secs) as usize;
        (0..n).flat_map(|i| {
            let s = (2.0 * PI * hz * i as f32 / RATE as f32).sin() * 0.5;
            [s, s]
        }).collect()
    }

    /// Level in dB of the second half (filters settled) against a sine of amplitude 0.5.
    fn level_db(x: &[f32]) -> f32 {
        let tail = &x[x.len() / 2..];
        let rms = (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32).sqrt();
        20.0 * (rms / (0.5 / 2f32.sqrt())).log10()
    }

    fn through_eq(hz: f32, low: f32, mid: f32, high: f32) -> f32 {
        let mut eq = ThreeBand::new(RATE);
        eq.set(low, mid, high);
        let mut x = tone(hz, 0.5);
        eq.process(&mut x);
        level_db(&x)
    }

    #[test]
    fn a_flat_eq_keeps_every_frequency_at_its_level() {
        for hz in [40.0, 300.0, 1000.0, 3000.0, 12_000.0] {
            let db = through_eq(hz, 0.0, 0.0, 0.0);
            assert!(db.abs() < 0.1, "{hz} Hz: {db} dB");
        }
    }

    #[test]
    fn each_band_can_be_killed_or_boosted() {
        assert!(through_eq(60.0, -1.0, 0.0, 0.0) < -20.0, "bass killed");
        assert!(through_eq(1000.0, -1.0, 0.0, 0.0).abs() < 1.5, "mids stay when the bass is killed");
        assert!(through_eq(1000.0, 0.0, -1.0, 0.0) < -15.0, "mids killed");
        assert!(through_eq(10_000.0, 0.0, 0.0, -1.0) < -20.0, "highs killed");
        assert!(through_eq(60.0, 0.0, 0.0, -1.0).abs() < 1.5, "bass stays when the highs are killed");
        let boosted = through_eq(60.0, 1.0, 0.0, 0.0);
        assert!((boosted - 6.0).abs() < 1.5, "full boost is about +6 dB, got {boosted}");
    }

    #[test]
    fn the_filter_knob_goes_from_low_pass_through_off_to_high_pass() {
        let run = |hz: f32, v: f32| {
            let mut f = DjFilter::new(RATE);
            f.set(v);
            let mut x = tone(hz, 0.5);
            f.process(&mut x);
            level_db(&x)
        };
        assert!(run(5000.0, 0.0).abs() < 0.1 && run(80.0, 0.0).abs() < 0.1, "off at the centre");
        assert!(run(5000.0, -0.8) < -20.0 && run(80.0, -0.8).abs() < 2.0, "turned left: low-pass");
        assert!(run(80.0, 0.8) < -20.0 && run(5000.0, 0.8).abs() < 2.0, "turned right: high-pass");
    }

    #[test]
    fn gain_trim_and_faders() {
        assert!((gain(0.5) - 1.0).abs() < 1e-6, "half way is unity");
        assert!((20.0 * gain(1.0).log10() - 12.0).abs() < 0.01);
        assert!((20.0 * gain(0.0).log10() + 12.0).abs() < 0.01);
        assert_eq!(fader(0.0), 0.0);
        assert_eq!(fader(1.0), 1.0);
        assert!(fader(0.5) < 0.5, "a fader curve gets quiet quickly at the bottom");
    }

    #[test]
    fn the_crossfader_keeps_the_loudness_steady() {
        assert_eq!(crossfader(-1.0), (1.0, 0.0));
        let (a, b) = crossfader(1.0);
        assert!(a.abs() < 1e-6 && (b - 1.0).abs() < 1e-6);
        for x in [-0.6f32, -0.2, 0.0, 0.3, 0.9] {
            let (a, b) = crossfader(x);
            assert!((a * a + b * b - 1.0).abs() < 1e-5, "constant power at {x}");
        }
        let (a, b) = crossfader(0.0);
        assert!((a - b).abs() < 1e-6);
    }
}
