//! Auto DJ at work (D.3, D.7): runs in the engine once per block. When the song on air reaches
//! the planned moment it plays the transition's script: the new song is cued (and synced), then
//! the script's ramps and moves drive the mixer until the old song is off and the new one on air.

use serde::Serialize;

use super::actions::{Action, Band, Deck as Side};
use super::analysis::Analysis;
use super::autodj::{self, Transition};
use super::deck::Deck;
use super::mixer::Mixer;
use super::script::{self, Entry, Move, Script, Side as Role, Target};

/// What Auto DJ did that the app has to follow up on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// This song is now on air: the other deck is free for the next one.
    OnAir { deck: Side, song_id: i64 },
}

/// The coming transition, for the screen.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Next {
    pub transition: Transition,
    pub starts_in_s: f64,
    /// Where it starts in the outgoing song and how long it runs there (seconds of that song).
    pub at_s: f64,
    pub length_s: f64,
}

struct Mixing {
    script: Script,
    /// Real time since the transition began, and how long it runs (seconds).
    elapsed_s: f64,
    duration_s: f64,
    incoming: Side,
    /// Progress at the last block (instant moves between it and now are due).
    prev_p: f32,
    /// Tempo ramp: the old song's speed at the start and the speed it glides to.
    speed_from: f64,
    speed_to: Option<f64>,
}

#[derive(Default)]
pub struct AutoMix {
    on: bool,
    style: autodj::Style,
    /// The kind of music of the playlist being played (Auto style), if any.
    vibe: Option<autodj::Vibe>,
    /// How much of each song plays (party edits).
    length: super::hype::SongLength,
    /// Mixes done this session (varies the party tricks).
    mixes: u64,
    live: Option<Side>,
    mixing: Option<Mixing>,
    next: Option<Next>,
    /// A transition chosen by hand for the next mix (used once).
    chosen: Option<Transition>,
    /// The kinds the DJ checked (none: any) and whether to pick among them at random.
    allowed: Vec<autodj::Kind>,
    mix_it_up: bool,
    /// Holding for a party video: the song on air fades out and nothing starts until it is over.
    hold: bool,
    /// Fade-out progress (seconds) and the master volume to come back to.
    fade_s: Option<f64>,
    master_before: Option<f32>,
    restore_master: bool,
}

fn other(s: Side) -> Side {
    match s {
        Side::A => Side::B,
        Side::B => Side::A,
    }
}

const SIDES: [Side; 2] = [Side::A, Side::B];
/// How long the song on air takes to fade out for a party video.
const HOLD_FADE_S: f64 = 4.0;

/// The deck's song analysis; a song not analysed yet is mixed by its length alone.
fn analysis_of(deck: &Deck) -> Option<Analysis> {
    let t = deck.track()?;
    let duration = t.duration_s();
    let mut a = t.analysis.clone().unwrap_or(Analysis {
        bpm: 0.0,
        beat_confidence: 0.0,
        first_beat_s: 0.0,
        downbeat_s: 0.0,
        key: None,
        energy: 5,
        mix_in_s: 0.0,
        mix_out_s: duration,
        duration_s: duration,
        ..Default::default()
    });
    if a.mix_out_s <= 0.0 || a.mix_out_s > duration {
        a.mix_out_s = duration;
    }
    Some(a)
}

/// Where the incoming song starts: its mix-in point, on the downbeat at or before it when it has a beat.
fn entry_s(a: &Analysis) -> f64 {
    if a.bpm <= 0.0 || a.beat_confidence < autodj::MIN_BEAT_CONFIDENCE {
        return a.mix_in_s;
    }
    let bar = 4.0 * 60.0 / a.bpm as f64;
    (a.downbeat_s + ((a.mix_in_s - a.downbeat_s) / bar).floor() * bar).max(0.0)
}

/// Where the incoming song starts for a script's entry.
fn start_point(a: &Analysis, entry: Entry) -> f64 {
    match entry {
        Entry::MixIn => entry_s(a),
        Entry::Drop => a.structure.drop_s.unwrap_or_else(|| entry_s(a)),
        // Its first downbeat (its very start without a beat).
        Entry::Intro if a.bpm > 0.0 && a.beat_confidence >= autodj::MIN_BEAT_CONFIDENCE => a.downbeat_s.rem_euclid(240.0 / a.bpm as f64),
        Entry::Intro => 0.0,
    }
}

fn flatten(m: &mut Mixer, deck: Side) {
    for band in [Band::Low, Band::Mid, Band::High] {
        m.apply(&Action::Eq { deck, band, value: 0.0 });
    }
    m.apply(&Action::Filter { deck, value: 0.0 });
    m.apply(&Action::Volume { deck, value: 1.0 });
}

fn full(deck: Side) -> f32 {
    match deck {
        Side::A => -1.0,
        Side::B => 1.0,
    }
}

/// Every ramp target a script moves.
fn targets(s: &Script) -> Vec<Target> {
    let mut out: Vec<Target> = Vec::new();
    for st in &s.steps {
        if let Move::Ramp { target, .. } = st.mv {
            if !out.contains(&target) {
                out.push(target);
            }
        }
    }
    out
}

