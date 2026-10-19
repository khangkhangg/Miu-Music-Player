//! Decoding any supported file to interleaved f32 samples (F7.1), with symphonia: MP3, AAC/M4A,
//! ALAC, FLAC, WAV, AIFF, Vorbis. Hi-res stays hi-res — no resampling here (see `resample`).

use std::fs::File;
use std::path::Path;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder as CodecDecoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct AudioError(pub String);

fn err(e: impl std::fmt::Display) -> AudioError {
    AudioError(e.to_string())
}

/// A whole decoded file (tests and short files).
#[derive(Debug)]
pub struct Decoded {
    pub sample_rate: u32,
    pub channels: usize,
    /// Interleaved, -1.0..=1.0.
    pub samples: Vec<f32>,
}

impl Decoded {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1)
    }
}

/// What the file itself is (before any resampling): shown in the output panel (F7.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceFormat {
    pub rate: u32,
    pub bits: Option<u32>,
    pub channels: usize,
}

/// Streaming decoder: `next_chunk` yields interleaved samples until the end of the file.
pub struct Decoder {
    format: Box<dyn FormatReader>,
    codec: Box<dyn CodecDecoder>,
    track_id: u32,
    pub sample_rate: u32,
    pub channels: usize,
    pub bits: Option<u32>,
    /// Frames still to drop after an accurate seek landed before the target.
    skip_frames: u64,
}

impl Decoder {
    pub fn open(path: &Path) -> Result<Self, AudioError> {
        let file = File::open(path).map_err(err)?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
            hint.with_extension(ext);
        }
        let opts = FormatOptions { enable_gapless: true, ..Default::default() };
        let probed = symphonia::default::get_probe().format(&hint, mss, &opts, &MetadataOptions::default()).map_err(err)?;
        let format = probed.format;
        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| AudioError("no audio track".into()))?;
        let params = track.codec_params.clone();
        let codec = symphonia::default::get_codecs().make(&params, &DecoderOptions::default()).map_err(err)?;
        Ok(Self {
            track_id: track.id,
            sample_rate: params.sample_rate.ok_or_else(|| AudioError("unknown sample rate".into()))?,
            channels: params.channels.map(|c| c.count()).ok_or_else(|| AudioError("unknown channel layout".into()))?,
            bits: params.bits_per_sample,
            format,
            codec,
            skip_frames: 0,
        })
    }

    pub fn format(&self) -> SourceFormat {
        SourceFormat { rate: self.sample_rate, bits: self.bits, channels: self.channels }
    }

    /// Next block of interleaved samples, or `None` at the end of the file.
    pub fn next_chunk(&mut self) -> Result<Option<Vec<f32>>, AudioError> {
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
                Err(SymError::ResetRequired) => {
                    self.codec.reset();
                    continue;
                }
                Err(e) => return Err(err(e)),
            };
            if packet.track_id() != self.track_id {
                continue;
            }
            let decoded = match self.codec.decode(&packet) {
                Ok(d) => d,
                Err(SymError::DecodeError(_)) => continue, // skip a damaged packet, keep playing
                Err(e) => return Err(err(e)),
            };
            let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
            buf.copy_interleaved_ref(decoded);
            let mut samples = buf.samples().to_vec();
            if self.skip_frames > 0 {
                let drop = (self.skip_frames as usize * self.channels).min(samples.len());
                samples.drain(..drop);
                self.skip_frames -= (drop / self.channels) as u64;
            }
            if !samples.is_empty() {
                return Ok(Some(samples));
            }
        }
    }

    /// Seek so the next chunk starts at `ms`. Returns that position in frames.
    pub fn seek_ms(&mut self, ms: u64) -> Result<u64, AudioError> {
        let time = Time::new(ms / 1000, (ms % 1000) as f64 / 1000.0);
        let seeked = self
            .format
            .seek(SeekMode::Accurate, SeekTo::Time { time, track_id: Some(self.track_id) })
            .map_err(err)?;
        self.codec.reset();
        self.skip_frames = seeked.required_ts.saturating_sub(seeked.actual_ts);
        Ok(seeked.required_ts)
    }
}

pub fn decode_all(path: &Path) -> Result<Decoded, AudioError> {
    let mut dec = Decoder::open(path)?;
    let mut samples = Vec::new();
    while let Some(chunk) = dec.next_chunk()? {
        samples.extend(chunk);
    }
    Ok(Decoded { sample_rate: dec.sample_rate, channels: dec.channels, samples })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::fixtures::{aiff, ramp, wav};

    fn close(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    #[test]
    fn decodes_16_bit_stereo_wav_exactly() {
        let d = tempfile::tempdir().unwrap();
        let p = wav(d.path(), "a.wav", 44_100, 2, 16, 44_100);
        let out = decode_all(&p).unwrap();
        assert_eq!((out.sample_rate, out.channels), (44_100, 2));
        assert_eq!(out.frames(), 44_100);
        for i in [0, 1, 999, 1000, 44_099] {
            assert!(close(out.samples[i * 2], ramp(i, 0), 1.0 / 16_000.0), "frame {i}");
            assert!(close(out.samples[i * 2 + 1], ramp(i, 1), 1.0 / 16_000.0), "frame {i}");
        }
    }

    #[test]
    fn decodes_hi_res_24_bit_96k() {
        let d = tempfile::tempdir().unwrap();
        let p = wav(d.path(), "hr.wav", 96_000, 1, 24, 24_000);
        let out = decode_all(&p).unwrap();
        assert_eq!((out.sample_rate, out.channels, out.frames()), (96_000, 1, 24_000));
        assert!(close(out.samples[1234], ramp(1234, 0), 1.0 / 4_000_000.0), "24-bit precision kept");
    }

    #[test]
    fn decodes_big_endian_aiff() {
        let d = tempfile::tempdir().unwrap();
        let p = aiff(d.path(), "a.aiff", 48_000, 2, 24_000);
        let out = decode_all(&p).unwrap();
        assert_eq!((out.sample_rate, out.channels, out.frames()), (48_000, 2, 24_000));
        assert!(close(out.samples[10 * 2], ramp(10, 0), 1.0 / 16_000.0));
    }

    #[test]
    fn seeking_lands_on_the_requested_frame() {
        let d = tempfile::tempdir().unwrap();
        let p = wav(d.path(), "s.wav", 44_100, 2, 16, 44_100);
        let mut dec = Decoder::open(&p).unwrap();
        let at = dec.seek_ms(500).unwrap();
        assert_eq!(at, 22_050);
        let chunk = dec.next_chunk().unwrap().expect("audio after seek");
        assert!(close(chunk[0], ramp(22_050, 0), 1.0 / 16_000.0));
    }

    #[test]
    fn reading_past_the_end_returns_none() {
        let d = tempfile::tempdir().unwrap();
        let p = wav(d.path(), "e.wav", 8_000, 1, 16, 100);
        let mut dec = Decoder::open(&p).unwrap();
        let mut frames = 0;
        while let Some(c) = dec.next_chunk().unwrap() {
            frames += c.len();
        }
        assert_eq!(frames, 100);
        assert!(dec.next_chunk().unwrap().is_none());
    }

    #[test]
    fn not_audio_is_an_error() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.wav");
        std::fs::write(&p, b"definitely not audio").unwrap();
        assert!(Decoder::open(&p).is_err());
    }
}
