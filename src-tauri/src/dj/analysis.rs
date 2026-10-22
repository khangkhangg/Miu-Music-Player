//! Song analysis for DJ mode (D.1): tempo and beat grid, key, energy, and where the music is at
//! full body (mix-in / mix-out points). Pure functions over mono PCM; the background analyser
//! decodes songs and stores the results.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Analysis {
    pub bpm: f32,
    /// 0..1: how clearly the song has a steady beat. Low for ballads without drums.
    pub beat_confidence: f32,
    /// Time of the first beat, seconds; beats follow every 60/bpm.
    pub first_beat_s: f64,
    /// Time of a bar's first beat (downbeat), seconds.
    pub downbeat_s: f64,
    /// Camelot code ("8A" = A minor, "8B" = C major), when the key is clear enough.
    pub key: Option<String>,
    /// 1 (calm) to 10 (peak-time).
    pub energy: u8,
    /// From here to `mix_out_s` the music is at full body.
    pub mix_in_s: f64,
    pub mix_out_s: f64,
    pub duration_s: f64,
}

/// Analyse mono samples at `rate`.
pub fn analyze(mono: &[f32], rate: u32) -> Analysis {
    let duration_s = mono.len() as f64 / rate.max(1) as f64;
    let (x, r) = downsample(mono, rate);
    let (env, hop_s) = onset_envelope(&x, r);
    let (bpm, beat_confidence, first_beat_s) = tempo_and_phase(&env, hop_s);
    let downbeat_s = downbeat(&env, hop_s, bpm, first_beat_s);
    let (mix_in_s, mix_out_s) = mix_points(&x, r);
    Analysis {
        bpm,
        beat_confidence,
        first_beat_s,
        downbeat_s,
        key: key(&x, r),
        energy: energy(&x, &env, hop_s),
        mix_in_s,
        mix_out_s,
        duration_s,
    }
}

/// Average down to about 11 kHz: enough for beats and keys, and quick.
fn downsample(x: &[f32], rate: u32) -> (Vec<f32>, u32) {
    let f = (rate / 11_025).max(1) as usize;
    (x.chunks(f).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect(), rate / f as u32)
}

const ONSET_N: usize = 512;
const ONSET_HOP: usize = 64;

/// Spectral flux (how much new sound starts in each short frame), with the local average taken
/// off so only sudden changes remain. Returns the envelope and the time between its values.
fn onset_envelope(x: &[f32], r: u32) -> (Vec<f32>, f64) {
    let hop_s = ONSET_HOP as f64 / r as f64;
    if x.len() < ONSET_N {
        return (Vec::new(), hop_s);
    }
    let mut planner = rustfft::FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(ONSET_N);
    let window: Vec<f32> = (0..ONSET_N).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / ONSET_N as f32).cos()).collect();
    let mut prev = vec![0.0f32; ONSET_N / 2];
    let mut flux = Vec::with_capacity(x.len() / ONSET_HOP);
    let mut buf = vec![rustfft::num_complex::Complex::new(0.0f32, 0.0); ONSET_N];
    for start in (0..=x.len() - ONSET_N).step_by(ONSET_HOP) {
        for (i, b) in buf.iter_mut().enumerate() {
            *b = rustfft::num_complex::Complex::new(x[start + i] * window[i], 0.0);
        }
        fft.process(&mut buf);
        let mut f = 0.0;
        for (k, p) in prev.iter_mut().enumerate() {
            let m = (1.0 + 100.0 * buf[k].norm()).ln();
            f += (m - *p).max(0.0);
            *p = m;
        }
        flux.push(f);
    }
    // Take off the local average (about half a second) and keep what rises above it.
    let w = ((0.5 / hop_s) as usize).max(1);
    let mut sum = 0.0f32;
    let mut avg = vec![0.0f32; flux.len()];
    for i in 0..flux.len() {
        sum += flux[i];
        if i >= w {
            sum -= flux[i - w];
        }
        avg[i] = sum / (i + 1).min(w) as f32;
    }
    let env = flux.iter().zip(&avg).map(|(f, a)| (f - a).max(0.0)).collect();
    (env, hop_s)
}

/// Time (s) of envelope frame `i`: the middle of its window.
fn frame_time(i: f64, hop_s: f64) -> f64 {
    i * hop_s + ONSET_N as f64 / 2.0 * hop_s / ONSET_HOP as f64
}

/// The envelope at a fractional frame, interpolated.
fn env_at(env: &[f32], pos: f64) -> f32 {
    let i = pos.floor();
    if i < 0.0 || i as usize + 1 >= env.len() {
        return 0.0;
    }
    let (a, b) = (env[i as usize], env[i as usize + 1]);
    a + (b - a) * (pos - i) as f32
}