impl AutoMix {
    /// Off leaves the mix exactly where it is, mid-transition or not.
    pub fn set_on(&mut self, on: bool) {
        self.on = on;
        if !on {
            self.mixing = None;
            self.next = None;
        }
    }

    pub fn is_on(&self) -> bool {
        self.on
    }

    pub fn set_style(&mut self, style: autodj::Style) {
        self.style = style;
    }

    pub fn set_vibe(&mut self, vibe: Option<autodj::Vibe>) {
        self.vibe = vibe;
    }

    /// Party edits: mix out after a chorus instead of at the end (already resolved from Auto).
    pub fn set_length(&mut self, length: super::hype::SongLength) {
        self.length = length;
    }

    /// Only these kinds of transition (none: any); `mix_it_up` picks among them at random.
    pub fn set_transitions(&mut self, allowed: Vec<autodj::Kind>, mix_it_up: bool) {
        self.allowed = allowed;
        self.mix_it_up = mix_it_up;
    }

    /// Use this transition for the next mix instead of the planned one (`None`: back to planning).
    pub fn set_next_transition(&mut self, t: Option<Transition>) {
        self.chosen = t;
    }

    /// Hold for a party video (the song on air fades out over a few seconds and comes off its deck;
    /// nothing starts) or let go (the master volume comes back and the next song starts).
    pub fn set_hold(&mut self, on: bool) {
        if on && !self.hold {
            self.mixing = None;
            self.next = None;
            self.fade_s = Some(0.0);
        }
        if !on && self.hold {
            self.restore_master = true;
        }
        self.hold = on;
    }

    pub fn is_holding(&self) -> bool {
        self.hold
    }

    /// The deck on air.
    pub fn live(&self) -> Option<Side> {
        self.live
    }

    pub fn next(&self) -> Option<Next> {
        self.next
    }

    /// Decks whose songs must be split into vocals and instrumental for the coming transition.
    pub fn stems_wanted(&self, m: &Mixer) -> Vec<Side> {
        let (Some(next), Some(live), None) = (self.next, self.live, self.mixing.as_ref()) else { return Vec::new() };
        if !script::script(next.transition, false).needs_stems {
            return Vec::new();
        }
        [live, other(live)].into_iter().filter(|s| m.deck(*s).track().is_some() && !m.deck(*s).has_stems()).collect()
    }

    pub fn is_mixing(&self) -> bool {
        self.mixing.is_some()
    }

    /// Called before each block of `frames` the engine renders.
    pub fn tick(&mut self, m: &mut Mixer, frames: usize) -> Option<Event> {
        if self.hold {
            self.fade_out(m, frames);
            return None;
        }
        if self.restore_master {
            self.restore_master = false;
            m.apply(&Action::MasterVolume { value: self.master_before.take().unwrap_or(1.0) });
        }
        if !self.on {
            return None;
        }
        if self.mixing.is_none() && !self.live.is_some_and(|s| m.deck(s).is_playing()) {
            if let Some(s) = SIDES.into_iter().find(|s| m.deck(*s).is_playing()) {
                self.live = Some(s);
            } else {
                return self.start_cold(m);
            }
        }
        let live = self.live?;
        let incoming = other(live);
        if self.mixing.is_none() {
            self.next = None;
            let waiting = m.deck(incoming).track().is_some() && !m.deck(incoming).is_playing() && !m.deck(incoming).ended();
            let (Some(from), Some(to)) = (analysis_of(m.deck(live)), analysis_of(m.deck(incoming))) else { return None };
            let from = super::hype::edited(&from, self.length);
            if !waiting {
                return None;
            }
            let style = self.style.for_pair(self.vibe, &from, &to);
            let transition = self.chosen.unwrap_or_else(|| autodj::plan_with(&from, &to, style, self.mixes, &self.allowed, self.mix_it_up));
            let start = autodj::start_at(&from, transition);
            let pos = m.deck(live).position_s();
            let speed = m.deck(live).speed();
            self.next = Some(Next { transition, starts_in_s: ((start - pos) / speed).max(0.0), at_s: start, length_s: transition.duration_s(&from) });
            if pos < start && !m.deck(live).ended() {
                return None;
            }
            self.chosen = None;
            let mut transition = transition;
            let mut script = script::script(transition, style == autodj::Style::Party);
            if script.needs_stems && !(m.deck(live).has_stems() && m.deck(incoming).has_stems()) {
                // Not split in time: a filter blend keeps one voice at a time too.
                transition = Transition::FilterBlend { bars: 8 };
                script = script::script(transition, style == autodj::Style::Party);
            }
            self.mixes += 1;
            let at = start_point(&to, script.entry);
            m.deck_mut(incoming).seek_s(at);
            if script.sync {
                m.apply(&Action::Sync { deck: incoming });
                if script.entry == Entry::Drop {
                    m.deck_mut(incoming).seek_s(at); // tempo matched; it starts right on its drop
                }
            }
            flatten(m, incoming);
            let duration_s = transition.duration_s(&from) / speed;
            let speed_to = autodj::ramp_target(from.bpm, to.bpm);
            self.mixing = Some(Mixing { script, elapsed_s: 0.0, duration_s, incoming, prev_p: -1.0, speed_from: speed, speed_to });
        }
        self.step(m, live, frames)
    }

