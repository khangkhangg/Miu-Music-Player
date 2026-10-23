//! One DJ deck (D.2): a whole song in memory (16-bit stereo, so cue points, loops and scrubbing
//! are instant), played at a tempo of ±8 % by resampling ("vinyl mode": pitch moves with it).

use std::sync::Arc;

use super::analysis::Analysis;

/// How far the tempo control reaches either way.
pub const TEMPO_RANGE: f64 = 0.08;
/// One full turn of a jog wheel while paused moves this far (a record at 33 1/3 RPM).
const JOG_TURN_S: f64 = 1.8;
/// A jog turned while playing bends the speed for this long.
const JOG_BEND_S: f64 = 0.05;

/// A song decoded for a deck.
pub struct Track {
    pub song_id: i64,
    pub rate: u32,
    /// Interleaved stereo.
    pub pcm: Vec<i16>,
    pub analysis: Option<Analysis>,
}

impl Track {
    pub fn frames(&self) -> usize {
        self.pcm.len() / 2
    }

    pub fn duration_s(&self) -> f64 {
        self.frames() as f64 / self.rate as f64
    }

    /// The whole song in `bins` loudness levels (RMS, 0..255), for the waveform overview: peaks
    /// would draw today's loud masters as one solid bar.
    pub fn overview(&self, bins: usize) -> Vec<u8> {
        let frames = self.frames();
        (0..bins)
            .map(|b| {
                let s = &self.pcm[b * frames / bins * 2..((b + 1) * frames / bins).min(frames) * 2];
                if s.is_empty() {
                    return 0;
                }
                let rms = (s.iter().map(|&v| (v as f64) * (v as f64)).sum::<f64>() / s.len() as f64).sqrt();
                (rms * 255.0 / 32768.0).min(255.0) as u8
            })
            .collect()
    }

    /// The beat grid (tempo, downbeat), when the song has one.
    fn beat(&self) -> Option<(f64, f64)> {
        let a = self.analysis.as_ref()?;
        (a.bpm > 0.0).then_some((a.bpm as f64, a.downbeat_s))
    }
}

#[derive(Default)]
pub struct Deck {
    track: Option<Arc<Track>>,
    /// Play position in the song's own frames (fractional between samples).
    pos: f64,
    playing: bool,
    ended: bool,
    /// -1..1 of `TEMPO_RANGE`.
    tempo: f64,
    /// A temporary speed change held by the nudge buttons (-1..1 = ±4 %).
    bend: f64,
    /// A short speed change from the jog wheel: (factor, output frames left).
    jog_bend: (f64, usize),
    cue_point: f64,
    /// Cue held from a pause: the deck plays only while held.
    previewing: bool,
    /// Hot cue points (frames).
    hot: [Option<f64>; HOT_CUES],
    /// A loop playing between these frames.
    looping: Option<(f64, f64)>,
    /// Spin-back in progress: the current (negative) speed.
    spin: Option<f64>,
    /// Brake in progress: the share of normal speed left (1 down to 0).
    braking: Option<f64>,
}

pub const HOT_CUES: usize = 4;
/// Spin-back: starts at three times normal speed backwards and stops within this long.
const SPIN_SPEED: f64 = 3.0;
const SPIN_S: f64 = 1.0;
/// The brake brings the record to a stop in this long.
const BRAKE_S: f64 = 1.5;

impl Deck {
    /// Puts a song on the deck, paused at its first beat (the cue point).
    pub fn load(&mut self, track: Arc<Track>) {
        let start = track.analysis.as_ref().map_or(0.0, |a| a.first_beat_s.max(0.0)) * track.rate as f64;
        *self = Deck { track: Some(track), pos: start, cue_point: start, ..Deck::default() };
    }

    pub fn eject(&mut self) {
        *self = Deck::default();
    }

    pub fn track(&self) -> Option<&Arc<Track>> {
        self.track.as_ref()
    }

    pub fn play(&mut self) {
        if self.track.is_some() && !self.at_end() {
            self.playing = true;
            self.ended = false;
        }
        self.previewing = false;
    }

    pub fn pause(&mut self) {
        self.playing = false;
        self.previewing = false;
    }

    pub fn play_pause(&mut self) {
        if self.playing {
            self.pause()
        } else {
            self.play()
        }
    }

    pub fn is_playing(&self) -> bool {
        self.playing
    }

    /// The song played to its end (cleared by playing or moving it again).
    pub fn ended(&self) -> bool {
        self.ended
    }

    fn at_end(&self) -> bool {
        self.track.as_ref().is_some_and(|t| self.pos >= t.frames() as f64)
    }

