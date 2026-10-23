//! Party pads (D.5): short sounds made in code (no sample files to ship or license): an air horn,
//! a siren, a laser and a bass drop.

use std::f32::consts::PI;

pub const PADS: usize = 4;
/// Sounds Auto DJ plays itself during transitions (not on the pads): a rising sweep before a
/// drop and a hit on it.
pub const RISER: usize = 4;
pub const IMPACT: usize = 5;
/// Hype's phrase marks: a cymbal crash on a new phrase and a snare fill into it.
pub const CRASH: usize = 6;
pub const FILL: usize = 7;

/// Attack/release envelope so a sound starts and ends without a click.
fn envelope(i: usize, n: usize, rate: u32) -> f32 {
    let edge = (0.01 * rate as f32) as usize;
    let a = (i as f32 / edge as f32).min(1.0);
    let r = ((n - i) as f32 / (edge * 4) as f32).min(1.0);
    a * r
}

/// White noise -0.5..0.5 as a function of the sample number (the same every time).
fn noise_at(i: u64) -> f32 {
    let mut x = i.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
    x ^= x >> 31;
    x = x.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x ^= x >> 29;
    (x >> 40) as f32 / (1u64 << 24) as f32 - 0.5
}

/// A band-limited-enough sawtooth: the first harmonics only (no aliasing whistle).
fn saw(phase: f32) -> f32 {
    (1..=8).map(|k| (phase * k as f32).sin() / k as f32).sum::<f32>() * 0.6
}