    /// Fades the song on air out over `HOLD_FADE_S`, then takes it off its deck.
    fn fade_out(&mut self, m: &mut Mixer, frames: usize) {
        let Some(t) = self.fade_s.as_mut() else { return };
        let from = *self.master_before.get_or_insert(m.master_volume());
        *t += frames as f64 / m.rate() as f64;
        let p = (*t / HOLD_FADE_S).min(1.0) as f32;
        m.apply(&Action::MasterVolume { value: from * (1.0 - p) });
        if p >= 1.0 {
            self.fade_s = None;
            for s in SIDES {
                if m.deck(s).is_playing() {
                    m.deck_mut(s).eject();
                    flatten(m, s);
                }
            }
        }
    }

    /// Nothing plays: start a loaded song that has not been played to its end.
    fn start_cold(&mut self, m: &mut Mixer) -> Option<Event> {
        let ready = |m: &Mixer, s: Side| m.deck(s).track().is_some() && !m.deck(s).ended();
        let prefer = self.live.map(other);
        let s = prefer.filter(|s| ready(m, *s)).or_else(|| SIDES.into_iter().find(|s| ready(m, *s)))?;
        flatten(m, s);
        m.apply(&Action::Crossfader { value: full(s) });
        m.deck_mut(s).play();
        self.live = Some(s);
        Some(Event::OnAir { deck: s, song_id: m.deck(s).track()?.song_id })
    }