    fn rate(&self) -> f64 {
        self.track.as_ref().map_or(1.0, |t| t.rate as f64)
    }

    pub fn position_s(&self) -> f64 {
        self.pos / self.rate()
    }

    pub fn seek_s(&mut self, s: f64) {
        let end = self.track.as_ref().map_or(0, |t| t.frames()) as f64;
        self.pos = (s * self.rate()).clamp(0.0, end);
        self.ended = false;
    }

    pub fn set_tempo(&mut self, value: f64) {
        self.tempo = value.clamp(-1.0, 1.0);
    }

    pub fn tempo(&self) -> f64 {
        self.tempo
    }

    pub fn set_bend(&mut self, amount: f64) {
        self.bend = amount.clamp(-1.0, 1.0);
    }

    /// Speed against the song's own, from the tempo control (what the BPM display follows).
    pub fn speed(&self) -> f64 {
        1.0 + self.tempo * TEMPO_RANGE
    }

    /// The tempo heard now, if the song has a beat.
    pub fn bpm(&self) -> Option<f64> {
        let (bpm, _) = self.track.as_ref()?.beat()?;
        Some(bpm * self.speed())
    }

    /// Where in its bar of four beats the deck is: 0 on the downbeat .. just under 4.
    pub fn bar_phase(&self) -> Option<f64> {
        let (bpm, downbeat) = self.track.as_ref()?.beat()?;
        Some(((self.position_s() - downbeat) * bpm / 60.0).rem_euclid(4.0))
    }

    /// Cue button. Paused: the cue point is set here and the deck plays while it is held. Playing:
    /// back to the cue point, paused. Letting go after a preview goes back to the cue point.
    pub fn cue(&mut self, pressed: bool) {
        if self.track.is_none() {
            return;
        }
        if pressed {
            if self.playing {
                self.playing = false;
                self.pos = self.cue_point;
            } else {
                self.cue_point = self.pos;
                self.playing = true;
                self.previewing = true;
            }
            self.ended = false;
        } else if self.previewing {
            self.previewing = false;
            self.playing = false;
            self.pos = self.cue_point;
        }
    }

    /// Jog wheel turned by `delta` of a full turn: scrubs while paused, bends the speed briefly while playing.
    pub fn jog(&mut self, delta: f64, out_rate: u32) {
        if self.playing {
            self.jog_bend = (1.0 + (delta * 4.0).clamp(-0.3, 0.3), (JOG_BEND_S * out_rate as f64) as usize);
        } else {
            self.seek_s(self.position_s() + delta * JOG_TURN_S);
        }
    }

    /// The beat at or before (`nearest`: closest to) `s`, when the song has a beat grid.
    fn snap(&self, s: f64, nearest: bool) -> Option<f64> {
        let (bpm, downbeat) = self.track.as_ref()?.beat()?;
        let beat = 60.0 / bpm;
        let n = (s - downbeat) / beat;
        let n = if nearest { n.round() } else { (n + 1e-9).floor() };
        Some((downbeat + n * beat).max(0.0))
    }

    /// Hot cue button: jump to it (playing on if playing), or set it here on the nearest beat.
    pub fn hot_cue(&mut self, slot: usize) {
        if self.track.is_none() || slot >= HOT_CUES {
            return;
        }
        match self.hot[slot] {
            Some(p) => {
                self.pos = p;
                self.ended = false;
            }
            None => {
                let s = self.position_s();
                self.hot[slot] = Some(self.snap(s, true).unwrap_or(s) * self.rate());
            }
        }
    }

    pub fn clear_hot_cue(&mut self, slot: usize) {
        if slot < HOT_CUES {
            self.hot[slot] = None;
        }
    }

    /// Hot cue points in seconds.
    pub fn hot_cues(&self) -> [Option<f64>; HOT_CUES] {
        self.hot.map(|h| h.map(|p| p / self.rate()))
    }

    /// Loop `beats` whole beats from the beat at or before here, or end the loop (`None`).
    /// False when the song has no beat grid.
    pub fn set_loop(&mut self, beats: Option<u8>) -> bool {
        let Some(beats) = beats else {
            self.looping = None;
            return true;
        };
        let (Some(start), Some((bpm, _))) = (self.snap(self.position_s(), false), self.track.as_ref().and_then(|t| t.beat())) else {
            return false;
        };
        let end = start + beats as f64 * 60.0 / bpm;
        self.looping = Some((start * self.rate(), end * self.rate()));
        true
    }

    pub fn loop_s(&self) -> Option<(f64, f64)> {
        self.looping.map(|(a, b)| (a / self.rate(), b / self.rate()))
    }

