//! Party pads (D.5): short sounds made in code (no sample files to ship or license): an air horn,
//! a siren, a laser and a bass drop.

use std::f32::consts::PI;

pub const PADS: usize = 4;

/// Attack/release envelope so a sound starts and ends without a click.
fn envelope(i: usize, n: usize, rate: u32) -> f32 {
    let edge = (0.01 * rate as f32) as usize;
    let a = (i as f32 / edge as f32).min(1.0);
    let r = ((n - i) as f32 / (edge * 4) as f32).min(1.0);
    a * r
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
        assert!(sound(PADS, 48_000).is_empty(), "no such pad");
    }
}