    fn step(&mut self, m: &mut Mixer, live: Side, frames: usize) -> Option<Event> {
        let mx = self.mixing.as_mut()?;
        let p = if mx.duration_s > 0.0 { (mx.elapsed_s / mx.duration_s).min(1.0) as f32 } else { 1.0 };
        mx.elapsed_s += frames as f64 / m.rate() as f64;
        let incoming = mx.incoming;
        let deck = |r: Role| if r == Role::Out { live } else { incoming };
        for target in targets(&mx.script) {
            let Some(v) = script::value_at(&mx.script, target, p) else { continue };
            if target == Target::SpeedToIncoming {
                if let Some(to) = mx.speed_to {
                    m.deck_mut(live).set_speed(mx.speed_from + (to - mx.speed_from) * v as f64);
                }
                continue;
            }
            m.apply(&match target {
                Target::Crossfader => Action::Crossfader { value: v * full(incoming) },
                Target::Volume(r) => Action::Volume { deck: deck(r), value: v },
                Target::Eq(r, band) => Action::Eq { deck: deck(r), band, value: v },
                Target::Filter(r) => Action::Filter { deck: deck(r), value: v },
                Target::SpeedToIncoming => continue,
            });
        }
        for mv in script::instants(&mx.script, mx.prev_p, p) {
            match *mv {
                Move::Effect { side, effect, on } => {
                    m.apply(&Action::Effect { deck: deck(side), effect, on });
                }
                Move::Loop { side, beats } => {
                    m.apply(&Action::Loop { deck: deck(side), beats });
                }
                Move::Play => m.deck_mut(incoming).play(),
                Move::Stop => m.deck_mut(live).pause(),
                Move::Pad(slot) => {
                    m.apply(&Action::Pad { slot: slot as u8 });
                }
                Move::Stem { side, mode } => m.deck_mut(deck(side)).set_stem_mode(mode),
                Move::Sync => {
                    m.apply(&Action::Sync { deck: incoming });
                }
                Move::KeyLock { side, on } => {
                    m.apply(&Action::KeyLock { deck: deck(side), on });
                }
                Move::KeyShift { side, semitones } => {
                    m.apply(&Action::KeyShift { deck: deck(side), semitones });
                }
                Move::Ramp { .. } => {}
            }
        }
        mx.prev_p = p;
        if p < 1.0 {
            return None;
        }
        // Off the deck, or it would count as the next song; the app loads the real next one there.
        m.deck_mut(live).eject();
        flatten(m, live);
        flatten(m, incoming);
        m.apply(&Action::Crossfader { value: full(incoming) });
        m.deck_mut(incoming).play();
        self.mixing = None;
        self.next = None;
        self.live = Some(incoming);
        Some(Event::OnAir { deck: incoming, song_id: m.deck(incoming).track()?.song_id })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dj::actions::Deck as Side;
    use crate::dj::analysis::Analysis;
    use crate::dj::deck::Track;
    use crate::dj::mixer::Mixer;
    use std::sync::Arc;

    const RATE: u32 = 8000;

    fn track(id: i64, secs: f64, bpm: f32, mix_out_s: f64, confidence: f32) -> Arc<Track> {
        let a = Analysis { bpm, beat_confidence: confidence, first_beat_s: 0.0, downbeat_s: 0.0, key: Some("8A".into()), energy: 6, mix_in_s: 2.0, mix_out_s, duration_s: secs, ..Default::default() };
        Arc::new(Track { song_id: id, rate: RATE, pcm: vec![0; (secs * RATE as f64) as usize * 2], analysis: Some(a) })
    }

    /// Renders `secs` with Auto DJ ticking before each 10 ms block; returns the events.
    fn run(m: &mut Mixer, auto: &mut AutoMix, secs: f64) -> Vec<Event> {
        let (mut a, mut b) = (vec![0.0; 160], vec![0.0; 160]);
        let mut events = Vec::new();
        for _ in 0..(secs * 100.0) as usize {
            events.extend(auto.tick(m, 80));
            m.render(&mut a, &mut b);
        }
        events
    }

    /// A 120 BPM song with 8-bar phrases from 0 and, if given, a drop.
    fn song(id: i64, secs: f64, mix_out_s: f64, drop_s: Option<f64>) -> Arc<Track> {
        let mut a = Analysis { bpm: 120.0, beat_confidence: 0.8, key: Some("8A".into()), energy: 7, mix_in_s: 2.0, mix_out_s, duration_s: secs, ..Default::default() };
        a.structure.phrase_start_s = Some(0.0);
        a.structure.drop_s = drop_s;
        a.structure.end_s = secs;
        Arc::new(Track { song_id: id, rate: RATE, pcm: vec![0; (secs * RATE as f64) as usize * 2], analysis: Some(a) })
    }

    fn ready(a: Arc<Track>, b: Arc<Track>, t: Transition) -> (Mixer, AutoMix) {
        let mut m = Mixer::new(RATE);
        m.deck_mut(Side::A).load(a);
        m.deck_mut(Side::B).load(b);
        let mut auto = AutoMix::default();
        auto.set_next_transition(Some(t));
        auto.set_on(true);
        (m, auto)
    }

    #[test]
    fn a_quick_party_edit_mixes_out_after_the_first_chorus() {
        let base = song(1, 240.0, 200.0, None);
        let mut an = base.analysis.clone().unwrap();
        an.structure.choruses = vec![[48.0, 64.0], [112.0, 128.0]];
        let a = Arc::new(Track { song_id: 1, rate: RATE, pcm: base.pcm.clone(), analysis: Some(an) });
        let (mut m, mut auto) = ready(a, song(2, 240.0, 200.0, None), Transition::BeatBlend { bars: 8 });
        auto.set_length(crate::dj::hype::SongLength::Quick);
        run(&mut m, &mut auto, 0.05);
        let next = auto.next().unwrap();
        assert!(next.at_s < 64.0 && next.at_s > 40.0, "the mix ends near the first chorus end, not at 200 s: {next:?}");
    }

    #[test]
    fn the_drop_swap_brings_the_new_song_in_on_its_drop() {
        // 8 bars = 16 s ending at the phrase at 48 s (mix out 50): the build starts at 32 s.
        let (mut m, mut auto) = ready(song(1, 60.0, 50.0, None), song(2, 90.0, 80.0, Some(30.0)), Transition::DropSwap { bars: 8 });
        run(&mut m, &mut auto, 0.05);
        assert!((auto.next().unwrap().starts_in_s - 32.0).abs() < 0.2, "{:?}", auto.next());
        // Where it sits in the old song, for the mix zone on its waveform: 32..48 s.
        let n = auto.next().unwrap();
        assert!((n.at_s - 32.0).abs() < 0.2 && (n.length_s - 16.0).abs() < 0.2, "{n:?}");
        run(&mut m, &mut auto, 47.9);
        assert!(!m.deck(Side::B).is_playing(), "still building");
        let events = run(&mut m, &mut auto, 0.2);
        assert_eq!(events, vec![Event::OnAir { deck: Side::B, song_id: 2 }]);
        let p = m.deck(Side::B).position_s();
        assert!((30.0..30.25).contains(&p), "the new song started on its drop: {p}");
    }

    #[test]
    fn the_tempo_ramp_glides_the_old_song_to_the_new_tempo() {
        let mut a = (*song(1, 120.0, 110.0, None)).analysis.clone().unwrap();
        a.bpm = 124.0;
        let old = Arc::new(Track { song_id: 1, rate: RATE, pcm: vec![0; 120 * RATE as usize * 2], analysis: Some(a) });
        let mut b = (*song(2, 120.0, 100.0, None)).analysis.clone().unwrap();
        b.bpm = 108.0;
        let new = Arc::new(Track { song_id: 2, rate: RATE, pcm: vec![0; 120 * RATE as usize * 2], analysis: Some(b) });
        let (mut m, mut auto) = ready(old, new, Transition::TempoRamp { bars: 32 });
        run(&mut m, &mut auto, 0.05);
        let starts = auto.next().unwrap().starts_in_s;
        run(&mut m, &mut auto, starts + 0.05);
        assert!(m.deck(Side::A).key_lock(), "key lock while the tempo moves");
        let bar = 240.0 / 124.0;
        run(&mut m, &mut auto, 16.0 * bar * 0.9);
        assert!((m.deck(Side::A).bpm().unwrap() - 108.0).abs() < 4.0, "nearly at the new tempo: {:?}", m.deck(Side::A).bpm());
        let events = run(&mut m, &mut auto, 20.0 * bar);
        assert_eq!(events, vec![Event::OnAir { deck: Side::B, song_id: 2 }]);
        assert!((m.deck(Side::B).bpm().unwrap() - 108.0).abs() < 0.5, "the new song at its own tempo: {:?}", m.deck(Side::B).bpm());
    }

    #[test]
    fn the_vocal_out_asks_for_stems_and_uses_them() {
        use crate::dj::deck::StemMode;
        let (mut m, mut auto) = ready(song(1, 90.0, 80.0, None), song(2, 90.0, 80.0, None), Transition::VocalOut { bars: 16 });
        run(&mut m, &mut auto, 0.05);
        assert_eq!(auto.stems_wanted(&m), vec![Side::A, Side::B], "both songs need splitting before it starts");
        for side in [Side::A, Side::B] {
            let n = m.deck(side).track().unwrap().pcm.len();
            m.deck_mut(side).set_stems(Arc::new(crate::dj::stems::Stems { vocals: vec![0; n], instrumental: vec![0; n] }));
        }
        assert!(auto.stems_wanted(&m).is_empty());
        let starts = auto.next().unwrap().starts_in_s;
        run(&mut m, &mut auto, starts + 16.0);
        assert_eq!(m.deck(Side::A).stem_mode(), StemMode::Vocals, "the old voice");
        assert_eq!(m.deck(Side::B).stem_mode(), StemMode::Instrumental, "over the new instrumental");
    }

    #[test]
    fn without_stems_in_time_the_vocal_out_becomes_a_filter_blend() {
        use crate::dj::deck::StemMode;
        let (mut m, mut auto) = ready(song(1, 90.0, 80.0, None), song(2, 90.0, 80.0, None), Transition::VocalOut { bars: 16 });
        let events = run(&mut m, &mut auto, 89.0);
        assert!(events.contains(&Event::OnAir { deck: Side::B, song_id: 2 }), "the mix still happens");
        assert_eq!(m.deck(Side::B).stem_mode(), StemMode::Full);
    }

    #[test]
    fn holding_for_a_video_fades_the_song_out_then_starts_the_next_one_after() {
        let (mut m, mut auto) = ready(song(1, 60.0, 50.0, None), song(2, 60.0, 50.0, None), Transition::Cut);
        auto.set_next_transition(None);
        run(&mut m, &mut auto, 1.0);
        assert!(m.deck(Side::A).is_playing());
        auto.set_hold(true);
        run(&mut m, &mut auto, 2.0);
        assert!(m.master_volume() < 0.6 && m.deck(Side::A).is_playing(), "fading: {}", m.master_volume());
        run(&mut m, &mut auto, 3.0);
        assert!(!m.deck(Side::A).is_playing() && m.deck(Side::A).track().is_none(), "faded out and taken off");
        assert!(!m.deck(Side::B).is_playing(), "nothing starts during the video");
        auto.set_hold(false);
        let events = run(&mut m, &mut auto, 0.2);
        assert_eq!(events, vec![Event::OnAir { deck: Side::B, song_id: 2 }], "after the video: the next song");
        assert_eq!(m.master_volume(), 1.0);
    }

    #[test]
    fn echo_out_uses_the_echo_and_lets_it_ring() {
        // 4 bars = 8 s; the new song comes in on the phrase at 48 (three quarters in): it starts at 42,
        // the echo comes on at 46.
        let (mut m, mut auto) = ready(song(1, 60.0, 50.0, None), song(2, 90.0, 80.0, None), Transition::EchoOut);
        run(&mut m, &mut auto, 47.0);
        assert!(m.effects(Side::A).0, "echo on before the drop");
        run(&mut m, &mut auto, 1.5);
        assert!(!m.effects(Side::A).0, "off at the drop: the repeats ring out");
        assert!(m.deck(Side::B).is_playing());
    }

    #[test]
    fn the_ballad_gap_has_a_moment_of_quiet() {
        let (mut m, mut auto) = ready(song(1, 40.0, 38.0, None), song(2, 60.0, 50.0, None), Transition::BalladGap);
        // It fades out by the real end (40 s): starts 4.4 s before; quiet from 4.4 s to 5.6 s in.
        run(&mut m, &mut auto, 35.6 + 5.0);
        assert!(m.volume(Side::A) < 0.01 && m.volume(Side::B) < 0.01, "quiet: {} {}", m.volume(Side::A), m.volume(Side::B));
        run(&mut m, &mut auto, 3.2);
        assert_eq!(m.volume(Side::B), 1.0, "the new song is up");
        assert_eq!(auto.live(), Some(Side::B));
    }

    #[test]
    fn starts_the_loaded_song_when_nothing_plays() {
        let mut m = Mixer::new(RATE);
        m.deck_mut(Side::B).load(track(1, 30.0, 120.0, 25.0, 0.8));
        let mut auto = AutoMix::default();
        auto.set_on(true);
        run(&mut m, &mut auto, 0.05);
        assert!(m.deck(Side::B).is_playing());
        assert_eq!(m.crossfader(), 1.0, "on air: deck B");
        assert_eq!(auto.live(), Some(Side::B));
    }

    #[test]
    fn does_nothing_while_off() {
        let mut m = Mixer::new(RATE);
        m.deck_mut(Side::A).load(track(1, 30.0, 120.0, 25.0, 0.8));
        let mut auto = AutoMix::default();
        run(&mut m, &mut auto, 0.05);
        assert!(!m.deck(Side::A).is_playing());
    }

    #[test]
    fn beat_blends_into_the_next_song_and_hands_over() {
        let mut m = Mixer::new(RATE);
        // 120 BPM: a bar is 2 s, 16 bars 32 s; mix out at 40 s so the blend starts at 8 s.
        m.deck_mut(Side::A).load(track(1, 50.0, 120.0, 40.0, 0.8));
        m.deck_mut(Side::B).load(track(2, 60.0, 123.0, 50.0, 0.8));
        let mut auto = AutoMix::default();
        auto.set_on(true);
        run(&mut m, &mut auto, 0.05);
        assert_eq!(auto.live(), Some(Side::A));
        let next = auto.next().unwrap();
        assert_eq!(next.transition, crate::dj::autodj::Transition::BeatBlend { bars: 16 });
        assert!((next.starts_in_s - 8.0).abs() < 0.1, "{}", next.starts_in_s);
        run(&mut m, &mut auto, 8.0);
        assert!(m.deck(Side::B).is_playing(), "the blend has begun");
        assert!((m.deck(Side::B).bpm().unwrap() - 120.0).abs() < 0.01, "synced to the song on air");
        let (pa, pb) = (m.deck(Side::A).bar_phase().unwrap(), m.deck(Side::B).bar_phase().unwrap());
        assert!((pa - pb).abs() < 0.05 || (pa - pb).abs() > 3.95, "bars lined up: {pa} {pb}");
        run(&mut m, &mut auto, 16.0);
        assert!(m.crossfader() > -0.1 && m.crossfader() < 0.1, "half way: {}", m.crossfader());
        let events = run(&mut m, &mut auto, 17.0);
        assert_eq!(events, vec![Event::OnAir { deck: Side::B, song_id: 2 }]);
        assert!(!m.deck(Side::A).is_playing());
        assert!(m.deck(Side::A).track().is_none(), "the old song is off the deck, ready for the next");
        assert_eq!(m.crossfader(), 1.0);
        assert_eq!(auto.live(), Some(Side::B));
    }

    #[test]
    fn a_ballad_crossfades_at_the_mix_points() {
        let mut m = Mixer::new(RATE);
        m.deck_mut(Side::A).load(track(1, 30.0, 70.0, 20.0, 0.1));
        m.deck_mut(Side::B).load(track(2, 30.0, 120.0, 25.0, 0.8));
        let mut auto = AutoMix::default();
        auto.set_style(autodj::Style::Smooth); // Auto would treat the ballad as chill
        auto.set_on(true);
        run(&mut m, &mut auto, 11.0);
        assert!(!m.deck(Side::B).is_playing(), "the crossfade starts 8 s before the mix out (12 s)");
        run(&mut m, &mut auto, 1.5);
        assert!(m.deck(Side::B).is_playing());
        assert!((m.deck(Side::B).tempo()).abs() < 1e-9, "no beat-matching");
        assert!((m.deck(Side::B).position_s() - 2.5).abs() < 0.1, "half a second from its mix-in point");
        let events = run(&mut m, &mut auto, 8.0);
        assert_eq!(events, vec![Event::OnAir { deck: Side::B, song_id: 2 }]);
    }

    #[test]
    fn a_song_ending_with_nothing_mixed_in_lets_the_next_one_start() {
        let mut m = Mixer::new(RATE);
        let mut auto = AutoMix::default();
        auto.set_on(true);
        m.deck_mut(Side::A).load(track(1, 3.0, 120.0, 2.9, 0.8));
        run(&mut m, &mut auto, 4.0);
        assert!(m.deck(Side::A).ended());
        m.deck_mut(Side::B).load(track(2, 30.0, 120.0, 25.0, 0.8));
        let events = run(&mut m, &mut auto, 0.05);
        assert!(m.deck(Side::B).is_playing());
        assert_eq!(events, vec![Event::OnAir { deck: Side::B, song_id: 2 }]);
    }

    #[test]
    fn turning_it_off_mid_transition_leaves_the_mix_as_it_is() {
        let mut m = Mixer::new(RATE);
        m.deck_mut(Side::A).load(track(1, 50.0, 120.0, 40.0, 0.8));
        m.deck_mut(Side::B).load(track(2, 60.0, 120.0, 50.0, 0.8));
        let mut auto = AutoMix::default();
        auto.set_on(true);
        run(&mut m, &mut auto, 12.0);
        auto.set_on(false);
        let x = m.crossfader();
        run(&mut m, &mut auto, 2.0);
        assert_eq!(m.crossfader(), x);
        assert!(m.deck(Side::A).is_playing() && m.deck(Side::B).is_playing());
    }
}

// Machine test: analyses `MIU_DJ_SONGS` (default 40) songs in `MIU_DJ_DIR` (default D:\Music),
// starts with `MIU_DJ_FIRST` (default the first), lets the picker choose the next one and Auto DJ
// mix them, and writes the 20 s before to 20 s after the transition to `MIU_DJ_OUT` (a WAV) to
// listen to. Run with --release.
#[cfg(test)]
mod machine_tests {
    use super::*;
    use crate::dj::autodj::{pick, Candidate, EnergyCurve};
    use crate::dj::deck::Track;
    use crate::dj::engine::decode_track;
    use std::collections::HashSet;
    use std::io::Write;
    use std::sync::Arc;

