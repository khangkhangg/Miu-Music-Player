//! Key lock (D.7): plays a song faster or slower without changing its pitch, or shifts its
//! pitch without changing its tempo. WSOLA: short overlapping grains of the song are laid one
//! after another, each picked within a few milliseconds of where it should come from so that it
//! lines up with the sound before it (no phase jumps), then read at the pitch wanted.

use std::collections::VecDeque;

/// Grain length and how far either way a grain may move to line up (seconds).
const GRAIN_S: f64 = 0.04;
const SEARCH_S: f64 = 0.012;

pub struct Stretcher {
    /// Grain length in output frames, and the hop between grains (half a grain).
    n: usize,
    hop: usize,
    window: Vec<f32>,
    search: f64,
    /// Overlap-add buffer (stereo, one grain long) and finished frames waiting to go out.
    ola: Vec<f32>,
    ready: VecDeque<f32>,
    /// Where the next grain should come from in the source (frames), and where the last one did.
    pos: f64,
    prev_start: Option<f64>,
    /// Source frames per output frame of playing time, at the last render (for `position`).
    rate_factor: f64,
}

#[inline]
fn hermite(a: f32, b: f32, c: f32, d: f32, t: f32) -> f32 {
    let c1 = 0.5 * (c - a);
    let c2 = a - 2.5 * b + 2.0 * c - 0.5 * d;
    let c3 = 0.5 * (d - a) + 1.5 * (b - c);
    ((c3 * t + c2) * t + c1) * t + b
}

