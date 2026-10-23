# DJ mode — spec and build plan

A party DJ for backyard parties, not a club rig: Miu mixes by itself by default (Auto DJ), and two
decks with a mixer, headphone cue and DJ controllers are there for people who want to play.
Agreed 2026-10-23. Built in phases; each phase ships on its own.

## Pieces and how they connect

```
library songs ──► analysis (once, background) ──► dj_analysis table
                                                     │
Auto DJ (picker + transition planner) ◄──────────────┘
        │ plans
        ▼
DJ actions ◄── DJ screen (mouse) ◄── keyboard shortcuts ◄── MIDI controllers (mappings, MIDI learn)
        │
        ▼
DJ engine: deck A + deck B ──► mixer (EQ, filter, gain, crossfader) ──► master output (speakers)
                                                            └─ cue mix ──► headphones (second output)
```

- **One vocabulary of DJ actions** (`dj::actions::Action`): play/pause/cue deck, sync, tempo,
  EQ low/mid/high, filter, gain, crossfader, cue on/off per deck, cue/master mix, hot cue set/jump,
  loop, effects, pads, load song to deck, Auto DJ on/off. The DJ screen, keyboard and controllers
  all send these; nothing else talks to the engine.
- **Analysis is pure Rust over decoded PCM** (`dj::analysis`), tested on synthetic audio; results
  are stored per song and shown as tempo, key (Camelot, e.g. 8A) and energy.
- **The DJ engine is separate from the normal player.** Entering DJ mode pauses the normal player
  and hands the audio devices to the DJ engine; leaving gives them back. It reuses the decoder,
  the shared-mode output and the per-device ring buffer + renderer from `audio::player`.

## Phase 1 — Auto DJ that sounds good (with headphone cue)

1. **Analysis** (`dj::analysis`), per song, mono at 11025 Hz:
   - onset strength (spectral flux) → tempo 70–180 BPM (autocorrelation with a preference around
     120, half/double resolved), beat phase → beat grid; downbeat = strongest beat of four;
     a confidence value (ballads and bolero often have soft or no drums).
   - key: chroma from an FFT, compared with major/minor key profiles → Camelot code.
   - energy 1–10 from loudness and onset density.
   - mix-in / mix-out points: first and last point where the music is at full body (skips silence,
     fade-ins, long fade-outs and spoken endings).
2. **Background analyser**: works through the library at low priority, saves to `dj_analysis`
   (keyed by song id + file size/mtime, re-analysed when the file changes), progress in DJ mode.
3. **DJ engine**: two decks (decoder, tempo by resampling ±8% — "vinyl mode", pitch moves a little;
   key lock comes later with a time-stretch library), per-deck 3-band EQ (kill-capable), one-knob
   low/high-pass filter, gain, VU meter; crossfader (smooth power curve); master to the main output;
   **headphone cue**: per-deck cue buttons and a cue/master blend to a second device (shared mode).
4. **Auto DJ**:
   - picker: next song by key compatibility (same, ±1, relative major/minor), tempo within ±8% or
     half/double, and the chosen energy curve (warm up, keep it going, build up, wind down);
     never repeats within the session; songs dragged into the queue win.
   - transition planner, chosen per pair:
     | from → to | transition |
     |---|---|
     | both beat-confident, tempo close | beat blend: sync tempo, align downbeats, 16-bar crossfade, bass swap halfway |
     | big tempo or key jump | echo out: filter sweep + echo tail on the outgoing, incoming drops on a downbeat |
     | either has weak beat confidence (ballad, bolero, acoustic) | smooth crossfade 6–10 s at the mix points |
     | high energy pop / hip hop | clean cut on the downbeat |
5. **Screens**: DJ mode (sidebar entry): waveforms on top, two decks, mixer, Auto DJ bar with the
   next transition, up-next with tempo/key/energy; **party screen**: big cover, visualizer, now/next,
   a guest lock.

## Phase 2 — hands-on

Hot cues (4 per deck), loops (4/8/16 beats, beat-snapped), effects (echo, filter sweep, spin-back),
party pads (short built-in sounds), keyboard map for DJ actions.

## Phase 3 — controllers

