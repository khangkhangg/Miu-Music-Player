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

## Testing

Test-first as everywhere in Miu: analysis on synthetic clicks/tones with known tempo and key, the
mixer and crossfader on synthetic buffers, the picker and planner on made-up song tables, MIDI
mappings on recorded message sequences. Machine tests (`#[ignore]`) for real devices.
