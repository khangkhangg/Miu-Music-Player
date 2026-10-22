//! Hearing the alignment clicks through a microphone (F7.9): the first output clicks at 1.2 kHz,
//! the second at 2.4 kHz, and every fourth beat both click together. In a recording, the time
//! between the low and the high click of that "both" beat is how far apart the two outputs really
//! sound, Bluetooth delays and all.

/// Pitches of the first and second output's clicks (see `dual::ClickTrack`).
pub const PITCH_HZ: [f32; 2] = [1_200.0, 2_400.0];

/// Times (seconds) where a click of `hz` starts in a mono recording.
pub fn onsets(samples: &[f32], rate: u32, hz: f32) -> Vec<f64> {
    let env = envelope(samples, rate, hz);
    if env.is_empty() {
        return Vec::new();
    }
    let mut sorted = env.clone();
    sorted.sort_by(f32::total_cmp);
    let floor = sorted[sorted.len() / 2].max(1e-9);
    let peak = sorted[sorted.len() - 1];
    let threshold = (floor * 10.0).max(peak * 0.2);
    if peak < floor * 10.0 {
        return Vec::new();
    }
    let dead = (DEAD_TIME_S / HOP_S) as usize;
    let mut out = Vec::new();
    let mut i = 0;
    while i < env.len() {
        if env[i] < threshold {
            i += 1;
            continue;
        }
        // This click's own peak (within 10 ms), then back to where it first reached half of it.
        let top = (i..(i + (0.010 / HOP_S) as usize).min(env.len())).map(|j| env[j]).fold(0.0f32, f32::max);
        let mut start = i;
        while start > 0 && env[start - 1] >= top * 0.5 {
            start -= 1;
        }
        while env[start] < top * 0.5 {
            start += 1;
        }
        out.push(start as f64 * HOP_S + WINDOW_S / 2.0);
        i += dead;
    }
    out
}

const WINDOW_S: f64 = 0.004;
const HOP_S: f64 = 0.0005;
/// After a click, ignore the rest of it and its echoes for this long.
const DEAD_TIME_S: f64 = 0.15;

/// Strength of `hz` in Hann-windowed slices of `WINDOW_S`, every `HOP_S` (Goertzel).
fn envelope(samples: &[f32], rate: u32, hz: f32) -> Vec<f32> {
    let n = (WINDOW_S * rate as f64) as usize;
    let hop = ((HOP_S * rate as f64) as usize).max(1);
    if samples.len() < n {
        return Vec::new();
    }
    let window: Vec<f32> = (0..n).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / n as f32).cos()).collect();
    let coeff = 2.0 * (2.0 * std::f32::consts::PI * hz / rate as f32).cos();
    (0..=(samples.len() - n) / hop)
        .map(|k| {
            let (mut s1, mut s2) = (0.0f32, 0.0f32);
            for (x, w) in samples[k * hop..k * hop + n].iter().zip(&window) {
                let s = x * w + coeff * s1 - s2;
                s2 = s1;
                s1 = s;
            }
            (s1 * s1 + s2 * s2 - coeff * s1 * s2).max(0.0).sqrt()
        })
        .collect()
}

/// How far the second output sounds after the first (seconds; negative = before), from the
/// "both" beats in a recording: the median over every low click that has a high click within
/// `max_apart`. `None` when no such pair was heard.
pub fn offset(samples: &[f32], rate: u32, max_apart: f64) -> Option<f64> {
    let low = onsets(samples, rate, PITCH_HZ[0]);
    let high = onsets(samples, rate, PITCH_HZ[1]);
    let mut gaps: Vec<f64> = low
        .iter()
        .filter_map(|l| high.iter().map(|h| h - l).filter(|d| d.abs() <= max_apart).min_by(|a, b| a.abs().total_cmp(&b.abs())))
        .collect();
    if gaps.is_empty() {
        return None;
    }
    gaps.sort_by(f64::total_cmp);
    Some(gaps[gaps.len() / 2])
}


/// Microphones Windows offers, by name.
pub fn microphones() -> Vec<String> {
    use cpal::traits::{DeviceTrait, HostTrait};
    cpal::default_host().input_devices().map(|it| it.filter_map(|d| d.name().ok()).collect()).unwrap_or_default()
}

