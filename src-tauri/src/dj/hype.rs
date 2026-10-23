//! Hype (DJ.B): what a DJ does inside a song, not only between songs. From a song's analysis the
//! planner places moves on its timeline: a build-up into each drop (the filter opens, a riser,
//! the last beats rolled shorter and shorter), a beat of silence before the drop (the fake-out),
//! an impact on the drop, echo throws at the end of sung lines and a beat gate in breakdowns.
//! The Hype level decides how much: Chill (echo throws now and then), Party (the big drop gets the
//! works), Festival (every drop, gates in breakdowns). Auto DJ performs them on the song on air.

use serde::{Deserialize, Serialize};

use super::analysis::Analysis;
use super::actions::{Action, Deck as Side, Effect};
use super::deck::StemMode;
use super::mixer::{HypeFx, Mixer};
use super::structure::SectionKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Hype {
    /// An echo on the end of a sung line now and then.
    Chill,
    /// The big drop gets a build-up, a fake-out and an impact; echo throws.
    #[default]
    Party,
    /// Every drop, beat gates in breakdowns, more echo throws.
    Festival,
}

/// A move on a song's timeline (seconds of the song).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Move {
    /// The high-pass filter opens across the build (a riser plays from its start).
    Build { from_s: f64, to_s: f64 },
    /// The last beats repeat, `beats` long.
    Roll { from_s: f64, to_s: f64, beats: f64 },
    /// Silence before the drop.
    FakeOut { from_s: f64, to_s: f64 },
    /// A hit on the drop.
    Impact { at_s: f64 },
    /// The echo catches the end of a sung line and rings on.
    EchoThrow { from_s: f64, to_s: f64 },
    /// The breakdown chopped on the eighth notes.
    Gate { from_s: f64, to_s: f64 },
    /// The singer drops out of the chorus and the crowd sings it (lyrics on the party screen).
    SingAlong { from_s: f64, to_s: f64 },
    /// From here the song plays a semitone higher (the last-chorus key change).
    KeyLift { at_s: f64 },
    /// "Pull up!": the record spins back and the drop plays again from `to_s`.
    Rewind { at_s: f64, to_s: f64 },
    /// The record brakes to a stop and the song carries on from `resume_s`.
    TapeStop { at_s: f64, resume_s: f64 },
    /// The reverb catches the last line before a breakdown and blooms.
    ReverbThrow { from_s: f64, to_s: f64 },
    /// A cymbal crash on a new phrase.
    Crash { at_s: f64 },
    /// A snare fill into a new phrase.
    Fill { at_s: f64 },
    /// Back to `to_s` once: the best bars of the drop again.
    Repeat { at_s: f64, to_s: f64 },
}

/// The channel effects at one moment.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Fx {
    /// How far the build has opened the filter, 0..1.
    pub rise: f32,
    /// Beat length of the roll in progress.
    pub roll: Option<f64>,
    pub gate: bool,
    pub mute: bool,
    pub echo: bool,
    /// Sing-along: the singer is out.
    pub sing: bool,
    pub reverb: bool,
}

/// What happens once as the song passes it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shot {
    Riser,
    Impact,
    Crash,
    Fill,
    KeyLift,
    Rewind { to_s: f64 },
    TapeStop { resume_s: f64 },
    Repeat { to_s: f64 },
}

impl Shot {
    /// Moves that change where the song is, or its key: once per song, however often the song
    /// passes the spot again (a rewind replays it).
    fn once(&self) -> bool {
        matches!(self, Shot::KeyLift | Shot::Rewind { .. } | Shot::TapeStop { .. } | Shot::Repeat { .. })
    }
}

/// Echo throws no closer than this (seconds).
fn echo_spacing(h: Hype) -> f64 {
    match h {
        Hype::Chill => 64.0,
        Hype::Party => 32.0,
        Hype::Festival => 16.0,
    }
}

