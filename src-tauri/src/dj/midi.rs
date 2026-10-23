//! DJ controllers (D.6): a mapping (data, JSON) says which MIDI message each control sends; the
//! translator turns raw messages into DJ actions and the engine state into LED messages.
//! Built-in mappings are written from each maker's published MIDI list; MIDI learn makes more.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::actions::{Action, Band, Deck, Effect};
use super::engine::EngineStatus;

/// What a control on a DJ controller does in Miu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Control {
    PlayPause,
    Cue,
    Sync,
    Shift,
    Tempo,
    Jog,
    JogTouch,
    EqHigh,
    EqMid,
    EqLow,
    Filter,
    Volume,
    Gain,
    HeadphoneCue,
    Crossfader,
    MasterVolume,
    HeadphoneVolume,
    CueMix,
    HotCue1,
    HotCue2,
    HotCue3,
    HotCue4,
    LoopIn,
    LoopOut,
    Loop4,
    LoopExit,
    FxOn,
    Browse,
    BrowsePush,
    LoadA,
    LoadB,
}

/// How a jog wheel or endless knob says which way it turned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Encoding {
    /// 0x40 is still; 0x41 one step forward, 0x3F one back (Pioneer).
    Centred,
    /// 0x01 one step forward, 0x7F one back (Numark, Hercules).
    TwosComplement,
}

/// A MIDI message as printed in a maker's list: channels 1-16.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Msg {
    Note { channel: u8, number: u8 },
    Cc { channel: u8, number: u8 },
    /// A fine fader or knob sent as two controllers: the coarse half and the fine half.
    Cc14 { channel: u8, msb: u8, lsb: u8 },
    CcRelative { channel: u8, number: u8, encoding: Encoding },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub control: Control,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deck: Option<Deck>,
    pub msg: Msg,
    /// The fader runs the other way (Pioneer tempo faders: up is slower).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub invert: bool,
    /// Jog wheel steps in one full turn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticks_per_turn: Option<f32>,
}

/// A light on the controller and the message that sets it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Led {
    pub control: Control,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deck: Option<Deck>,
    #[serde(flatten)]
    pub msg: Msg,
    #[serde(default = "full")]
    pub on: u8,
    #[serde(default)]
    pub off: u8,
}

fn full() -> u8 {
    0x7F
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mapping {
    pub name: String,
    /// Any of these in the device's MIDI port name (any case) picks this mapping.
    pub port_match: Vec<String>,
    /// False until someone with the controller has checked every control.
    #[serde(default)]
    pub tried_on_device: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// What the user should know (what is left to the hardware, what is a guess).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub controls: Vec<Entry>,
    #[serde(default)]
    pub leds: Vec<Led>,
}

impl Mapping {
    /// Adds a control, replacing what was there for the same control and deck (MIDI learn).
    pub fn set(&mut self, e: Entry) {
        self.controls.retain(|c| !(c.control == e.control && c.deck == e.deck));
        self.controls.push(e);
    }
}

/// The mappings Miu ships, written from the makers' MIDI lists (see each `source`).
pub fn built_in() -> Vec<Mapping> {
    [include_str!("mappings/ddj-flx4.json"), include_str!("mappings/ddj-flx2.json"), include_str!("mappings/ddj-400.json"),
        include_str!("mappings/inpulse-300.json"),
        include_str!("mappings/inpulse-200.json"),
    ]
        .iter()
        .map(|s| serde_json::from_str(s).expect("built-in mapping"))
        .collect()
}

/// The mapping for a MIDI port, by name.
pub fn find<'a>(list: &'a [Mapping], port: &str) -> Option<&'a Mapping> {
    let port = port.to_lowercase();
    list.iter().find(|m| m.port_match.iter().any(|p| port.contains(&p.to_lowercase())))
}

const DEFAULT_TICKS_PER_TURN: f32 = 128.0;

fn deck_index(d: Deck) -> usize {
    match d {
        Deck::A => 0,
        Deck::B => 1,
    }
}