    fn write_wav(path: &str, rate: u32, stereo: &[f32]) {
        let mut f = std::io::BufWriter::new(std::fs::File::create(path).unwrap());
        let data = (stereo.len() * 2) as u32;
        f.write_all(b"RIFF").unwrap();
        f.write_all(&(36 + data).to_le_bytes()).unwrap();
        f.write_all(b"WAVEfmt ").unwrap();
        for v in [16u32.to_le_bytes().as_slice(), &1u16.to_le_bytes(), &2u16.to_le_bytes(), &rate.to_le_bytes(), &(rate * 4).to_le_bytes(), &4u16.to_le_bytes(), &16u16.to_le_bytes()] {
            f.write_all(v).unwrap();
        }
        f.write_all(b"data").unwrap();
        f.write_all(&data.to_le_bytes()).unwrap();
        for s in stereo {
            f.write_all(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).unwrap();
        }
    }

    #[test]
    #[ignore]
    fn mixes_two_real_songs() {
        let dir = std::env::var("MIU_DJ_DIR").unwrap_or_else(|_| r"D:\Music".into());
        let n: usize = std::env::var("MIU_DJ_SONGS").ok().and_then(|s| s.parse().ok()).unwrap_or(40);
        let out = std::env::var("MIU_DJ_OUT").unwrap_or_else(|_| "automix.wav".into());
        let mut files: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok().map(|e| e.path())).filter(|p| matches!(p.extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref(), Some("mp3" | "flac" | "m4a"))).collect();
        files.sort();
        let step = (files.len() / n).max(1);
        let files: Vec<_> = files.into_iter().step_by(step).take(n).collect();
        let first = std::env::var("MIU_DJ_FIRST").ok().and_then(|f| files.iter().position(|p| p.to_string_lossy().contains(&f))).unwrap_or(0);
        let candidates: Vec<Candidate> = files
            .iter()
            .enumerate()
            .filter_map(|(i, p)| {
                let (mono, rate) = crate::dj::analysis::decode_mono(p).ok()?;
                Some(Candidate { song_id: i as i64, analysis: crate::dj::analysis::analyze(&mono, rate), liked: false })
            })
            .collect();
        let now = candidates.iter().find(|c| c.song_id == first as i64).unwrap();
        let next = pick(&now.analysis, &candidates, &[now.song_id].into_iter().collect::<HashSet<_>>(), &[], EnergyCurve::Keep, 1).unwrap();
        let next_c = candidates.iter().find(|c| c.song_id == next).unwrap();
        let t = crate::dj::autodj::plan(&now.analysis, &next_c.analysis, crate::dj::autodj::Style::Smooth, 0);
        let show = |c: &Candidate| format!("{} ({:.1} BPM conf {:.2}, key {}, energy {}, mix {:.0}-{:.0} s)", files[c.song_id as usize].file_name().unwrap().to_string_lossy(), c.analysis.bpm, c.analysis.beat_confidence, c.analysis.key.as_deref().unwrap_or("-"), c.analysis.energy, c.analysis.mix_in_s, c.analysis.mix_out_s);
        println!("from {}\n  to {}\n  by {t:?}, starting at {:.1} s", show(now), show(next_c), crate::dj::autodj::start_at(&now.analysis, t));
        let load = |c: &Candidate| -> Arc<Track> { Arc::new(decode_track(&files[c.song_id as usize], c.song_id, Some(c.analysis.clone())).unwrap()) };
        let rate = 44_100;
        let mut m = Mixer::new(rate);
        m.deck_mut(Side::A).load(load(now));
        m.deck_mut(Side::B).load(load(next_c));
        let start = crate::dj::autodj::start_at(&now.analysis, t);
        // Skip ahead to 20 s before the transition, then record until 20 s after it ends.
        m.deck_mut(Side::A).seek_s((start - 20.0).max(0.0));
        let mut auto = AutoMix::default();
        auto.set_on(true);
        let (mut a, mut b) = (vec![0.0; 882], vec![0.0; 882]);
        let mut wav = Vec::new();
        let mut after = None;
        for i in 0.. {
            if let Some(Event::OnAir { deck: Side::B, .. }) = auto.tick(&mut m, 441) {
                after = Some(i);
                println!("on air: deck B at block {i}; its tempo {:+.1} %", m.deck(Side::B).tempo() * 8.0);
            }
            m.render(&mut a, &mut b);
            wav.extend_from_slice(&a);
            if after.is_some_and(|s| i > s + 2000) || i > 100 * 300 {
                break;
            }
        }
        write_wav(&out, rate, &wav);
        println!("wrote {} s to {out}", wav.len() / 2 / rate as usize);
    }

