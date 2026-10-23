//! The DJ mixer (D.2): two decks, each through gain → 3-band EQ → filter → channel fader, then
//! the crossfader into the master (speakers, with a limiter so two loud songs never clip), and a
//! headphone mix of the cued decks (before their faders) blended with the master.

use std::sync::Arc;

use super::actions::{Action, Band, Deck as Side, Effect};
use super::deck::Deck;
use super::dsp::{self, DjFilter, Echo, Reverb, ThreeBand};

/// The limiter leaves the music alone below this and bends it smoothly under 1.0 above.
const KNEE: f32 = 0.8;
/// How fast the meters fall back, per block.
const METER_FALL: f32 = 0.85;
/// The filter sweep effect opens the high-pass this far over this many beats.
const SWEEP_TO: f32 = 0.9;
const SWEEP_BEATS: f32 = 8.0;
/// Echo repeats every three quarters of a beat.
const ECHO_BEATS: f32 = 0.75;

/// One deck's channel strip and its control positions.
struct Channel {
    eq: ThreeBand,
    eq_values: [f32; 3],
    filter: DjFilter,
    /// Where the filter knob is (the sweep effect moves the filter from here and back).
    filter_value: f32,
    /// Filter sweep effect in progress, 0..1.
    sweep: Option<f32>,
    echo: Echo,
    reverb: Reverb,
    gain: f32,
    volume: f32,
    cue: bool,
    /// Gains reached at the end of the last block (trim, fader, crossfader): changes ramp from here.
    last: (f32, f32, f32),
    level: f32,
}

impl Channel {
    fn new(rate: u32) -> Self {
        Self { eq: ThreeBand::new(rate), eq_values: [0.0; 3], filter: DjFilter::new(rate), filter_value: 0.0, sweep: None, echo: Echo::new(rate), reverb: Reverb::new(rate), gain: 0.5, volume: 1.0, cue: false, last: (1.0, 1.0, std::f32::consts::FRAC_1_SQRT_2), level: 0.0 }
    }
}

pub struct Mixer {
    rate: u32,
    decks: [Deck; 2],
    channels: [Channel; 2],
    crossfader: f32,
    cue_mix: f32,
    master_volume: f32,
    phones_volume: f32,
    /// Master and headphone volume reached at the end of the last block.
    last_out: (f32, f32),
    scratch: [Vec<f32>; 2],
    /// Party pad sounds, and the ones playing (sound, position).
    pads: Vec<Arc<Vec<f32>>>,
    voices: Vec<(Arc<Vec<f32>>, usize)>,
}

fn index(side: Side) -> usize {
    match side {
        Side::A => 0,
        Side::B => 1,
    }
}

/// Multiplies `x` by a gain moving from `from` to `to` across the block (no zipper noise).
fn ramp(x: &mut [f32], from: f32, to: f32) {
    let n = (x.len() / 2).max(1) as f32;
    for (i, frame) in x.chunks_exact_mut(2).enumerate() {
        let g = from + (to - from) * (i + 1) as f32 / n;
        frame[0] *= g;
        frame[1] *= g;
    }
}

/// Untouched below `KNEE`, then a smooth bend that never reaches past 1.0.
fn limit(s: f32) -> f32 {
    let a = s.abs();
    if a <= KNEE {
        s
    } else {
        s.signum() * (KNEE + (1.0 - KNEE) * ((a - KNEE) / (1.0 - KNEE)).tanh())
    }
}

