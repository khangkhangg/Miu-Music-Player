//! Transition scripts (D.7): each transition is a timeline of moves over its progress 0..1:
//! ramps (crossfader, channel volume, EQ, filter, tempo) and instant moves (effects on/off,
//! loops, start the new song, stop the old one, transition sounds). Auto DJ plays any script
//! the same way, so a new transition is mostly a new script.

use super::actions::{Band, Effect};
use super::autodj::Transition;
use super::deck::StemMode;
use super::pads::{IMPACT, RISER};

/// The song going out or the one coming in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Side {
    Out,
    In,
}

/// Something a ramp moves. The crossfader runs from the outgoing song (-1) to the incoming (1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    Crossfader,
    /// Channel fader 0..1.
    Volume(Side),
    /// -1 (kill) .. 0 (flat).
    Eq(Side, Band),
    /// -1 low-pass .. 0 off .. 1 high-pass.
    Filter(Side),
    /// The old song's speed from where it was (0) to the new song's tempo (1), key-locked.
    SpeedToIncoming,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Move {
    /// Move `target` from `from` to `to` between the step's `at` and `until`.
    Ramp { target: Target, from: f32, to: f32 },
    Effect { side: Side, effect: Effect, on: bool },
    Loop { side: Side, beats: Option<u8> },
    /// Start the new song.
    Play,
    /// Stop the old song (it is taken off its deck after the transition).
    Stop,
    /// A transition sound (riser, impact).
    Pad(usize),
    /// Match the new song's tempo and bars to the old one's (as it is now).
    Sync,
    KeyLock { side: Side, on: bool },
    KeyShift { side: Side, semitones: i8 },
    /// Play the whole song, its vocals or its instrumental.
    Stem { side: Side, mode: StemMode },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    pub at: f32,
    pub until: f32,
    pub mv: Move,
}

/// Where the new song starts playing from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    /// Its mix-in point, on a downbeat (or phrase).
    MixIn,
    /// Its drop, landing exactly when the script plays it.
    Drop,
    /// Its very beginning (intro under a breakdown, a ballad starting softly).
    Intro,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Script {
    pub steps: Vec<Step>,
    pub entry: Entry,
    /// Match the new song's tempo and bars to the old one's before it plays.
    pub sync: bool,
    /// Both songs must be split into vocals and instrumental first.
    pub needs_stems: bool,
}

/// The value a ramp gives `target` at progress `p`: the latest ramp that has started wins.
/// None when no ramp for it has started yet.
pub fn value_at(s: &Script, target: Target, p: f32) -> Option<f32> {
    s.steps
        .iter()
        .filter(|st| st.at <= p + 1e-6)
        .filter_map(|st| match st.mv {
            Move::Ramp { target: t, from, to } if t == target => {
                let k = if st.until > st.at { ((p - st.at) / (st.until - st.at)).clamp(0.0, 1.0) } else { 1.0 };
                Some((st.at, from + (to - from) * k))
            }
            _ => None,
        })
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, v)| v)
}

/// The instant moves due after `prev` up to and including `p`, in order.
pub fn instants(s: &Script, prev: f32, p: f32) -> Vec<&Move> {
    let mut due: Vec<&Step> = s.steps.iter().filter(|st| !matches!(st.mv, Move::Ramp { .. }) && st.at > prev && st.at <= p + 1e-6).collect();
    due.sort_by(|a, b| a.at.total_cmp(&b.at));
    due.into_iter().map(|st| &st.mv).collect()
}

fn ramp(at: f32, until: f32, target: Target, from: f32, to: f32) -> Step {
    Step { at, until, mv: Move::Ramp { target, from, to } }
}

fn at(at: f32, mv: Move) -> Step {
    Step { at, until: at, mv }
}

fn fx(p: f32, side: Side, effect: Effect, on: bool) -> Step {
    at(p, Move::Effect { side, effect, on })
}

