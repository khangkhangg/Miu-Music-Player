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

/// What kind of music a playlist is, from its songs' analyses: it decides the Auto style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Vibe {
    /// Steady beat around 115–140 BPM with drops.
    Dance,
    /// Punchy beat around 75–105 BPM (or double time).
    HipHop,
    /// Ballads, acoustic, lounge: soft or no beat.
    Chill,
    /// Everything else: mixed tempos, vocal songs.
    Pop,
}

/// A tempo folded into 70–150 BPM, so double- and half-time songs count as what dancers feel.
fn felt_bpm(bpm: f32) -> f32 {
    let mut b = bpm;
    while b > 150.0 {
        b /= 2.0;
    }
    while b > 0.0 && b < 70.0 {
        b *= 2.0;
    }
    b
}

fn median(mut v: Vec<f32>) -> f32 {
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    if n % 2 == 1 { v[n / 2] } else { (v[n / 2 - 1] + v[n / 2]) / 2.0 }
}

/// The kind of music in a set of songs (None when there are none).
pub fn vibe_of(songs: &[Analysis]) -> Option<Vibe> {
    if songs.is_empty() {
        return None;
    }
    let n = songs.len() as f32;
    let strong = songs.iter().filter(|a| a.beat_confidence >= 0.5).count() as f32 / n;
    let energy = median(songs.iter().map(|a| a.energy as f32).collect());
    let bpms: Vec<f32> = songs.iter().filter(|a| a.bpm > 0.0).map(|a| felt_bpm(a.bpm)).collect();
    let bpm = if bpms.is_empty() { 0.0 } else { median(bpms) };
    let drops = songs.iter().filter(|a| a.structure.drop_s.is_some()).count() as f32 / n;
    Some(if energy <= QUIET as f32 || strong < 0.4 {
        Vibe::Chill
    } else if (115.0..=140.0).contains(&bpm) && drops >= 0.3 {
        Vibe::Dance
    } else if (75.0..=105.0).contains(&bpm) && energy >= 5.0 {
        Vibe::HipHop
    } else {
        Vibe::Pop
    })
}

fn is_quiet(a: &Analysis) -> bool {
    a.energy <= QUIET || a.beat_confidence < 0.4
}

/// The style for one mix under the Auto style: the playlist's kind of music, but a ballad on
/// either side always gets a gentle change. Without a playlist, two big songs get the party moves.
pub fn auto_style(vibe: Option<Vibe>, from: &Analysis, to: &Analysis) -> Style {
    if is_quiet(from) || is_quiet(to) {
        return Style::Chill;
    }
    match vibe {
        Some(Vibe::Dance | Vibe::HipHop) => Style::Party,
        Some(Vibe::Pop) => Style::Smooth,
        Some(Vibe::Chill) => Style::Chill,
        None if from.energy >= 7 && to.energy >= 7 => Style::Party,
        None => Style::Smooth,
    }
}

/// A playlist as a DJ set: starts warm, climbs to its peak about two-thirds in, then comes down a
/// little; each next song is the one closest to that energy that mixes well (reachable tempo,
/// compatible key).
pub fn smart_order(songs: &[(i64, Analysis)], seed: u64) -> Vec<i64> {
    let n = songs.len();
    if n == 0 {
        return Vec::new();
    }
    let (lo, hi) = songs.iter().fold((u8::MAX, 0u8), |(lo, hi), (_, a)| (lo.min(a.energy), hi.max(a.energy)));
    let target = |i: usize| {
        let p = if n > 1 { i as f64 / (n - 1) as f64 } else { 0.0 };
        let arc = if p <= 0.7 { p / 0.7 } else { 1.0 - 0.45 * (p - 0.7) / 0.3 };
        lo as f64 + (hi as f64 - lo as f64) * arc
    };
    let mut left: Vec<&(i64, Analysis)> = songs.iter().collect();
    let mut order = Vec::with_capacity(n);
    let mut prev: Option<&Analysis> = None;
    for i in 0..n {
        let t = target(i);
        let cost = |(id, a): &(i64, Analysis)| {
            let energy = (a.energy as f64 - t).abs();
            let mix = prev.map_or(0.0, |p| {
                let tempo = tempo_change(p.bpm, a.bpm).map_or(3.0, |c| c.abs() / TEMPO_RANGE * 0.5);
                let key = match keys_fit(p, a) {
                    Some(true) => 0.0,
                    Some(false) => 0.7,
                    None => 0.2,
                };
                tempo + key
            });
            energy + mix + jitter(*id, seed)
        };
        let best = (0..left.len()).min_by(|&x, &y| cost(left[x]).total_cmp(&cost(left[y]))).unwrap_or(0);
        let chosen = left.remove(best);
        prev = Some(&chosen.1);
        order.push(chosen.0);
    }
    order
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

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
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
    /// The old song's voice over the new song's instrumental, then the new vocals come back.
    VocalOut { bars: u8 },
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

/// A kind of transition without its settings, for the DJ's checklist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Kind {
    BeatBlend,
    FilterBlend,
    BreakdownBlend,
    LoopOut,
    DropSwap,
    VocalOut,
    KeyShiftBlend,
    TempoRamp,
    EchoOut,
    ReverbWash,
    SpinBack,
    Brake,
    Crossfade,
    BalladGap,
    Cut,
}