/// -1..1 from 0..1, with the centre detent exactly 0.
fn centred(v: f32) -> f32 {
    let c = v * 2.0 - 1.0;
    if c.abs() < 0.02 {
        0.0
    } else {
        c.clamp(-1.0, 1.0)
    }
}

fn steps(value: u8, encoding: Encoding) -> i32 {
    match encoding {
        Encoding::Centred => value as i32 - 64,
        Encoding::TwosComplement => {
            if value < 64 {
                value as i32
            } else {
                value as i32 - 128
            }
        }
    }
}

fn hot_slot(c: Control) -> Option<u8> {
    match c {
        Control::HotCue1 => Some(0),
        Control::HotCue2 => Some(1),
        Control::HotCue3 => Some(2),
        Control::HotCue4 => Some(3),
        _ => None,
    }
}

/// Turns one controller's MIDI into DJ actions, and the engine state into its lights.
pub struct Translator {
    mapping: Mapping,
    shift: bool,
    /// Last coarse and fine halves of 14-bit controllers, by (channel, controller).
    halves: HashMap<(u8, u8), u8>,
    /// Buttons that switch something on and off (headphone cue, echo), by control and deck.
    toggles: HashMap<(Control, Option<Deck>), bool>,
    /// What each light was last told (status byte, number) → value.
    lit: HashMap<(u8, u8), u8>,
    /// Which decks are looping (from the engine), so one button can start and end a loop.
    looping: [bool; 2],
}

impl Translator {
    pub fn new(mapping: Mapping) -> Self {
        Self { mapping, shift: false, halves: HashMap::new(), toggles: HashMap::new(), lit: HashMap::new(), looping: [false; 2] }
    }

    pub fn mapping(&self) -> &Mapping {
        &self.mapping
    }

    fn toggle(&mut self, c: Control, deck: Option<Deck>) -> bool {
        let on = self.toggles.entry((c, deck)).or_insert(false);
        *on = !*on;
        *on
    }

    /// The actions for one raw MIDI message (none for messages the mapping does not know).
    pub fn handle(&mut self, raw: &[u8]) -> Vec<Action> {
        let (Some(&status), Some(&a), Some(&b)) = (raw.first(), raw.get(1), raw.get(2)) else {
            return Vec::new();
        };
        let channel = (status & 0x0F) + 1;
        let kind = status & 0xF0;
        let entries: Vec<Entry> = self.mapping.controls.clone();
        let mut out = Vec::new();
        for e in &entries {
            match (kind, e.msg) {
                (0x80 | 0x90, Msg::Note { channel: c, number }) if c == channel && number == a => {
                    let pressed = kind == 0x90 && b > 0;
                    out.extend(self.button(e, pressed));
                }
                (0xB0, Msg::Cc { channel: c, number }) if c == channel && number == a => {
                    out.extend(self.continuous(e, b as f32 / 127.0));
                }
                (0xB0, Msg::Cc14 { channel: c, msb, lsb }) if c == channel && (msb == a || lsb == a) => {
                    self.halves.insert((channel, a), b);
                    let hi = *self.halves.get(&(channel, msb)).unwrap_or(&0) as u32;
                    let lo = *self.halves.get(&(channel, lsb)).unwrap_or(&0) as u32;
                    out.extend(self.continuous(e, ((hi << 7) | lo) as f32 / 16383.0));
                }
                (0xB0, Msg::CcRelative { channel: c, number, encoding }) if c == channel && number == a => {
                    let n = steps(b, encoding);
                    match e.control {
                        Control::Jog => {
                            if let Some(deck) = e.deck {
                                out.push(Action::Jog { deck, delta: n as f32 / e.ticks_per_turn.unwrap_or(DEFAULT_TICKS_PER_TURN) });
                            }
                        }
                        Control::Browse => out.push(Action::Browse { steps: n }),
                        _ => {}
                    }
                }
                _ => {}
            }
        }
        out
    }