/// Sum of the envelope on a grid of `period` frames starting at `phase`.
fn comb(env: &[f32], period: f64, phase: f64) -> f32 {
    let mut s = 0.0;
    let mut p = phase;
    while p < env.len() as f64 - 1.0 {
        s += env_at(env, p);
        p += period;
    }
    s
}

/// Tempo (BPM), how clear the beat is (0..1) and the time of the first beat.
fn tempo_and_phase(env: &[f32], hop_s: f64) -> (f32, f32, f64) {
    if env.len() < 64 {
        return (120.0, 0.0, 0.0);
    }
    let mean = env.iter().sum::<f32>() / env.len() as f32;
    let e: Vec<f32> = env.iter().map(|v| v - mean).collect();
    let acf = |lag: usize| -> f32 {
        if lag >= e.len() {
            return 0.0;
        }
        e[..e.len() - lag].iter().zip(&e[lag..]).map(|(a, b)| a * b).sum::<f32>() / (e.len() - lag) as f32
    };
    // Coarse: the lag that best repeats (with its multiples), leaning towards ~120 BPM.
    let lag_of = |bpm: f64| 60.0 / bpm / hop_s;
    let (lo, hi) = (lag_of(200.0).floor() as usize, lag_of(60.0).ceil() as usize);
    let mut best = (0.0f32, lo);
    for lag in lo..=hi {
        let bpm = 60.0 / (lag as f64 * hop_s);
        let lean = (-0.5 * ((bpm / 120.0).log2() / 0.9).powi(2)).exp() as f32;
        let score = (acf(lag) + 0.5 * acf(2 * lag) + 0.25 * acf(4 * lag)) * lean;
        if score > best.0 {
            best = (score, lag);
        }
    }
    let coarse = 60.0 / (best.1 as f64 * hop_s);
    // Fine: the period and phase whose beat grid lands on the most onset energy, over the song.
    let mut fine = (0.0f32, coarse, 0.0f64);
    let mut bpm = coarse * 0.97;
    while bpm <= coarse * 1.03 {
        let period = lag_of(bpm);
        let mut ph = 0.0;
        while ph < period {
            let s = comb(env, period, ph) / (env.len() as f64 / period) as f32;
            if s > fine.0 {
                fine = (s, bpm, ph);
            }
            ph += 0.5;
        }
        bpm += 0.05;
    }
    let (_, bpm, phase) = fine;
    let period = lag_of(bpm);
    // Confidence: the share of all onset energy within two frames of a beat, against the share
    // onsets at random would give (5 frames out of every period).
    let total: f32 = env.iter().sum();
    let on: f32 = (-2..=2).map(|d| comb(env, period, phase + d as f64)).sum();
    let chance = (5.0 / period).min(0.99) as f32;
    let confidence = if total > 0.0 { ((on / total - chance) / (1.0 - chance)).clamp(0.0, 1.0) } else { 0.0 };
    ((bpm * 10.0).round() as f32 / 10.0, confidence, frame_time(phase, hop_s))
}

/// The bar's first beat: of every four beats, the one with the most onset energy.
fn downbeat(env: &[f32], hop_s: f64, bpm: f32, first_beat_s: f64) -> f64 {
    let period = 60.0 / bpm as f64 / hop_s;
    let first = (first_beat_s - frame_time(0.0, hop_s)) / hop_s;
    let strength: Vec<f32> = (0..4).map(|j| comb(env, period * 4.0, first + j as f64 * period)).collect();
    let j = (0..4).max_by(|&a, &b| strength[a].total_cmp(&strength[b])).unwrap_or(0);
    first_beat_s + j as f64 * period * hop_s
}

/// Major and minor key profiles (Krumhansl and Kessler), tonic first.
const MAJOR: [f32; 12] = [6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88];
const MINOR: [f32; 12] = [6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17];