impl Transition {
    pub fn kind(self) -> Kind {
        match self {
            Transition::BeatBlend { .. } => Kind::BeatBlend,
            Transition::FilterBlend { .. } => Kind::FilterBlend,
            Transition::BreakdownBlend { .. } => Kind::BreakdownBlend,
            Transition::LoopOut { .. } => Kind::LoopOut,
            Transition::DropSwap { .. } => Kind::DropSwap,
            Transition::VocalOut { .. } => Kind::VocalOut,
            Transition::KeyShiftBlend { .. } => Kind::KeyShiftBlend,
            Transition::TempoRamp { .. } => Kind::TempoRamp,
            Transition::EchoOut => Kind::EchoOut,
            Transition::ReverbWash => Kind::ReverbWash,
            Transition::SpinBack => Kind::SpinBack,
            Transition::Brake => Kind::Brake,
            Transition::Crossfade { .. } => Kind::Crossfade,
            Transition::BalladGap => Kind::BalladGap,
            Transition::Cut => Kind::Cut,
        }
    }
}

/// The transition of `kind` for this pair, if it can work: beat-matched ones need both songs to
/// have a beat within reach (and a drop, a breakdown or a key that shifts into place where they
/// need one). The others work for any pair.
pub fn suitable(kind: Kind, from: &Analysis, to: &Analysis, style: Style) -> Option<Transition> {
    let beats = from.beat_confidence >= MIN_BEAT_CONFIDENCE && to.beat_confidence >= MIN_BEAT_CONFIDENCE;
    let in_tempo = beats && tempo_change(from.bpm, to.bpm).is_some();
    let bars = if style == Style::Party { 8 } else { 16 };
    Some(match kind {
        Kind::BeatBlend if in_tempo => Transition::BeatBlend { bars },
        Kind::FilterBlend if in_tempo => Transition::FilterBlend { bars },
        Kind::BreakdownBlend if in_tempo && last_breakdown(from).is_some() => Transition::BreakdownBlend { bars: 8 },
        Kind::LoopOut if in_tempo => Transition::LoopOut { bars: 8 },
        Kind::DropSwap if in_tempo && to.structure.drop_s.is_some() => Transition::DropSwap { bars: 8 },
        Kind::VocalOut if in_tempo => Transition::VocalOut { bars: 16 },
        Kind::KeyShiftBlend if in_tempo => Transition::KeyShiftBlend { bars: 16, semitones: key_shift_to_fit(from, to)? },
        Kind::TempoRamp if beats && !in_tempo && ramp_target(from.bpm, to.bpm).is_some() => Transition::TempoRamp { bars: 32 },
        Kind::EchoOut => Transition::EchoOut,
        Kind::ReverbWash => Transition::ReverbWash,
        Kind::SpinBack => Transition::SpinBack,
        Kind::Brake => Transition::Brake,
        Kind::Crossfade => Transition::Crossfade { secs: 8.0 },
        Kind::BalladGap => Transition::BalladGap,
        Kind::Cut => Transition::Cut,
        _ => return None,
    })
}

/// When the planner's own pick is not checked: the checked kinds tried in this order (smooth blends
/// first, hard cuts last).
const FALLBACK_ORDER: [Kind; 15] = [
    Kind::BeatBlend, Kind::FilterBlend, Kind::BreakdownBlend, Kind::KeyShiftBlend, Kind::TempoRamp, Kind::LoopOut,
    Kind::VocalOut, Kind::DropSwap, Kind::EchoOut, Kind::ReverbWash, Kind::Crossfade, Kind::BalladGap, Kind::Brake,
    Kind::SpinBack, Kind::Cut,
];

