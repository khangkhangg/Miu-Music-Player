//! Auto DJ (D.3): which song comes next (key, tempo and the chosen energy curve), how to mix into
//! it (beat blend, echo out, crossfade or cut), and the moves of the mixer during the transition.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::analysis::{keys_compatible, Analysis};
use super::deck::TEMPO_RANGE;

/// Below this beat confidence a song is mixed without beat-matching (ballads, bolero, acoustic).
pub const MIN_BEAT_CONFIDENCE: f32 = 0.2;
/// Energy 7 and up counts as high energy (pop, hip hop, dance).
const HIGH_ENERGY: u8 = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EnergyCurve {
    /// Start gentle and lift slowly (early in the party).
    WarmUp,
    #[default]
    Keep,
    BuildUp,
    WindDown,
}

impl EnergyCurve {
    fn target(self, now: u8) -> i32 {
        let now = now as i32;
        match self {
            EnergyCurve::WarmUp => now + 1,
            EnergyCurve::Keep => now,
            EnergyCurve::BuildUp => now + 2,
            EnergyCurve::WindDown => now - 2,
        }
        .clamp(1, 10)
    }
}

/// A song Auto DJ may play next.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub song_id: i64,
    pub analysis: Analysis,
    pub liked: bool,
}

/// The tempo change the incoming song needs to match `from_bpm` (counting half and double time),
/// if it is within the tempo range.
pub fn tempo_change(from_bpm: f32, to_bpm: f32) -> Option<f64> {
    if from_bpm <= 0.0 || to_bpm <= 0.0 {
        return None;
    }
    [1.0, 2.0, 0.5]
        .into_iter()
        .map(|per| from_bpm as f64 / (to_bpm as f64 * per) - 1.0)
        .filter(|c| c.abs() <= TEMPO_RANGE + 1e-9)
        .min_by(|a, b| a.abs().total_cmp(&b.abs()))
}

fn keys_fit(a: &Analysis, b: &Analysis) -> Option<bool> {
    Some(keys_compatible(a.key.as_deref()?, b.key.as_deref()?))
}

/// A small, stable per-party shuffle so equal songs don't always come in the same order.
fn jitter(song_id: i64, seed: u64) -> f64 {
    let mut x = (song_id as u64) ^ seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    (x % 1000) as f64 / 1000.0 * 0.2
}

/// The next song after `now`: the first unplayed song in `queue` if there is one, else the
/// unplayed candidate that fits best (tempo in reach, compatible key, energy on the curve, liked).
pub fn pick(now: &Analysis, candidates: &[Candidate], played: &HashSet<i64>, queue: &[i64], curve: EnergyCurve, seed: u64) -> Option<i64> {
    if let Some(id) = queue.iter().find(|id| !played.contains(id)) {
        return Some(*id);
    }
    let target = curve.target(now.energy);
    let score = |c: &Candidate| {
        let a = &c.analysis;
        let tempo = tempo_change(now.bpm, a.bpm).map_or(10.0, |d| d.abs() / TEMPO_RANGE);
        let key = match keys_fit(now, a) {
            Some(true) => 0.0,
            Some(false) => 2.0,
            None => 0.5,
        };
        let energy = (a.energy as i32 - target).abs() as f64 * 0.5;
        let liked = if c.liked { -0.3 } else { 0.0 };
        tempo + key + energy + liked + jitter(c.song_id, seed)
    };
    candidates
        .iter()
        .filter(|c| !played.contains(&c.song_id))
        .map(|c| (score(c), c.song_id))
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, id)| id)
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Transition {
    /// Tempo synced, downbeats lined up, a long crossfade with lows, mids and highs swapped in turn.
    BeatBlend { bars: u8 },
    /// The new song comes in thin (high-pass) and opens up while the old one goes dark (low-pass).
    FilterBlend { bars: u8 },
    /// The new song's intro comes in under the old song's breakdown (no two sets of drums).
    BreakdownBlend { bars: u8 },
    /// The end of the old song is looped to make room for a full blend.
    LoopOut { bars: u8 },
    /// The old song builds up; the new one comes in right on its drop.
    DropSwap { bars: u8 },
    /// A wash of reverb freezes the old song; the new one rises out of it.
    ReverbWash,
    /// The old record is spun back; the new one slams in.
    SpinBack,
    /// The old record slows to a stop; the new one starts.
    Brake,
    /// A ballad ends almost by itself, a moment of quiet, the next one starts softly.
    BalladGap,
    /// Big tempo gap: the old song glides to the new tempo with key lock, then a beat blend.
    TempoRamp { bars: u8 },
    /// Clashing keys: the new song is shifted a semitone or two to fit, then a beat blend.
    KeyShiftBlend { bars: u8, semitones: i8 },
    /// The outgoing song is swept away with a high-pass; the new one drops in on a downbeat.
    EchoOut,
    /// A plain crossfade at the mix points, no beat-matching.
    Crossfade { secs: f32 },
    /// Straight over on a downbeat.
    Cut,
}

