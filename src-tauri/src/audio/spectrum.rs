//! Frequency bands for the visualizer (F7.6): 2048-point FFT with a Hann window, log-spaced
//! bands from 40 Hz to 16 kHz, levels mapped from -70..0 dB onto 0..1.

use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

const N: usize = 2048;
const LOW_HZ: f32 = 40.0;
const HIGH_HZ: f32 = 16_000.0;
const FLOOR_DB: f32 = -70.0;

pub struct Spectrum {
    rate: u32,
    edges: Vec<f32>,
    window: Vec<f32>,
    fft: Arc<dyn Fft<f32>>,
    buf: Vec<Complex<f32>>,
}

impl Spectrum {
    pub fn new(rate: u32, bands: usize) -> Self {
        let ratio = (HIGH_HZ / LOW_HZ).powf(1.0 / bands as f32);
        let edges = (0..=bands).map(|i| LOW_HZ * ratio.powi(i as i32)).collect();
        let window = (0..N).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / (N - 1) as f32).cos()).collect();
        let fft = FftPlanner::new().plan_fft_forward(N);
        Self { rate, edges, window, fft, buf: vec![Complex::default(); N] }
    }

    pub fn band_edges(&self) -> &[f32] {
        &self.edges
    }

    /// Band levels (0..=1) for the most recent mono samples (uses the last 2048; pads shorter input).
    pub fn levels(&mut self, mono: &[f32]) -> Vec<f32> {
        let tail = &mono[mono.len().saturating_sub(N)..];
        for (i, slot) in self.buf.iter_mut().enumerate() {
            let s = tail.get(i).copied().unwrap_or(0.0);
            *slot = Complex::new(s * self.window[i], 0.0);
        }
        self.fft.process(&mut self.buf);
        // A full-scale sine gives |X| ≈ N/4 with a Hann window; scale so that reads as 1.0.
        let scale = 4.0 / N as f32;
        let bin_hz = self.rate as f32 / N as f32;
        let mags: Vec<f32> = self.buf[..N / 2].iter().map(|c| c.norm() * scale).collect();
        self.edges
            .windows(2)
            .map(|w| {
                let (lo, hi) = ((w[0] / bin_hz).ceil() as usize, (w[1] / bin_hz).ceil() as usize);
                let peak = if lo < hi.min(mags.len()) {
                    mags[lo..hi.min(mags.len())].iter().copied().fold(0.0, f32::max)
                } else {
                    // Band narrower than one bin (low end): use the nearest bin.
                    mags[(((w[0] + w[1]) / 2.0 / bin_hz).round() as usize).min(mags.len() - 1)]
                };
                let db = 20.0 * peak.max(1e-9).log10();
                ((db - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0)
            })
            .collect()
    }
}

/// Decides when to send a visualizer frame: only while the visualizer is on, the mini player is
/// visible and music is playing — and at most every 33 ms (~30 fps). Otherwise no work at all.
#[derive(Debug, Default)]
pub struct FrameGate {
    last_ms: Option<u64>,
}

impl FrameGate {
    pub fn tick(&mut self, now_ms: u64, enabled: bool, visible: bool, playing: bool) -> bool {
        if !(enabled && visible && playing) {
            return false;
        }
        if self.last_ms.is_some_and(|last| now_ms.saturating_sub(last) < 33) {
            return false;
        }
        self.last_ms = Some(now_ms);
        true
    }
}

/// 0..=1 levels as bytes, to keep the 30-per-second UI events small.
pub fn to_bytes(levels: &[f32]) -> Vec<u8> {
    levels.iter().map(|l| (l.clamp(0.0, 1.0) * 255.0).round() as u8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(rate: u32, freq: f32, n: usize, amp: f32) -> Vec<f32> {
        (0..n).map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin() * amp).collect()
    }

    fn loudest(levels: &[f32]) -> usize {
        levels.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0
    }

    #[test]
    fn bands_are_log_spaced_from_40_hz_to_16_khz() {
        let s = Spectrum::new(48_000, 28);
        let edges = s.band_edges();
        assert_eq!(edges.len(), 29);
        assert!((edges[0] - 40.0).abs() < 0.5);
        assert!((edges[28] - 16_000.0).abs() < 1.0);
        let ratios: Vec<f32> = edges.windows(2).map(|w| w[1] / w[0]).collect();
        assert!(ratios.iter().all(|r| (r - ratios[0]).abs() < 1e-3), "equal ratio per band");
    }

    #[test]
    fn a_1_khz_tone_peaks_in_the_band_containing_1_khz() {
        let mut s = Spectrum::new(48_000, 28);
        let levels = s.levels(&tone(48_000, 1_000.0, 2048, 0.5));
        let band = loudest(&levels);
        let e = s.band_edges();
        assert!(e[band] <= 1_000.0 && 1_000.0 < e[band + 1], "peak in band {band} ({}-{} Hz)", e[band], e[band + 1]);
    }

    #[test]
    fn low_and_high_tones_land_in_different_bands() {
        let mut s = Spectrum::new(44_100, 28);
        let low = loudest(&s.levels(&tone(44_100, 100.0, 2048, 0.5)));
        let high = loudest(&s.levels(&tone(44_100, 8_000.0, 2048, 0.5)));
        assert!(low < 8 && high > 20, "low {low}, high {high}");
    }

    #[test]
    fn silence_reads_as_nothing_and_levels_stay_in_range() {
        let mut s = Spectrum::new(48_000, 28);
        assert!(s.levels(&vec![0.0; 2048]).iter().all(|&l| l < 0.01));
        assert!(s.levels(&tone(48_000, 440.0, 2048, 1.0)).iter().all(|&l| (0.0..=1.0).contains(&l)));
    }

    #[test]
    fn short_input_is_zero_padded_not_a_panic() {
        let mut s = Spectrum::new(48_000, 28);
        assert_eq!(s.levels(&tone(48_000, 1_000.0, 100, 0.5)).len(), 28);
    }

    #[test]
    fn levels_pack_into_bytes_for_the_ui() {
        assert_eq!(to_bytes(&[0.0, 0.5, 1.0, 1.7]), vec![0, 128, 255, 255]);
    }
}

#[cfg(test)]
mod gate_tests {
    use super::*;

    #[test]
    fn frames_flow_only_when_on_visible_and_playing() {
        let mut g = FrameGate::default();
        assert!(g.tick(0, true, true, true));
        let mut g = FrameGate::default();
        assert!(!g.tick(0, false, true, true), "visualizer off");
        assert!(!g.tick(0, true, false, true), "mini player hidden");
        assert!(!g.tick(0, true, true, false), "paused");
    }

    #[test]
    fn at_most_about_30_frames_a_second() {
        let mut g = FrameGate::default();
        assert!(g.tick(1_000, true, true, true));
        assert!(!g.tick(1_020, true, true, true));
        assert!(g.tick(1_034, true, true, true));
        let sent = (0..1_000).step_by(5).filter(|t| g.tick(2_000 + t, true, true, true)).count();
        assert!((28..=31).contains(&sent), "{sent} frames in 1 s");
    }
}