    /// The record slows to a stop, like a turntable switched off.
    pub fn brake(&mut self) {
        if self.playing {
            self.braking = Some(1.0);
            self.looping = None;
        }
    }

    /// The record is spun back by hand and stops (a party trick to end a song).
    pub fn spin_back(&mut self) {
        if self.playing {
            self.spin = Some(-SPIN_SPEED);
            self.looping = None;
        }
    }

    /// Match `other`'s tempo (counting half and double time) and line this deck's bars up with
    /// its bars. False, with nothing changed, when either has no beat or the tempo is out of reach.
    pub fn sync_to(&mut self, other: &Deck) -> bool {
        let (Some((mine, downbeat)), Some(target), Some(their_phase)) =
            (self.track.as_ref().and_then(|t| t.beat()), other.bpm(), other.bar_phase())
        else {
            return false;
        };
        // How many of their beats one of ours counts for; the smallest tempo change wins.
        let Some((per, speed)) = [1.0, 2.0, 0.5]
            .into_iter()
            .map(|per| (per, target / (mine * per)))
            .filter(|(_, speed)| (speed - 1.0).abs() <= TEMPO_RANGE + 1e-9)
            .min_by(|a, b| (a.1 - 1.0).abs().total_cmp(&(b.1 - 1.0).abs()))
        else {
            return false;
        };
        self.tempo = ((speed - 1.0) / TEMPO_RANGE).clamp(-1.0, 1.0);
        // Our position counted in their beats, then the shortest move (±2 beats) onto their phase.
        let beat_s = 60.0 / mine;
        let ours = (self.position_s() - downbeat) / beat_s * per;
        let shift = (their_phase - ours).rem_euclid(4.0);
        let shift = if shift > 2.0 { shift - 4.0 } else { shift };
        let s = self.position_s() + shift / per * beat_s;
        // Before the start of the song: a bar later instead.
        let s = if s < 0.0 { s + 4.0 / per * beat_s } else { s };
        self.seek_s(s);
        true
    }

    /// This deck's audio, stereo at `out_rate`, written over `out` (silence when stopped).
    pub fn render(&mut self, out: &mut [f32], out_rate: u32) {
        let Some(track) = self.track.clone().filter(|_| self.playing) else {
            out.fill(0.0);
            return;
        };
        let frames = track.frames();
        let base = track.rate as f64 / out_rate as f64 * self.speed() * (1.0 + self.bend * 0.04);
        let pcm = &track.pcm;
        let sample = |i: isize, ch: usize| -> f32 {
            let i = i.clamp(0, frames as isize - 1) as usize;
            pcm[i * 2 + ch] as f32 / 32768.0
        };
        for frame in out.chunks_exact_mut(2) {
            if !self.playing || self.pos >= frames as f64 {
                frame.fill(0.0);
                if self.playing {
                    self.playing = false;
                    self.previewing = false;
                    self.ended = true;
                    self.pos = frames as f64;
                }
                continue;
            }
            let i = self.pos.floor() as isize;
            let t = (self.pos - i as f64) as f32;
            for (ch, o) in frame.iter_mut().enumerate() {
                *o = if t == 0.0 {
                    sample(i, ch)
                } else {
                    hermite(sample(i - 1, ch), sample(i, ch), sample(i + 1, ch), sample(i + 2, ch), t)
                };
            }
            let mut step = base;
            if self.jog_bend.1 > 0 {
                step *= self.jog_bend.0;
                self.jog_bend.1 -= 1;
            }
            if let Some(left) = self.braking.as_mut() {
                step *= *left;
                *left -= 1.0 / (BRAKE_S * out_rate as f64);
                if *left <= 0.0 {
                    self.braking = None;
                    self.playing = false;
                }
            }
            if let Some(spin) = self.spin.as_mut() {
                // Backwards, slowing down to a stop.
                step = track.rate as f64 / out_rate as f64 * *spin;
                *spin += SPIN_SPEED / (SPIN_S * out_rate as f64);
                if *spin >= 0.0 {
                    self.spin = None;
                    self.playing = false;
                }
            }
            self.pos = (self.pos + step).max(0.0);
            if let Some((start, end)) = self.looping {
                if self.pos >= end {
                    self.pos -= end - start;
                }
            }
        }
    }
}