/// Like `plan`, but only with the kinds the DJ checked (none checked: any). `mix_it_up` picks at
/// random among the checked kinds that suit the pair. When none of them can work for this pair,
/// the planner's own pick is used rather than a mix that falls apart.
pub fn plan_with(from: &Analysis, to: &Analysis, style: Style, seed: u64, allowed: &[Kind], mix_it_up: bool) -> Transition {
    let best = plan(from, to, style, seed);
    if allowed.is_empty() {
        return best;
    }
    let fits: Vec<Transition> = FALLBACK_ORDER.iter().filter(|k| allowed.contains(k)).filter_map(|&k| suitable(k, from, to, style)).collect();
    if fits.is_empty() {
        return best;
    }
    if mix_it_up {
        // jitter() is a stable 0..0.2 hash of the seed: spread it over the fitting kinds.
        let r = jitter(seed as i64, seed.wrapping_add(7)) / 0.2;
        return fits[((r * fits.len() as f64) as usize).min(fits.len() - 1)];
    }
    if allowed.contains(&best.kind()) {
        return best;
    }
    fits[0]
}

/// How Auto DJ mixes: long smooth blends, party drama (drops, risers, tricks) or chilled filters,
/// washes and fades.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Style {
    /// Picked for each mix from the music (see `auto_style`).
    #[default]
    Auto,
    Smooth,
    Party,
    Chill,
}