impl Mixer {
    pub fn new(rate: u32) -> Self {
        Self {
            rate,
            decks: [Deck::default(), Deck::default()],
            channels: [Channel::new(rate), Channel::new(rate)],
            crossfader: 0.0,
            cue_mix: 0.5,
            master_volume: 1.0,
            phones_volume: 1.0,
            last_out: (1.0, 1.0),
            scratch: [Vec::new(), Vec::new()],
            pads: (0..=super::pads::IMPACT).map(|p| Arc::new(super::pads::sound(p, rate))).collect(),
            voices: Vec::new(),
        }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    pub fn deck(&self, side: Side) -> &Deck {
        &self.decks[index(side)]
    }

    pub fn deck_mut(&mut self, side: Side) -> &mut Deck {
        &mut self.decks[index(side)]
    }

    /// Meter levels (peak, 0..1+) of deck A and B after their faders.
    pub fn levels(&self) -> [f32; 2] {
        [self.channels[0].level, self.channels[1].level]
    }

    /// Whether the echo and the filter sweep are on for a deck.
    pub fn effects(&self, side: Side) -> (bool, bool) {
        let ch = &self.channels[index(side)];
        (ch.echo.is_on(), ch.sweep.is_some())
    }

    pub fn crossfader(&self) -> f32 {
        self.crossfader
    }

    /// Applies a mixer or deck action. False for actions handled elsewhere (loading songs, Auto DJ,
    /// hot cues, loops, effects, pads).
    pub fn apply(&mut self, action: &Action) -> bool {
        match *action {
            Action::PlayPause { deck } => self.deck_mut(deck).play_pause(),
            Action::Cue { deck, pressed } => self.deck_mut(deck).cue(pressed),
            Action::Sync { deck } => {
                let (a, b) = self.decks.split_at_mut(1);
                let _ = match deck {
                    Side::A => a[0].sync_to(&b[0]),
                    Side::B => b[0].sync_to(&a[0]),
                };
            }
            Action::Tempo { deck, value } => self.deck_mut(deck).set_tempo(value as f64),
            Action::Nudge { deck, amount } => self.deck_mut(deck).set_bend(amount as f64),
            Action::Jog { deck, delta } => {
                let rate = self.rate;
                self.deck_mut(deck).jog(delta as f64, rate);
            }
            Action::Eq { deck, band, value } => {
                let ch = &mut self.channels[index(deck)];
                let b = match band {
                    Band::Low => 0,
                    Band::Mid => 1,
                    Band::High => 2,
                };
                ch.eq_values[b] = value.clamp(-1.0, 1.0);
                let [l, m, h] = ch.eq_values;
                ch.eq.set(l, m, h);
            }
            Action::Filter { deck, value } => {
                let ch = &mut self.channels[index(deck)];
                ch.filter_value = value.clamp(-1.0, 1.0);
                if ch.sweep.is_none() {
                    ch.filter.set(ch.filter_value);
                }
            }
            Action::HotCue { deck, slot } => self.deck_mut(deck).hot_cue(slot as usize),
            Action::ClearHotCue { deck, slot } => self.deck_mut(deck).clear_hot_cue(slot as usize),
            Action::Loop { deck, beats } => {
                let _ = self.deck_mut(deck).set_loop(beats);
            }
            Action::Effect { deck, effect, on } => {
                let ch = &mut self.channels[index(deck)];
                match effect {
                    Effect::Echo => ch.echo.set_on(on),
                    Effect::FilterSweep => {
                        ch.sweep = on.then_some(0.0);
                        if !on {
                            ch.filter.set(ch.filter_value);
                        }
                    }
                    Effect::SpinBack => {
                        if on {
                            self.deck_mut(deck).spin_back();
                        }
                    }
                    Effect::Reverb => ch.reverb.set_on(on),
                    Effect::Freeze => ch.reverb.set_freeze(on),
                    Effect::Brake => {
                        if on {
                            self.deck_mut(deck).brake();
                        }
                    }
                }
            }
            Action::Pad { slot } => {
                if let Some(sound) = self.pads.get(slot as usize) {
                    self.voices.push((sound.clone(), 0));
                }
            }
            Action::Volume { deck, value } => self.channels[index(deck)].volume = value.clamp(0.0, 1.0),
            Action::Gain { deck, value } => self.channels[index(deck)].gain = value.clamp(0.0, 1.0),
            Action::Crossfader { value } => self.crossfader = value.clamp(-1.0, 1.0),
            Action::HeadphoneCue { deck, on } => self.channels[index(deck)].cue = on,
            Action::CueMix { value } => self.cue_mix = value.clamp(0.0, 1.0),
            Action::MasterVolume { value } => self.master_volume = value.clamp(0.0, 1.0),
            Action::HeadphoneVolume { value } => self.phones_volume = value.clamp(0.0, 1.0),
            _ => return false,
        }
        true
    }

    /// Renders one block: the speakers into `master`, the headphones into `phones` (both stereo,
    /// same length, at `rate`).
    pub fn render(&mut self, master: &mut [f32], phones: &mut [f32]) {
        let n = master.len();
        master.fill(0.0);
        let mut cue = vec![0.0f32; n];
        let mut cued = false;
        let (xa, xb) = dsp::crossfader(self.crossfader);
        let block_s = (n / 2) as f32 / self.rate as f32;
        for i in 0..2 {
            let buf = &mut self.scratch[i];
            buf.resize(n, 0.0);
            self.decks[i].render(buf, self.rate);
            // Beat-timed effects follow the tempo heard now (or a steady 120 BPM without a beat grid).
            let beat_s = self.decks[i].bpm().map_or(0.5, |b| 60.0 / b) as f32;
            let ch = &mut self.channels[i];
            let trim = dsp::gain(ch.gain);
            ramp(buf, ch.last.0, trim);
            ch.eq.process(buf);
            if let Some(p) = ch.sweep.as_mut() {
                *p = (*p + block_s / (SWEEP_BEATS * beat_s)).min(1.0);
                let from = ch.filter_value.max(0.0);
                ch.filter.set(from + (SWEEP_TO - from) * *p);
            }
            ch.filter.process(buf);
            if ch.cue {
                cued = true;
                for (c, s) in cue.iter_mut().zip(buf.iter()) {
                    *c += s;
                }
            }
            let fader = dsp::fader(ch.volume);
            ramp(buf, ch.last.1, fader);
            let peak = buf.iter().fold(0.0f32, |p, s| p.max(s.abs()));
            ch.level = peak.max(ch.level * METER_FALL);
            let x = [xa, xb][i];
            ramp(buf, ch.last.2, x);
            for (m, s) in master.iter_mut().zip(buf.iter()) {
                *m += s;
            }
            // After the crossfader, so the repeats ring on when the deck is faded out.
            ch.echo.set_delay_s(ECHO_BEATS * beat_s);
            ch.echo.process(buf, master);
            ch.reverb.process(buf, master);
            ch.last = (trim, fader, x);
        }
        self.voices.retain_mut(|(sound, pos)| {
            for (frame, s) in master.chunks_exact_mut(2).zip(sound[*pos..].iter()) {
                frame[0] += s;
                frame[1] += s;
            }
            *pos += n / 2;
            *pos < sound.len()
        });
        let master_gain = dsp::fader(self.master_volume);
        let phones_gain = dsp::fader(self.phones_volume);
        let (last_master, last_phones) = self.last_out;
        // The headphones blend the cued decks with the master before the master volume.
        for (i, p) in phones.iter_mut().enumerate() {
            let c = if cued { cue[i] } else { master[i] };
            *p = c * (1.0 - self.cue_mix) + master[i] * self.cue_mix;
        }
        ramp(master, last_master, master_gain);
        ramp(phones, last_phones, phones_gain);
        self.last_out = (master_gain, phones_gain);
        for s in master.iter_mut().chain(phones.iter_mut()) {
            *s = limit(*s);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dj::actions::{Action, Band, Deck as D, Effect};
    use crate::dj::deck::Track;
    use std::sync::Arc;

    const RATE: u32 = 48_000;

    /// A steady tone (`hz`, amplitude `amp`) long enough for every test.
    fn tone(hz: f32, amp: f32) -> Arc<Track> {
        let pcm = (0..RATE as usize * 4)
            .flat_map(|i| {
                let v = ((2.0 * std::f32::consts::PI * hz * i as f32 / RATE as f32).sin() * amp * 32767.0) as i16;
                [v, v]
            })
            .collect();
        Arc::new(Track { song_id: 1, rate: RATE, pcm, analysis: None })
    }

    fn mixer(a: Arc<Track>, b: Arc<Track>) -> Mixer {
        let mut m = Mixer::new(RATE);
        m.deck_mut(D::A).load(a);
        m.deck_mut(D::B).load(b);
        m.apply(&Action::PlayPause { deck: D::A });
        m.apply(&Action::PlayPause { deck: D::B });
        m
    }

    /// Renders `secs`, returns the last block of master and headphones.
    fn run(m: &mut Mixer, secs: f32) -> (Vec<f32>, Vec<f32>) {
        let (mut master, mut phones) = (vec![0.0; 960], vec![0.0; 960]);
        for _ in 0..(secs * RATE as f32 / 480.0) as usize {
            m.render(&mut master, &mut phones);
        }
        (master, phones)
    }

    fn peak(x: &[f32]) -> f32 {
        x.iter().fold(0.0, |p, s| p.max(s.abs()))
    }

    #[test]
    fn the_crossfader_picks_the_deck_the_speakers_play() {
        let mut m = mixer(tone(440.0, 0.5), tone(440.0, 0.25));
        m.apply(&Action::Crossfader { value: -1.0 });
        let (master, _) = run(&mut m, 0.2);
        assert!((peak(&master) - 0.5).abs() < 0.02, "only deck A: {}", peak(&master));
        m.apply(&Action::Crossfader { value: 1.0 });
        let (master, _) = run(&mut m, 0.2);
        assert!((peak(&master) - 0.25).abs() < 0.02, "only deck B: {}", peak(&master));
    }

    #[test]
    fn headphones_hear_the_cued_deck_even_when_the_speakers_do_not() {
        let mut m = mixer(tone(440.0, 0.5), tone(440.0, 0.25));
        m.apply(&Action::Crossfader { value: -1.0 });
        m.apply(&Action::Volume { deck: D::B, value: 0.0 });
        m.apply(&Action::HeadphoneCue { deck: D::B, on: true });
        m.apply(&Action::CueMix { value: 0.0 });
        let (master, phones) = run(&mut m, 0.2);
        assert!((peak(&master) - 0.5).abs() < 0.02, "the speakers play A");
        assert!((peak(&phones) - 0.25).abs() < 0.02, "the headphones play B: {}", peak(&phones));
        m.apply(&Action::CueMix { value: 1.0 });
        let (master, phones) = run(&mut m, 0.2);
        assert!((peak(&phones) - peak(&master)).abs() < 0.02, "cue mix fully over is the master");
        m.apply(&Action::HeadphoneVolume { value: 0.0 });
        let (_, phones) = run(&mut m, 0.2);
        assert_eq!(peak(&phones), 0.0);
    }

    #[test]
    fn nothing_cued_means_the_master_in_the_headphones() {
        let mut m = mixer(tone(440.0, 0.5), tone(440.0, 0.25));
        m.apply(&Action::Crossfader { value: -1.0 });
        m.apply(&Action::CueMix { value: 0.0 });
        let (master, phones) = run(&mut m, 0.2);
        assert!((peak(&phones) - peak(&master)).abs() < 0.02);
    }

    #[test]
    fn eq_kill_and_filter_reach_the_master() {
        let mut m = mixer(tone(60.0, 0.5), tone(60.0, 0.5));
        m.apply(&Action::Crossfader { value: -1.0 });
        m.apply(&Action::Eq { deck: D::A, band: Band::Low, value: -1.0 });
        let (master, _) = run(&mut m, 0.5);
        assert!(peak(&master) < 0.05, "bass killed: {}", peak(&master));
        m.apply(&Action::Eq { deck: D::A, band: Band::Low, value: 0.0 });
        m.apply(&Action::Filter { deck: D::A, value: 0.9 });
        let (master, _) = run(&mut m, 0.5);
        assert!(peak(&master) < 0.05, "high-pass removes the bass: {}", peak(&master));
    }

    #[test]
    fn two_loud_songs_never_clip_the_master() {
        let mut m = mixer(tone(440.0, 1.0), tone(440.0, 1.0));
        m.apply(&Action::Crossfader { value: 0.0 });
        m.apply(&Action::Gain { deck: D::A, value: 1.0 });
        m.apply(&Action::Gain { deck: D::B, value: 1.0 });
        let (master, _) = run(&mut m, 0.2);
        assert!(peak(&master) <= 1.0);
        assert!(peak(&master) > 0.9);
    }

    #[test]
    fn faders_move_smoothly_without_a_click() {
        let mut m = mixer(tone(440.0, 0.0), tone(440.0, 0.0));
        // A constant signal shows the gain directly.
        let dc = Arc::new(Track { song_id: 2, rate: RATE, pcm: vec![16384; RATE as usize * 4], analysis: None });
        m.deck_mut(D::A).load(dc);
        m.apply(&Action::PlayPause { deck: D::A });
        m.apply(&Action::Crossfader { value: -1.0 });
        m.apply(&Action::Volume { deck: D::A, value: 0.0 });
        run(&mut m, 0.1);
        m.apply(&Action::Volume { deck: D::A, value: 1.0 });
        let (mut master, mut phones) = (vec![0.0; 960], vec![0.0; 960]);
        m.render(&mut master, &mut phones);
        assert!(master[0].abs() < 0.01, "starts where it was");
        assert!(master.windows(2).all(|w| (w[1] - w[0]).abs() < 0.01), "no jumps");
    }

    #[test]
    fn meters_show_each_deck_after_its_fader() {
        let mut m = mixer(tone(440.0, 0.5), tone(440.0, 0.5));
        m.apply(&Action::Volume { deck: D::B, value: 0.0 });
        run(&mut m, 0.5); // the meter falls back from the first block
        let [a, b] = m.levels();
        assert!((a - 0.5).abs() < 0.03, "{a}");
        assert!(b < 0.01, "{b}");
    }

    #[test]
    fn deck_actions_reach_the_decks() {
        let mut m = Mixer::new(RATE);
        let mut a = crate::dj::analysis::analyze(&[0.0; 100], RATE);
        a.bpm = 120.0;
        let t = Arc::new(Track { song_id: 3, rate: RATE, pcm: vec![0; RATE as usize * 20], analysis: Some(a) });
        m.deck_mut(D::A).load(t.clone());
        m.deck_mut(D::B).load(t);
        m.apply(&Action::Tempo { deck: D::A, value: 0.5 });
        assert!((m.deck(D::A).bpm().unwrap() - 124.8).abs() < 0.01);
        m.apply(&Action::Sync { deck: D::B });
        assert!((m.deck(D::B).bpm().unwrap() - 124.8).abs() < 0.01);
        m.apply(&Action::PlayPause { deck: D::B });
        assert!(m.deck(D::B).is_playing());
        m.apply(&Action::HotCue { deck: D::B, slot: 1 });
        assert!(m.deck(D::B).hot_cues()[1].is_some());
        m.apply(&Action::ClearHotCue { deck: D::B, slot: 1 });
        assert!(m.deck(D::B).hot_cues()[1].is_none());
        m.apply(&Action::Loop { deck: D::B, beats: Some(8) });
        assert!(m.deck(D::B).loop_s().is_some());
        m.apply(&Action::Effect { deck: D::B, effect: Effect::SpinBack, on: true });
        run(&mut m, 1.5);
        assert!(!m.deck(D::B).is_playing(), "spun back to a stop");
    }

    /// A 120 BPM song that is silent except for one click at 0.1 s.
    fn click() -> Arc<Track> {
        let mut pcm = vec![0i16; RATE as usize * 4 * 2];
        for s in &mut pcm[(RATE as usize / 10) * 2..(RATE as usize / 10 + 48) * 2] {
            *s = 20_000;
        }
        let mut a = crate::dj::analysis::analyze(&[0.0; 100], RATE);
        a.bpm = 120.0;
        Arc::new(Track { song_id: 9, rate: RATE, pcm, analysis: Some(a) })
    }

    /// Master output for `secs`, all of it.
    fn record(m: &mut Mixer, secs: f32) -> Vec<f32> {
        let mut all = Vec::new();
        let (mut master, mut phones) = (vec![0.0; 960], vec![0.0; 960]);
        for _ in 0..(secs * RATE as f32 / 480.0) as usize {
            m.render(&mut master, &mut phones);
            all.extend_from_slice(&master);
        }
        all
    }

    fn peak_between(x: &[f32], from_s: f32, to_s: f32) -> f32 {
        let i = |s: f32| (s * RATE as f32) as usize * 2;
        peak(&x[i(from_s)..i(to_s).min(x.len())])
    }

    #[test]
    fn echo_repeats_on_the_beat_and_rings_on_after_the_deck_is_faded_out() {
        let silent = Arc::new(Track { song_id: 2, rate: RATE, pcm: vec![0; RATE as usize * 8], analysis: None });
        let mut m = mixer(click(), silent);
        m.apply(&Action::Crossfader { value: -1.0 });
        let dry = record(&mut m, 1.0);
        assert!(peak_between(&dry, 0.3, 1.0) < 0.01, "no echo while it is off");

        let mut m = mixer(click(), Arc::new(Track { song_id: 2, rate: RATE, pcm: vec![0; RATE as usize * 8], analysis: None }));
        m.apply(&Action::Crossfader { value: -1.0 });
        m.apply(&Action::Effect { deck: D::A, effect: Effect::Echo, on: true });
        let start = record(&mut m, 0.2);
        // Echo off and the deck faded out: the repeats carry on and die away.
        m.apply(&Action::Effect { deck: D::A, effect: Effect::Echo, on: false });
        m.apply(&Action::Crossfader { value: 1.0 });
        let tail = [start, record(&mut m, 2.8)].concat();
        let beat = 0.375; // three quarters of a beat at 120 BPM
        let first = peak_between(&tail, 0.1 + beat - 0.02, 0.1 + beat + 0.03);
        let later = peak_between(&tail, 0.1 + 3.0 * beat - 0.02, 0.1 + 3.0 * beat + 0.03);
        assert!(first > 0.1, "the first repeat: {first}");
        assert!(later > 0.01 && later < first, "fading repeats: {later} < {first}");
        assert!(peak_between(&tail, 2.6, 3.0) < 0.02, "and gone");
    }

    #[test]
    fn a_reverb_wash_rings_on_after_the_deck_is_faded_out_and_the_brake_stops_it() {
        let silent = || Arc::new(Track { song_id: 2, rate: RATE, pcm: vec![0; RATE as usize * 8], analysis: None });
        let mut m = mixer(click(), silent());
        m.apply(&Action::Crossfader { value: -1.0 });
        m.apply(&Action::Effect { deck: D::A, effect: Effect::Reverb, on: true });
        record(&mut m, 0.2);
        m.apply(&Action::Effect { deck: D::A, effect: Effect::Reverb, on: false });
        m.apply(&Action::Crossfader { value: 1.0 });
        let tail = record(&mut m, 1.0);
        assert!(peak_between(&tail, 0.1, 0.5) > 0.01, "the wash rings on: {}", peak_between(&tail, 0.1, 0.5));
        m.apply(&Action::Effect { deck: D::A, effect: Effect::Brake, on: true });
        record(&mut m, 2.0);
        assert!(!m.deck(D::A).is_playing(), "braked to a stop");
    }

    #[test]
    fn the_filter_sweep_rises_over_two_bars_and_lets_go() {
        let mut low = tone(60.0, 0.5);
        Arc::get_mut(&mut low).unwrap().analysis = Some({
            let mut a = crate::dj::analysis::analyze(&[0.0; 100], RATE);
            a.bpm = 240.0; // two bars in 2 s
            a
        });
        let mut m = mixer(low, tone(60.0, 0.0));
        m.apply(&Action::Crossfader { value: -1.0 });
        m.apply(&Action::Effect { deck: D::A, effect: Effect::FilterSweep, on: true });
        let (early, _) = run(&mut m, 0.3);
        assert!(peak(&early) > 0.3, "starts open: {}", peak(&early));
        let (late, _) = run(&mut m, 1.9);
        assert!(peak(&late) < 0.05, "two bars later the bass is gone: {}", peak(&late));
        m.apply(&Action::Effect { deck: D::A, effect: Effect::FilterSweep, on: false });
        let (back, _) = run(&mut m, 0.3);
        assert!(peak(&back) > 0.4, "back to the knob: {}", peak(&back));
    }

    #[test]
    fn party_pads_play_over_the_mix() {
        let mut m = Mixer::new(RATE);
        assert_eq!(peak(&run(&mut m, 0.1).0), 0.0);
        m.apply(&Action::Pad { slot: 0 });
        let (now, phones) = run(&mut m, 0.1);
        assert!(peak(&now) > 0.3, "air horn");
        assert!(peak(&phones) > 0.3, "heard in the headphones with the master");
        run(&mut m, 3.0);
        assert_eq!(peak(&run(&mut m, 0.1).0), 0.0, "and over");
    }
}
