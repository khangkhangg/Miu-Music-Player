//! Song structure for smarter transitions (D.7): the 8-bar phrase grid, sections (intro, main,
//! breakdown, build, drop, outro), where the big drop lands, where vocals probably are, and
//! where the music really ends. Measured bar by bar from loudness, bass, mid and high bands.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SectionKind {
    Intro,
    Main,
    Breakdown,
    Build,
    Drop,
    Outro,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Section {
    pub kind: SectionKind,
    pub start_s: f64,
    pub end_s: f64,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Structure {
    /// A phrase (8 bars) starts here and every 8 bars from it; None without a usable beat.
    pub phrase_start_s: Option<f64>,
    pub sections: Vec<Section>,
    /// Where the biggest drop lands (full bass back after a breakdown).
    pub drop_s: Option<f64>,
    /// Stretches that probably have singing or rapping, [start, end] seconds.
    pub vocals: Vec<[f64; 2]>,
    /// Where the music really stops (trailing silence left out).
    pub end_s: f64,
}

/// Bars per phrase.
const PHRASE_BARS: usize = 8;
/// A bar counts as "full" (the song at full body) at this share of the song's usual loudness.
const FULL: f32 = 0.6;
/// Below this share of the usual bass, the bass has dropped out (breakdown, build).
const NO_BASS: f32 = 0.35;
/// Shorter bass-less stretches are fills, not breakdowns.
const MIN_BREAK_BARS: usize = 2;
/// A breakdown this many bars long or more makes the full section after it a drop.
const DROP_AFTER_BARS: usize = 4;

/// One-pole low-pass, twice (12 dB/octave): cheap band splitting for loudness measurements.
fn low_pass(x: &[f32], r: u32, hz: f32) -> Vec<f32> {
    let a = (-2.0 * std::f32::consts::PI * hz / r as f32).exp();
    let (mut y1, mut y2) = (0.0f32, 0.0f32);
    x.iter()
        .map(|&s| {
            y1 = s + a * (y1 - s);
            y2 = y1 + a * (y2 - y1);
            y2
        })
        .collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

/// The value below which `share` of `v` lies.
fn percentile(v: &[f32], share: f32) -> f32 {
    let mut s: Vec<f32> = v.to_vec();
    s.sort_by(f32::total_cmp);
    s.get(((s.len() as f32 * share) as usize).min(s.len().saturating_sub(1))).copied().unwrap_or(0.0)
}

/// Each band measured per bar, as a share of its usual (80th percentile) level.
struct Bars {
    start_s: Vec<f64>,
    len_s: f64,
    loud: Vec<f32>,
    bass: Vec<f32>,
    mid_ratio: Vec<f32>,
    high: Vec<f32>,
}

fn measure(x: &[f32], r: u32, first_s: f64, len_s: f64, end_s: f64) -> Bars {
    let bass_band = low_pass(x, r, 150.0);
    let below_300 = low_pass(x, r, 300.0);
    let below_3k = low_pass(x, r, 3000.0);
    let mid_band: Vec<f32> = below_3k.iter().zip(&below_300).map(|(a, b)| a - b).collect();
    let high_band: Vec<f32> = x.iter().zip(&below_3k).map(|(a, b)| a - b).collect();
    let mut b = Bars { start_s: Vec::new(), len_s, loud: Vec::new(), bass: Vec::new(), mid_ratio: Vec::new(), high: Vec::new() };
    let mut t = first_s;
    while t + len_s * 0.5 < end_s {
        let (i, j) = ((t * r as f64) as usize, (((t + len_s).min(end_s)) * r as f64) as usize);
        let (i, j) = (i.min(x.len()), j.min(x.len()));
        let whole = rms(&x[i..j]);
        b.start_s.push(t);
        b.loud.push(whole);
        b.bass.push(rms(&bass_band[i..j]));
        b.mid_ratio.push(rms(&mid_band[i..j]) / whole.max(1e-6));
        b.high.push(rms(&high_band[i..j]));
        t += len_s;
    }
    for v in [&mut b.loud, &mut b.bass, &mut b.high] {
        let usual = percentile(v, 0.8).max(1e-6);
        for x in v.iter_mut() {
            *x /= usual;
        }
    }
    b
}

/// Where the music really stops: the end of the last quarter second above -26 dB of its usual level.
fn music_end(x: &[f32], r: u32) -> f64 {
    let w = (r as usize / 4).max(1);
    let levels: Vec<f32> = x.chunks(w).map(rms).collect();
    let usual = percentile(&levels, 0.8);
    let last = levels.iter().rposition(|&v| v > usual * 0.05).map_or(0, |i| i + 1);
    (last * w).min(x.len()) as f64 / r as f64
}

/// `beat`: (bpm, downbeat) when the song has a usable beat grid.
pub fn structure(x: &[f32], r: u32, beat: Option<(f32, f64)>) -> Structure {
    let end_s = music_end(x, r);
    // Bars on the beat grid, or plain 2 s windows for songs without a beat.
    let (first_s, len_s) = match beat {
        Some((bpm, downbeat)) if bpm > 0.0 => {
            let bar = 240.0 / bpm as f64;
            (downbeat.rem_euclid(bar), bar)
        }
        _ => (0.0, 2.0),
    };
    let b = measure(x, r, first_s, len_s, end_s);
    let n = b.loud.len();
    if n == 0 {
        return Structure { end_s, ..Default::default() };
    }
    let bar_end = |i: usize| (b.start_s[i] + b.len_s).min(end_s);
    let full: Vec<bool> = b.loud.iter().map(|&v| v >= FULL).collect();
    let (Some(first_full), Some(last_full)) = (full.iter().position(|&f| f), full.iter().rposition(|&f| f)) else {
        return Structure { end_s, sections: vec![Section { kind: SectionKind::Main, start_s: 0.0, end_s }], ..Default::default() };
    };

    // Kind of every bar: intro and outro around the full body; inside it, runs without bass are a
    // breakdown, their rising-noise tail a build, and the full bars after a long one a drop.
    let mut kind = vec![SectionKind::Main; n];
    for k in kind.iter_mut().take(first_full) {
        *k = SectionKind::Intro;
    }
    for k in kind.iter_mut().skip(last_full + 1) {
        *k = SectionKind::Outro;
    }
    let mut drops: Vec<(usize, f32)> = Vec::new();
    let mut i = first_full;
    while i <= last_full {
        if b.bass[i] >= NO_BASS {
            i += 1;
            continue;
        }
        let start = i;
        while i <= last_full && b.bass[i] < NO_BASS {
            i += 1;
        }
        let run = start..i;
        if run.len() < MIN_BREAK_BARS {
            continue; // a dip of a bar is a fill, not a breakdown
        }
        let peak_high = run.clone().map(|j| b.high[j]).fold(0.0f32, f32::max);
        // The build: from the first bar after which the highs stay up (a riser, a snare roll).
        let build_from = run.clone().find(|&j| (j..i).all(|k| b.high[k] >= 0.25 * peak_high) && b.high[j] > b.high[start] * 2.0 + 1e-3);
        for j in run.clone() {
            kind[j] = if build_from.is_some_and(|f| j >= f) { SectionKind::Build } else { SectionKind::Breakdown };
        }
        if run.len() >= DROP_AFTER_BARS && i <= last_full {
            let jump = b.bass[i] - b.bass[i - 1];
            let mut j = i;
            while j <= last_full && b.bass[j] >= NO_BASS {
                kind[j] = SectionKind::Drop;
                j += 1;
            }
            drops.push((i, jump));
        }
    }

    let mut sections: Vec<Section> = Vec::new();
    for (j, &k) in kind.iter().enumerate() {
        match sections.last_mut() {
            Some(s) if s.kind == k => s.end_s = bar_end(j),
            _ => sections.push(Section { kind: k, start_s: if j == 0 { 0.0 } else { b.start_s[j] }, end_s: bar_end(j) }),
        }
    }

    let drop_s = beat.and(drops.iter().max_by(|a, c| a.1.total_cmp(&c.1)).map(|&(i, _)| b.start_s[i]));

    // Phrases start where the music changes most, every 8 bars.
    let phrase_start_s = beat.map(|_| {
        let change = |j: usize| (b.loud[j] - b.loud[j - 1]).abs() + (b.bass[j] - b.bass[j - 1]).abs() + (b.high[j] - b.high[j - 1]).abs();
        let best = (0..PHRASE_BARS.min(n))
            .max_by(|&o, &p| {
                let score = |o: usize| (o.max(1)..n).filter(|j| j % PHRASE_BARS == o).map(change).sum::<f32>();
                score(o).total_cmp(&score(p))
            })
            .unwrap_or(0);
        b.start_s[best]
    });

    // Vocals: bars with clearly more in the middle band than the song usually has. Only where the
    // bass plays, so pads in a breakdown are not taken for a voice.
    let with_bass: Vec<usize> = (0..n).filter(|&j| b.bass[j] >= NO_BASS && b.loud[j] > 0.2).collect();
    let usual_mid = percentile(&with_bass.iter().map(|&j| b.mid_ratio[j]).collect::<Vec<_>>(), 0.5);
    let mut vocals: Vec<[f64; 2]> = Vec::new();
    for &j in &with_bass {
        if b.mid_ratio[j] > usual_mid * 1.15 {
            match vocals.last_mut() {
                Some(v) if (v[1] - b.start_s[j]).abs() < 1e-6 => v[1] = bar_end(j),
                _ => vocals.push([b.start_s[j], bar_end(j)]),
            }
        }
    }

    Structure { phrase_start_s, sections, drop_s, vocals, end_s }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    const R: u32 = 11_025;

    /// A made-up 120 BPM dance song (a bar is 2 s), every part 8 bars (16 s) or more:
    /// intro 0-16 (soft kick and hats), main 16-48 (kick, hats, bass, pad; a voice 24-40),
    /// breakdown 48-64 (pad only), build 64-80 (pad and a rising noise roll),
    /// drop 80-112 (everything, louder), outro 112-128 (soft kick and hats), then 6 s of silence.
    fn song() -> Vec<f32> {
        let n = (134.0 * R as f32) as usize;
        let mut seed = 3u64;
        let mut noise = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as f32 / (1u64 << 31) as f32 - 0.5
        };
        (0..n)
            .map(|i| {
                let t = i as f32 / R as f32;
                let beat_t = t % 0.5;
                let kick = (2.0 * PI * 55.0 * t).sin() * (-beat_t * 18.0).exp();
                let hat = noise() * (-((t + 0.25) % 0.5) * 60.0).exp() * 0.5;
                let bass = (2.0 * PI * 55.0 * t).sin() * 0.35;
                let pad = [220.0, 277.2, 329.6].iter().map(|f| (2.0 * PI * f * t).sin()).sum::<f32>() * 0.06;
                let vib = 1.0 + 0.02 * (2.0 * PI * 5.0 * t).sin();
                let voice = (1..=6).map(|k| (2.0 * PI * 300.0 * vib * k as f32 * t).sin() / k as f32).sum::<f32>()
                    * 0.22 * if beat_t < 0.4 { 1.0 } else { 0.2 };
                let roll = noise() * ((t - 64.0) / 16.0).clamp(0.0, 1.0) * 0.6;
                match t {
                    t if t < 16.0 => 0.4 * (kick + hat),
                    t if t < 48.0 => kick + hat + bass + pad + if (24.0..40.0).contains(&t) { voice } else { 0.0 },
                    t if t < 64.0 => pad,
                    t if t < 80.0 => pad + roll,
                    t if t < 112.0 => 1.3 * (kick + hat + bass + pad),
                    t if t < 128.0 => 0.4 * (kick + hat),
                    _ => 0.0,
                }
            })
            .collect()
    }

    fn find(s: &Structure, kind: SectionKind) -> Vec<&Section> {
        s.sections.iter().filter(|x| x.kind == kind).collect()
    }

    #[test]
    fn finds_the_sections_of_a_dance_song() {
        let s = structure(&song(), R, Some((120.0, 0.0)));
        let near = |a: f64, b: f64| (a - b).abs() <= 4.0;
        let intro = find(&s, SectionKind::Intro);
        assert!(intro.len() == 1 && near(intro[0].end_s, 16.0), "intro {intro:?}");
        let breakdown = find(&s, SectionKind::Breakdown);
        assert!(breakdown.iter().any(|b| near(b.start_s, 48.0)), "breakdown {:?}", s.sections);
        let build = find(&s, SectionKind::Build);
        assert!(build.iter().any(|b| b.start_s >= 56.0 && near(b.end_s, 80.0)), "build {:?}", s.sections);
        let drops = find(&s, SectionKind::Drop);
        assert!(drops.iter().any(|d| near(d.start_s, 80.0)), "drop {:?}", s.sections);
        assert!(s.drop_s.is_some_and(|d| near(d, 80.0)), "drop at {:?}", s.drop_s);
        let outro = find(&s, SectionKind::Outro);
        assert!(outro.len() == 1 && near(outro[0].start_s, 112.0), "outro {outro:?}");
        // Sections follow each other without gaps.
        for w in s.sections.windows(2) {
            assert!((w[0].end_s - w[1].start_s).abs() < 1e-6, "{:?}", s.sections);
        }
    }

    #[test]
    fn a_one_bar_dip_in_the_bass_is_not_a_breakdown() {
        // The main part of the song with the bass and kick gone for one bar (36-38 s).
        let x: Vec<f32> = song().iter().enumerate().map(|(i, &s)| {
            let t = i as f32 / R as f32;
            if (36.0..38.0).contains(&t) { s * 0.1 } else { s }
        }).collect();
        let s = structure(&x, R, Some((120.0, 0.0)));
        assert!(!s.sections.iter().any(|x| x.kind != SectionKind::Main && x.start_s >= 30.0 && x.end_s <= 44.0), "{:?}", s.sections);
    }

    #[test]
    fn phrases_start_where_the_music_changes() {
        let s = structure(&song(), R, Some((120.0, 0.0)));
        let p = s.phrase_start_s.expect("a phrase grid");
        let off = (p / 16.0).fract();
        assert!(off < 0.01 || off > 0.99, "phrases every 16 s from {p}");
        // A grid shifted by two bars from the real phrases is put back on them.
        let s = structure(&song(), R, Some((120.0, 4.0)));
        let off = (s.phrase_start_s.unwrap() / 16.0).fract();
        assert!(off < 0.01 || off > 0.99, "{:?}", s.phrase_start_s);
    }

    #[test]
    fn finds_where_someone_sings() {
        let s = structure(&song(), R, Some((120.0, 0.0)));
        assert!(!s.vocals.is_empty(), "no vocals found");
        let sung: f64 = s.vocals.iter().map(|v| (v[1].min(40.0) - v[0].max(24.0)).max(0.0)).sum();
        let wrong: f64 = s.vocals.iter().map(|v| v[1] - v[0]).sum::<f64>() - sung;
        assert!(sung >= 10.0, "found {sung} s of the 16 s sung: {:?}", s.vocals);
        assert!(wrong <= 6.0, "{wrong} s marked sung that is not: {:?}", s.vocals);
    }

    #[test]
    fn the_real_end_leaves_out_trailing_silence() {
        let s = structure(&song(), R, Some((120.0, 0.0)));
        assert!((s.end_s - 128.0).abs() < 1.0, "end {}", s.end_s);
    }

    #[test]
    fn a_song_without_a_beat_has_no_phrases_or_drop() {
        let pad: Vec<f32> = (0..R as usize * 40)
            .map(|i| {
                let t = i as f32 / R as f32;
                (2.0 * PI * 220.0 * t).sin() * 0.3 * (t / 4.0).min(1.0)
            })
            .collect();
        let s = structure(&pad, R, None);
        assert_eq!(s.phrase_start_s, None);
        assert_eq!(s.drop_s, None);
        assert!((s.end_s - 40.0).abs() < 1.0);
    }
}
