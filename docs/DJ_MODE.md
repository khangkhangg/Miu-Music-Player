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

- MIDI input (`midir`), hot-plug, one controller at a time.
- **Built-in mappings for five popular party/entry controllers**, written from each maker's
  published MIDI specification (not copied from other apps):
  Pioneer DJ DDJ-FLX4, Pioneer DJ DDJ-FLX2, Numark Mixtrack Platinum FX, Numark Party Mix Live,
  Hercules DJControl Inpulse 300 MK2. Jog wheels, LEDs where the spec allows.
- **MIDI learn** for any other controller: click a control in Miu, move a knob, saved per device.
- Mappings are data (JSON) so more can be added without code.
- Built and unit-tested without the hardware; each mapping is marked "not yet tried on the device"
  until someone with that controller checks it.

## Testing

Test-first as everywhere in Miu: analysis on synthetic clicks/tones with known tempo and key, the
mixer and crossfader on synthetic buffers, the picker and planner on made-up song tables, MIDI
mappings on recorded message sequences. Machine tests (`#[ignore]`) for real devices.