impl Stretcher {
    pub fn new(rate: u32) -> Self {
        let n = ((GRAIN_S * rate as f64) as usize / 2 * 2).max(64);
        let window = (0..n).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / n as f32).cos()).collect();
        Self {
            n,
            hop: n / 2,
            window,
            search: SEARCH_S * rate as f64,
            ola: vec![0.0; n * 2],
            ready: VecDeque::new(),
            pos: 0.0,
            prev_start: None,
            rate_factor: 1.0,
        }
    }

    /// Start again from source frame `pos` (after a seek, cue, loop or hot cue).
    pub fn reset(&mut self, pos: f64) {
        self.ola.fill(0.0);
        self.ready.clear();
        self.prev_start = None;
        self.pos = pos;
    }

    /// The source frame being heard now (what is still buffered taken off).
    pub fn position(&self) -> f64 {
        let pending = if self.prev_start.is_some() { self.hop } else { 0 };
        let buffered = (self.ready.len() / 2 + pending) as f64;
        (self.pos - buffered * self.rate_factor).max(0.0)
    }

    /// Fills `out` (stereo) from `src(frame, channel)`: `tempo` = playing speed, `pitch` = pitch
    /// factor (1 unchanged), `src_per_out` = source rate / output rate.
    pub fn render(&mut self, out: &mut [f32], src: &dyn Fn(isize, usize) -> f32, tempo: f64, pitch: f64, src_per_out: f64) {
        self.rate_factor = tempo * src_per_out;
        let mut i = 0;
        while i < out.len() {
            if self.ready.is_empty() {
                self.grain(src, tempo, pitch, src_per_out);
            }
            while i < out.len() {
                let Some(s) = self.ready.pop_front() else { break };
                out[i] = s;
                i += 1;
            }
        }
    }

    fn grain(&mut self, src: &dyn Fn(isize, usize) -> f32, tempo: f64, pitch: f64, src_per_out: f64) {
        // Source frames per output frame inside a grain: this sets the pitch.
        let g = pitch * src_per_out;
        let mono = |x: f64| {
            let i = x.floor() as isize;
            let t = (x - i as f64) as f32;
            let a = (src(i, 0) + src(i, 1)) * 0.5;
            let b = (src(i + 1, 0) + src(i + 1, 1)) * 0.5;
            a + (b - a) * t
        };
        let start = match self.prev_start {
            None => self.pos,
            Some(prev) => {
                // Line the new grain up with where the last one would have carried on.
                let natural = prev + self.hop as f64 * g;
                let probe = |c: f64| -> f32 {
                    let (mut dot, mut energy) = (0.0f32, 1e-9f32);
                    for k in (0..self.hop).step_by(4) {
                        let v = mono(c + k as f64 * g);
                        dot += v * mono(natural + k as f64 * g);
                        energy += v * v;
                    }
                    dot / energy.sqrt()
                };
                let mut best = (self.pos, f32::MIN);
                let mut d = -self.search;
                while d <= self.search {
                    let c = (self.pos + d).max(0.0);
                    let score = probe(c);
                    if score > best.1 {
                        best = (c, score);
                    }
                    d += 2.0;
                }
                best.0
            }
        };
        for i in 0..self.n {
            let x = start + i as f64 * g;
            let k = x.floor() as isize;
            let t = (x - k as f64) as f32;
            let w = self.window[i];
            for ch in 0..2 {
                self.ola[i * 2 + ch] += w * hermite(src(k - 1, ch), src(k, ch), src(k + 1, ch), src(k + 2, ch), t);
            }
        }
        // The first half is finished: out it goes; the rest waits for the next grain.
        self.ready.extend(self.ola.drain(..self.hop * 2));
        self.ola.resize(self.n * 2, 0.0);
        self.prev_start = Some(start);
        self.pos += self.hop as f64 * tempo * src_per_out;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: u32 = 44_100;

    /// A source that is one steady tone, both channels.
    fn tone(hz: f64) -> impl Fn(isize, usize) -> f32 {
        move |i: isize, _ch: usize| (2.0 * std::f64::consts::PI * hz * i as f64 / R as f64).sin() as f32 * 0.5
    }

    /// Frequency of a stereo signal from its rising zero crossings (left channel).
    fn frequency(x: &[f32]) -> f64 {
        let left: Vec<f32> = x.chunks_exact(2).map(|f| f[0]).collect();
        let ups = left.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        ups as f64 / (left.len() as f64 / R as f64)
    }

    /// Runs the stretcher for `secs` of output; returns the output and how far it read.
    fn run(tempo: f64, pitch: f64, secs: f64) -> (Vec<f32>, f64) {
        let src = tone(440.0);
        let mut s = Stretcher::new(R);
        s.reset(0.0);
        let mut out = vec![0.0f32; (secs * R as f64) as usize * 2];
        for block in out.chunks_mut(512) {
            s.render(block, &src, tempo, pitch, 1.0);
        }
        (out, s.position())
    }

    #[test]
    fn faster_keeps_the_pitch() {
        for tempo in [1.16, 1.08, 0.92, 0.85] {
            let (out, read) = run(tempo, 1.0, 2.0);
            let f = frequency(&out[R as usize..]);
            assert!((f - 440.0).abs() < 4.0, "tempo {tempo}: {f} Hz");
            assert!((read / R as f64 - 2.0 * tempo).abs() < 0.06, "tempo {tempo}: read {} s for 2 s", read / R as f64);
        }
    }

    #[test]
    fn the_pitch_moves_by_semitones_at_the_same_tempo() {
        let up2 = 2f64.powf(2.0 / 12.0);
        let (out, read) = run(1.0, up2, 2.0);
        let f = frequency(&out[R as usize..]);
        assert!((f - 440.0 * up2).abs() < 5.0, "two semitones up: {f} Hz");
        assert!((read / R as f64 - 2.0).abs() < 0.06, "same tempo: read {} s", read / R as f64);
        let (out, _) = run(1.0, 1.0 / up2, 2.0);
        assert!((frequency(&out[R as usize..]) - 440.0 / up2).abs() < 5.0);
    }

    #[test]
    fn the_sound_stays_steady() {
        let (out, _) = run(1.12, 1.0, 2.0);
        let level = |x: &[f32]| (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt();
        let chunks: Vec<f32> = out[R as usize / 2..].chunks(R as usize / 10).map(level).collect();
        let (lo, hi) = chunks.iter().fold((f32::MAX, 0.0f32), |(a, b), &v| (a.min(v), b.max(v)));
        assert!(lo > hi * 0.8, "level from {lo} to {hi}");
        assert!((hi - 0.5 / 2f32.sqrt()).abs() < 0.06, "about the level that went in: {hi}");
    }

    #[test]
    fn a_jump_starts_again_from_the_new_place() {
        let src = tone(440.0);
        let mut s = Stretcher::new(R);
        s.reset(0.0);
        let mut out = vec![0.0f32; 4096];
        s.render(&mut out, &src, 1.0, 1.0, 1.0);
        s.reset(R as f64 * 10.0);
        assert!((s.position() - R as f64 * 10.0).abs() < 1.0);
    }
}