/// How to mix from one song into the next.
/// The speed the old song glides to so it plays at the new song's tempo (counting half and double
/// time), when that is within 25 %: key lock keeps the pitch while it moves.
pub fn ramp_target(from_bpm: f32, to_bpm: f32) -> Option<f64> {
    if from_bpm <= 0.0 || to_bpm <= 0.0 {
        return None;
    }
    [1.0, 2.0, 0.5]
        .into_iter()
        .map(|per| to_bpm as f64 * per / from_bpm as f64)
        .filter(|r| (r - 1.0).abs() <= 0.25)
        .min_by(|a, b| (a - 1.0).abs().total_cmp(&(b - 1.0).abs()))
}

/// The smallest pitch shift (up to two semitones) that puts the new song in a key that fits.
fn key_shift_to_fit(from: &Analysis, to: &Analysis) -> Option<i8> {
    let (a, b) = (from.key.as_deref()?, to.key.as_deref()?);
    [1i8, -1, 2, -2].into_iter().find(|&s| super::analysis::shift_camelot(b, s).is_some_and(|k| keys_compatible(a, &k)))
}

/// How Auto DJ mixes: long smooth blends, party drama (drops, risers, tricks) or chilled filters,
/// washes and fades.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Style {
    #[default]
    Smooth,
    Party,
    Chill,
}

/// Energy up to this counts as a quiet song (ballad, acoustic).
const QUIET: u8 = 4;
/// How far from the mix points vocals are checked for overlap (seconds).
const VOCAL_WINDOW_S: f64 = 40.0;

fn sings_between(a: &Analysis, from_s: f64, to_s: f64) -> bool {
    a.structure.vocals.iter().any(|v| v[0] < to_s && v[1] > from_s)
}

/// How to mix from one song into the next, by tempo, key, energy and the songs' structure, in
/// the chosen style. `seed` varies the party tricks.
pub fn plan(from: &Analysis, to: &Analysis, style: Style, seed: u64) -> Transition {
    use Transition::*;
    let party = style == Style::Party;
    // No clear beat (ballads, bolero, acoustic): no beat-matching.
    if from.beat_confidence < MIN_BEAT_CONFIDENCE || to.beat_confidence < MIN_BEAT_CONFIDENCE {
        return if from.energy <= QUIET && to.energy <= QUIET + 1 {
            if party { Crossfade { secs: 6.0 } } else { BalladGap }
        } else if style == Style::Chill {
            ReverbWash
        } else {
            Crossfade { secs: 8.0 }
        };
    }
    let tempo_ok = tempo_change(from.bpm, to.bpm).is_some();
    let key_ok = keys_fit(from, to) != Some(false);
    if !tempo_ok && key_ok && style != Style::Party && ramp_target(from.bpm, to.bpm).is_some() {
        return TempoRamp { bars: 32 };
    }
    if tempo_ok && key_ok {
        if party && to.structure.drop_s.is_some() && from.energy >= 6 {
            return DropSwap { bars: 8 };
        }
        if style != Style::Party && last_breakdown(from).is_some() {
            return BreakdownBlend { bars: 8 };
        }
        let has_outro = from.structure.sections.iter().any(|s| s.kind == super::structure::SectionKind::Outro);
        if !has_outro && real_end(from) - from.mix_out_s < 4.0 {
            return LoopOut { bars: 8 };
        }
        // One voice at a time: a short filter blend when both sing around the mix.
        let clash = sings_between(from, from.mix_out_s - VOCAL_WINDOW_S, from.mix_out_s) && sings_between(to, to.mix_in_s, to.mix_in_s + VOCAL_WINDOW_S);
        if clash {
            return FilterBlend { bars: 8 };
        }
        return match style {
            Style::Chill => FilterBlend { bars: 16 },
            Style::Party => BeatBlend { bars: 8 },
            Style::Smooth => BeatBlend { bars: 16 },
        };
    }
    if tempo_ok {
        // In tempo but the keys clash: tune the new song to fit, or get the old one out of the way.
        if let Some(semitones) = key_shift_to_fit(from, to) {
            return KeyShiftBlend { bars: 16, semitones };
        }
        return if style == Style::Chill { ReverbWash } else { EchoOut };
    }
    // Far apart in tempo.
    if from.energy >= HIGH_ENERGY && to.energy >= HIGH_ENERGY {
        return if party { if seed % 2 == 0 { SpinBack } else { Brake } } else { Cut };
    }
    match style {
        Style::Party => Brake,
        Style::Chill => ReverbWash,
        Style::Smooth => EchoOut,
    }
}