/// The moves for a song at a Hype level. Nothing without a steady beat, and nothing in the last
/// 8 bars before the mix out (the transition has the stage then).
pub fn plan(a: &Analysis, hype: Hype) -> Vec<Move> {
    if a.bpm <= 0.0 || a.beat_confidence < super::autodj::MIN_BEAT_CONFIDENCE {
        return Vec::new();
    }
    let beat = 60.0 / a.bpm as f64;
    let bar = 4.0 * beat;
    let limit = a.mix_out_s - 8.0 * bar;
    let mut moves = Vec::new();
    let drops: Vec<f64> = a
        .structure
        .sections
        .iter()
        .filter(|s| s.kind == SectionKind::Drop && s.start_s >= 8.0 * bar && s.start_s <= limit)
        .map(|s| s.start_s)
        .collect();
    let main = a.structure.drop_s;
    for d in drops {
        match hype {
            Hype::Festival => moves.extend([
                Move::Build { from_s: d - 8.0 * bar, to_s: d },
                Move::Roll { from_s: d - 2.0 * bar, to_s: d - bar, beats: 0.5 },
                Move::Roll { from_s: d - bar, to_s: d - beat, beats: 0.25 },
                Move::FakeOut { from_s: d - beat, to_s: d },
                Move::Impact { at_s: d },
            ]),
            Hype::Party if main.is_some_and(|m| (m - d).abs() < beat) => moves.extend([
                Move::Build { from_s: d - 4.0 * bar, to_s: d },
                Move::Roll { from_s: d - bar, to_s: d - beat, beats: 0.5 },
                Move::FakeOut { from_s: d - beat, to_s: d },
                Move::Impact { at_s: d },
            ]),
            _ => {}
        }
    }
    let builds: Vec<(f64, f64)> = moves.iter().filter_map(|m| if let Move::Build { from_s, to_s } = *m { Some((from_s, to_s)) } else { None }).collect();
    let in_build = |t: f64| builds.iter().any(|&(a, b)| t >= a && t <= b);
    if hype == Hype::Festival {
        for s in a.structure.sections.iter().filter(|s| s.kind == SectionKind::Breakdown) {
            let until_build = builds.iter().map(|&(f, _)| f).filter(|&f| f > s.start_s && f <= s.end_s).fold(s.end_s, f64::min);
            let to = until_build.min(s.start_s + 8.0 * bar);
            if to - s.start_s >= 2.0 * bar && to <= limit {
                moves.push(Move::Gate { from_s: s.start_s, to_s: to });
            }
        }
    }
    let party = hype != Hype::Chill;
    let festival = hype == Hype::Festival;
    let mut ends: Vec<f64> = a.structure.vocals.iter().map(|v| v[1]).collect();
    ends.sort_by(|x, y| x.total_cmp(y));
    // Into each breakdown: the last sung line blooms in the reverb; the record brakes into it
    // (the first one at Party, all at Festival).
    let breakdowns: Vec<f64> = a.structure.sections.iter().filter(|s| s.kind == SectionKind::Breakdown && s.start_s >= 8.0 * bar && s.start_s <= limit).map(|s| s.start_s).collect();
    let mut thrown: Vec<f64> = Vec::new();
    if party {
        for (i, &b) in breakdowns.iter().enumerate() {
            if let Some(&v) = ends.iter().rev().find(|&&v| v <= b && v >= b - 8.0 * bar && !in_build(v)) {
                moves.push(Move::ReverbThrow { from_s: v, to_s: v + beat });
                thrown.push(v);
            }
            if festival || i == 0 {
                moves.push(Move::TapeStop { at_s: b - beat, resume_s: b });
            }
        }
    }
    // The chorus everyone knows by its last time: the crowd sings it, then it comes back a key up.
    let choruses: Vec<[f64; 2]> = a.structure.choruses.iter().copied().filter(|c| c[0] <= limit).collect();
    if party && choruses.len() >= 2 {
        let last = choruses[choruses.len() - 1];
        let end = (last[0] + if festival { 8.0 } else { 4.0 } * bar).min(last[1]);
        moves.push(Move::SingAlong { from_s: last[0], to_s: end });
        if end <= limit {
            moves.push(Move::KeyLift { at_s: end });
        }
    }
    // Festival's big drop: 4 bars in, the record spins back to the build and the drop hits again;
    // after its first phrase, its best 4 bars once more.
    if festival {
        if let Some(d) = main.filter(|&d| d >= 8.0 * bar && d + 8.0 * bar <= limit) {
            moves.push(Move::Rewind { at_s: d + 4.0 * bar, to_s: d - 2.0 * bar });
            moves.push(Move::Repeat { at_s: d + 8.0 * bar, to_s: d + 4.0 * bar });
        }
    }
    // A crash on each new phrase inside a drop (a fill into it at Festival).
    if let (true, Some(p0)) = (party, a.structure.phrase_start_s) {
        let phrase = 8.0 * bar;
        for s in a.structure.sections.iter().filter(|s| s.kind == SectionKind::Drop) {
            let mut t = p0 + ((s.start_s - p0) / phrase).floor() * phrase;
            while t < s.end_s.min(limit) {
                if t > s.start_s + beat {
                    moves.push(Move::Crash { at_s: t });
                    if festival {
                        moves.push(Move::Fill { at_s: t - beat });
                    }
                }
                t += phrase;
            }
        }
    }
    // Echo throws on line ends, every other one a stutter at Festival.
    let mut last = f64::NEG_INFINITY;
    let mut n = 0;
    for at in ends {
        if at <= limit && !in_build(at) && !thrown.contains(&at) && at - last >= echo_spacing(hype) {
            if festival && n % 2 == 1 {
                moves.push(Move::Roll { from_s: at - beat / 2.0, to_s: at, beats: 0.125 });
            } else {
                moves.push(Move::EchoThrow { from_s: at, to_s: at + beat });
            }
            last = at;
            n += 1;
        }
    }
    moves
}