/// 4-point, 3rd-order Hermite interpolation between `b` and `c`.
#[inline]
fn hermite(a: f32, b: f32, c: f32, d: f32, t: f32) -> f32 {
    let c1 = 0.5 * (c - a);
    let c2 = a - 2.5 * b + 2.0 * c - 0.5 * d;
    let c3 = 0.5 * (d - a) + 1.5 * (b - c);
    ((c3 * t + c2) * t + c1) * t + b
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dj::analysis::Analysis;
    use std::sync::Arc;

    const OUT: u32 = 48_000;

    fn analysis(bpm: f32, first_beat_s: f64, downbeat_s: f64) -> Analysis {
        Analysis { bpm, beat_confidence: 0.8, first_beat_s, downbeat_s, key: None, energy: 5, mix_in_s: 0.0, mix_out_s: 0.0, duration_s: 0.0, ..Default::default() }
    }

    /// `secs` of a slow ramp at `rate`, both channels equal.
    fn track(rate: u32, secs: f64, a: Option<Analysis>) -> Arc<Track> {
        let n = (rate as f64 * secs) as usize;
        let pcm = (0..n).flat_map(|i| {
            let v = ((i % 20_000) as i32 - 10_000) as i16;
            [v, v]
        }).collect();
        Arc::new(Track { song_id: 1, rate, pcm, analysis: a })
    }

    fn render_secs(deck: &mut Deck, secs: f64) -> Vec<f32> {
        let mut out = vec![0.0; (OUT as f64 * secs) as usize * 2];
        for block in out.chunks_mut(960) {
            deck.render(block, OUT);
        }
        out
    }

    #[test]
    fn plays_the_samples_untouched_at_its_own_speed() {
        let t = track(OUT, 1.0, None);
        let mut d = Deck::default();
        d.load(t.clone());
        d.play();
        let out = render_secs(&mut d, 0.5);
        for (i, s) in out.iter().enumerate().step_by(97) {
            assert!((s - t.pcm[i] as f32 / 32768.0).abs() < 1e-6, "sample {i}");
        }
        assert!((d.position_s() - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_song_at_another_rate_keeps_its_speed() {
        let mut d = Deck::default();
        d.load(track(44_100, 3.0, None));
        d.play();
        render_secs(&mut d, 1.0);
        assert!((d.position_s() - 1.0).abs() < 1e-3);
    }

    #[test]
    fn tempo_speeds_up_by_up_to_eight_percent() {
        let mut d = Deck::default();
        d.load(track(44_100, 3.0, Some(analysis(120.0, 0.0, 0.0))));
        d.set_tempo(1.0);
        d.play();
        render_secs(&mut d, 1.0);
        assert!((d.position_s() - 1.08).abs() < 1e-3);
        assert!((d.bpm().unwrap() - 129.6).abs() < 0.01);
        d.set_tempo(-0.5);
        assert!((d.bpm().unwrap() - 115.2).abs() < 0.01);
    }

    #[test]
    fn stops_at_the_end_with_silence() {
        let mut d = Deck::default();
        d.load(track(OUT, 0.25, None));
        d.play();
        let out = render_secs(&mut d, 0.5);
        assert!(!d.is_playing());
        assert!(d.ended());
        assert!(out[out.len() / 2 + 2000..].iter().all(|s| *s == 0.0));
    }

    #[test]
    fn cue_previews_from_the_cue_point_and_returns_to_it() {
        let mut d = Deck::default();
        d.load(track(OUT, 30.0, None));
        d.seek_s(5.0);
        d.cue(true); // paused: sets the cue point here and previews
        assert!(d.is_playing());
        render_secs(&mut d, 0.5);
        d.cue(false); // let go: back to the cue point, paused
        assert!(!d.is_playing());
        assert!((d.position_s() - 5.0).abs() < 1e-6);
        d.play();
        render_secs(&mut d, 1.0);
        d.cue(true); // playing: back to the cue point and stop
        assert!(!d.is_playing());
        assert!((d.position_s() - 5.0).abs() < 1e-6);
    }

    #[test]
    fn sync_matches_tempo_and_lines_up_the_bars() {
        let mut a = Deck::default();
        a.load(track(OUT, 60.0, Some(analysis(120.0, 0.0, 0.0))));
        a.seek_s(10.25); // 120 BPM from a downbeat at 0: half a beat into a bar
        let mut b = Deck::default();
        b.load(track(OUT, 60.0, Some(analysis(125.0, 0.1, 0.58))));
        b.seek_s(3.0);
        assert!(b.sync_to(&a));
        assert!((b.bpm().unwrap() - 120.0).abs() < 0.01);
        assert!((b.bar_phase().unwrap() - a.bar_phase().unwrap()).abs() < 1e-3, "{:?} vs {:?}", b.bar_phase(), a.bar_phase());
        assert!((b.position_s() - 3.0).abs() < 1.0, "the jump stays within two beats");
    }

    #[test]
    fn hot_cues_are_set_on_the_beat_and_jumped_to() {
        let mut d = Deck::default();
        d.load(track(OUT, 60.0, Some(analysis(120.0, 0.0, 0.0)))); // a beat every 0.5 s
        d.seek_s(10.2);
        d.hot_cue(0); // empty: set it, snapped to the nearest beat
        assert_eq!(d.hot_cues()[0], Some(10.0));
        d.seek_s(30.0);
        d.play();
        d.hot_cue(0); // set: jump there and keep playing
        assert!((d.position_s() - 10.0).abs() < 1e-9);
        assert!(d.is_playing());
        d.clear_hot_cue(0);
        assert_eq!(d.hot_cues()[0], None);
        let mut plain = Deck::default();
        plain.load(track(OUT, 60.0, None));
        plain.seek_s(10.2);
        plain.hot_cue(3);
        assert!((plain.hot_cues()[3].unwrap() - 10.2).abs() < 1e-9, "no beat grid: exactly where it is");
    }

    #[test]
    fn a_loop_repeats_whole_beats_until_released() {
        let mut d = Deck::default();
        d.load(track(OUT, 60.0, Some(analysis(120.0, 0.0, 0.0))));
        d.seek_s(10.1);
        d.play();
        assert!(d.set_loop(Some(4))); // from the beat at 10.0 for 2 s
        assert_eq!(d.loop_s(), Some((10.0, 12.0)));
        render_secs(&mut d, 5.0);
        let p = d.position_s();
        assert!((10.0..12.0).contains(&p), "still inside the loop: {p}");
        d.set_loop(None);
        render_secs(&mut d, 3.0);
        assert!(d.position_s() > 12.0, "released: plays on");
        let mut plain = Deck::default();
        plain.load(track(OUT, 60.0, None));
        assert!(!plain.set_loop(Some(4)), "no beat grid, no beat loop");
    }

    #[test]
    fn the_brake_slows_the_record_to_a_stop() {
        let mut d = Deck::default();
        d.load(track(OUT, 60.0, None));
        d.seek_s(20.0);
        d.play();
        d.brake();
        render_secs(&mut d, 2.0);
        assert!(!d.is_playing(), "stopped");
        let p = d.position_s();
        assert!(p > 20.4 && p < 21.2, "slowed down over about a second and a half: {p}");
    }

    #[test]
    fn spin_back_winds_the_record_back_and_stops() {
        let mut d = Deck::default();
        d.load(track(OUT, 60.0, None));
        d.seek_s(20.0);
        d.play();
        d.spin_back();
        render_secs(&mut d, 2.0);
        assert!(!d.is_playing(), "stopped at the end of the spin");
        assert!(d.position_s() < 19.0 && d.position_s() > 15.0, "{}", d.position_s());
    }

    #[test]
    fn the_overview_shows_where_the_song_is_loud() {
        let mut pcm = vec![0i16; 2000 * 2];
        for s in &mut pcm[2000..] {
            *s = 16384; // the second half at half scale
        }
        let t = Track { song_id: 1, rate: 1000, pcm, analysis: None };
        let o = t.overview(4);
        assert_eq!(o.len(), 4);
        assert_eq!(&o[..2], &[0, 0]);
        assert!(o[2] > 120 && o[2] < 135 && o[3] == o[2], "{o:?}");
        assert!(Track { song_id: 1, rate: 1000, pcm: vec![], analysis: None }.overview(4).iter().all(|v| *v == 0));
        // Loudness, not peaks: one full-scale click in a quiet bin barely shows.
        let mut pcm = vec![0i16; 4000 * 2];
        pcm[100] = i16::MAX;
        let t = Track { song_id: 1, rate: 1000, pcm, analysis: None };
        assert!(t.overview(4)[0] < 20, "{:?}", t.overview(4));
    }

    #[test]
    fn sync_counts_double_time_and_refuses_what_is_out_of_range() {
        let mut a = Deck::default();
        a.load(track(OUT, 10.0, Some(analysis(140.0, 0.0, 0.0))));
        let mut b = Deck::default();
        b.load(track(OUT, 10.0, Some(analysis(71.0, 0.0, 0.0))));
        assert!(b.sync_to(&a), "71 is half of 142, close to 140");
        assert!((b.bpm().unwrap() * 2.0 - 140.0).abs() < 0.01);
        let mut c = Deck::default();
        c.load(track(OUT, 10.0, Some(analysis(100.0, 0.0, 0.0))));
        c.set_tempo(0.3);
        assert!(!c.sync_to(&a), "100 cannot reach 140 within 8 %");
        assert!((c.tempo() - 0.3).abs() < 1e-6, "nothing changed");
    }
}
