//! Stems (D.7): a song split into vocals and the rest, for the vocal-out blend (the old song's
//! voice over the new song's instrumental). Lead vocals are almost always in the middle of the
//! stereo mix, so each short slice of sound is split per frequency: what is the same in both
//! channels and in the voice range goes to the vocals, the rest stays with the instrumental.
//! Lighter and rougher than a trained separator, and needs nothing downloaded.

use rustfft::num_complex::Complex32;
use rustfft::FftPlanner;

/// A song in two parts that add back up to it (both interleaved stereo, the song's length).
pub struct Stems {
    pub vocals: Vec<i16>,
    pub instrumental: Vec<i16>,
}

/// Voices live here (Hz); outside it everything stays with the instrumental.
const VOICE_LOW: f32 = 150.0;
const VOICE_HIGH: f32 = 7000.0;
/// How alike both channels must be (1 = identical) for a sound to count as in the middle.
const CENTRE_FROM: f32 = 0.85;
const CENTRE_FULL: f32 = 0.98;

fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// Splits interleaved stereo `pcm` at `rate` into vocals and instrumental.
pub fn split(pcm: &[i16], rate: u32) -> Stems {
    let frames = pcm.len() / 2;
    // About 46 ms slices (a power of two), overlapping by half; square-root Hann windows on the
    // way in and out multiply to a Hann window, which adds back up to exactly one.
    let n = (0.046 * rate as f32).max(256.0).log2().round().exp2() as usize;
    let hop = n / 2;
    let window: Vec<f32> = (0..n).map(|i| (0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / n as f32).cos()).sqrt()).collect();
    let bin_hz = rate as f32 / n as f32;
    let mask_band: Vec<f32> = (0..n)
        .map(|k| {
            let f = (if k <= n / 2 { k } else { n - k }) as f32 * bin_hz;
            smoothstep((f - VOICE_LOW * 0.7) / (VOICE_LOW * 0.6)) * smoothstep((VOICE_HIGH * 1.3 - f) / (VOICE_HIGH * 0.6))
        })
        .collect();
    let mut planner = FftPlanner::<f32>::new();
    let fwd = planner.plan_fft_forward(n);
    let inv = planner.plan_fft_inverse(n);
    let sample = |i: isize, ch: usize| -> f32 {
        if i < 0 || i as usize >= frames { 0.0 } else { pcm[i as usize * 2 + ch] as f32 / 32768.0 }
    };
    let mut voc = vec![0.0f32; frames + n];
    let (mut l_buf, mut r_buf, mut v_buf) = (vec![Complex32::default(); n], vec![Complex32::default(); n], vec![Complex32::default(); n]);
    // Frames start half a slice early so the very start is covered by two slices like the rest.
    let mut start = -(hop as isize);
    while start < frames as isize {
        for i in 0..n {
            let w = window[i];
            l_buf[i] = Complex32::new(sample(start + i as isize, 0) * w, 0.0);
            r_buf[i] = Complex32::new(sample(start + i as isize, 1) * w, 0.0);
        }
        fwd.process(&mut l_buf);
        fwd.process(&mut r_buf);
        for k in 0..n {
            let (l, r) = (l_buf[k], r_buf[k]);
            let sum = l + r;
            let centre = sum.norm_sqr() / (2.0 * (l.norm_sqr() + r.norm_sqr()) + 1e-12);
            let mask = smoothstep((centre - CENTRE_FROM) / (CENTRE_FULL - CENTRE_FROM)) * mask_band[k];
            v_buf[k] = sum * (0.5 * mask);
        }
        inv.process(&mut v_buf);
        for i in 0..n {
            let at = start + i as isize;
            if at >= 0 && (at as usize) < frames {
                voc[at as usize] += v_buf[i].re * window[i] / n as f32;
            }
        }
        start += hop as isize;
    }
    let to_i16 = |x: f32| (x * 32768.0).round().clamp(-32768.0, 32767.0) as i16;
    let mut vocals = Vec::with_capacity(pcm.len());
    let mut instrumental = Vec::with_capacity(pcm.len());
    for (i, &v) in voc.iter().take(frames).enumerate() {
        let vi = to_i16(v);
        vocals.extend([vi, vi]);
        // The rest is what the vocals leave, so the two add back up to the song.
        instrumental.push(pcm[i * 2].saturating_sub(vi));
        instrumental.push(pcm[i * 2 + 1].saturating_sub(vi));
    }
    Stems { vocals, instrumental }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    const R: u32 = 22_050;

    /// Power of `hz` in one channel of a stereo signal (Goertzel).
    fn power(x: &[f32], ch: usize, hz: f32) -> f32 {
        let k = 2.0 * (2.0 * PI * hz / R as f32).cos();
        let (mut s1, mut s2) = (0.0f32, 0.0f32);
        let mut n = 0;
        for f in x.chunks_exact(2) {
            let s = f[ch] + k * s1 - s2;
            s2 = s1;
            s1 = s;
            n += 1;
        }
        (s1 * s1 + s2 * s2 - k * s1 * s2) / (n as f32 * n as f32)
    }

    /// A "voice" (440 Hz) in the middle, a "guitar" (660 Hz) on the left only, a bass (60 Hz) in
    /// the middle. Interleaved stereo i16.
    fn mix(secs: f32) -> Vec<i16> {
        (0..(secs * R as f32) as usize)
            .flat_map(|i| {
                let t = i as f32 / R as f32;
                let voice = (2.0 * PI * 440.0 * t).sin() * 0.3;
                let guitar = (2.0 * PI * 660.0 * t).sin() * 0.3;
                let bass = (2.0 * PI * 60.0 * t).sin() * 0.3;
                let l = voice + guitar + bass;
                let r = voice + bass;
                [(l * 20000.0) as i16, (r * 20000.0) as i16]
            })
            .collect()
    }

    fn to_f32(x: &[i16]) -> Vec<f32> {
        x.iter().map(|&s| s as f32 / 32768.0).collect()
    }

    #[test]
    fn the_voice_in_the_middle_goes_to_the_vocals() {
        let s = split(&mix(4.0), R);
        let (v, inst) = (to_f32(&s.vocals), to_f32(&s.instrumental));
        let mid = |x: &[f32]| x[x.len() / 4..x.len() * 3 / 4].to_vec();
        let (v, inst) = (mid(&v), mid(&inst));
        assert!(power(&v, 1, 440.0) > 20.0 * power(&inst, 1, 440.0), "the voice left the instrumental");
        assert!(power(&inst, 0, 660.0) > 20.0 * power(&v, 0, 660.0), "the guitar on one side stays");
        assert!(power(&inst, 0, 60.0) > 20.0 * power(&v, 0, 60.0), "the bass stays though it is in the middle");
    }

    #[test]
    fn vocals_and_instrumental_add_back_up_to_the_song() {
        let song = mix(3.0);
        let s = split(&song, R);
        assert_eq!(s.vocals.len(), song.len());
        assert_eq!(s.instrumental.len(), song.len());
        let err: f64 = song
            .iter()
            .zip(s.vocals.iter().zip(&s.instrumental))
            .skip(song.len() / 10)
            .take(song.len() * 8 / 10)
            .map(|(&o, (&v, &i))| ((o as f64 - (v as f64 + i as f64)) / 32768.0).powi(2))
            .sum::<f64>()
            / (song.len() as f64 * 0.8);
        assert!(err.sqrt() < 0.01, "rms difference {}", err.sqrt());
    }
}