/// What the channel should be doing at song position `pos`.
pub fn fx_at(moves: &[Move], pos: f64) -> Fx {
    let mut fx = Fx::default();
    let within = |from: f64, to: f64| pos >= from && pos < to;
    for m in moves {
        match *m {
            Move::Build { from_s, to_s } if within(from_s, to_s) => fx.rise = ((pos - from_s) / (to_s - from_s)) as f32,
            Move::Roll { from_s, to_s, beats } if within(from_s, to_s) => fx.roll = Some(beats),
            Move::FakeOut { from_s, to_s } if within(from_s, to_s) => fx.mute = true,
            Move::Gate { from_s, to_s } if within(from_s, to_s) => fx.gate = true,
            Move::EchoThrow { from_s, to_s } if within(from_s, to_s) => fx.echo = true,
            Move::SingAlong { from_s, to_s } if within(from_s, to_s) => fx.sing = true,
            Move::ReverbThrow { from_s, to_s } if within(from_s, to_s) => fx.reverb = true,
            _ => {}
        }
    }
    fx
}

/// What the song passed between `prev` and `pos` (nothing when it jumped back).
pub fn shots(moves: &[Move], prev: f64, pos: f64) -> Vec<Shot> {
    shots_at(moves, prev, pos).into_iter().map(|(_, s)| s).collect()
}

/// Like `shots`, with where each one is in the song.
fn shots_at(moves: &[Move], prev: f64, pos: f64) -> Vec<(f64, Shot)> {
    if pos < prev {
        return Vec::new();
    }
    let crossed = |t: f64| t > prev && t <= pos;
    moves
        .iter()
        .filter_map(|m| match *m {
            Move::Build { from_s, .. } if crossed(from_s) => Some((from_s, Shot::Riser)),
            Move::Impact { at_s } if crossed(at_s) => Some((at_s, Shot::Impact)),
            Move::Crash { at_s } if crossed(at_s) => Some((at_s, Shot::Crash)),
            Move::Fill { at_s } if crossed(at_s) => Some((at_s, Shot::Fill)),
            Move::KeyLift { at_s } if crossed(at_s) => Some((at_s, Shot::KeyLift)),
            Move::Rewind { at_s, to_s } if crossed(at_s) => Some((at_s, Shot::Rewind { to_s })),
            Move::TapeStop { at_s, resume_s } if crossed(at_s) => Some((at_s, Shot::TapeStop { resume_s })),
            Move::Repeat { at_s, to_s } if crossed(at_s) => Some((at_s, Shot::Repeat { to_s })),
            _ => None,
        })
        .collect()
}

/// Where the next built-up drop lands after `pos` (for the countdown on the party screen).
pub fn next_drop(moves: &[Move], pos: f64) -> Option<f64> {
    moves.iter().filter_map(|m| if let Move::Build { to_s, .. } = *m { Some(to_s) } else { None }).filter(|&t| t > pos).fold(None, |a: Option<f64>, t| Some(a.map_or(t, |a| a.min(t))))
}

/// What the party screen needs for its light show and countdown.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Show {
    /// One beat in real seconds (tempo as heard).
    pub beat_s: f64,
    /// Beats since the song's first downbeat, now (bars start on multiples of 4).
    pub beat_pos: f64,
    /// Real seconds to the next built-up drop, while Auto DJ performs one.
    pub drop_in_s: Option<f64>,
    /// A sing-along now: the party screen shows the lyrics big.
    pub sing: bool,
    /// The song on air and where it is (seconds of the song, and how fast it plays), for the lyrics.
    pub song_id: i64,
    pub pos_s: f64,
    pub speed: f64,
}