impl Style {
    /// The style for one mix: Auto decides from the pair and the playlist's kind of music.
    pub fn for_pair(self, vibe: Option<Vibe>, from: &Analysis, to: &Analysis) -> Style {
        if self == Style::Auto { auto_style(vibe, from, to) } else { self }
    }
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
            return if party { VocalOut { bars: 16 } } else { FilterBlend { bars: 8 } };
        }
        return match style {
            Style::Chill => FilterBlend { bars: 16 },
            Style::Party => BeatBlend { bars: 8 },
            Style::Smooth | Style::Auto => BeatBlend { bars: 16 },
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
        Style::Smooth | Style::Auto => EchoOut,
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
            Transition::FilterBlend { bars } | Transition::BreakdownBlend { bars } | Transition::LoopOut { bars } | Transition::DropSwap { bars } | Transition::TempoRamp { bars } | Transition::KeyShiftBlend { bars, .. } | Transition::VocalOut { bars } => {
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

    fn with_drop(mut a: Analysis) -> Analysis {
        a.structure.drop_s = Some(60.0);
        a
    }

    #[test]
    fn a_playlist_says_what_kind_of_music_it_is() {
        let dance: Vec<Analysis> = (0..6).map(|i| with_drop(song(124.0 + i as f32, "8A", 7, 0.9))).collect();
        assert_eq!(vibe_of(&dance), Some(Vibe::Dance));
        let hiphop: Vec<Analysis> = (0..6).map(|i| song(88.0 + i as f32, "4A", 6, 0.8)).collect();
        assert_eq!(vibe_of(&hiphop), Some(Vibe::HipHop));
        let ballads: Vec<Analysis> = (0..6).map(|_| song(72.0, "5B", 3, 0.3)).collect();
        assert_eq!(vibe_of(&ballads), Some(Vibe::Chill));
        let pop: Vec<Analysis> = [100.0, 110.0, 118.0, 140.0, 105.0, 96.0].iter().map(|&b| song(b, "8B", 6, 0.7)).collect();
        assert_eq!(vibe_of(&pop), Some(Vibe::Pop));
        let double_time: Vec<Analysis> = (0..6).map(|i| song(176.0 + i as f32, "4A", 6, 0.8)).collect();
        assert_eq!(vibe_of(&double_time), Some(Vibe::HipHop), "176 BPM rap counts as 88");
        assert_eq!(vibe_of(&[]), None);
    }

    #[test]
    fn the_auto_style_follows_the_playlist_and_spares_ballads() {
        let banger = with_drop(song(126.0, "8A", 8, 0.9));
        let ballad = song(70.0, "8A", 3, 0.3);
        assert_eq!(auto_style(Some(Vibe::Dance), &banger, &banger), Style::Party);
        assert_eq!(auto_style(Some(Vibe::Pop), &banger, &banger), Style::Smooth);
        assert_eq!(auto_style(Some(Vibe::Chill), &banger, &banger), Style::Chill);
        assert_eq!(auto_style(Some(Vibe::Dance), &banger, &ballad), Style::Chill, "a ballad never gets a drop swap");
        assert_eq!(auto_style(None, &banger, &banger), Style::Party, "no playlist: two bangers still get the party moves");
        assert_eq!(auto_style(None, &song(110.0, "8A", 5, 0.7), &song(112.0, "8A", 5, 0.7)), Style::Smooth);
    }

    #[test]
    fn smart_order_warms_up_peaks_and_lands() {
        let list: Vec<(i64, Analysis)> = vec![
            (1, song(124.0, "8A", 9, 0.9)),
            (2, song(122.0, "8A", 4, 0.9)),
            (3, song(126.0, "9A", 7, 0.9)),
            (4, song(124.0, "8A", 6, 0.9)),
            (5, song(125.0, "8A", 8, 0.9)),
            (6, song(120.0, "8A", 3, 0.9)),
        ];
        let order = smart_order(&list, 0);
        assert_eq!(order.len(), 6);
        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, vec![1, 2, 3, 4, 5, 6], "every song once");
        let energy = |id: i64| list.iter().find(|(i, _)| *i == id).unwrap().1.energy;
        assert!(energy(order[0]) <= 4, "starts warm: {order:?}");
        let peak = order.iter().position(|&id| energy(id) == 9).unwrap();
        assert!((2..6).contains(&peak), "peaks past the middle: {order:?}");
        assert!(energy(order[5]) < 9, "does not end on the peak: {order:?}");
    }

    #[test]
    fn smart_order_keeps_neighbours_mixable() {
        // Two tempo groups: the order should not zig-zag between them.
        let list: Vec<(i64, Analysis)> = vec![
            (1, song(90.0, "8A", 5, 0.9)),
            (2, song(128.0, "8A", 5, 0.9)),
            (3, song(91.0, "8A", 5, 0.9)),
            (4, song(127.0, "8A", 5, 0.9)),
        ];
        let order = smart_order(&list, 0);
        let jumps = order.windows(2).filter(|w| tempo_change(list[(w[0] - 1) as usize].1.bpm, list[(w[1] - 1) as usize].1.bpm).is_none()).count();
        assert_eq!(jumps, 1, "only one jump between the tempo groups: {order:?}");
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
    fn only_the_transitions_the_dj_checked_are_used() {
        use Kind::*;
        use Style::*;
        let house = song(124.0, "8A", 6, 0.8);
        let near = song(126.0, "9A", 6, 0.8);
        let far = song(95.0, "4A", 8, 0.8);
        let pick = |a: &Analysis, b: &Analysis, allowed: &[Kind], mix: bool, seed: u64| plan_with(a, b, Smooth, seed, allowed, mix);
        assert_eq!(pick(&house, &near, &[], false, 0), plan(&house, &near, Smooth, 0), "nothing checked: the best fit");
        assert_eq!(pick(&house, &near, &[EchoOut, SpinBack], false, 0), Transition::EchoOut, "the best fit among the checked ones");
        assert_eq!(pick(&house, &near, &[BeatBlend, Cut], false, 0), Transition::BeatBlend { bars: 16 }, "the planner's own pick when it is checked");
        assert_eq!(pick(&house, &far, &[BeatBlend], false, 0), plan(&house, &far, Smooth, 0), "a beat blend cannot bridge that gap: the best fit instead");
        assert_eq!(pick(&house, &near, &[DropSwap, Crossfade], false, 0), Transition::Crossfade { secs: 8.0 }, "no drop to swap into");
        let mut seen = std::collections::HashSet::new();
        for seed in 0..40 {
            let t = pick(&house, &near, &[EchoOut, Cut, Brake, BeatBlend], true, seed);
            assert!([Kind::EchoOut, Kind::Cut, Kind::Brake, Kind::BeatBlend].contains(&t.kind()), "{t:?}");
            seen.insert(t.kind());
        }
        assert!(seen.len() >= 3, "mix it up: {seen:?}");
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
        assert_eq!(plan(&singer, &next, Party, 0), Transition::VocalOut { bars: 16 }, "a mashup moment at a party");
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