/// The key, from how much of each pitch class sounds over the song.
fn key(x: &[f32], r: u32) -> Option<String> {
    const N: usize = 4096;
    if x.len() < N {
        return None;
    }
    let mut planner = rustfft::FftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(N);
    let window: Vec<f32> = (0..N).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / N as f32).cos()).collect();
    let mut chroma = [0.0f32; 12];
    let mut buf = vec![rustfft::num_complex::Complex::new(0.0f32, 0.0); N];
    for start in (0..=x.len() - N).step_by(N / 2) {
        for (i, b) in buf.iter_mut().enumerate() {
            *b = rustfft::num_complex::Complex::new(x[start + i] * window[i], 0.0);
        }
        fft.process(&mut buf);
        for (k, b) in buf.iter().enumerate().take(N / 2).skip(1) {
            let f = k as f32 * r as f32 / N as f32;
            if !(60.0..2000.0).contains(&f) {
                continue;
            }
            let midi = 69.0 + 12.0 * (f / 440.0).log2();
            chroma[(midi.round() as i32).rem_euclid(12) as usize] += b.norm();
        }
    }
    if chroma.iter().all(|&c| c <= 0.0) {
        return None;
    }
    let corr = |profile: &[f32; 12], tonic: usize| -> f32 {
        let p: Vec<f32> = (0..12).map(|i| profile[(i + 12 - tonic) % 12]).collect();
        pearson(&chroma, &p)
    };
    let mut best = (f32::MIN, 0usize, false);
    for tonic in 0..12 {
        for (minor, profile) in [(false, &MAJOR), (true, &MINOR)] {
            let c = corr(profile, tonic);
            if c > best.0 {
                best = (c, tonic, minor);
            }
        }
    }
    (best.0 > 0.4).then(|| camelot(best.1, best.2))
}

fn pearson(a: &[f32], b: &[f32]) -> f32 {
    let n = a.len() as f32;
    let (ma, mb) = (a.iter().sum::<f32>() / n, b.iter().sum::<f32>() / n);
    let cov: f32 = a.iter().zip(b).map(|(x, y)| (x - ma) * (y - mb)).sum();
    let va: f32 = a.iter().map(|x| (x - ma).powi(2)).sum();
    let vb: f32 = b.iter().map(|y| (y - mb).powi(2)).sum();
    if va <= 0.0 || vb <= 0.0 {
        0.0
    } else {
        cov / (va * vb).sqrt()
    }
}

/// 1 to 10 from loudness (RMS of the louder part) and how many hits per second.
fn energy(x: &[f32], env: &[f32], hop_s: f64) -> u8 {
    let rms = |c: &[f32]| (c.iter().map(|v| v * v).sum::<f32>() / c.len().max(1) as f32).sqrt();
    let mut frames: Vec<f32> = x.chunks(4096).map(rms).collect();
    frames.sort_by(f32::total_cmp);
    let loud_rms = frames.get(frames.len() * 3 / 4).copied().unwrap_or(0.0);
    let db = 20.0 * loud_rms.max(1e-6).log10();
    let loud = ((db + 40.0) / 40.0).clamp(0.0, 1.0);
    let peak = env.iter().cloned().fold(0.0f32, f32::max);
    let hits = env.windows(3).filter(|w| w[1] > w[0] && w[1] >= w[2] && w[1] > 0.3 * peak).count();
    let per_s = hits as f64 / (env.len() as f64 * hop_s).max(1.0);
    let busy = (per_s / 8.0).clamp(0.0, 1.0) as f32;
    (1.0 + (9.0 * (0.6 * loud + 0.4 * busy)).round()).clamp(1.0, 10.0) as u8
}

/// Where the music is at full body: the first and last half-second within about 3 dB of the
/// song's usual level.
fn mix_points(x: &[f32], r: u32) -> (f64, f64) {
    let w = (r as usize / 2).max(1);
    let rms: Vec<f32> = x.chunks(w).map(|c| (c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).sqrt()).collect();
    let duration = x.len() as f64 / r as f64;
    if rms.is_empty() {
        return (0.0, duration);
    }
    let mut sorted = rms.clone();
    sorted.sort_by(f32::total_cmp);
    let level = sorted[sorted.len() * 3 / 4];
    let full = |v: &f32| *v >= level * 0.7;
    let first = rms.iter().position(full).unwrap_or(0);
    let last = rms.iter().rposition(full).unwrap_or(rms.len() - 1);
    let secs = w as f64 / r as f64;
    (first as f64 * secs, ((last + 1) as f64 * secs).min(duration))
}

/// Camelot code for a key: `tonic` 0 = C up to 11 = B, `minor` or major.
pub fn camelot(tonic: usize, minor: bool) -> String {
    // Majors: C = 8B, and each fifth up adds one. A minor shares 8 with its relative major C.
    let major_tonic = if minor { (tonic + 3) % 12 } else { tonic % 12 };
    let n = (major_tonic * 7 % 12 + 7) % 12 + 1;
    format!("{n}{}", if minor { 'A' } else { 'B' })
}