    fn button(&mut self, e: &Entry, pressed: bool) -> Option<Action> {
        if e.control == Control::Shift {
            self.shift = pressed;
            return None;
        }
        // Cue is held; everything else acts on the press.
        if e.control == Control::Cue {
            return e.deck.map(|deck| Action::Cue { deck, pressed });
        }
        if !pressed {
            return None;
        }
        match e.control {
            Control::LoadA => return Some(Action::LoadSelected { deck: Deck::A }),
            Control::LoadB => return Some(Action::LoadSelected { deck: Deck::B }),
            _ => {}
        }
        let deck = e.deck?;
        if let Some(slot) = hot_slot(e.control) {
            return Some(if self.shift { Action::ClearHotCue { deck, slot } } else { Action::HotCue { deck, slot } });
        }
        Some(match e.control {
            Control::PlayPause => Action::PlayPause { deck },
            Control::Sync => Action::Sync { deck },
            Control::Loop4 if self.looping[deck_index(deck)] => Action::Loop { deck, beats: None },
            Control::Loop4 | Control::LoopIn => Action::Loop { deck, beats: Some(4) },
            Control::LoopOut | Control::LoopExit => Action::Loop { deck, beats: None },
            Control::HeadphoneCue => Action::HeadphoneCue { deck, on: self.toggle(e.control, e.deck) },
            Control::FxOn => Action::Effect { deck, effect: Effect::Echo, on: self.toggle(e.control, e.deck) },
            _ => return None,
        })
    }

    fn continuous(&mut self, e: &Entry, v: f32) -> Option<Action> {
        let v = if e.invert { 1.0 - v } else { v };
        let deck = e.deck;
        Some(match e.control {
            Control::Crossfader => Action::Crossfader { value: centred(v) },
            Control::MasterVolume => Action::MasterVolume { value: v },
            Control::HeadphoneVolume => Action::HeadphoneVolume { value: v },
            Control::CueMix => Action::CueMix { value: v },
            Control::Tempo => Action::Tempo { deck: deck?, value: centred(v) },
            Control::EqHigh => Action::Eq { deck: deck?, band: Band::High, value: centred(v) },
            Control::EqMid => Action::Eq { deck: deck?, band: Band::Mid, value: centred(v) },
            Control::EqLow => Action::Eq { deck: deck?, band: Band::Low, value: centred(v) },
            Control::Filter => Action::Filter { deck: deck?, value: centred(v) },
            Control::Volume => Action::Volume { deck: deck?, value: v },
            Control::Gain => Action::Gain { deck: deck?, value: v },
            _ => return None,
        })
    }

    /// Messages that bring the controller's lights in line with `status`; only changes are sent.
    pub fn leds(&mut self, status: &EngineStatus) -> Vec<Vec<u8>> {
        self.looping = [status.decks[0].loop_s.is_some(), status.decks[1].loop_s.is_some()];
        let leds = self.mapping.leds.clone();
        let mut out = Vec::new();
        for led in &leds {
            let d = led.deck.map(|d| &status.decks[deck_index(d)]);
            let on = match led.control {
                Control::PlayPause => d.is_some_and(|d| d.playing),
                Control::Cue => d.is_some_and(|d| d.song_id.is_some() && !d.playing),
                Control::FxOn => d.is_some_and(|d| d.echo),
                Control::LoopIn | Control::Loop4 => d.is_some_and(|d| d.loop_s.is_some()),
                Control::HeadphoneCue => *self.toggles.get(&(Control::HeadphoneCue, led.deck)).unwrap_or(&false),
                c => match hot_slot(c) {
                    Some(slot) => d.is_some_and(|d| d.hot_cues[slot as usize].is_some()),
                    None => false,
                },
            };
            let (status_byte, number) = match led.msg {
                Msg::Note { channel, number } => (0x90 | (channel - 1), number),
                Msg::Cc { channel, number } | Msg::CcRelative { channel, number, .. } => (0xB0 | (channel - 1), number),
                Msg::Cc14 { channel, msb, .. } => (0xB0 | (channel - 1), msb),
            };
            let value = if on { led.on } else { led.off };
            if self.lit.insert((status_byte, number), value) != Some(value) {
                out.push(vec![status_byte, number, value]);
            }
        }
        out
    }
}

