//! 10-band graphic equalizer + preamp (F7.7): RBJ peaking biquads, f64 state, applied in the
//! device callback. Off or flat ⇒ the samples are not touched at all (bit-identical).

use serde::{Deserialize, Serialize};

pub const BANDS_HZ: [f32; 10] = [31.0, 62.0, 125.0, 250.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0, 16_000.0];
pub const MAX_DB: f32 = 12.0;
const Q: f64 = std::f64::consts::SQRT_2; // one octave per band

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EqSettings {
    pub enabled: bool,
    pub preamp_db: f32,
    pub gains_db: [f32; 10],
}

impl Default for EqSettings {
    fn default() -> Self {
        Self { enabled: true, preamp_db: 0.0, gains_db: [0.0; 10] }
    }
}

impl EqSettings {
    pub fn clamped(mut self) -> Self {
        self.preamp_db = self.preamp_db.clamp(-MAX_DB, MAX_DB);
        for g in &mut self.gains_db {
            *g = g.clamp(-MAX_DB, MAX_DB);
        }
        self
    }

    /// Whether processing changes anything at all.
    pub fn is_active(&self) -> bool {
        self.enabled && (self.preamp_db != 0.0 || self.gains_db.iter().any(|&g| g != 0.0))
    }
}

#[derive(Clone, Copy)]
struct Coeffs {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

fn peaking(rate: u32, freq: f32, gain_db: f32) -> Coeffs {
    let a = 10f64.powf(gain_db as f64 / 40.0);
    let w0 = 2.0 * std::f64::consts::PI * freq as f64 / rate as f64;
    let alpha = w0.sin() / (2.0 * Q);
    let cos = w0.cos();
    let a0 = 1.0 + alpha / a;
    Coeffs {
        b0: (1.0 + alpha * a) / a0,
        b1: (-2.0 * cos) / a0,
        b2: (1.0 - alpha * a) / a0,
        a1: (-2.0 * cos) / a0,
        a2: (1.0 - alpha / a) / a0,
    }
}

/// One band: coefficients plus transposed-direct-form-II state per channel.
struct Band {
    c: Coeffs,
    z: Vec<[f64; 2]>,
}

pub struct Equalizer {
    rate: u32,
    channels: usize,
    active: bool,
    preamp: f64,
    bands: Vec<Band>,
}

impl Equalizer {
    pub fn new(rate: u32, channels: usize) -> Self {
        Self { rate, channels, active: false, preamp: 1.0, bands: Vec::new() }
    }

    pub fn set(&mut self, settings: &EqSettings) {
        let s = settings.clamped();
        self.active = s.is_active();
        self.preamp = 10f64.powf(s.preamp_db as f64 / 20.0);
        self.bands = BANDS_HZ
            .iter()
            .zip(s.gains_db)
            // Flat bands cost nothing; bands too close to Nyquist are skipped.
            .filter(|&(&f, g)| g != 0.0 && f < 0.45 * self.rate as f32)
            .map(|(&f, g)| Band { c: peaking(self.rate, f, g), z: vec![[0.0; 2]; self.channels] })
            .collect();
    }