fn bar_s(a: &Analysis) -> Option<f64> {
    (a.bpm > 0.0).then(|| 4.0 * 60.0 / a.bpm as f64)
}

impl Transition {
    /// How long the transition takes, in the outgoing song's time.
    pub fn duration_s(self, from: &Analysis) -> f64 {
        match self {
            Transition::BeatBlend { bars } => bar_s(from).map_or(16.0, |b| b * bars as f64),
            Transition::FilterBlend { bars } | Transition::BreakdownBlend { bars } | Transition::LoopOut { bars } | Transition::DropSwap { bars } | Transition::TempoRamp { bars } | Transition::KeyShiftBlend { bars, .. } => {
                bar_s(from).map_or(16.0, |b| b * bars as f64)
            }
            Transition::EchoOut => bar_s(from).map_or(8.0, |b| b * 4.0),
            Transition::ReverbWash => 8.0,
            Transition::SpinBack => 1.5,
            Transition::Brake => 2.0,
            Transition::BalladGap => 8.0,
            Transition::Crossfade { secs } => secs as f64,
            Transition::Cut => 0.0,
        }
    }

    fn on_the_beat(self) -> bool {
        !matches!(self, Transition::Crossfade { .. } | Transition::ReverbWash | Transition::BalladGap)
    }
}

/// The grid transitions line up with: 8-bar phrases when known, else bars.
fn grid(from: &Analysis) -> Option<(f64, f64)> {
    let bar = bar_s(from).filter(|_| from.beat_confidence >= MIN_BEAT_CONFIDENCE)?;
    Some(match from.structure.phrase_start_s {
        Some(p) => (p, bar * 8.0),
        None => (from.downbeat_s, bar),
    })
}

/// Where the outgoing song really ends (trailing silence left out).
fn real_end(from: &Analysis) -> f64 {
    if from.structure.end_s > 0.0 { from.structure.end_s } else { from.duration_s.max(from.mix_out_s) }
}

/// Where in the outgoing song the transition starts. The moment the new song takes over lands
/// on a phrase (or bar) and the transition is over by the mix-out point; a ballad gap fades out
/// by the real end; a breakdown blend starts on the old song's last breakdown.
pub fn start_at(from: &Analysis, t: Transition) -> f64 {
    let dur = t.duration_s(from);
    if t == Transition::BalladGap {
        return (real_end(from) - BALLAD_FADE * dur).max(0.0);
    }
    if let Transition::BreakdownBlend { .. } = t {
        if let Some(b) = last_breakdown(from) {
            return b;
        }
    }
    let switch_p = super::script::switch_point(t) as f64;
    let latest_switch = from.mix_out_s - (1.0 - switch_p) * dur;
    let switch = match grid(from).filter(|_| t.on_the_beat()) {
        Some((anchor, step)) => anchor + ((latest_switch - anchor) / step + 1e-9).floor() * step,
        None => latest_switch,
    };
    (switch - switch_p * dur).max(0.0)
}

/// Share of a ballad gap spent fading the old song out (it is silent by the real end).
pub const BALLAD_FADE: f64 = 0.55;

