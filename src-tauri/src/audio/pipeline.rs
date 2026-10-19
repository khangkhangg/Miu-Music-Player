//! Between decoder and speakers (F7.1): channel remix, resampling to the device rate, and a
//! gapless feeder that runs a queue of files as one continuous stream.

use std::path::PathBuf;

use rubato::{Resampler as _, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction};

use crate::audio::decode::{AudioError, Decoder};

/// Convert interleaved `samples` from `from` channels to `to` channels.
pub fn remix(samples: &[f32], from: usize, to: usize) -> Vec<f32> {
    if from == to || from == 0 {
        return samples.to_vec();
    }
    let frames = samples.chunks_exact(from);
    match (from, to) {
        (1, _) => frames.flat_map(|f| std::iter::repeat(f[0]).take(to)).collect(),
        (_, 1) => frames.map(|f| (f[0] + f[1]) / 2.0).collect(),
        // More channels than the output: keep the front pair (FL, FR), the standard first two.
        _ => frames.flat_map(|f| (0..to).map(move |c| if c < f.len() { f[c] } else { 0.0 })).collect(),
    }
}

/// Sample-rate converter for interleaved audio. Same rate = exact pass-through.
pub struct Resampler {
    channels: usize,
    inner: Option<SincFixedIn<f32>>,
    pending: Vec<Vec<f32>>,
    ratio: f64,
    frames_in: u64,
    out: Output,
}

/// Output bookkeeping, separate from the rubato state so both can be borrowed at once.
struct Output {
    channels: usize,
    frames_out: u64,
    delay_left: usize,
}

impl Output {
    /// Drop the initial filter delay, cap at `limit` frames total, interleave.
    fn take(&mut self, res: Vec<Vec<f32>>, limit: u64) -> Vec<f32> {
        let frames = res[0].len();
        let skip = self.delay_left.min(frames);
        self.delay_left -= skip;
        let keep = ((frames - skip) as u64).min(limit.saturating_sub(self.frames_out)) as usize;
        self.frames_out += keep as u64;
        let mut out = Vec::with_capacity(keep * self.channels);
        for i in skip..skip + keep {
            for ch in &res {
                out.push(ch[i]);
            }
        }
        out
    }
}

impl Resampler {
    pub fn new(from_rate: u32, to_rate: u32, channels: usize) -> Result<Self, AudioError> {
        let ratio = to_rate as f64 / from_rate as f64;
        let inner = if from_rate == to_rate {
            None
        } else {
            let params = SincInterpolationParameters {
                sinc_len: 256,
                f_cutoff: 0.95,
                interpolation: SincInterpolationType::Cubic,
                oversampling_factor: 256,
                window: WindowFunction::BlackmanHarris2,
            };
            Some(SincFixedIn::<f32>::new(ratio, 1.0, params, 1024, channels).map_err(|e| AudioError(e.to_string()))?)
        };
        let delay_left = inner.as_ref().map_or(0, |r| r.output_delay());
        Ok(Self {
            channels,
            inner,
            pending: vec![Vec::new(); channels],
            ratio,
            frames_in: 0,
            out: Output { channels, frames_out: 0, delay_left },
        })
    }

    pub fn process(&mut self, interleaved: &[f32]) -> Result<Vec<f32>, AudioError> {
        let Some(inner) = self.inner.as_mut() else { return Ok(interleaved.to_vec()) };
        for frame in interleaved.chunks_exact(self.channels) {
            for (c, s) in frame.iter().enumerate() {
                self.pending[c].push(*s);
            }
        }
        self.frames_in += (interleaved.len() / self.channels) as u64;
        let mut out = Vec::new();
        while self.pending[0].len() >= inner.input_frames_next() {
            let n = inner.input_frames_next();
            let chunk: Vec<Vec<f32>> = self.pending.iter_mut().map(|ch| ch.drain(..n).collect()).collect();
            let res = inner.process(&chunk, None).map_err(|e| AudioError(e.to_string()))?;
            out.extend(self.out.take(res, u64::MAX));
        }
        Ok(out)
    }