/// Keys that mix well: the same, one step around the wheel, or the relative major/minor.
pub fn keys_compatible(a: &str, b: &str) -> bool {
    let parse = |k: &str| -> Option<(i32, char)> {
        let (n, l) = k.split_at(k.len().checked_sub(1)?);
        Some((n.parse().ok()?, l.chars().next()?))
    };
    let (Some((na, la)), Some((nb, lb))) = (parse(a), parse(b)) else { return false };
    if la == lb {
        let d = (na - nb).rem_euclid(12);
        d == 0 || d == 1 || d == 11
    } else {
        na == nb
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 22_050;

    /// A short decaying noise burst (a drum hit) at each time in `hits`, `level` each.
    fn hits(len_s: f64, hits: &[(f64, f32)]) -> Vec<f32> {
        let mut out = vec![0.0f32; (len_s * RATE as f64) as usize];
        let mut seed = 1u64;
        for &(t, level) in hits {
            let start = (t * RATE as f64) as usize;
            for i in 0..(RATE as usize / 25) {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                let noise = (seed >> 33) as f32 / (1u64 << 31) as f32 - 0.5;
                if let Some(s) = out.get_mut(start + i) {
                    *s += noise * level * (-(i as f32) / (RATE as f32 * 0.01)).exp();
                }
            }
        }
        out
    }

    fn beats(bpm: f64, len_s: f64, offset_s: f64) -> Vec<(f64, f32)> {
        let period = 60.0 / bpm;
        (0..).map(|k| offset_s + k as f64 * period).take_while(|&t| t < len_s - 0.1).map(|t| (t, 0.8)).collect()
    }

    #[test]
    fn finds_the_tempo_of_a_steady_beat() {
        for bpm in [90.0, 128.0, 150.0] {
            let a = analyze(&hits(30.0, &beats(bpm, 30.0, 0.25)), RATE);
            assert!((a.bpm as f64 - bpm).abs() < 1.0, "{bpm}: got {}", a.bpm);
            assert!(a.beat_confidence > 0.5, "{bpm}: confidence {}", a.beat_confidence);
        }
    }

    #[test]
    fn places_the_beat_grid_on_the_hits() {
        let a = analyze(&hits(20.0, &beats(120.0, 20.0, 0.31)), RATE);
        let period = 0.5;
        let off = ((a.first_beat_s - 0.31) / period).fract().abs();
        assert!(off < 0.06 || off > 0.94, "first beat at {} s", a.first_beat_s);
    }

    #[test]
    fn the_downbeat_is_the_accented_first_beat_of_each_bar() {
        // 4/4 at 120 BPM: every fourth hit (bar starts at 1.0 s) is twice as loud.
        let h: Vec<(f64, f32)> = (0..80).map(|k| (0.5 + k as f64 * 0.5, if k % 4 == 1 { 1.0 } else { 0.4 })).collect();
        let a = analyze(&hits(42.0, &h), RATE);
        let off = ((a.downbeat_s - 1.0) / 2.0).fract().abs();
        assert!(off < 0.03 || off > 0.97, "downbeat at {} s", a.downbeat_s);
    }

    #[test]
    fn noise_without_a_beat_has_low_confidence() {
        let mut seed = 7u64;
        let noise: Vec<f32> = (0..RATE as usize * 20)
            .map(|_| {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                ((seed >> 33) as f32 / (1u64 << 31) as f32 - 0.5) * 0.3
            })
            .collect();
        assert!(analyze(&noise, RATE).beat_confidence < 0.3);
    }

    /// Sustained chords: each a list of MIDI notes, `secs` long.
    fn chords(progression: &[&[u8]], secs: f64, times: usize) -> Vec<f32> {
        let mut out = Vec::new();
        for _ in 0..times {
            for chord in progression {
                let n = (secs * RATE as f64) as usize;
                for i in 0..n {
                    let t = i as f32 / RATE as f32;
                    let s: f32 = chord.iter().map(|&m| (2.0 * std::f32::consts::PI * 440.0 * 2f32.powf((m as f32 - 69.0) / 12.0) * t).sin()).sum();
                    out.push(s * 0.2);
                }
            }
        }
        out
    }

    #[test]
    fn finds_major_and_minor_keys() {
        // C major: C, F, G, C.
        let c_major = chords(&[&[60, 64, 67], &[65, 69, 72], &[67, 71, 74], &[60, 64, 67]], 1.0, 4);
        assert_eq!(analyze(&c_major, RATE).key.as_deref(), Some("8B"));
        // A minor: Am, Dm, E, Am.
        let a_minor = chords(&[&[57, 60, 64], &[62, 65, 69], &[64, 68, 71], &[57, 60, 64]], 1.0, 4);
        assert_eq!(analyze(&a_minor, RATE).key.as_deref(), Some("8A"));
    }

    #[test]
    fn camelot_codes_and_compatible_keys() {
        assert_eq!(camelot(0, false), "8B");
        assert_eq!(camelot(9, true), "8A");
        assert_eq!(camelot(7, false), "9B");
        assert_eq!(camelot(4, true), "9A");
        assert_eq!(camelot(11, false), "1B");
        assert!(keys_compatible("8A", "8A"));
        assert!(keys_compatible("8A", "9A") && keys_compatible("8A", "7A"));
        assert!(keys_compatible("12A", "1A"), "the wheel wraps");
        assert!(keys_compatible("8A", "8B"), "relative major");
        assert!(!keys_compatible("8A", "10A"));
        assert!(!keys_compatible("8A", "9B"));
    }

    #[test]
    fn loud_busy_music_has_more_energy_than_quiet_sparse_music() {
        let quiet: Vec<f32> = hits(20.0, &beats(70.0, 20.0, 0.2)).iter().map(|s| s * 0.15).collect();
        let loud = hits(20.0, &[beats(128.0, 20.0, 0.2), beats(256.0, 20.0, 0.1)].concat());
        let (q, l) = (analyze(&quiet, RATE).energy, analyze(&loud, RATE).energy);
        assert!(q < l, "quiet {q}, loud {l}");
        assert!((1..=10).contains(&q) && (1..=10).contains(&l));
    }

    #[test]
    fn mix_points_skip_silence_and_fades() {
        // 4 s silence, 4 s fade-in, 30 s of beat, 6 s fade-out.
        let body = hits(44.0, &beats(120.0, 44.0, 4.0));
        let n = body.len();
        let at = |s: f64| (s * RATE as f64) as usize;
        let song: Vec<f32> = body
            .iter()
            .enumerate()
            .map(|(i, &x)| {
                let g = if i < at(4.0) { 0.0 } else if i < at(8.0) { (i - at(4.0)) as f32 / (at(8.0) - at(4.0)) as f32 } else if i > at(38.0) { 1.0 - (i - at(38.0)) as f32 / (n - at(38.0)) as f32 } else { 1.0 };
                x * g
            })
            .collect();
        let a = analyze(&song, RATE);
        assert!((5.5..9.0).contains(&a.mix_in_s), "mix in {}", a.mix_in_s);
        assert!((36.0..41.0).contains(&a.mix_out_s), "mix out {}", a.mix_out_s);
        assert!((a.duration_s - 44.0).abs() < 0.1);
    }
}

/// Decode a song to mono for analysis: (samples, rate).
pub fn decode_mono(path: &std::path::Path) -> Result<(Vec<f32>, u32), String> {
    let mut dec = crate::audio::decode::Decoder::open(path).map_err(|e| e.to_string())?;
    let (rate, ch) = (dec.sample_rate, dec.channels.max(1));
    let mut mono = Vec::new();
    while let Some(chunk) = dec.next_chunk().map_err(|e| e.to_string())? {
        mono.extend(chunk.chunks_exact(ch).map(|f| f.iter().sum::<f32>() / ch as f32));
    }
    Ok((mono, rate))
}

// Machine test: analyses the first `MIU_DJ_SONGS` (default 12) audio files in `MIU_DJ_DIR`
// (default D:\Music) and prints what it finds. Run with --release: debug builds are slow.
#[cfg(test)]
mod machine_tests {
    #[test]
    #[ignore]
    fn analyses_real_songs() {
        let dir = std::env::var("MIU_DJ_DIR").unwrap_or_else(|_| r"D:\Music".into());
        let n: usize = std::env::var("MIU_DJ_SONGS").ok().and_then(|s| s.parse().ok()).unwrap_or(12);
        let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok().map(|e| e.path())).filter(|p| matches!(p.extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref(), Some("mp3" | "flac" | "m4a"))).collect();
        files.sort();
        let step = (files.len() / n).max(1);
        for p in files.iter().step_by(step).take(n) {
            let t = std::time::Instant::now();
            let (mono, rate) = super::decode_mono(p).unwrap();
            let decoded = t.elapsed();
            let a = super::analyze(&mono, rate);
            println!(
                "{:>6.1} BPM conf {:.2}  key {:>3}  energy {:>2}  mix {:>5.1}-{:<5.1}/{:<5.1}  decode {:>4} ms analyse {:>4} ms  {}",
                a.bpm, a.beat_confidence, a.key.as_deref().unwrap_or("-"), a.energy, a.mix_in_s, a.mix_out_s, a.duration_s,
                decoded.as_millis(), (t.elapsed() - decoded).as_millis(),
                p.file_name().unwrap().to_string_lossy()
            );
        }
    }
}