// Machine test: splits the song `MIU_DJ_STEM_SONG` (a path) and writes its vocals and
// instrumental next to `MIU_DJ_OUT` (a folder) as WAVs to listen to. Run with --release.
#[cfg(test)]
mod machine_tests {
    use std::io::Write;

    fn write_wav(path: &std::path::Path, rate: u32, stereo: &[i16]) {
        let mut f = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
        let data = (stereo.len() * 2) as u32;
        f.write_all(b"RIFF").unwrap();
        f.write_all(&(36 + data).to_le_bytes()).unwrap();
        f.write_all(b"WAVEfmt ").unwrap();
        for v in [16u32.to_le_bytes().as_slice(), &1u16.to_le_bytes(), &2u16.to_le_bytes(), &rate.to_le_bytes(), &(rate * 4).to_le_bytes(), &4u16.to_le_bytes(), &16u16.to_le_bytes()] {
            f.write_all(v).unwrap();
        }
        f.write_all(b"data").unwrap();
        f.write_all(&data.to_le_bytes()).unwrap();
        for s in stereo {
            f.write_all(&s.to_le_bytes()).unwrap();
        }
    }

    #[test]
    #[ignore]
    fn splits_a_real_song() {
        let song = std::env::var("MIU_DJ_STEM_SONG").expect("MIU_DJ_STEM_SONG");
        let out = std::path::PathBuf::from(std::env::var("MIU_DJ_OUT").unwrap_or_else(|_| ".".into()));
        let t = crate::dj::engine::decode_track(std::path::Path::new(&song), 1, None).unwrap();
        let start = std::time::Instant::now();
        let s = super::split(&t.pcm, t.rate);
        println!("split {:.0} s of music in {} ms", t.duration_s(), start.elapsed().as_millis());
        // The first minute of each is enough to listen to.
        let minute = (t.rate as usize * 60 * 2).min(s.vocals.len());
        write_wav(&out.join("stem-vocals.wav"), t.rate, &s.vocals[..minute]);
        write_wav(&out.join("stem-instrumental.wav"), t.rate, &s.instrumental[..minute]);
    }
}