/// Mono sound for pad `slot` at `rate`; empty for a pad that does not exist.
pub fn sound(slot: usize, rate: u32) -> Vec<f32> {
    let r = rate as f32;
    let render = |secs: f32, f: &dyn Fn(f32) -> f32| -> Vec<f32> {
        let n = (secs * r) as usize;
        (0..n).map(|i| f(i as f32 / r) * envelope(i, n, rate)).collect()
    };
    let mut out = match slot {
        // Air horn: three short blasts and a long one, a detuned brassy chord.
        0 => render(1.6, &|t| {
            let on = [(0.0, 0.18), (0.25, 0.43), (0.5, 0.68), (0.75, 1.6)].iter().any(|(a, b)| t >= *a && t < *b);
            if !on {
                return 0.0;
            }
            let w = 2.0 * PI * t;
            (saw(w * 466.0) + saw(w * 470.0) + 0.6 * saw(w * 698.0)) / 2.6
        }),
        // Siren: a tone sweeping up and down twice a second.
        1 => render(2.0, &|t| {
            // 900 ± 350 Hz; the phase of a sweeping tone is the running sum of its frequency.
            let phase = 2.0 * PI * (900.0 * t - 350.0 / (2.0 * PI * 2.0) * ((2.0 * PI * 2.0 * t).cos() - 1.0));
            phase.sin() * 0.8
        }),
        // Laser: three fast falling zaps.
        2 => render(0.75, &|t| {
            let local = t % 0.25;
            let phase = 2.0 * PI * (3000.0 / 12.0) * (1.0 - (-12.0 * local).exp());
            phase.sin() * (1.0 - local / 0.25)
        }),
        // Bass drop: a deep sine falling from 110 to 40 Hz.
        3 => render(1.8, &|t| {
            let phase = 2.0 * PI * (40.0 * t + 70.0 / 2.0 * (1.0 - (-2.0 * t).exp()));
            phase.sin() * (1.0 - t / 1.8)
        }),
        // Riser: noise and a tone sweeping up, getting louder, for six seconds.
        RISER => {
            let mut seed = 11u64;
            let mut lp = 0.0f32;
            let n = (6.0 * r) as usize;
            (0..n)
                .map(|i| {
                    let t = i as f32 / r;
                    let p = t / 6.0;
                    seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                    let noise = (seed >> 33) as f32 / (1u64 << 31) as f32 - 0.5;
                    // The noise opens up as it rises (a one-pole low-pass sweeping up).
                    let a = (-2.0 * PI * (300.0 + 8000.0 * p * p) / r).exp();
                    lp = noise + a * (lp - noise);
                    let tone = (2.0 * PI * (200.0 * t + 1800.0 / 2.0 * t * p)).sin() * 0.3;
                    (lp * 1.5 + tone) * p * p * envelope(i, n, rate)
                })
                .collect()
        }
        // Impact: a deep boom and a burst of noise, dying away over two and a half seconds.
        IMPACT => render(2.5, &|t| {
            let phase = 2.0 * PI * (35.0 * t + 25.0 / 3.0 * (1.0 - (-3.0 * t).exp()));
            phase.sin() * (-t * 2.0).exp() + noise_at((t * r) as u64) * (-t * 4.0).exp() * 0.5
        }),
        // Crash: bright noise (a high-pass of white noise) ringing out over two seconds.
        CRASH => {
            render(2.2, &|t| noise_at((t * r) as u64) * (-t * 2.2).exp())
            .windows(2)
            .map(|w| w[1] - w[0] * 0.95)
            .chain(std::iter::once(0.0))
            .collect()
        }
        // Fill: a snare roll of six hits getting closer together over half a second.
        FILL => render(0.55, &|t| {
            let hits = [0.0f32, 0.12, 0.22, 0.30, 0.37, 0.43, 0.48];
            hits.iter()
                .filter(|&&h| t >= h)
                .map(|&h| {
                    let d = t - h;
                    let body = (2.0 * PI * 190.0 * d).sin() * (-d * 30.0).exp();
                    let rattle = noise_at((t * r) as u64) * (-d * 22.0).exp();
                    (0.6 * body + rattle) * (0.6 + 0.4 * h / 0.48)
                })
                .sum::<f32>()
        }),
        _ => Vec::new(),
    };
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    if peak > 0.0 {
        for s in &mut out {
            *s *= 0.85 / peak;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transition_sounds_rise_and_hit() {
        let rms = |x: &[f32]| (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt();
        let riser = sound(RISER, 48_000);
        let secs = riser.len() as f32 / 48_000.0;
        assert!((5.0..=8.0).contains(&secs), "riser {secs} s");
        let q = riser.len() / 4;
        assert!(rms(&riser[3 * q..]) > rms(&riser[..q]) * 3.0, "it rises");
        let impact = sound(IMPACT, 48_000);
        let t = impact.len() / 8;
        assert!(rms(&impact[..t]) > rms(&impact[impact.len() - t..]) * 5.0, "it hits, then fades");
        assert!(impact.iter().chain(&riser).all(|s| s.abs() <= 0.9));
    }

    #[test]
    fn a_crash_rings_bright_and_a_fill_rolls() {
        let rms = |x: &[f32]| (x.iter().map(|s| s * s).sum::<f32>() / x.len().max(1) as f32).sqrt();
        let crash = sound(CRASH, 48_000);
        let secs = crash.len() as f32 / 48_000.0;
        assert!((1.5..=3.0).contains(&secs), "crash {secs} s");
        // Bright: sample-to-sample changes are large next to the level (lots of high frequencies).
        let diff = crash.windows(2).map(|w| (w[1] - w[0]).powi(2)).sum::<f32>().sqrt() / crash.iter().map(|v| v * v).sum::<f32>().sqrt();
        assert!(diff > 0.5, "bright: {diff}");
        assert!(rms(&crash[..crash.len() / 8]) > rms(&crash[crash.len() * 7 / 8..]) * 4.0, "rings out");
        let fill = sound(FILL, 48_000);
        let secs = fill.len() as f32 / 48_000.0;
        assert!((0.3..=0.8).contains(&secs), "a beat or so: {secs} s");
        // Separate hits: the loudness drops between them.
        let w = 480;
        let env: Vec<f32> = fill.chunks(w).map(rms).collect();
        let peaks = env.windows(3).filter(|x| x[1] > x[0] && x[1] >= x[2] && x[1] > 0.1).count();
        assert!(peaks >= 4, "{peaks} hits");
        assert!(crash.iter().chain(&fill).all(|s| s.abs() <= 0.9));
    }

    #[test]
    fn every_pad_has_a_short_sound_that_does_not_clip() {
        let sounds: Vec<Vec<f32>> = (0..PADS).map(|p| sound(p, 48_000)).collect();
        for (p, s) in sounds.iter().enumerate() {
            let secs = s.len() as f32 / 48_000.0;
            assert!(secs > 0.3 && secs <= 3.0, "pad {p}: {secs} s");
            let peak = s.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!(peak > 0.3 && peak <= 0.9, "pad {p}: peak {peak}");
            assert!(s.last().unwrap().abs() < 0.01, "pad {p} ends quietly, no click");
        }
        assert_ne!(sounds[0], sounds[1]);
        assert!(sound(99, 48_000).is_empty(), "no such pad");
    }
}