/// Lows, mids and highs of the new song swapped in over `from`..`to` of the transition.
fn eq_swap(from: f32, to: f32) -> Vec<Step> {
    let span = to - from;
    let p = |k: f32| from + span * k;
    vec![
        // The new song comes in with its bass out and mids and highs down.
        ramp(0.0, 0.0, Target::Eq(Side::In, Band::Low), -1.0, -1.0),
        ramp(0.0, 0.0, Target::Eq(Side::In, Band::Mid), -0.5, -0.5),
        ramp(0.0, 0.0, Target::Eq(Side::In, Band::High), -0.5, -0.5),
        ramp(p(0.0), p(0.5), Target::Eq(Side::In, Band::High), -0.5, 0.0),
        ramp(p(0.25), p(0.75), Target::Eq(Side::In, Band::Mid), -0.5, 0.0),
        // Bass swap: one bass at a time.
        ramp(p(0.5), p(0.5), Target::Eq(Side::In, Band::Low), 0.0, 0.0),
        ramp(p(0.5), p(0.5), Target::Eq(Side::Out, Band::Low), -1.0, -1.0),
        ramp(p(0.5), p(1.0), Target::Eq(Side::Out, Band::Mid), 0.0, -0.6),
        ramp(p(0.75), p(1.0), Target::Eq(Side::Out, Band::High), 0.0, -1.0),
    ]
}

/// How far into a transition the new song takes over (where it starts playing).
pub fn switch_point(t: Transition) -> f32 {
    script(t, false).steps.iter().find(|s| s.mv == Move::Play).map_or(0.0, |s| s.at)
}