- MIDI input (`midir`), hot-plug; every MIDI input is listened to while DJ mode is open.
- **Built-in mappings for five popular party/entry controllers**, written from each maker's
  published MIDI list (not copied from other apps), in `src-tauri/src/dj/mappings/`:
  Pioneer DJ DDJ-FLX4, DDJ-FLX2 and DDJ-400 (AlphaTheta's lists), Hercules DJControl Inpulse 300
  and Inpulse 200 (Hercules' lists for the original models; the MK2s are expected to match).
  Jog wheels, shift layers, LEDs for play, cue, headphone cue, hot cues and loops.
  Master / headphone level and headphone mix are left to the controller's own sound card.
- **Numark Mixtrack Platinum FX and Party Mix Live** were in the first plan, but Numark publishes no
  MIDI list for them (checked 2026-10-23: user guides, product pages, support site). The only
  numbers around come from other apps' mapping files, so they are set up with MIDI learn instead.
- **MIDI learn** for any other controller: click a control in Miu, move a knob, saved per device
  (on top of a built-in mapping, or on its own).
- Mappings are data (JSON) so more can be added without code.
- Built and unit-tested without the hardware; each mapping is marked "not yet tried on the device"
  until someone with that controller checks it.

## Phase 4 — smarter transitions (agreed 2026-10-23)

- **Song structure** (`dj::structure`, analysis version 2; older analyses are redone in the
  background): 8-bar phrase grid, sections (intro, main, breakdown, build, drop, outro), the drop,
  where vocals probably are, the real end.
- **Transitions are scripts** (`dj::script`): ramps (crossfader, volume, EQ, filter, tempo) and
  instant moves (effects, loops, sync, key lock, key shift, stems, play, stop, risers and impacts)
  over the transition's progress. The new song takes over on a phrase.
- **Fifteen transitions:** beat blend (lows, mids, highs swapped in turn), filter blend, breakdown
  blend, loop out, drop swap, vocal out, key-shift blend, tempo ramp, echo out, reverb wash,
  spin-back, brake, crossfade, ballad fade-and-pause, cut.
- **Planner by music type**, in a chosen **style** (smooth, party, chill): no clear beat → ballad
  fade or crossfade; in tempo and key → drop swap (party), breakdown blend, loop out (no outro),
  one voice at a time (filter blend, or vocal out at a party), beat blend; key clash → key shift
  up to two semitones, else echo out / reverb wash; tempo gap up to 25 % → tempo ramp; far apart →
  cut, or spin-back / brake at a party. The next transition can also be picked by hand.
- **Key lock** (`dj::stretch`): our own WSOLA time-stretcher (no GPL/LGPL library); per-deck key
  lock and key shift.
- **Stems** (`dj::stems`): centre-channel vocal/instrumental split by frequency (no model to
  download, about 0.2 s a song); rougher than a trained separator, which could replace it later.

## Phase 5 — playlists, Auto style and Hype (agreed 2026-10-24)

- **Playlists** (`library/playlists.rs`, `src/js/playlists.js`): the user's own lists in their own
  order. In DJ mode, **Play from** picks a playlist; Auto DJ plays songs queued by hand first, then the
  playlist (in its order, or **Smart order**: warm up, peak about two-thirds in, come down a little,
  each next song the closest in energy that mixes well), and starts the playlist over when done.
- **Auto style** (the default): the playlist's kind of music (`autodj::vibe_of`: dance, hip-hop, pop,
  chill, from tempo, groove, energy and drops) picks each mix's feel; a ballad on either side always
  gets a gentle change. Real songs score 0.2–0.5 for beat clarity (synthetic beats near 1), so the
  thresholds are 0.2 (has a beat) and 0.3 (clear groove).
- **Hype** (`hype.rs`): moves inside the song on air while Auto DJ runs and no transition or video is
  under way. Chill: an echo throw at the end of a sung line about once a minute. Party: the big drop
  gets a 4-bar build (filter opens, riser), a half-beat roll in the last bar, a beat of silence and an
  impact. Festival: every drop gets an 8-bar build with rolls getting shorter, the fake-out and the
  impact; breakdowns are gated on the eighth notes; more echo throws. Nothing in the last 8 bars before
  the mix out. The mixer does the rise, the roll (a slice caught and repeated while the song runs on
  underneath), the gate and the mute per channel (`mixer::HypeFx`).
- **Party screen**: lights pulse on every beat (harder on the bar, colour by bar) and a countdown shows
  the bars left before a built-up drop, then DROP!. With Windows set to reduce motion the lights stay
  subtle and nothing flashes.

## Phase 6 — choruses, stage moves and the crowd (agreed 2026-10-24)

- **Choruses** (`structure::choruses`, analysis version 3): the harmony of every bar (chroma); the
  8-bar phrase that comes back most often (cosine 0.88 or more), loud and sung, is the chorus. Songs
  are analysed again in the background.
- **Sing-along and key lift**: on the last chorus (when the song has two or more) the singer drops
  out for 4 bars (8 at Festival) — the instrumental stem when the deck is split, else a cut in the
  voice range — and the party screen shows the lines big (synced lyrics, or "Sing it!"). Then the
  song comes back a semitone up until it leaves the deck.
- **Stage moves** (Party and up unless noted): a reverb throw on the last line into a breakdown; a
  tape-stop into the first breakdown (all of them at Festival); crashes on each new phrase inside a
  drop (with a snare fill at Festival); at Festival the big drop rewinds once to the build and hits
  again, then its best 4 bars play once more, and every other line end stutters.
- **Song length** (party edits): Full, Short (mix out after the second chorus, not before 90 s), Quick
  (after the first, not before 45 s), Auto (the default: full, Short at 6 guest songs waiting, Quick
  at 12).
- **Auto Hype**: follows the night from when DJ mode starts performing — Chill for 20 minutes, Party
  to the hour, Festival to two and a half hours, then Party; a level up while the guests are busy
  (adds and votes in the last ten minutes, 12 or more of 20). The menu shows the level playing.
- **Shout-outs**: when a guest's song starts the party screen says "Linh's pick!" for ten seconds.

## Testing

Test-first as everywhere in Miu: analysis on synthetic clicks/tones with known tempo and key, the
mixer and crossfader on synthetic buffers, the picker and planner on made-up song tables, MIDI
mappings on recorded message sequences. Machine tests (`#[ignore]`) for real devices.