    /// Filter interleaved samples in place.
    pub fn process(&mut self, samples: &mut [f32]) {
        if !self.active {
            return;
        }
        for frame in samples.chunks_exact_mut(self.channels) {
            for (ch, s) in frame.iter_mut().enumerate() {
                let mut x = *s as f64 * self.preamp;
                for band in &mut self.bands {
                    let (c, z) = (band.c, &mut band.z[ch]);
                    let y = c.b0 * x + z[0];
                    z[0] = c.b1 * x - c.a1 * y + z[1];
                    z[1] = c.b2 * x - c.a2 * y;
                    x = y;
                }
                *s = x as f32;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, freq: f32, secs: f32, amp: f32, channels: usize) -> Vec<f32> {
        let n = (rate as f32 * secs) as usize;
        (0..n)
            .flat_map(|i| std::iter::repeat((2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin() * amp).take(channels))
            .collect()
    }

    /// RMS of channel 0 after the first 0.2 s (filters settled), in dB.
    fn rms_db(samples: &[f32], channels: usize, rate: u32) -> f32 {
        let ch0: Vec<f32> = samples.iter().step_by(channels).skip(rate as usize / 5).copied().collect();
        let rms = (ch0.iter().map(|s| s * s).sum::<f32>() / ch0.len() as f32).sqrt();
        20.0 * rms.log10()
    }

    fn settings(preamp: f32, gains: [f32; 10]) -> EqSettings {
        EqSettings { enabled: true, preamp_db: preamp, gains_db: gains }
    }

    #[test]
    fn flat_eq_leaves_samples_bit_identical() {
        let input = tone(48_000, 1_000.0, 0.1, 0.3, 2);
        let mut out = input.clone();
        let mut eq = Equalizer::new(48_000, 2);
        eq.set(&settings(0.0, [0.0; 10]));
        eq.process(&mut out);
        assert_eq!(out, input);
    }

    #[test]
    fn a_disabled_eq_leaves_samples_bit_identical_whatever_its_gains() {
        let input = tone(48_000, 1_000.0, 0.1, 0.3, 2);
        let mut out = input.clone();
        let mut eq = Equalizer::new(48_000, 2);
        eq.set(&EqSettings { enabled: false, preamp_db: -6.0, gains_db: [12.0; 10] });
        eq.process(&mut out);
        assert_eq!(out, input);
    }

    #[test]
    fn six_db_at_1_khz_raises_a_1_khz_tone_by_six_db() {
        let rate = 48_000;
        let input = tone(rate, 1_000.0, 1.0, 0.1, 2);
        let mut out = input.clone();
        let mut eq = Equalizer::new(rate, 2);
        let mut g = [0.0; 10];
        g[BANDS_HZ.iter().position(|&f| f == 1_000.0).unwrap()] = 6.0;
        eq.set(&settings(0.0, g));
        eq.process(&mut out);
        let gain = rms_db(&out, 2, rate) - rms_db(&input, 2, rate);
        assert!((gain - 6.0).abs() <= 0.5, "gain {gain:.2} dB");
    }

    #[test]
    fn a_boost_at_1_khz_leaves_100_hz_alone() {
        let rate = 44_100;
        let input = tone(rate, 100.0, 1.0, 0.1, 1);
        let mut out = input.clone();
        let mut eq = Equalizer::new(rate, 1);
        let mut g = [0.0; 10];
        g[5] = 6.0; // 1 kHz
        eq.set(&settings(0.0, g));
        eq.process(&mut out);
        let gain = rms_db(&out, 1, rate) - rms_db(&input, 1, rate);
        assert!(gain.abs() <= 0.5, "100 Hz moved by {gain:.2} dB");
    }

    #[test]
    fn preamp_scales_everything() {
        let rate = 48_000;
        let input = tone(rate, 440.0, 0.5, 0.2, 2);
        let mut out = input.clone();
        let mut eq = Equalizer::new(rate, 2);
        eq.set(&settings(-4.0, [0.0; 10]));
        eq.process(&mut out);
        let gain = rms_db(&out, 2, rate) - rms_db(&input, 2, rate);
        assert!((gain + 4.0).abs() <= 0.05, "preamp gain {gain:.3} dB");
    }

    #[test]
    fn channels_are_filtered_independently() {
        // Left: 1 kHz tone, right: silence. A boost must not leak into the right channel.
        let rate = 48_000;
        let mut out: Vec<f32> = tone(rate, 1_000.0, 0.2, 0.1, 1).into_iter().flat_map(|s| [s, 0.0]).collect();
        let mut eq = Equalizer::new(rate, 2);
        eq.set(&settings(0.0, [6.0; 10]));
        eq.process(&mut out);
        assert!(out.iter().skip(1).step_by(2).all(|&r| r == 0.0));
    }

    #[test]
    fn there_are_ten_bands_from_31_hz_to_16_khz() {
        assert_eq!(BANDS_HZ, [31.0, 62.0, 125.0, 250.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0, 16_000.0]);
    }

    #[test]
    fn gains_are_clamped_to_plus_minus_12_db() {
        let s = EqSettings { enabled: true, preamp_db: -30.0, gains_db: [20.0; 10] }.clamped();
        assert_eq!(s.gains_db, [12.0; 10]);
        assert_eq!(s.preamp_db, -12.0);
    }
}
