//! Everything you can do to the DJ engine (D.0). The DJ screen, keyboard shortcuts and MIDI
//! controllers all send these; nothing else talks to the engine. Continuous controls carry
//! normalised values so a controller mapping only has to scale its 0..127 (or 14-bit) range.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Deck {
    A,
    B,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Band {
    Low,
    Mid,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Effect {
    Echo,
    FilterSweep,
    SpinBack,
    /// A wash of reverb that rings on after the deck is faded out.
    Reverb,
    /// Holds the reverb tail as it is (nothing new goes in).
    Freeze,
    /// The record slows to a stop.
    Brake,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Action {
    PlayPause { deck: Deck },
    /// Pressed: jump to the cue point and preview while held (or set it while paused); released: back.
    Cue { deck: Deck, pressed: bool },
    /// Match this deck's tempo and beat phase to the other deck.
    Sync { deck: Deck },
    /// -1..1 = the tempo range (±8 %); 0 = the song's own tempo.
    Tempo { deck: Deck, value: f32 },
    /// Push the deck a little ahead (+) or behind (-) for a moment, to line up beats by hand.
    Nudge { deck: Deck, amount: f32 },
    /// Jog wheel turned by `delta` (one full turn = 1.0): nudges while playing, scrubs while paused.
    Jog { deck: Deck, delta: f32 },
    /// -1 (kill) .. 0 (flat) .. 1 (+6 dB).
    Eq { deck: Deck, band: Band, value: f32 },
    /// -1 (low-pass, closed) .. 0 (off) .. 1 (high-pass, closed).
    Filter { deck: Deck, value: f32 },
    /// 0..1, the channel fader.
    Volume { deck: Deck, value: f32 },
    /// 0..1 trim before the fader, 0.5 = unity.
    Gain { deck: Deck, value: f32 },
    /// -1 = only deck A, 1 = only deck B.
    Crossfader { value: f32 },
    /// Send this deck to the headphones.
    HeadphoneCue { deck: Deck, on: bool },
    /// 0 = only the cued decks in the headphones, 1 = only the master mix.
    CueMix { value: f32 },
    MasterVolume { value: f32 },
    HeadphoneVolume { value: f32 },
    /// Hot cue `slot` (0..4): jump to it, or set it when empty.
    HotCue { deck: Deck, slot: u8 },
    ClearHotCue { deck: Deck, slot: u8 },
    /// Loop the next `beats` beats (4, 8, 16), or end the loop with `None`.
    Loop { deck: Deck, beats: Option<u8> },
    Effect { deck: Deck, effect: Effect, on: bool },
    /// Party pad `slot` (air horn and friends).
    Pad { slot: u8 },
    /// Put a song on a deck.
    Load { deck: Deck, song: i64 },
    /// Move the selection in the song list (controller browse knob), then load the selection.
    Browse { steps: i32 },
    LoadSelected { deck: Deck },
    AutoDj { on: bool },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_travel_as_plain_json_for_the_screen_and_controller_mappings() {
        let a = Action::Eq { deck: Deck::B, band: Band::Low, value: -1.0 };
        let json = serde_json::to_value(&a).unwrap();
        assert_eq!(json, serde_json::json!({ "type": "eq", "deck": "B", "band": "low", "value": -1.0 }));
        assert_eq!(serde_json::from_value::<Action>(json).unwrap(), a);
        let cue: Action = serde_json::from_str(r#"{ "type": "headphoneCue", "deck": "A", "on": true }"#).unwrap();
        assert_eq!(cue, Action::HeadphoneCue { deck: Deck::A, on: true });
        let l: Action = serde_json::from_str(r#"{ "type": "loop", "deck": "A", "beats": null }"#).unwrap();
        assert_eq!(l, Action::Loop { deck: Deck::A, beats: None });
    }
}