/// Plays a song's moves on the deck on air while Auto DJ runs and no transition is under way.
#[derive(Default)]
pub struct Performer {
    hype: Hype,
    song: Option<(Side, i64)>,
    moves: Vec<Move>,
    prev: f64,
    echo_on: bool,
    reverb_on: bool,
    /// The singer is out (stems: instrumental only).
    sing_on: bool,
    engaged: Option<Side>,
    /// One-off moves done for this song (by where they are), so a rewind does not repeat them.
    fired: Vec<u64>,
    /// After a spin-back or a brake: where the song carries on once the record has stopped.
    resume: Option<f64>,
    /// The deck and song lifted a semitone (back to normal when that song leaves the deck).
    lifted: Option<(Side, i64)>,
}

impl Performer {
    pub fn hype(&self) -> Hype {
        self.hype
    }

    pub fn set_hype(&mut self, hype: Hype) {
        self.hype = hype;
        self.song = None; // planned again on the next block
    }

    fn let_go(&mut self, m: &mut Mixer) {
        if let Some(side) = self.engaged.take() {
            m.set_fx(side, HypeFx::default());
            for (on, effect) in [(&mut self.echo_on, Effect::Echo), (&mut self.reverb_on, Effect::Reverb)] {
                if *on {
                    m.apply(&Action::Effect { deck: side, effect, on: false });
                    *on = false;
                }
            }
            if self.sing_on {
                m.deck_mut(side).set_stem_mode(StemMode::Full);
                self.sing_on = false;
            }
        }
    }

    /// The deck to split into vocals and instrumental for a coming sing-along.
    pub fn stems_wanted(&self, m: &Mixer) -> Option<Side> {
        let (side, _) = self.song?;
        let wants = self.moves.iter().any(|x| matches!(x, Move::SingAlong { .. }));
        (wants && m.deck(side).track().is_some() && !m.deck(side).has_stems()).then_some(side)
    }

    /// Before each block: `live` is the deck on air, `active` whether to perform now.
    pub fn tick(&mut self, m: &mut Mixer, live: Option<Side>, active: bool) {
        // A lifted song goes back to its own key when it leaves its deck.
        if let Some((side, id)) = self.lifted {
            if m.deck(side).track().map(|t| t.song_id) != Some(id) {
                m.deck_mut(side).set_key_shift(0);
                self.lifted = None;
            }
        }
        let Some(side) = live.filter(|_| active) else {
            self.let_go(m);
            return;
        };
        if self.engaged.is_some_and(|s| s != side) {
            self.let_go(m);
        }
        let deck = m.deck(side);
        let Some(track) = deck.track() else {
            self.let_go(m);
            return;
        };
        let (song_id, pos) = (track.song_id, deck.position_s());
        if self.song != Some((side, song_id)) {
            self.song = Some((side, song_id));
            self.moves = track.analysis.as_ref().map(|a| plan(a, self.hype)).unwrap_or_default();
            self.prev = pos;
            self.fired.clear();
            self.resume = None;
        }
        // Spun back or braked: when the record has stopped, the song carries on from there.
        if let Some(to) = self.resume {
            if !m.deck(side).is_playing() {
                let d = m.deck_mut(side);
                d.seek_s(to);
                d.play();
                self.resume = None;
                self.prev = to;
            }
            self.engaged = Some(side);
            return;
        }
        let fx = fx_at(&self.moves, pos);
        let has_stems = m.deck(side).has_stems();
        m.set_fx(side, HypeFx { rise: fx.rise, roll: fx.roll, gate: fx.gate, mute: fx.mute, vocal_cut: fx.sing && !has_stems });
        if fx.sing != self.sing_on && has_stems {
            m.deck_mut(side).set_stem_mode(if fx.sing { StemMode::Instrumental } else { StemMode::Full });
        }
        self.sing_on = fx.sing && has_stems;
        for (want, on, effect) in [(fx.echo, &mut self.echo_on, Effect::Echo), (fx.reverb, &mut self.reverb_on, Effect::Reverb)] {
            if want != *on {
                m.apply(&Action::Effect { deck: side, effect, on: want });
                *on = want;
            }
        }
        for (at, shot) in shots_at(&self.moves, self.prev, pos) {
            if shot.once() {
                if self.fired.contains(&at.to_bits()) {
                    continue;
                }
                self.fired.push(at.to_bits());
            }
            match shot {
                Shot::Riser => {
                    m.apply(&Action::Pad { slot: super::pads::RISER as u8 });
                }
                Shot::Impact => {
                    m.apply(&Action::Pad { slot: super::pads::IMPACT as u8 });
                }
                Shot::Crash => {
                    m.apply(&Action::Pad { slot: super::pads::CRASH as u8 });
                }
                Shot::Fill => {
                    m.apply(&Action::Pad { slot: super::pads::FILL as u8 });
                }
                Shot::KeyLift => {
                    let d = m.deck_mut(side);
                    d.set_key_lock(true);
                    d.set_key_shift(1);
                    self.lifted = Some((side, song_id));
                }
                Shot::Rewind { to_s } => {
                    m.deck_mut(side).spin_back();
                    self.resume = Some(to_s);
                }
                Shot::TapeStop { resume_s } => {
                    m.deck_mut(side).brake();
                    self.resume = Some(resume_s);
                }
                Shot::Repeat { to_s } => m.deck_mut(side).seek_s(to_s),
            }
        }
        self.prev = m.deck(side).position_s();
        self.engaged = Some(side);
    }