/// Start of the old song's last breakdown in its second half, if it has one.
pub fn last_breakdown(from: &Analysis) -> Option<f64> {
    from.structure
        .sections
        .iter()
        .filter(|s| s.kind == super::structure::SectionKind::Breakdown && s.start_s > from.duration_s / 2.0 && s.start_s < from.mix_out_s)
        .map(|s| s.start_s)
        .last()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dj::analysis::Analysis;
    use std::collections::HashSet;

    fn song(bpm: f32, key: &str, energy: u8, confidence: f32) -> Analysis {
        Analysis {
            bpm,
            beat_confidence: confidence,
            first_beat_s: 0.5,
            downbeat_s: 0.5,
            key: Some(key.into()),
            energy,
            mix_in_s: 8.0,
            mix_out_s: 200.0,
            duration_s: 210.0,
            ..Default::default()
        }
    }

    fn cand(id: i64, a: Analysis) -> Candidate {
        Candidate { song_id: id, analysis: a, liked: false }
    }

    #[test]
    fn picks_a_song_in_a_compatible_key_and_reachable_tempo() {
        let now = song(124.0, "8A", 6, 0.8);
        let list = vec![
            cand(1, song(124.0, "3B", 6, 0.8)), // key clash
            cand(2, song(150.0, "8A", 6, 0.8)), // tempo out of reach
            cand(3, song(126.0, "9A", 6, 0.8)), // fits
        ];
        assert_eq!(pick(&now, &list, &HashSet::new(), &[], EnergyCurve::Keep, 0), Some(3));
    }

    #[test]
    fn never_repeats_and_queued_songs_win() {
        let now = song(124.0, "8A", 6, 0.8);
        let list = vec![cand(1, song(124.0, "8A", 6, 0.8)), cand(2, song(90.0, "1B", 2, 0.1))];
        let played: HashSet<i64> = [1].into();
        assert_eq!(pick(&now, &list, &played, &[], EnergyCurve::Keep, 0), Some(2), "only one left");
        assert_eq!(pick(&now, &list, &HashSet::new(), &[2], EnergyCurve::Keep, 0), Some(2), "the queue wins");
        let all: HashSet<i64> = [1, 2].into();
        assert_eq!(pick(&now, &list, &all, &[], EnergyCurve::Keep, 0), None);
    }

    #[test]
    fn the_energy_curve_steers_the_choice() {
        let now = song(124.0, "8A", 5, 0.8);
        let list = vec![cand(1, song(124.0, "8A", 3, 0.8)), cand(2, song(124.0, "8A", 5, 0.8)), cand(3, song(124.0, "8A", 7, 0.8))];
        let none = HashSet::new();
        assert_eq!(pick(&now, &list, &none, &[], EnergyCurve::BuildUp, 0), Some(3));
        assert_eq!(pick(&now, &list, &none, &[], EnergyCurve::Keep, 0), Some(2));
        assert_eq!(pick(&now, &list, &none, &[], EnergyCurve::WindDown, 0), Some(1));
    }

    #[test]
    fn liked_songs_are_preferred_among_equals() {
        let now = song(124.0, "8A", 5, 0.8);
        let mut liked = cand(2, song(124.0, "8A", 5, 0.8));
        liked.liked = true;
        let list = vec![cand(1, song(124.0, "8A", 5, 0.8)), liked];
        assert_eq!(pick(&now, &list, &HashSet::new(), &[], EnergyCurve::Keep, 0), Some(2));
    }

    #[test]
    fn the_transition_suits_the_pair() {
        use Style::*;
        let house = song(124.0, "8A", 6, 0.8);
        let p = |a: &Analysis, b: &Analysis, s| plan(a, b, s, 0);
        assert_eq!(p(&house, &song(126.0, "9A", 6, 0.8), Smooth), Transition::BeatBlend { bars: 16 });
        assert_eq!(p(&house, &song(63.0, "8A", 6, 0.8), Smooth), Transition::BeatBlend { bars: 16 }, "half time counts");
        assert_eq!(p(&house, &song(126.0, "9A", 6, 0.8), Party), Transition::BeatBlend { bars: 8 }, "shorter at a party");
        assert_eq!(p(&house, &song(126.0, "9A", 6, 0.8), Chill), Transition::FilterBlend { bars: 16 });
        assert_eq!(p(&house, &song(90.0, "8A", 6, 0.8), Smooth), Transition::EchoOut, "big tempo jump");
        assert_eq!(p(&house, &song(108.0, "8A", 6, 0.8), Smooth), Transition::TempoRamp { bars: 32 }, "a tempo gap key lock can bridge");
        assert_eq!(p(&house, &song(124.0, "10A", 6, 0.8), Smooth), Transition::KeyShiftBlend { bars: 16, semitones: -2 }, "two semitones down fit the key");
        assert_eq!(ramp_target(124.0, 108.0), Some(108.0 / 124.0));
        assert_eq!(ramp_target(124.0, 60.0), Some(120.0 / 124.0), "half time");
        assert_eq!(ramp_target(124.0, 80.0), None, "too far");
        assert_eq!(p(&house, &song(124.0, "2B", 6, 0.8), Smooth), Transition::EchoOut, "key clash");
        assert_eq!(p(&house, &song(124.0, "2B", 6, 0.8), Chill), Transition::ReverbWash, "key clash, chilled");
        assert_eq!(p(&house, &song(124.0, "8A", 6, 0.1), Smooth), Transition::Crossfade { secs: 8.0 }, "no clear beat");
        let far_a = song(95.0, "4A", 8, 0.7);
        let far_b = song(128.0, "11B", 8, 0.7);
        assert_eq!(p(&far_a, &far_b, Smooth), Transition::Cut, "high energy, far apart");
        assert!(matches!(p(&far_a, &far_b, Party), Transition::SpinBack | Transition::Brake), "a party trick");
        assert_ne!(plan(&far_a, &far_b, Party, 0), plan(&far_a, &far_b, Party, 1), "spin-back and brake take turns");
        assert_eq!(p(&song(90.0, "4A", 4, 0.8), &song(128.0, "11B", 5, 0.8), Party), Transition::Brake);
    }

    #[test]
    fn ballads_get_a_gap_and_vocals_are_not_stacked() {
        use Style::*;
        let ballad = song(70.0, "5A", 3, 0.1);
        assert_eq!(plan(&ballad, &song(66.0, "6A", 4, 0.12), Smooth, 0), Transition::BalladGap);
        assert_eq!(plan(&ballad, &song(66.0, "6A", 4, 0.12), Party, 0), Transition::Crossfade { secs: 6.0 });
        // The old song sings to its end and the new one starts singing: one voice at a time.
        let mut singer = song(124.0, "8A", 6, 0.8);
        singer.structure.vocals = vec![[180.0, 200.0]];
        let mut next = song(124.0, "8A", 6, 0.8);
        next.structure.vocals = vec![[10.0, 30.0]];
        assert_eq!(plan(&singer, &next, Smooth, 0), Transition::FilterBlend { bars: 8 });
    }

    #[test]
    fn song_structure_picks_drop_swaps_breakdown_blends_and_loop_outs() {
        use crate::dj::structure::{Section, SectionKind};
        use Style::*;
        let mut from = song(124.0, "8A", 7, 0.8);
        let mut to = song(124.0, "8A", 8, 0.8);
        to.structure.drop_s = Some(60.0);
        assert_eq!(plan(&from, &to, Party, 0), Transition::DropSwap { bars: 8 }, "into the drop at a party");
        from.structure.sections = vec![
            Section { kind: SectionKind::Main, start_s: 0.0, end_s: 150.0 },
            Section { kind: SectionKind::Breakdown, start_s: 150.0, end_s: 170.0 },
            Section { kind: SectionKind::Main, start_s: 170.0, end_s: 200.0 },
            Section { kind: SectionKind::Outro, start_s: 200.0, end_s: 210.0 },
        ];
        assert_eq!(plan(&from, &to, Smooth, 0), Transition::BreakdownBlend { bars: 8 });
        // No outro: the song stops at full body; loop its end to make room.
        let mut abrupt = song(124.0, "8A", 6, 0.8);
        abrupt.mix_out_s = 209.0;
        abrupt.structure.end_s = 210.0;
        assert_eq!(plan(&abrupt, &song(124.0, "8A", 6, 0.8), Smooth, 0), Transition::LoopOut { bars: 8 });
    }

    #[test]
    fn transitions_start_on_a_downbeat_to_end_at_the_mix_out_point() {
        let a = song(120.0, "8A", 6, 0.8); // 2 s a bar, downbeat at 0.5
        let blend = Transition::BeatBlend { bars: 16 };
        assert!((blend.duration_s(&a) - 32.0).abs() < 1e-9);
        let start = start_at(&a, blend);
        assert!((start - 166.5).abs() < 1e-9, "200 - 32 = 168, the downbeat at or before it: {start}");
        assert!((start_at(&a, Transition::Crossfade { secs: 8.0 }) - 192.0).abs() < 1e-9);
        assert!((start_at(&a, Transition::Cut) - 198.5).abs() < 1e-9, "the last downbeat before the mix out");
    }
}