    // Machine test: every transition between two real songs (`MIU_DJ_A`, `MIU_DJ_B`; the tempo
    // ramp goes into `MIU_DJ_C`, a song at another tempo), each into `MIU_DJ_OUT`/demo-<name>.wav,
    // from 20 s before the switch to 15 s after. Run with --release.
    #[test]
    #[ignore]
    fn demo_every_transition() {
        use crate::dj::autodj::Style;
        let env = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k}"));
        let out_dir = std::path::PathBuf::from(env("MIU_DJ_OUT"));
        let load = |p: &str, id: i64| -> Arc<Track> {
            let (mono, r) = crate::dj::analysis::decode_mono(std::path::Path::new(p)).unwrap();
            let a = crate::dj::analysis::analyze(&mono, r);
            println!("{p}: {:.1} BPM conf {:.2} key {:?} energy {} phrases {:?} drop {:?}", a.bpm, a.beat_confidence, a.key, a.energy, a.structure.phrase_start_s, a.structure.drop_s);
            Arc::new(decode_track(std::path::Path::new(p), id, Some(a)).unwrap())
        };
        let (a, b, c) = (load(&env("MIU_DJ_A"), 1), load(&env("MIU_DJ_B"), 2), load(&env("MIU_DJ_C"), 3));
        let stems = |t: &Arc<Track>| Arc::new(crate::dj::stems::split(&t.pcm, t.rate));
        let (sa, sb) = (stems(&a), stems(&b));
        let all = [
            ("beat-blend", Transition::BeatBlend { bars: 16 }, Style::Smooth),
            ("filter-blend", Transition::FilterBlend { bars: 16 }, Style::Smooth),
            ("breakdown-blend", Transition::BreakdownBlend { bars: 8 }, Style::Smooth),
            ("loop-out", Transition::LoopOut { bars: 8 }, Style::Smooth),
            ("drop-swap", Transition::DropSwap { bars: 8 }, Style::Party),
            ("vocal-out", Transition::VocalOut { bars: 16 }, Style::Smooth),
            ("key-shift-blend", Transition::KeyShiftBlend { bars: 16, semitones: 1 }, Style::Smooth),
            ("echo-out", Transition::EchoOut, Style::Party),
            ("reverb-wash", Transition::ReverbWash, Style::Smooth),
            ("spin-back", Transition::SpinBack, Style::Party),
            ("brake", Transition::Brake, Style::Party),
            ("crossfade", Transition::Crossfade { secs: 8.0 }, Style::Smooth),
            ("ballad-gap", Transition::BalladGap, Style::Smooth),
            ("cut", Transition::Cut, Style::Party),
            ("tempo-ramp", Transition::TempoRamp { bars: 32 }, Style::Smooth),
        ];
        for (name, t, style) in all {
            let rate = 44_100;
            let mut m = Mixer::new(rate);
            let incoming = if name == "tempo-ramp" { c.clone() } else { b.clone() };
            m.deck_mut(Side::A).load(a.clone());
            m.deck_mut(Side::B).load(incoming);
            if name == "vocal-out" {
                m.deck_mut(Side::A).set_stems(sa.clone());
                m.deck_mut(Side::B).set_stems(sb.clone());
            }
            let from = a.analysis.clone().unwrap();
            let start = crate::dj::autodj::start_at(&from, t);
            let switch = start + crate::dj::script::switch_point(t) as f64 * t.duration_s(&from);
            m.deck_mut(Side::A).seek_s((switch.min(start) - 20.0).max(0.0));
            let mut auto = AutoMix::default();
            auto.set_style(style);
            auto.set_next_transition(Some(t));
            auto.set_on(true);
            let (mut ma, mut mb) = (vec![0.0; 882], vec![0.0; 882]);
            let mut wav = Vec::new();
            let mut done = None;
            for i in 0.. {
                if let Some(Event::OnAir { deck: Side::B, .. }) = auto.tick(&mut m, 441) {
                    done = Some(i);
                }
                m.render(&mut ma, &mut mb);
                wav.extend_from_slice(&ma);
                if done.is_some_and(|d| i > d + 1500) || i > 100 * 240 {
                    break;
                }
            }
            let path = out_dir.join(format!("demo-{name}.wav"));
            write_wav(path.to_str().unwrap(), rate, &wav);
            println!("{name}: {} s", wav.len() / 2 / rate as usize);
        }
    }
}
