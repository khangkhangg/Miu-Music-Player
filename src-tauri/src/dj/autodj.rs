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
    /// The outgoing song is swept away with a high-pass; the new one drops in on a downbeat.
    EchoOut,
    /// A plain crossfade at the mix points, no beat-matching.
    Crossfade { secs: f32 },
    /// Straight over on a downbeat.
    Cut,
}

/// How to mix from one song into the next.
pub fn plan(from: &Analysis, to: &Analysis) -> Transition {
    if from.beat_confidence < MIN_BEAT_CONFIDENCE || to.beat_confidence < MIN_BEAT_CONFIDENCE {
        return Transition::Crossfade { secs: 8.0 };
    }
    if tempo_change(from.bpm, to.bpm).is_some() && keys_fit(from, to) != Some(false) {
        return Transition::BeatBlend { bars: 16 };
    }
    if from.energy >= HIGH_ENERGY && to.energy >= HIGH_ENERGY {
        return Transition::Cut;
    }
    Transition::EchoOut
}

fn bar_s(a: &Analysis) -> Option<f64> {
    (a.bpm > 0.0).then(|| 4.0 * 60.0 / a.bpm as f64)
}

impl Transition {
    /// How long the transition takes, in the outgoing song's time.
    pub fn duration_s(self, from: &Analysis) -> f64 {
        match self {
            Transition::BeatBlend { bars } => bar_s(from).map_or(16.0, |b| b * bars as f64),
            Transition::FilterBlend { bars } | Transition::BreakdownBlend { bars } | Transition::LoopOut { bars } | Transition::DropSwap { bars } => {
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
        let house = song(124.0, "8A", 6, 0.8);
        assert_eq!(plan(&house, &song(126.0, "9A", 6, 0.8)), Transition::BeatBlend { bars: 16 });
        assert_eq!(plan(&house, &song(63.0, "8A", 6, 0.8)), Transition::BeatBlend { bars: 16 }, "half time counts");
        assert_eq!(plan(&house, &song(90.0, "8A", 6, 0.8)), Transition::EchoOut, "big tempo jump");
        assert_eq!(plan(&house, &song(124.0, "2B", 6, 0.8)), Transition::EchoOut, "key clash");
        assert_eq!(plan(&house, &song(124.0, "8A", 6, 0.1)), Transition::Crossfade { secs: 8.0 }, "a ballad");
        assert_eq!(plan(&song(95.0, "4A", 8, 0.7), &song(128.0, "11B", 8, 0.7)), Transition::Cut, "high energy, far apart");
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
