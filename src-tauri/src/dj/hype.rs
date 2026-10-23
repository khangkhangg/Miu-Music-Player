//! Hype (DJ.B): what a DJ does inside a song, not only between songs. From a song's analysis the
//! planner places moves on its timeline: a build-up into each drop (the filter opens, a riser,
//! the last beats rolled shorter and shorter), a beat of silence before the drop (the fake-out),
//! an impact on the drop, echo throws at the end of sung lines and a beat gate in breakdowns.
//! The Hype level decides how much: Chill (echo throws now and then), Party (the big drop gets the
//! works), Festival (every drop, gates in breakdowns). Auto DJ performs them on the song on air.

use serde::{Deserialize, Serialize};

use super::analysis::Analysis;
use super::actions::{Action, Deck as Side, Effect};
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
}

/// Sounds fired once as the song passes them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shot {
    Riser,
    Impact,
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
    let mut last = f64::NEG_INFINITY;
    let mut ends: Vec<f64> = a.structure.vocals.iter().map(|v| v[1]).collect();
    ends.sort_by(|x, y| x.total_cmp(y));
    for at in ends {
        if at <= limit && !in_build(at) && at - last >= echo_spacing(hype) {
            moves.push(Move::EchoThrow { from_s: at, to_s: at + beat });
            last = at;
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
            _ => {}
        }
    }
    fx
}

/// The one-shot sounds the song passed between `prev` and `pos` (none when it jumped back).
pub fn shots(moves: &[Move], prev: f64, pos: f64) -> Vec<Shot> {
    if pos < prev {
        return Vec::new();
    }
    let crossed = |t: f64| t > prev && t <= pos;
    moves
        .iter()
        .filter_map(|m| match *m {
            Move::Build { from_s, .. } if crossed(from_s) => Some(Shot::Riser),
            Move::Impact { at_s } if crossed(at_s) => Some(Shot::Impact),
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
}

/// Plays a song's moves on the deck on air while Auto DJ runs and no transition is under way.
#[derive(Default)]
pub struct Performer {
    hype: Hype,
    song: Option<(Side, i64)>,
    moves: Vec<Move>,
    prev: f64,
    echo_on: bool,
    engaged: Option<Side>,
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
            if self.echo_on {
                m.apply(&Action::Effect { deck: side, effect: Effect::Echo, on: false });
                self.echo_on = false;
            }
        }
    }

    /// Before each block: `live` is the deck on air, `active` whether to perform now.
    pub fn tick(&mut self, m: &mut Mixer, live: Option<Side>, active: bool) {
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
        let pos = deck.position_s();
        if self.song != Some((side, track.song_id)) {
            self.song = Some((side, track.song_id));
            self.moves = track.analysis.as_ref().map(|a| plan(a, self.hype)).unwrap_or_default();
            self.prev = pos;
        }
        let fx = fx_at(&self.moves, pos);
        m.set_fx(side, HypeFx { rise: fx.rise, roll: fx.roll, gate: fx.gate, mute: fx.mute });
        if fx.echo != self.echo_on {
            m.apply(&Action::Effect { deck: side, effect: Effect::Echo, on: fx.echo });
            self.echo_on = fx.echo;
        }
        for shot in shots(&self.moves, self.prev, pos) {
            let slot = match shot {
                Shot::Riser => super::pads::RISER,
                Shot::Impact => super::pads::IMPACT,
            };
            m.apply(&Action::Pad { slot: slot as u8 });
        }
        self.prev = pos;
        self.engaged = Some(side);
    }

    /// The beat and the coming drop of the deck on air (`None` without a beat grid).
    pub fn show(&self, m: &Mixer) -> Option<Show> {
        let (side, _) = self.song?;
        let deck = m.deck(side);
        let a = deck.track()?.analysis.as_ref()?;
        if a.bpm <= 0.0 {
            return None;
        }
        let pos = deck.position_s();
        let speed = deck.speed().max(0.01);
        let drop_in_s = self.engaged.and_then(|_| next_drop(&self.moves, pos)).map(|d| (d - pos) / speed);
        Some(Show { beat_s: 60.0 / a.bpm as f64 / speed, beat_pos: (pos - a.downbeat_s) * a.bpm as f64 / 60.0, drop_in_s })
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
        assert!(m.contains(&Move::EchoThrow { from_s: 110.0, to_s: 110.5 }), "{m:?}");
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
        assert!(fx_at(&m, 110.2).echo);
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

    #[test]
    fn the_next_drop_for_the_countdown() {
        let m = plan(&song(), Hype::Festival);
        assert_eq!(next_drop(&m, 50.0), Some(60.0));
        assert_eq!(next_drop(&m, 61.0), Some(150.0));
        assert_eq!(next_drop(&m, 151.0), None);
        assert_eq!(next_drop(&plan(&song(), Hype::Chill), 0.0), None, "no build, no countdown");
    }
}