    /// The beat, the coming drop and a sing-along of the deck on air (`None` without a beat grid).
    pub fn show(&self, m: &Mixer) -> Option<Show> {
        let (side, song_id) = self.song?;
        let deck = m.deck(side);
        let a = deck.track()?.analysis.as_ref()?;
        if a.bpm <= 0.0 {
            return None;
        }
        let pos = deck.position_s();
        let speed = deck.speed().max(0.01);
        let drop_in_s = self.engaged.and_then(|_| next_drop(&self.moves, pos)).map(|d| (d - pos) / speed);
        let sing = self.engaged.is_some() && fx_at(&self.moves, pos).sing;
        Some(Show { beat_s: 60.0 / a.bpm as f64 / speed, beat_pos: (pos - a.downbeat_s) * a.bpm as f64 / 60.0, drop_in_s, sing, song_id, pos_s: pos, speed })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dj::analysis::Analysis;
    use crate::dj::structure::{Section, SectionKind, Structure};

    /// 120 BPM (beat 0.5 s, bar 2 s): intro, drop at 60 s, breakdown at 120 s, drop at 150 s.
    fn song() -> Analysis {
        let sec = |kind, start_s, end_s| Section { kind, start_s, end_s };
        Analysis {
            bpm: 120.0,
            beat_confidence: 0.9,
            first_beat_s: 0.0,
            downbeat_s: 0.0,
            energy: 8,
            mix_in_s: 8.0,
            mix_out_s: 230.0,
            duration_s: 240.0,
            structure: Structure {
                phrase_start_s: Some(0.0),
                sections: vec![
                    sec(SectionKind::Intro, 0.0, 30.0),
                    sec(SectionKind::Main, 30.0, 60.0),
                    sec(SectionKind::Drop, 60.0, 120.0),
                    sec(SectionKind::Breakdown, 120.0, 150.0),
                    sec(SectionKind::Drop, 150.0, 210.0),
                    sec(SectionKind::Outro, 210.0, 240.0),
                ],
                drop_s: Some(60.0),
                vocals: vec![[30.0, 50.0], [90.0, 110.0]],
                end_s: 238.0,
                choruses: vec![[60.0, 76.0], [150.0, 166.0]],
            },
            ..Default::default()
        }
    }

    #[test]
    fn festival_builds_into_every_drop() {
        let m = plan(&song(), Hype::Festival);
        for drop in [60.0, 150.0] {
            assert!(m.contains(&Move::Build { from_s: drop - 16.0, to_s: drop }), "8 bars of build before {drop}: {m:?}");
            assert!(m.contains(&Move::Roll { from_s: drop - 4.0, to_s: drop - 2.0, beats: 0.5 }), "{m:?}");
            assert!(m.contains(&Move::Roll { from_s: drop - 2.0, to_s: drop - 0.5, beats: 0.25 }), "{m:?}");
            assert!(m.contains(&Move::FakeOut { from_s: drop - 0.5, to_s: drop }), "{m:?}");
            assert!(m.contains(&Move::Impact { at_s: drop }), "{m:?}");
        }
        assert!(m.contains(&Move::Gate { from_s: 120.0, to_s: 134.0 }), "gated breakdown, until the build: {m:?}");
        assert!(m.contains(&Move::ReverbThrow { from_s: 110.0, to_s: 110.5 }), "the line before the breakdown blooms: {m:?}");
        assert!(!m.iter().any(|x| matches!(x, Move::EchoThrow { from_s, .. } if *from_s == 50.0)), "no echo throw inside a build: {m:?}");
    }

    #[test]
    fn party_gives_the_big_drop_the_works() {
        let m = plan(&song(), Hype::Party);
        assert!(m.contains(&Move::Build { from_s: 52.0, to_s: 60.0 }), "4 bars: {m:?}");
        assert!(m.contains(&Move::Roll { from_s: 58.0, to_s: 59.5, beats: 0.5 }), "{m:?}");
        assert!(m.contains(&Move::FakeOut { from_s: 59.5, to_s: 60.0 }), "{m:?}");
        assert!(m.contains(&Move::Impact { at_s: 60.0 }), "{m:?}");
        assert!(!m.iter().any(|x| matches!(x, Move::Build { to_s, .. } if *to_s == 150.0)), "only the big drop: {m:?}");
        assert!(!m.iter().any(|x| matches!(x, Move::Gate { .. })), "no gates at Party: {m:?}");
        assert!(m.contains(&Move::EchoThrow { from_s: 50.0, to_s: 50.5 }) || m.contains(&Move::EchoThrow { from_s: 110.0, to_s: 110.5 }), "{m:?}");
    }

    #[test]
    fn the_last_chorus_becomes_a_sing_along_then_goes_up_a_key() {
        let m = plan(&song(), Hype::Party);
        assert!(m.contains(&Move::SingAlong { from_s: 150.0, to_s: 158.0 }), "4 bars without the singer: {m:?}");
        assert!(m.contains(&Move::KeyLift { at_s: 158.0 }), "then the band comes back a key higher: {m:?}");
        let m = plan(&song(), Hype::Festival);
        assert!(m.contains(&Move::SingAlong { from_s: 150.0, to_s: 166.0 }), "{m:?}");
        assert!(m.contains(&Move::KeyLift { at_s: 166.0 }), "{m:?}");
        assert!(!plan(&song(), Hype::Chill).iter().any(|x| matches!(x, Move::SingAlong { .. } | Move::KeyLift { .. })));
        let mut once = song();
        once.structure.choruses = vec![[60.0, 76.0]];
        assert!(!plan(&once, Hype::Festival).iter().any(|x| matches!(x, Move::SingAlong { .. })), "a chorus heard once is not known yet");
    }

    #[test]
    fn festival_rewinds_and_stretches_the_big_drop_once() {
        let m = plan(&song(), Hype::Festival);
        assert!(m.contains(&Move::Rewind { at_s: 68.0, to_s: 56.0 }), "4 bars in: back to the build, again: {m:?}");
        assert!(m.contains(&Move::Repeat { at_s: 76.0, to_s: 68.0 }), "the best 4 bars once more: {m:?}");
        assert!(!plan(&song(), Hype::Party).iter().any(|x| matches!(x, Move::Rewind { .. } | Move::Repeat { .. })));
    }

    #[test]
    fn breakdowns_get_a_tape_stop_and_a_reverb_throw() {
        let m = plan(&song(), Hype::Party);
        assert!(m.contains(&Move::TapeStop { at_s: 119.5, resume_s: 120.0 }), "{m:?}");
        assert!(m.contains(&Move::ReverbThrow { from_s: 110.0, to_s: 110.5 }), "the last line before the breakdown: {m:?}");
        assert!(!m.contains(&Move::EchoThrow { from_s: 110.0, to_s: 110.5 }), "not both on one line");
    }

    #[test]
    fn phrases_in_a_drop_get_crashes_and_at_festival_fills() {
        let p = plan(&song(), Hype::Party);
        assert!(p.contains(&Move::Crash { at_s: 80.0 }) && p.contains(&Move::Crash { at_s: 96.0 }), "{p:?}");
        assert!(!p.contains(&Move::Crash { at_s: 60.0 }), "the drop itself has the impact");
        assert!(!p.iter().any(|x| matches!(x, Move::Fill { .. })));
        assert!(plan(&song(), Hype::Festival).contains(&Move::Fill { at_s: 79.5 }));
    }

    #[test]
    fn festival_stutters_every_other_sung_line_end() {
        let mut a = song();
        a.structure.vocals = vec![[20.0, 24.0], [180.0, 184.0], [196.0, 200.0]];
        let m = plan(&a, Hype::Festival);
        assert!(m.contains(&Move::EchoThrow { from_s: 24.0, to_s: 24.5 }), "{m:?}");
        assert!(m.contains(&Move::Roll { from_s: 183.75, to_s: 184.0, beats: 0.125 }), "a stutter on the last syllable: {m:?}");
        assert!(m.contains(&Move::EchoThrow { from_s: 200.0, to_s: 200.5 }), "{m:?}");
    }

    #[test]
    fn what_the_channel_does_in_a_sing_along_and_a_reverb_throw() {
        let m = plan(&song(), Hype::Party);
        assert!(fx_at(&m, 152.0).sing);
        assert!(!fx_at(&m, 158.5).sing);
        assert!(fx_at(&m, 110.2).reverb);
        assert_eq!(shots(&m, 157.9, 158.1), vec![Shot::KeyLift]);
        assert_eq!(shots(&m, 119.4, 119.6), vec![Shot::TapeStop { resume_s: 120.0 }]);
        assert_eq!(shots(&m, 79.9, 80.1), vec![Shot::Crash]);
    }

    #[test]
    fn chill_only_throws_an_echo_now_and_then() {
        let m = plan(&song(), Hype::Chill);
        assert_eq!(m, vec![Move::EchoThrow { from_s: 50.0, to_s: 50.5 }], "one per minute at most");
    }

    #[test]
    fn songs_without_a_beat_get_nothing_and_the_mix_out_is_left_alone() {
        let mut a = song();
        a.bpm = 0.0;
        assert!(plan(&a, Hype::Festival).is_empty());
        let mut a = song();
        a.beat_confidence = 0.3; // a real song with a clear groove
        assert!(!plan(&a, Hype::Festival).is_empty());
        let mut a = song();
        a.mix_out_s = 160.0; // the next mix starts soon after the second drop
        let m = plan(&a, Hype::Festival);
        assert!(!m.iter().any(|x| matches!(x, Move::Build { to_s, .. } if *to_s == 150.0)), "too close to the mix: {m:?}");
    }

    #[test]
    fn what_the_channel_does_at_each_moment() {
        let m = plan(&song(), Hype::Festival);
        assert_eq!(fx_at(&m, 30.0), Fx::default());
        let half = fx_at(&m, 52.0);
        assert!((half.rise - 0.5).abs() < 1e-6, "half way up the build: {half:?}");
        assert_eq!(fx_at(&m, 57.0).roll, Some(0.5));
        assert_eq!(fx_at(&m, 59.0).roll, Some(0.25));
        assert!(fx_at(&m, 59.7).mute);
        assert_eq!(fx_at(&m, 60.2), Fx::default(), "the drop lands clean");
        assert!(fx_at(&m, 125.0).gate);
        assert!(fx_at(&m, 110.2).reverb, "the line before the breakdown blooms in the reverb");
    }

    #[test]
    fn one_shots_fire_once_as_the_song_passes_them() {
        let m = plan(&song(), Hype::Festival);
        assert_eq!(shots(&m, 43.9, 44.1), vec![Shot::Riser]);
        assert_eq!(shots(&m, 59.9, 60.1), vec![Shot::Impact]);
        assert!(shots(&m, 60.1, 60.3).is_empty());
        assert!(shots(&m, 70.0, 40.0).is_empty(), "jumping back fires nothing");
    }

    /// The test song as a deck track: a steady tone, 4 minutes at 8 kHz.
    fn track() -> std::sync::Arc<crate::dj::deck::Track> {
        let rate = 8000;
        let pcm = (0..rate as usize * 240)
            .flat_map(|i| {
                let v = ((2.0 * std::f32::consts::PI * 440.0 * i as f32 / rate as f32).sin() * 0.5 * 32767.0) as i16;
                [v, v]
            })
            .collect();
        std::sync::Arc::new(crate::dj::deck::Track { song_id: 7, rate, pcm, analysis: Some(song()) })
    }

    fn render(m: &mut crate::dj::mixer::Mixer, p: &mut Performer, secs: f64, active: bool) -> f32 {
        let (mut master, mut phones) = (vec![0.0; 960], vec![0.0; 960]);
        let mut last = 0.0;
        for _ in 0..(secs * 100.0) as usize {
            p.tick(m, Some(Side::A), active);
            m.render(&mut master, &mut phones);
            last = master.iter().fold(0.0f32, |a, s| a.max(s.abs()));
        }
        last
    }

    #[test]
    fn the_performer_plays_the_fake_out_and_lands_the_drop() {
        use crate::dj::actions::Action;
        let mut m = crate::dj::mixer::Mixer::new(48_000);
        m.deck_mut(Side::A).load(track());
        m.deck_mut(Side::A).seek_s(58.0);
        m.apply(&Action::PlayPause { deck: Side::A });
        m.apply(&Action::Crossfader { value: -1.0 });
        let mut p = Performer::default();
        p.set_hype(Hype::Festival);
        assert!(render(&mut m, &mut p, 1.6, true) < 0.02, "silent in the beat before the drop (59.5–60)");
        let show = p.show(&m).unwrap();
        assert!(show.drop_in_s.is_some_and(|d| d < 0.5), "{show:?}");
        assert!(render(&mut m, &mut p, 0.6, true) > 0.3, "the drop hits (with the impact)");
        assert!(p.show(&m).unwrap().drop_in_s.is_some_and(|d| d > 80.0), "next drop at 150 s");
    }

    #[test]
    fn the_performer_lets_go_when_auto_dj_stops_or_mixes() {
        use crate::dj::actions::Action;
        let mut m = crate::dj::mixer::Mixer::new(48_000);
        m.deck_mut(Side::A).load(track());
        m.deck_mut(Side::A).seek_s(59.0);
        m.apply(&Action::PlayPause { deck: Side::A });
        m.apply(&Action::Crossfader { value: -1.0 });
        let mut p = Performer::default();
        p.set_hype(Hype::Festival);
        render(&mut m, &mut p, 0.6, true);
        assert!(render(&mut m, &mut p, 0.05, false) > 0.3, "not performing: the deck plays plainly");
        assert!(p.show(&m).unwrap().drop_in_s.is_none(), "no countdown when not performing");
    }

    fn deck_at(t: f64) -> crate::dj::mixer::Mixer {
        use crate::dj::actions::Action;
        let mut m = crate::dj::mixer::Mixer::new(48_000);
        m.deck_mut(Side::A).load(track());
        m.deck_mut(Side::A).seek_s(t);
        m.apply(&Action::PlayPause { deck: Side::A });
        m.apply(&Action::Crossfader { value: -1.0 });
        m
    }

    #[test]
    fn the_best_bars_play_again_once() {
        let mut m = deck_at(75.0);
        let mut p = Performer::default();
        p.set_hype(Hype::Festival);
        render(&mut m, &mut p, 1.5, true);
        let after = m.deck(Side::A).position_s();
        assert!((68.0..69.0).contains(&after), "back to the drop's 5th bar: {after}");
        render(&mut m, &mut p, 8.5, true);
        let later = m.deck(Side::A).position_s();
        assert!(later > 76.5, "passed the spot again without jumping: {later}");
    }

    #[test]
    fn the_key_lift_holds_until_the_song_leaves() {
        let mut m = deck_at(157.0);
        let mut p = Performer::default();
        p.set_hype(Hype::Party);
        render(&mut m, &mut p, 1.5, true);
        assert_eq!(m.deck(Side::A).key_shift(), 1, "a semitone up after the sing-along");
        render(&mut m, &mut p, 0.2, false);
        assert_eq!(m.deck(Side::A).key_shift(), 1, "kept through the transition");
        m.deck_mut(Side::A).load(track_with_id(8));
        render(&mut m, &mut p, 0.1, false);
        assert_eq!(m.deck(Side::A).key_shift(), 0, "the next song plays in its own key");
    }

    fn track_with_id(id: i64) -> std::sync::Arc<crate::dj::deck::Track> {
        let t = track();
        std::sync::Arc::new(crate::dj::deck::Track { song_id: id, rate: t.rate, pcm: t.pcm.clone(), analysis: t.analysis.clone() })
    }

    #[test]
    fn the_party_screen_hears_about_the_sing_along() {
        let mut m = deck_at(149.0);
        let mut p = Performer::default();
        p.set_hype(Hype::Party);
        render(&mut m, &mut p, 2.0, true);
        let show = p.show(&m).unwrap();
        assert!(show.sing && show.song_id == 7 && (show.pos_s - 151.0).abs() < 0.2, "{show:?}");
        assert_eq!(p.stems_wanted(&m), Some(Side::A), "split the song so the singer can drop out");
    }

    #[test]
    fn the_next_drop_for_the_countdown() {
        let m = plan(&song(), Hype::Festival);
        assert_eq!(next_drop(&m, 50.0), Some(60.0));
        assert_eq!(next_drop(&m, 61.0), Some(150.0));
        assert_eq!(next_drop(&m, 151.0), None);
        assert_eq!(next_drop(&plan(&song(), Hype::Chill), 0.0), None, "no build, no countdown");
    }
}