/// A device's name without the "2- " Windows puts in front when there is more than one.
pub fn port_key(port: &str) -> String {
    let t = port.trim();
    match t.split_once("- ") {
        Some((n, rest)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => rest.to_string(),
        _ => t.to_string(),
    }
}

/// The mapping a device uses: the built-in one (if any) with the controls learnt for it on top;
/// a device with only learnt controls gets a mapping named after it. None when neither exists.
pub fn effective(builtin: Option<&Mapping>, learnt: &[Entry], port: &str) -> Option<Mapping> {
    let mut m = match builtin {
        Some(b) => b.clone(),
        None if learnt.is_empty() => return None,
        None => Mapping {
            name: port_key(port),
            port_match: vec![port_key(port)],
            tried_on_device: false,
            source: None,
            note: None,
            controls: Vec::new(),
            leds: Vec::new(),
        },
    };
    for e in learnt {
        m.set(e.clone());
    }
    Some(m)
}

/// Actions the DJ screen carries out (moving through and loading from its song list).
pub fn for_screen(a: &Action) -> bool {
    matches!(a, Action::Browse { .. } | Action::LoadSelected { .. })
}

/// MIDI learn: the mapping entry for `control` from the first message a control sends.
/// Jog wheels and browse knobs are learnt as endless (relative) controls; None for a release.
pub fn learn(control: Control, deck: Option<Deck>, raw: &[u8]) -> Option<Entry> {
    let (&status, &a, &b) = (raw.first()?, raw.get(1)?, raw.get(2)?);
    let channel = (status & 0x0F) + 1;
    let msg = match status & 0xF0 {
        0x90 if b > 0 => Msg::Note { channel, number: a },
        0xB0 if matches!(control, Control::Jog | Control::Browse) => {
            let encoding = if (54..=74).contains(&b) { Encoding::Centred } else { Encoding::TwosComplement };
            Msg::CcRelative { channel, number: a, encoding }
        }
        0xB0 => Msg::Cc { channel, number: a },
        _ => return None,
    };
    Some(Entry { control, deck, msg, invert: false, ticks_per_turn: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dj::actions::{Action, Band, Deck, Effect};

    fn mapping() -> Mapping {
        serde_json::from_value(serde_json::json!({
            "name": "Test DJ", "portMatch": ["TEST DJ"], "triedOnDevice": false,
            "controls": [
                { "control": "playPause", "deck": "A", "msg": { "note": { "channel": 1, "number": 11 } } },
                { "control": "cue", "deck": "A", "msg": { "note": { "channel": 1, "number": 12 } } },
                { "control": "shift", "msg": { "note": { "channel": 1, "number": 63 } } },
                { "control": "hotCue2", "deck": "B", "msg": { "note": { "channel": 9, "number": 1 } } },
                { "control": "headphoneCue", "deck": "A", "msg": { "note": { "channel": 1, "number": 84 } } },
                { "control": "fxOn", "deck": "A", "msg": { "note": { "channel": 1, "number": 71 } } },
                { "control": "eqLow", "deck": "A", "msg": { "cc": { "channel": 1, "number": 15 } } },
                { "control": "tempo", "deck": "A", "msg": { "cc14": { "channel": 1, "msb": 0, "lsb": 32 } }, "invert": true },
                { "control": "jog", "deck": "A", "msg": { "ccRelative": { "channel": 1, "number": 33, "encoding": "centred" } }, "ticksPerTurn": 100 },
                { "control": "jog", "deck": "B", "msg": { "ccRelative": { "channel": 2, "number": 6, "encoding": "twosComplement" } } },
                { "control": "browse", "msg": { "ccRelative": { "channel": 7, "number": 64, "encoding": "twosComplement" } } },
                { "control": "loadB", "msg": { "note": { "channel": 7, "number": 71 } } },
                { "control": "loop4", "deck": "A", "msg": { "note": { "channel": 1, "number": 20 } } },
                { "control": "crossfader", "msg": { "cc": { "channel": 7, "number": 31 } } }
            ],
            "leds": [
                { "control": "playPause", "deck": "A", "note": { "channel": 1, "number": 11 } },
                { "control": "hotCue2", "deck": "B", "note": { "channel": 9, "number": 1 } }
            ]
        }))
        .unwrap()
    }

    fn deck_state(playing: bool, hot: Option<f64>) -> crate::dj::engine::DeckStatus {
        crate::dj::engine::DeckStatus { playing, hot_cues: [None, hot, None, None], song_id: Some(1), ..Default::default() }
    }

    #[test]
    fn buttons_become_actions() {
        let mut c = Translator::new(mapping());
        assert_eq!(c.handle(&[0x90, 11, 127]), vec![Action::PlayPause { deck: Deck::A }]);
        assert_eq!(c.handle(&[0x90, 11, 0]), vec![], "a note-on with velocity 0 is a release");
        assert_eq!(c.handle(&[0x90, 12, 127]), vec![Action::Cue { deck: Deck::A, pressed: true }]);
        assert_eq!(c.handle(&[0x80, 12, 0]), vec![Action::Cue { deck: Deck::A, pressed: false }], "cue is held");
        assert_eq!(c.handle(&[0x98, 1, 127]), vec![Action::HotCue { deck: Deck::B, slot: 1 }]);
        assert_eq!(c.handle(&[0x90, 20, 127]), vec![Action::Loop { deck: Deck::A, beats: Some(4) }]);
        assert_eq!(c.handle(&[0x96, 71, 127]), vec![Action::LoadSelected { deck: Deck::B }]);
        assert_eq!(c.handle(&[0x91, 11, 127]), vec![], "another channel is another control");
        assert_eq!(c.handle(&[0xF8]), vec![], "clock and other messages are ignored");
    }

    #[test]
    fn shift_turns_a_hot_cue_press_into_clear() {
        let mut c = Translator::new(mapping());
        c.handle(&[0x90, 63, 127]);
        assert_eq!(c.handle(&[0x98, 1, 127]), vec![Action::ClearHotCue { deck: Deck::B, slot: 1 }]);
        c.handle(&[0x90, 63, 0]);
        assert_eq!(c.handle(&[0x98, 1, 127]), vec![Action::HotCue { deck: Deck::B, slot: 1 }]);
    }

    #[test]
    fn toggles_remember_their_state() {
        let mut c = Translator::new(mapping());
        assert_eq!(c.handle(&[0x90, 84, 127]), vec![Action::HeadphoneCue { deck: Deck::A, on: true }]);
        assert_eq!(c.handle(&[0x90, 84, 127]), vec![Action::HeadphoneCue { deck: Deck::A, on: false }]);
        assert_eq!(c.handle(&[0x90, 71, 127]), vec![Action::Effect { deck: Deck::A, effect: Effect::Echo, on: true }]);
    }

    #[test]
    fn knobs_and_faders_are_scaled() {
        let mut c = Translator::new(mapping());
        assert_eq!(c.handle(&[0xB0, 15, 64]), vec![Action::Eq { deck: Deck::A, band: Band::Low, value: 0.0 }], "the centre detent is flat");
        assert_eq!(c.handle(&[0xB0, 15, 0]), vec![Action::Eq { deck: Deck::A, band: Band::Low, value: -1.0 }]);
        assert_eq!(c.handle(&[0xB0, 15, 127]), vec![Action::Eq { deck: Deck::A, band: Band::Low, value: 1.0 }]);
        assert_eq!(c.handle(&[0xB6, 31, 0]), vec![Action::Crossfader { value: -1.0 }]);
    }

    #[test]
    fn fourteen_bit_faders_combine_both_halves() {
        let mut c = Translator::new(mapping());
        let a = c.handle(&[0xB0, 0, 127]);
        let b = c.handle(&[0xB0, 32, 127]);
        // Top of an inverted tempo fader (Pioneer: up is slower).
        assert_eq!(b, vec![Action::Tempo { deck: Deck::A, value: -1.0 }]);
        assert_eq!(a.len(), 1);
        c.handle(&[0xB0, 0, 64]);
        let mid = c.handle(&[0xB0, 32, 0]);
        let Action::Tempo { value, .. } = mid[0] else { panic!() };
        assert!(value.abs() < 0.01, "{value}");
    }

    #[test]
    fn jog_wheels_in_both_encodings() {
        let mut c = Translator::new(mapping());
        assert_eq!(c.handle(&[0xB0, 33, 0x41]), vec![Action::Jog { deck: Deck::A, delta: 0.01 }]);
        assert_eq!(c.handle(&[0xB0, 33, 0x3E]), vec![Action::Jog { deck: Deck::A, delta: -0.02 }]);
        let Action::Jog { delta, .. } = c.handle(&[0xB1, 6, 0x7F])[0] else { panic!() };
        assert!(delta < 0.0);
        assert_eq!(c.handle(&[0xB6, 64, 0x01]), vec![Action::Browse { steps: 1 }]);
        assert_eq!(c.handle(&[0xB6, 64, 0x7E]), vec![Action::Browse { steps: -2 }]);
    }

    #[test]
    fn leds_follow_the_engine_and_only_changes_are_sent() {
        let mut c = Translator::new(mapping());
        let mut s = crate::dj::engine::EngineStatus::default();
        s.decks = [deck_state(true, None), deck_state(false, Some(3.0))];
        let first = c.leds(&s);
        assert!(first.contains(&vec![0x90, 11, 127]), "play lit: {first:?}");
        assert!(first.contains(&vec![0x98, 1, 127]), "hot cue 2 on deck B set");
        assert!(c.leds(&s).is_empty(), "nothing changed");
        s.decks[0].playing = false;
        assert_eq!(c.leds(&s), vec![vec![0x90, 11, 0]]);
    }

    #[test]
    fn a_four_beat_loop_button_ends_the_loop_it_started() {
        let mut c = Translator::new(mapping());
        let mut s = crate::dj::engine::EngineStatus::default();
        assert_eq!(c.handle(&[0x90, 20, 127]), vec![Action::Loop { deck: Deck::A, beats: Some(4) }]);
        s.decks[0].loop_s = Some((1.0, 3.0));
        c.leds(&s);
        assert_eq!(c.handle(&[0x90, 20, 127]), vec![Action::Loop { deck: Deck::A, beats: None }], "4 BEAT/EXIT");
    }

    #[test]
    fn the_built_in_mappings_cover_the_decks() {
        let list = built_in();
        assert_eq!(list.len(), 5);
        for m in &list {
            assert!(!m.tried_on_device, "{}: not checked on a device yet", m.name);
            assert!(m.source.is_some(), "{} says where its numbers come from", m.name);
            for deck in [Deck::A, Deck::B] {
                for control in [Control::PlayPause, Control::Cue, Control::Jog, Control::Volume, Control::EqLow, Control::HotCue1] {
                    assert!(m.controls.iter().any(|c| c.control == control && c.deck == Some(deck)), "{}: {control:?} on deck {deck:?}", m.name);
                }
            }
            assert!(m.controls.iter().any(|c| c.control == Control::Crossfader), "{}: crossfader", m.name);
            // No two controls answer the same message (shift layers use their own channel or number).
            let mut seen = std::collections::HashSet::new();
            for c in &m.controls {
                let key = match c.msg {
                    Msg::Note { channel, number } => (0, channel, number),
                    Msg::Cc { channel, number } | Msg::CcRelative { channel, number, .. } => (1, channel, number),
                    Msg::Cc14 { channel, msb, .. } => (1, channel, msb),
                };
                assert!(seen.insert(key), "{}: {:?} is used twice", m.name, c.msg);
            }
        }
        assert_eq!(find(&list, "DDJ-FLX4").map(|m| m.name.as_str()), Some("Pioneer DJ DDJ-FLX4"));
        assert_eq!(find(&list, "DJControl Inpulse 300 MK2").map(|m| m.name.as_str()), Some("Hercules DJControl Inpulse 300"));
    }

    #[test]
    fn midi_learn_builds_a_mapping_entry() {
        let e = learn(Control::PlayPause, Some(Deck::B), &[0x93, 0x0B, 127]).unwrap();
        assert_eq!(e.msg, Msg::Note { channel: 4, number: 11 });
        let e = learn(Control::EqHigh, Some(Deck::A), &[0xB0, 0x07, 90]).unwrap();
        assert_eq!(e.msg, Msg::Cc { channel: 1, number: 7 });
        let e = learn(Control::Jog, Some(Deck::A), &[0xB0, 0x21, 0x41]).unwrap();
        assert_eq!(e.msg, Msg::CcRelative { channel: 1, number: 0x21, encoding: Encoding::Centred });
        let e = learn(Control::Browse, None, &[0xB0, 0x40, 0x7F]).unwrap();
        assert_eq!(e.msg, Msg::CcRelative { channel: 1, number: 0x40, encoding: Encoding::TwosComplement });
        assert!(learn(Control::PlayPause, Some(Deck::A), &[0x90, 11, 0]).is_none(), "a release teaches nothing");
        let mut m = mapping();
        m.set(e.clone());
        m.set(e);
        assert_eq!(m.controls.iter().filter(|c| c.control == Control::Browse).count(), 1, "learning again replaces");
    }

    #[test]
    fn learnt_controls_sit_on_top_of_the_built_in_mapping() {
        let builtin = built_in();
        let flx4 = find(&builtin, "DDJ-FLX4").unwrap();
        let mine = learn(Control::PlayPause, Some(Deck::A), &[0x9F, 0x01, 127]).unwrap();
        let m = effective(Some(flx4), &[mine.clone()], "DDJ-FLX4").unwrap();
        assert_eq!(m.name, flx4.name);
        let plays: Vec<_> = m.controls.iter().filter(|c| c.control == Control::PlayPause && c.deck == Some(Deck::A)).collect();
        assert_eq!(plays, vec![&mine], "the learnt one replaces the built-in one");
        let unknown = effective(None, &[mine.clone()], "My Old Controller").unwrap();
        assert_eq!(unknown.name, "My Old Controller");
        assert!(!unknown.tried_on_device && unknown.controls.len() == 1);
        assert!(effective(None, &[], "Keyboard").is_none(), "nothing known, nothing learnt");
    }

    #[test]
    fn windows_port_numbers_do_not_change_the_device() {
        assert_eq!(port_key("2- DDJ-FLX4"), "DDJ-FLX4");
        assert_eq!(port_key("DDJ-FLX4"), "DDJ-FLX4");
        assert_eq!(port_key("MIDIIN2 (DDJ-FLX4)"), "MIDIIN2 (DDJ-FLX4)");
    }

    #[test]
    fn browsing_and_loading_go_to_the_screen() {
        assert!(for_screen(&Action::Browse { steps: 1 }));
        assert!(for_screen(&Action::LoadSelected { deck: Deck::A }));
        assert!(!for_screen(&Action::PlayPause { deck: Deck::A }));
    }

    #[test]
    fn a_mapping_is_found_by_port_name() {
        let list = vec![mapping()];
        assert!(find(&list, "2- Test DJ MIDI").is_some(), "case-insensitive, any part of the name");
        assert!(find(&list, "Some keyboard").is_none());
    }
}