/// The script for a transition. `party`: add risers, impacts and more drama.
pub fn script(t: Transition, party: bool) -> Script {
    let x = |a: f32, u: f32, f: f32, to: f32| ramp(a, u, Target::Crossfader, f, to);
    let cut_at = |p: f32| vec![x(0.0, 0.0, -1.0, -1.0), at(p, Move::Play), x(p, p, 1.0, 1.0), at(p, Move::Stop)];
    let (mut steps, entry, sync) = match t {
        Transition::BeatBlend { .. } => {
            let mut s = vec![at(0.0, Move::Play), x(0.0, 1.0, -1.0, 1.0), at(1.0, Move::Stop)];
            s.extend(eq_swap(0.0, 1.0));
            (s, Entry::MixIn, true)
        }
        Transition::BreakdownBlend { .. } => {
            // The old song is in its breakdown, so the new one's intro can come in full.
            let s = vec![
                at(0.0, Move::Play),
                x(0.0, 1.0, -1.0, 1.0),
                ramp(0.0, 0.0, Target::Eq(Side::In, Band::Low), -1.0, -1.0),
                ramp(0.5, 0.5, Target::Eq(Side::In, Band::Low), 0.0, 0.0),
                ramp(0.5, 0.5, Target::Eq(Side::Out, Band::Low), -1.0, -1.0),
                at(1.0, Move::Stop),
            ];
            (s, Entry::Intro, true)
        }
        Transition::FilterBlend { .. } => {
            let s = vec![
                at(0.0, Move::Play),
                x(0.0, 0.0, -1.0, -1.0),
                x(0.2, 0.9, -1.0, 1.0),
                x(0.9, 0.9, 1.0, 1.0),
                ramp(0.0, 0.6, Target::Filter(Side::In), 0.75, 0.0),
                ramp(0.4, 1.0, Target::Filter(Side::Out), 0.0, -0.75),
                ramp(0.0, 0.0, Target::Eq(Side::In, Band::Low), -1.0, -1.0),
                ramp(0.5, 0.5, Target::Eq(Side::In, Band::Low), 0.0, 0.0),
                ramp(0.5, 0.5, Target::Eq(Side::Out, Band::Low), -1.0, -1.0),
                at(1.0, Move::Stop),
            ];
            (s, Entry::MixIn, true)
        }
        Transition::LoopOut { bars } => {
            let mut s = vec![
                at(0.0, Move::Loop { side: Side::Out, beats: Some(8) }),
                at(0.0, Move::Play),
                x(0.0, 1.0, -1.0, 1.0),
                at(1.0, Move::Loop { side: Side::Out, beats: None }),
                at(1.0, Move::Stop),
            ];
            let _ = bars;
            s.extend(eq_swap(0.0, 1.0));
            (s, Entry::MixIn, true)
        }
        Transition::DropSwap { .. } => {
            let mut s = vec![
                x(0.0, 0.0, -1.0, -1.0),
                ramp(0.0, 0.95, Target::Filter(Side::Out), 0.0, 0.7),
                // The last bar rolls on a two-beat loop.
                at(0.875, Move::Loop { side: Side::Out, beats: Some(2) }),
                at(1.0, Move::Play),
                x(1.0, 1.0, 1.0, 1.0),
                at(1.0, Move::Loop { side: Side::Out, beats: None }),
                at(1.0, Move::Stop),
            ];
            if party {
                s.push(at(0.5, Move::Pad(RISER)));
                s.push(at(1.0, Move::Pad(IMPACT)));
            }
            (s, Entry::Drop, true)
        }
        Transition::EchoOut => {
            let mut s = vec![
                x(0.0, 0.0, -1.0, -1.0),
                ramp(0.0, 0.75, Target::Filter(Side::Out), 0.0, 0.8),
                fx(0.5, Side::Out, Effect::Echo, true),
                fx(0.75, Side::Out, Effect::Echo, false),
                at(0.75, Move::Play),
                x(0.75, 0.75, 1.0, 1.0),
                at(0.75, Move::Stop),
            ];
            if party {
                s.push(at(0.75, Move::Pad(IMPACT)));
            }
            (s, Entry::MixIn, false)
        }
        Transition::ReverbWash => {
            let s = vec![
                x(0.0, 0.0, -1.0, -1.0),
                fx(0.0, Side::Out, Effect::Reverb, true),
                ramp(0.1, 0.6, Target::Volume(Side::Out), 1.0, 0.0),
                fx(0.55, Side::Out, Effect::Freeze, true),
                fx(0.6, Side::Out, Effect::Reverb, false),
                at(0.5, Move::Play),
                ramp(0.5, 1.0, Target::Filter(Side::In), 0.6, 0.0),
                x(0.45, 0.6, -1.0, 1.0),
                fx(0.85, Side::Out, Effect::Freeze, false),
                at(1.0, Move::Stop),
            ];
            (s, Entry::MixIn, false)
        }
        Transition::SpinBack | Transition::Brake => {
            let effect = if t == Transition::SpinBack { Effect::SpinBack } else { Effect::Brake };
            let mut s = vec![fx(0.0, Side::Out, effect, true)];
            s.extend(cut_at(0.75));
            if party {
                s.push(at(0.75, Move::Pad(IMPACT)));
            }
            (s, Entry::MixIn, false)
        }
        Transition::VocalOut { .. } => {
            let s = vec![
                at(0.0, Move::Stem { side: Side::In, mode: StemMode::Instrumental }),
                at(0.0, Move::Play),
                x(0.0, 0.25, -1.0, 0.0),
                ramp(0.0, 0.0, Target::Eq(Side::In, Band::Low), -1.0, -1.0),
                // The old beat drops out, its voice stays over the new instrumental.
                at(0.25, Move::Stem { side: Side::Out, mode: StemMode::Vocals }),
                ramp(0.25, 0.25, Target::Eq(Side::In, Band::Low), 0.0, 0.0),
                x(0.25, 1.0, 0.0, 1.0),
                ramp(0.65, 0.9, Target::Volume(Side::Out), 1.0, 0.0),
                // Then the new song's own vocals come back.
                at(0.75, Move::Stem { side: Side::In, mode: StemMode::Full }),
                at(1.0, Move::Stop),
            ];
            (s, Entry::MixIn, true)
        }
        Transition::TempoRamp { .. } => {
            let mut s = vec![
                at(0.0, Move::KeyLock { side: Side::Out, on: true }),
                x(0.0, 0.0, -1.0, -1.0),
                ramp(0.0, 0.5, Target::SpeedToIncoming, 0.0, 1.0),
                at(0.5, Move::Sync),
                at(0.5, Move::Play),
                x(0.5, 1.0, -1.0, 1.0),
                at(1.0, Move::Stop),
            ];
            s.extend(eq_swap(0.5, 1.0));
            (s, Entry::MixIn, false)
        }
        Transition::KeyShiftBlend { semitones, .. } => {
            let mut s = vec![
                at(0.0, Move::KeyLock { side: Side::In, on: true }),
                at(0.0, Move::KeyShift { side: Side::In, semitones }),
                at(0.0, Move::Play),
                x(0.0, 1.0, -1.0, 1.0),
                at(1.0, Move::Stop),
            ];
            s.extend(eq_swap(0.0, 1.0));
            (s, Entry::MixIn, true)
        }
        Transition::Crossfade { .. } => (vec![at(0.0, Move::Play), x(0.0, 1.0, -1.0, 1.0), at(1.0, Move::Stop)], Entry::MixIn, false),
        Transition::BalladGap => {
            let s = vec![
                x(0.0, 0.0, -1.0, -1.0),
                ramp(0.0, 0.55, Target::Volume(Side::Out), 1.0, 0.0),
                ramp(0.0, 0.0, Target::Volume(Side::In), 0.0, 0.0),
                at(0.7, Move::Play),
                x(0.7, 0.7, 1.0, 1.0),
                ramp(0.7, 1.0, Target::Volume(Side::In), 0.0, 1.0),
                at(0.7, Move::Stop),
            ];
            (s, Entry::Intro, false)
        }
        Transition::Cut => {
            let mut s = vec![at(0.0, Move::Play), x(0.0, 0.0, 1.0, 1.0), at(0.0, Move::Stop)];
            if party {
                s.push(at(0.0, Move::Pad(IMPACT)));
            }
            (s, Entry::MixIn, false)
        }
    };
    steps.sort_by(|a, b| a.at.total_cmp(&b.at));
    Script { steps, entry, sync, needs_stems: matches!(t, Transition::VocalOut { .. }) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dj::actions::{Band, Effect};
    use crate::dj::autodj::Transition;
    use crate::dj::pads::{IMPACT, RISER};
    use crate::dj::deck::StemMode;

    const ALL: [Transition; 15] = [
        Transition::VocalOut { bars: 16 },
        Transition::TempoRamp { bars: 32 },
        Transition::KeyShiftBlend { bars: 16, semitones: -2 },
        Transition::BeatBlend { bars: 16 },
        Transition::FilterBlend { bars: 8 },
        Transition::BreakdownBlend { bars: 8 },
        Transition::LoopOut { bars: 8 },
        Transition::DropSwap { bars: 8 },
        Transition::EchoOut,
        Transition::ReverbWash,
        Transition::SpinBack,
        Transition::Brake,
        Transition::Crossfade { secs: 8.0 },
        Transition::BalladGap,
        Transition::Cut,
    ];

    fn moves_by(s: &Script, upto: f32) -> Vec<Move> {
        instants(s, -1.0, upto).into_iter().cloned().collect()
    }

    #[test]
    fn every_transition_hands_the_speakers_to_the_new_song() {
        for t in ALL {
            for party in [false, true] {
                let s = script(t, party);
                assert_eq!(value_at(&s, Target::Crossfader, 1.0), Some(1.0), "{t:?} ends on the new song");
                if t != Transition::Cut {
                    assert_eq!(value_at(&s, Target::Crossfader, 0.0), Some(-1.0), "{t:?} starts on the old song");
                }
                let all = moves_by(&s, 1.0);
                assert_eq!(all.iter().filter(|m| **m == Move::Play).count(), 1, "{t:?} starts the new song once");
                assert!(all.contains(&Move::Stop), "{t:?} stops the old song");
                for step in &s.steps {
                    assert!((0.0..=1.0).contains(&step.at) && step.at <= step.until && step.until <= 1.0, "{t:?}: {step:?}");
                }
            }
        }
    }

    #[test]
    fn the_beat_blend_swaps_low_mid_and_high_in_turn() {
        let s = script(Transition::BeatBlend { bars: 16 }, false);
        let v = |t, p| value_at(&s, t, p).unwrap_or(0.0);
        assert_eq!(v(Target::Eq(Side::In, Band::Low), 0.4), -1.0, "the new bass waits");
        assert_eq!(v(Target::Eq(Side::Out, Band::Low), 0.4), 0.0);
        assert_eq!(v(Target::Eq(Side::In, Band::Low), 0.6), 0.0, "bass swapped half way");
        assert_eq!(v(Target::Eq(Side::Out, Band::Low), 0.6), -1.0);
        assert!(v(Target::Eq(Side::In, Band::Mid), 0.1) < 0.0 && v(Target::Eq(Side::In, Band::Mid), 0.8) == 0.0, "mids come in by three quarters");
        assert_eq!(v(Target::Eq(Side::Out, Band::High), 1.0), -1.0, "highs go last");
    }

    #[test]
    fn the_filter_blend_opens_the_new_song_and_closes_the_old() {
        let s = script(Transition::FilterBlend { bars: 8 }, false);
        assert_eq!(value_at(&s, Target::Filter(Side::In), 0.0), Some(0.75), "the new song comes in thin (high-pass)");
        assert_eq!(value_at(&s, Target::Filter(Side::In), 0.6), Some(0.0));
        assert_eq!(value_at(&s, Target::Filter(Side::Out), 1.0), Some(-0.75), "the old one goes dark (low-pass)");
    }

    #[test]
    fn the_drop_swap_cuts_to_the_new_song_on_its_drop() {
        let s = script(Transition::DropSwap { bars: 8 }, true);
        assert_eq!(s.entry, Entry::Drop);
        assert!(moves_by(&s, 0.99).iter().all(|m| *m != Move::Play), "the old song builds up first");
        let at_drop = instants(&s, 0.99, 1.0);
        assert!(at_drop.contains(&&Move::Play) && at_drop.contains(&&Move::Pad(IMPACT)), "{at_drop:?}");
        assert!(moves_by(&s, 0.99).contains(&Move::Pad(RISER)), "a riser leads up to it at a party");
        assert!(value_at(&s, Target::Filter(Side::Out), 0.9).unwrap() > 0.4, "the build sweeps up");
        assert!(!moves_by(&script(Transition::DropSwap { bars: 8 }, false), 1.0).contains(&Move::Pad(IMPACT)), "no sounds when smooth");
    }

    #[test]
    fn the_ballad_gap_lets_the_song_end_then_starts_the_next_softly() {
        let s = script(Transition::BalladGap, false);
        assert_eq!(s.entry, Entry::Intro);
        let vol = |side, p| value_at(&s, Target::Volume(side), p).unwrap_or(1.0);
        assert!(vol(Side::Out, 0.62) < 0.01 && vol(Side::In, 0.62) < 0.01, "a moment of quiet");
        assert!(vol(Side::In, 0.85) > 0.3 && vol(Side::In, 1.0) == 1.0, "the next one fades in");
    }

    #[test]
    fn the_reverb_wash_freezes_the_old_song_and_lets_the_new_one_rise() {
        let s = script(Transition::ReverbWash, false);
        let early = moves_by(&s, 0.6);
        assert!(early.contains(&Move::Effect { side: Side::Out, effect: Effect::Reverb, on: true }));
        assert!(early.contains(&Move::Effect { side: Side::Out, effect: Effect::Freeze, on: true }));
        assert_eq!(value_at(&s, Target::Volume(Side::Out), 0.6), Some(0.0));
        assert!(value_at(&s, Target::Filter(Side::In), 0.55).unwrap() > 0.3);
        assert!(moves_by(&s, 1.0).contains(&Move::Effect { side: Side::Out, effect: Effect::Freeze, on: false }), "the wash is let go");
    }

    #[test]
    fn spin_back_and_brake_stop_the_record_first() {
        for (t, fx) in [(Transition::SpinBack, Effect::SpinBack), (Transition::Brake, Effect::Brake)] {
            let s = script(t, false);
            assert_eq!(instants(&s, -1.0, 0.0).first(), Some(&&Move::Effect { side: Side::Out, effect: fx, on: true }));
            assert!(moves_by(&s, 0.3).iter().all(|m| *m != Move::Play), "{t:?}: the new song after the stop");
        }
    }

    #[test]
    fn the_loop_out_holds_the_end_of_the_old_song_while_blending() {
        let s = script(Transition::LoopOut { bars: 8 }, false);
        assert_eq!(instants(&s, -1.0, 0.0).first(), Some(&&Move::Loop { side: Side::Out, beats: Some(8) }));
        assert!(moves_by(&s, 1.0).contains(&Move::Loop { side: Side::Out, beats: None }));
    }

    #[test]
    fn the_tempo_ramp_glides_with_key_lock_then_blends() {
        let s = script(Transition::TempoRamp { bars: 32 }, false);
        assert_eq!(instants(&s, -1.0, 0.0).first(), Some(&&Move::KeyLock { side: Side::Out, on: true }));
        assert_eq!(value_at(&s, Target::SpeedToIncoming, 0.0), Some(0.0));
        assert_eq!(value_at(&s, Target::SpeedToIncoming, 0.5), Some(1.0), "at the new tempo half way");
        let at_half: Vec<&Move> = instants(&s, 0.49, 0.5);
        assert_eq!(at_half, vec![&Move::Sync, &Move::Play], "synced to the ramped tempo, then in");
        assert_eq!(value_at(&s, Target::Crossfader, 0.5), Some(-1.0));
    }

    #[test]
    fn the_key_shift_blend_tunes_the_new_song_first() {
        let s = script(Transition::KeyShiftBlend { bars: 16, semitones: -2 }, false);
        let first = moves_by(&s, 0.0);
        assert_eq!(&first[..3], &[Move::KeyLock { side: Side::In, on: true }, Move::KeyShift { side: Side::In, semitones: -2 }, Move::Play]);
    }

    #[test]
    fn the_vocal_out_puts_the_old_voice_over_the_new_instrumental() {
        let s = script(Transition::VocalOut { bars: 16 }, false);
        assert!(s.needs_stems);
        let start = moves_by(&s, 0.0);
        assert_eq!(&start[..2], &[Move::Stem { side: Side::In, mode: StemMode::Instrumental }, Move::Play]);
        assert!(moves_by(&s, 0.5).contains(&Move::Stem { side: Side::Out, mode: StemMode::Vocals }), "the old beat drops out, its voice stays");
        assert!(moves_by(&s, 0.8).contains(&Move::Stem { side: Side::In, mode: StemMode::Full }), "then the new song's own vocals");
        assert_eq!(value_at(&s, Target::Volume(Side::Out), 1.0), Some(0.0));
        assert!(!script(Transition::BeatBlend { bars: 16 }, false).needs_stems);
    }

    #[test]
    fn instant_moves_happen_once() {
        let s = script(Transition::EchoOut, false);
        let mut seen = Vec::new();
        let mut prev = -1.0;
        for i in 0..=100 {
            let p = i as f32 / 100.0;
            seen.extend(instants(&s, prev, p).into_iter().cloned());
            prev = p;
        }
        assert_eq!(seen, moves_by(&s, 1.0));
    }
}