/// Record `secs` from the microphone named `name`, mixed to mono: (samples, rate).
pub fn record(name: &str, secs: f64) -> Result<(Vec<f32>, u32), String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::{Arc, Mutex};
    let mic = cpal::default_host()
        .input_devices()
        .map_err(|e| e.to_string())?
        .find(|d| d.name().ok().as_deref() == Some(name))
        .ok_or_else(|| format!("microphone {name:?} not found"))?;
    let config = mic.default_input_config().map_err(|e| e.to_string())?;
    let (rate, channels) = (config.sample_rate().0, config.channels() as usize);
    let got = Arc::new(Mutex::new(Vec::<f32>::with_capacity((secs * rate as f64) as usize)));
    let sink = got.clone();
    let stream = mic
        .build_input_stream(
            &config.config(),
            move |data: &[f32], _| {
                if let Ok(mut g) = sink.lock() {
                    g.extend(data.chunks_exact(channels).map(|f| f.iter().sum::<f32>() / channels as f32));
                }
            },
            |e| log::warn!("microphone error: {e}"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    std::thread::sleep(std::time::Duration::from_secs_f64(secs));
    drop(stream);
    let samples = got.lock().map_err(|e| e.to_string())?.clone();
    Ok((samples, rate))
}
#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    /// A recording: clicks of `hz` at `times`, added into `out`, with an echo 12 ms later.
    fn add_clicks(out: &mut [f32], times: &[f64], hz: f32, level: f32) {
        for &t in times {
            for (delay, gain) in [(0.0, 1.0), (0.012, 0.4)] {
                let start = ((t + delay) * RATE as f64) as usize;
                let len = RATE as usize * 8 / 1000;
                for i in 0..len {
                    if let Some(s) = out.get_mut(start + i) {
                        let x = i as f32 / RATE as f32;
                        *s += (2.0 * std::f32::consts::PI * hz * x).sin() * level * gain * (1.0 - i as f32 / len as f32);
                    }
                }
            }
        }
    }

    fn noise(len: usize, level: f32) -> Vec<f32> {
        let mut seed = 99u64;
        (0..len)
            .map(|_| {
                seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                ((seed >> 33) as f32 / (1u64 << 31) as f32 - 0.5) * level
            })
            .collect()
    }

    /// Two bars of the click pattern (first, second, both, rest at 400 ms a beat), the second
    /// output sounding `apart` seconds after the first, recorded from 0.3 s.
    fn recording(apart: f64) -> Vec<f32> {
        let mut out = noise(RATE as usize * 4, 0.02);
        let bars = [0.3, 1.9];
        let low: Vec<f64> = bars.iter().flat_map(|b| [b + 0.0, b + 0.8]).collect();
        let high: Vec<f64> = bars.iter().flat_map(|b| [b + 0.4 + apart, b + 0.8 + apart]).collect();
        add_clicks(&mut out, &low, PITCH_HZ[0], 0.3);
        add_clicks(&mut out, &high, PITCH_HZ[1], 0.15); // one speaker quieter, as in a room
        out
    }

    #[test]
    fn finds_each_click_of_one_pitch_and_ignores_the_other() {
        let r = recording(0.0);
        let low = onsets(&r, RATE, PITCH_HZ[0]);
        assert_eq!(low.len(), 4, "{low:?}");
        for (got, want) in low.iter().zip([0.3, 1.1, 1.9, 2.7]) {
            assert!((got - want).abs() < 0.002, "{low:?}");
        }
        assert_eq!(onsets(&r, RATE, PITCH_HZ[1]).len(), 4);
    }

    #[test]
    fn measures_how_far_the_second_output_sounds_after_the_first() {
        for apart in [0.0, 0.037, -0.052, 0.120] {
            let got = offset(&recording(apart), RATE, 0.2).expect("both beats heard");
            assert!((got - apart).abs() < 0.0015, "want {apart}, got {got}");
        }
    }

    #[test]
    fn silence_gives_no_answer() {
        assert_eq!(offset(&noise(RATE as usize * 2, 0.02), RATE, 0.2), None);
    }
}

// Machine test: records a microphone (`MIU_TEST_MIC`, default "Microphone (USB Audio and HID)")
// for `MIU_RECORD_SECS` (default 8) while something plays the alignment clicks, and prints what
// it hears.
#[cfg(test)]
mod machine_tests {
    use super::*;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use std::sync::{Arc, Mutex};

    #[test]
    #[ignore]
    fn hears_the_alignment_clicks_through_a_microphone() {
        let wanted = std::env::var("MIU_TEST_MIC").unwrap_or_else(|_| "Microphone (USB Audio and HID)".into());
        let secs: f64 = std::env::var("MIU_RECORD_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(8.0);
        let host = cpal::default_host();
        let mic = host.input_devices().unwrap().find(|d| d.name().ok().as_deref() == Some(wanted.as_str())).expect("microphone");
        let config = mic.default_input_config().unwrap();
        let (rate, channels) = (config.sample_rate().0, config.channels() as usize);
        let got = Arc::new(Mutex::new(Vec::<f32>::new()));
        let sink = got.clone();
        let stream = mic
            .build_input_stream(
                &config.config(),
                move |data: &[f32], _| {
                    let mut g = sink.lock().unwrap();
                    g.extend(data.chunks_exact(channels).map(|f| f.iter().sum::<f32>() / channels as f32));
                },
                |e| eprintln!("mic error: {e}"),
                None,
            )
            .unwrap();
        stream.play().unwrap();
        std::thread::sleep(std::time::Duration::from_secs_f64(secs));
        drop(stream);
        let samples = got.lock().unwrap().clone();
        let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        println!("{} s at {rate} Hz, peak {peak:.3}", samples.len() as f64 / rate as f64);
        println!("low (first output) clicks: {:?}", onsets(&samples, rate, PITCH_HZ[0]).iter().map(|t| (t * 1000.0).round()).collect::<Vec<_>>());
        println!("high (second output) clicks: {:?}", onsets(&samples, rate, PITCH_HZ[1]).iter().map(|t| (t * 1000.0).round()).collect::<Vec<_>>());
        match offset(&samples, rate, 0.2) {
            Some(o) => println!("MEASURED: the second output sounds {:+.1} ms after the first", o * 1000.0),
            None => println!("MEASURED: no pair of clicks heard"),
        }
    }
}

// Machine test: records the microphone for the calibration sound's length (start it in the app at
// the same time) and prints the level every 100 ms, to see what the microphone lets through.
#[cfg(test)]
mod level_profile {
    #[test]
    #[ignore]
    fn prints_the_microphone_level_over_time() {
        let mic = std::env::var("MIU_TEST_MIC").unwrap_or_else(|_| "Microphone (USB Audio and HID)".into());
        let (s, rate) = super::record(&mic, crate::audio::calibrate::duration_s() + 1.0).unwrap();
        let hop = rate as usize / 10;
        let line: Vec<String> = s.chunks(hop).map(|c| format!("{:.0}", 20.0 * ((c.iter().map(|x| x * x).sum::<f32>() / c.len() as f32).sqrt().max(1e-6)).log10())).collect();
        println!("dBFS every 100 ms: {}", line.join(" "));
    }
}