    /// Push out what is still buffered (end of track); output is trimmed to the exact length.
    pub fn flush(&mut self) -> Result<Vec<f32>, AudioError> {
        let expected = (self.frames_in as f64 * self.ratio).round() as u64;
        let Some(inner) = self.inner.as_mut() else { return Ok(Vec::new()) };
        let mut out = Vec::new();
        let rest: Vec<Vec<f32>> = self.pending.iter_mut().map(std::mem::take).collect();
        let res = inner.process_partial(Some(&rest), None).map_err(|e| AudioError(e.to_string()))?;
        out.extend(self.out.take(res, expected));
        // The filter delay still holds the tail; feed silence until it is all out.
        while self.out.frames_out < expected {
            let res = inner.process_partial::<Vec<f32>>(None, None).map_err(|e| AudioError(e.to_string()))?;
            if res[0].is_empty() {
                break;
            }
            out.extend(self.out.take(res, expected));
        }
        Ok(out)
    }

}

/// Interleaved samples at the output rate/channels, and which queue entry they come from.
#[derive(Debug)]
pub struct Block {
    pub track: usize,
    pub samples: Vec<f32>,
}

/// Plays `tracks[start..]` as one stream: when a file ends, the next one continues immediately.
pub struct Feeder {
    tracks: Vec<PathBuf>,
    index: usize,
    out_rate: u32,
    out_channels: usize,
    current: Option<(Decoder, Resampler)>,
}

impl Feeder {
    pub fn new(tracks: Vec<PathBuf>, start: usize, out_rate: u32, out_channels: usize) -> Self {
        Self { tracks, index: start, out_rate, out_channels, current: None }
    }

    pub fn current_track(&self) -> usize {
        self.index
    }

    /// Seek within the current track (opening it if needed).
    pub fn seek_ms(&mut self, ms: u64) -> Result<(), AudioError> {
        if self.current.is_none() {
            self.open_current()?;
        }
        if let Some((dec, res)) = self.current.as_mut() {
            dec.seek_ms(ms)?;
            *res = Resampler::new(dec.sample_rate, self.out_rate, self.out_channels)?;
        }
        Ok(())
    }

    fn open_current(&mut self) -> Result<(), AudioError> {
        let path = &self.tracks[self.index];
        let dec = Decoder::open(path)?;
        let res = Resampler::new(dec.sample_rate, self.out_rate, self.out_channels)?;
        self.current = Some((dec, res));
        Ok(())
    }

    pub fn next_block(&mut self) -> Result<Option<Block>, AudioError> {
        loop {
            if self.current.is_none() {
                if self.index >= self.tracks.len() {
                    return Ok(None);
                }
                if let Err(e) = self.open_current() {
                    log::warn!("skipping {}: {e}", self.tracks[self.index].display());
                    self.index += 1;
                    continue;
                }
            }
            let (dec, res) = self.current.as_mut().expect("opened above");
            let chunk = match dec.next_chunk() {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("stopped decoding {}: {e}", self.tracks[self.index].display());
                    None
                }
            };
            let track = self.index;
            let samples = match chunk {
                Some(c) => res.process(&remix(&c, dec.channels, self.out_channels))?,
                None => {
                    let tail = res.flush()?;
                    self.current = None;
                    self.index += 1;
                    tail
                }
            };
            if !samples.is_empty() {
                return Ok(Some(Block { track, samples }));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::fixtures::{ramp, wav};

    fn sine(rate: u32, freq: f32, frames: usize) -> Vec<f32> {
        (0..frames).map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / rate as f32).sin() * 0.5).collect()
    }

    fn zero_crossings(mono: &[f32]) -> usize {
        mono.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count()
    }

    #[test]
    fn mono_becomes_stereo_by_duplicating() {
        assert_eq!(remix(&[0.1, 0.2], 1, 2), vec![0.1, 0.1, 0.2, 0.2]);
    }

    #[test]
    fn stereo_becomes_mono_by_averaging() {
        assert_eq!(remix(&[0.2, 0.4, -0.2, 0.2], 2, 1), vec![0.3, 0.0]);
    }

    #[test]
    fn surround_folds_into_stereo_from_the_front_pair() {
        // 6 channels: FL FR C LFE SL SR → L = FL, R = FR (simple fold, no clipping)
        let out = remix(&[0.1, 0.2, 0.9, 0.9, 0.9, 0.9], 6, 2);
        assert_eq!(out, vec![0.1, 0.2]);
    }

    #[test]
    fn same_channel_count_is_untouched() {
        assert_eq!(remix(&[0.1, 0.2, 0.3, 0.4], 2, 2), vec![0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn same_rate_passes_samples_through_bit_for_bit() {
        let input = sine(48_000, 1000.0, 4800);
        let mut r = Resampler::new(48_000, 48_000, 1).unwrap();
        let mut out = r.process(&input).unwrap();
        out.extend(r.flush().unwrap());
        assert_eq!(out, input);
    }

    #[test]
    fn resampling_keeps_length_and_pitch() {
        let input = sine(44_100, 1000.0, 44_100); // 1 s of 1 kHz
        let mut r = Resampler::new(44_100, 48_000, 1).unwrap();
        let mut out = Vec::new();
        for block in input.chunks(4096) {
            out.extend(r.process(block).unwrap());
        }
        out.extend(r.flush().unwrap());
        assert!((out.len() as i64 - 48_000).abs() <= 64, "got {} frames", out.len());
        let crossings = zero_crossings(&out[2000..46_000]) as f32 / (44_000.0 / 48_000.0);
        assert!((crossings - 1000.0).abs() <= 3.0, "pitch drifted: {crossings} Hz");
    }

    #[test]
    fn feeder_plays_tracks_back_to_back_without_a_gap() {
        let d = tempfile::tempdir().unwrap();
        let a = wav(d.path(), "a.wav", 48_000, 2, 16, 4_800);
        let b = wav(d.path(), "b.wav", 48_000, 2, 16, 2_400);
        let mut f = Feeder::new(vec![a, b], 0, 48_000, 2);
        let mut out = Vec::new();
        while let Some(block) = f.next_block().unwrap() {
            out.extend(block.samples);
        }
        assert_eq!(out.len(), (4_800 + 2_400) * 2, "no padding between tracks");
        // The ramp never touches zero, so any inserted silence would show up here.
        assert!(out.iter().all(|s| s.abs() > 0.05));
        assert!((out[4_800 * 2] - ramp(0, 0)).abs() < 1e-3, "track 2 starts right after track 1");
    }

    #[test]
    fn feeder_reports_which_track_each_block_belongs_to() {
        let d = tempfile::tempdir().unwrap();
        let tracks: Vec<PathBuf> = (0..3).map(|i| wav(d.path(), &format!("{i}.wav"), 48_000, 1, 16, 1_000)).collect();
        let mut f = Feeder::new(tracks, 1, 48_000, 1);
        let mut seen = Vec::new();
        while let Some(block) = f.next_block().unwrap() {
            if seen.last() != Some(&block.track) {
                seen.push(block.track);
            }
        }
        assert_eq!(seen, vec![1, 2], "starts at the chosen index and continues");
    }

    #[test]
    fn feeder_skips_an_unplayable_file_and_continues() {
        let d = tempfile::tempdir().unwrap();
        let bad = d.path().join("bad.mp3");
        std::fs::write(&bad, b"not audio").unwrap();
        let good = wav(d.path(), "g.wav", 48_000, 1, 16, 500);
        let mut f = Feeder::new(vec![bad, good], 0, 48_000, 1);
        let mut n = 0;
        while let Some(block) = f.next_block().unwrap() {
            assert_eq!(block.track, 1);
            n += block.samples.len();
        }
        assert_eq!(n, 500);
    }
}

#[cfg(test)]
mod seek_tests {
    use super::*;
    use crate::audio::fixtures::{ramp, wav};

    #[test]
    fn feeder_seek_starts_the_next_block_at_the_new_position() {
        let d = tempfile::tempdir().unwrap();
        let p = wav(d.path(), "s.wav", 48_000, 1, 16, 48_000 * 3);
        let mut f = Feeder::new(vec![p], 0, 48_000, 1);
        f.next_block().unwrap();
        f.seek_ms(2_005).unwrap(); // frame 96_240: the ramp repeats every 1000 frames, so not a multiple of it
        let block = f.next_block().unwrap().expect("audio after seek");
        assert!((block.samples[0] - ramp(96_240, 0)).abs() < 1e-3, "got {} want {}", block.samples[0], ramp(96_240, 0));
        assert!((ramp(96_240, 0) - ramp(0, 0)).abs() > 0.1, "test would not notice a seek to 0");
    }
}
