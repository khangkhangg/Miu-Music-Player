# Miu Music — build checklist (TDD)

This is the single working checklist. Feature details live in `PLAN.md` §5, prompts in §6.
Tick a box only when its **Done when** is true and the full test suite is green.

## How we work (every item)

1. **Red** — write the test(s) listed under *Test first*. Run them and watch them fail for the
   right reason (feature missing, not a typo or build error).
2. **Green** — write the minimum code to pass. Run the **whole** suite, not just the new test.
3. **Refactor** — clean up with the suite green.
4. Commit (small, one item or sub-step per commit), tick the box, note anything learned.

Bug found later → first a failing test that reproduces it, then the fix.

### Test layers

| Layer | Tool | Runs | Notes |
|---|---|---|---|
| Rust unit/integration | `cargo test` | always | core logic, DB, conversion policy, sync diff, DSP |
| UI logic | `node --test` (built in, no deps) | always | pure modules in `src/js/` (no DOM); keep DOM code thin |
| Hardware (iPhone) | `cargo test -- --include-ignored --test-threads=1` | manually, phone plugged in | marked `#[ignore]`; results logged in `AIRTRAFFIC_NOTES.md` |
| Manual check | checklist in this file | per phase | only for things a test can't see (sound, look, "plays in Music app") |

Rules for tests:
- No real device DB or `research/` files in tests (private). Use a synthetic DB built from
  `MEDIALIBRARY_SCHEMA.md` under `tests/fixtures/`.
- Audio fixtures are tiny generated files (1 s sine, made with ffmpeg) committed under `tests/fixtures/audio/`.
- Anything that writes files works in a temp dir. Never touch the user's music in tests.
- Mocks only at the Apple-DLL boundary (`apple/` behind a trait) — everything above it uses real code.

Commands: `cargo test` (in `src-tauri/`), `node --test` (repo root), both must pass before a commit.

---

## Phase 0 – Setup and safety
- [x] **Toolchain** — Rust (rustup, MSVC target), VS Build Tools C++ workload, Tauri CLI, Python not required.
  *Done when:* `cargo --version`, `cargo tauri --version` work.
- [x] **F0.1 Skeleton + theme tokens**
  *Test first:* Rust — app builds and a `app_info` command returns name "Miu Music" + version;
  JS — `theme.resolve('system', prefersDark)` picks light/dark; tokens file defines every token for both themes.
  *Done when:* window opens with "No device connected" placeholder in light and dark.
- [x] **F0.2 Logging**
  *Test first:* log dir resolves to `%APPDATA%\MiuMusic\logs`; files rotate at the size limit; old files pruned.
- [x] **F0.3 Prerequisite check**
  *Test first:* `find_apple_dlls` with a fake directory tree (temp dir) finds classic, Program Files and
  Store-app layouts; returns a clear "missing" result when absent.
- [x] **F0.4 Local SQLite**
  *Test first:* migrations create schema from empty; re-running is a no-op; settings round-trip.
  Scope: migration runner + settings table. Songs (F3) and sync history (F4) are added as new migrations.
- [x] Research files copied into `research/db-baseline/` (gitignored, never in tests) — fresh F1.6 backup, 69 songs.
- [x] CI: GitHub Actions on `windows-latest` running `cargo test` + `node --test`.

## Phase 1 – DLL spike (gate)
- [x] **F2.1 DLL export map** — `apple/exports.rs` (`object` crate) + `cargo run --bin dll-exports`; required exports pinned and machine-tested.
  *Test first:* parses exports of a known system DLL (e.g. `kernel32.dll`) and finds `CreateFileW`.
- [x] **F1.1 DLL loader** — Store packages are copied to `%LOCALAPPDATA%\MiuMusic\apple\<package>` (Windows blocks loading from WindowsApps); classic installs load in place.
  *Test first:* search-path builder orders registry → Program Files → WindowsApps; missing DLL → typed error.
- [x] **F1.2 CoreFoundation bridge**
  *Test first (hardware-free once DLL present, else `#[ignore]`):* round-trip String (incl. Vietnamese/CJK),
  i64, f64, bool, Vec, HashMap, bytes through CF types; no leaks (retain counts back to 1).
- [x] **F1.3 Device discovery + hot-plug** — *Test first:* event mapping (connect/disconnect/untrusted) via a fake
  device source; `#[ignore]` test lists the real iPhone.
- [x] **F1.4 Device info** — *Test first:* parse lockdown values into `DeviceInfo` (model "iPhone6,1" → "iPhone 5s").
- [x] **F1.5 AFC list/pull/push** — *Test first:* path validation (stays inside media root); `#[ignore]` real list.
- [x] **F1.6 DB backup + restore + db diff**
  *Test first:* backup copies all three files into a timestamped dir and refuses to proceed if any copy fails;
  diff of two synthetic DBs reports added/changed rows per table.
- [x] **F2.2 ATC message logger** — *Test first:* message → JSON serialization; `#[ignore]` live capture.
- [x] **Gate decision:** stay Rust FFI / switch device layer to C# helper. → **Stay with Rust FFI** (user, 2026-10-20): every Phase 1 call works from Rust.

## Phase 2 – One-track sync (gate)

> **Paused 2026-10-20.** The phone requires a Grappa (FairPlay) session that Apple's AirTrafficHost only creates
> with a properly installed CoreFP. We don't reverse-engineer or bypass that, and Miu doesn't change Apple's system
> configuration (CLAUDE.md rule 10). Revisit with a standard Apple install, or decide on FB.2. Details:
> AIRTRAFFIC_NOTES.md. Work continues on the PC side (Phase 3+).
- [ ] **F2.3 Sync session handshake** — *Test first:* session state machine accepts the recorded message
  Status 2026-10-20: handshake built and run once; phone replies `SyncFailed {ErrorCode: 4}` (likely Grappa). Blocked.
  sequence from `research/captures` (replayed as a sanitized fixture) and rejects out-of-order messages.
- [ ] **F2.4 Track metadata** — *Test first:* `Track` → metadata dictionary matches the captured format.
- [ ] **F2.5 File transfer + progress** — *Test first:* progress events are monotonic and end at 100 %.
- [ ] **F2.6 Session finish + integrity check** — *Test first:* post-sync verifier flags a NULL `integrity`.
- [ ] **Manual:** song plays in the Music app on the iPhone 5s after unplug, library not reset.
- [ ] **F2.7 Delete tracks** — *Test first:* delete request built for given pids; `#[ignore]` live delete.
- [ ] **F2.8 Error recovery** — *Test first:* state machine handles unplug, locked, untrusted, cancel → clean state.
- [ ] **Gate decision:** proceed with A / run FB.1 / ship FB.2.

## Phase 3 – Library and conversion
- [x] **F3.1 Watched folders + drag-and-drop** — *Test first:* add/remove folders; watcher events
  (create/rename/delete in a temp dir) update the library; nested/duplicate folders rejected.
- [x] **F3.2 Content-based format detection** — *Test first:* a FLAC renamed `.mp3` is detected as FLAC.
  Done with file signatures in Rust (no ffprobe): MP3, AAC (ADTS + M4A), ALAC, FLAC, WAV, AIFF, Opus, Vorbis, WMA, APE, WavPack; non-audio content is skipped.
- [x] **F3.3 Tags + artwork** — *Test first:* fixtures with Vietnamese, CJK and RTL tags read exactly; artwork extracted.
  Done with `lofty`; tags stored on scan (migration 4), read only for new/changed files. Benchmark: 500 tagged files in 81 ms (release), unchanged rescan 29 ms.
- [ ] **F3.4 Conversion policy + Hi-res for iPhone** — *Test first:* table test format/bit depth/rate → action
  (keep / ALAC / AAC 256k / ALAC 24-48); hi-res setting respected.
- [ ] **F3.5 Conversion cache** — *Test first:* second conversion of the same audio is a cache hit; tag-only change re-tags, no re-encode.
- [ ] **F3.6 Duplicate detection** — *Test first:* same audio, different names → duplicate; different audio same tags → not.

## Phase 4 – Sync engine
- [ ] **F4.1 Sync plan diff** — *Test first:* local liked set vs device set → add / update metadata / remove.
- [ ] **F4.2 Liked-only sync set** — *Test first:* not rated and disliked never in plan; liked → disliked = remove.
- [ ] **F4.3 Queue, pause/resume/cancel** — *Test first:* resume continues from the next unfinished track.
- [ ] **F4.4 Free-space check** — *Test first:* plan larger than free space (minus margin) → blocked with sizes.
- [ ] **F4.5 Post-sync verification** — *Test first:* mismatch between plan and pulled DB is reported per track.
- [ ] **F4.6 Sync history** — *Test first:* each run stored with per-track status; re-run of unchanged library → empty plan.

## Phase 5 – UI
- [ ] **F5.1 Device card** — *Test first (JS):* storage bar percentages and labels from device info.
- [ ] **F5.2 Library view** — *Test first (JS):* virtual list window math (visible range for scroll offset).
- [ ] **F5.3 Sync button + plan preview** — *Test first (JS):* plan → "+42 add · 3 remove · 1.2 GB" text.
- [ ] **F5.4 Live progress + ETA** — *Test first (JS):* ETA from bytes/sec, smoothed, never negative.
- [ ] **F5.5 Settings** — *Test first:* settings schema defaults + validation.
- [ ] **F5.6 Error/empty states** — *Test first (JS):* each error code maps to a message and an action.
- [ ] **F5.7 Accessibility** — *Manual:* keyboard-only pass, AA contrast check in both themes.
- [ ] Idle RAM < 60 MB measured and recorded.

## Phase 5b – Player and mini player
- [x] **F7.1 Native audio engine** — *Test first:* decode each fixture format to the expected sample count and rate;
  gapless join has no inserted silence; seek lands within one packet.
  Status 2026-10-20: done and verified by listening — decode (symphonia, hi-res kept), sinc resampling to the
  device rate, gapless queue, seek, volume, WASAPI shared output. Exclusive (bit-perfect) mode done and verified
  2026-10-20: device opened at each file's own rate (Bit-perfect shown on the LG at 44.1/48 kHz); other apps silent
  while held, released on switching back. Gapless only between files at the same rate in exclusive mode.
- [x] **F7.2 Ratings + filter chips** — *Test first:* rating transitions and counts; filters return the right songs.
- [x] **F7.3 Disliked view + delete** — *Test first:* delete moves the file to Recycle Bin (temp-dir stub of the
  trash API), removes it from the library and queues device removal; undo dislike restores "not rated".
- [x] **F7.4 Mini player window** — *Test first:* open-on-minimize setting decides mini vs taskbar.
- [x] **F7.5 Always on top** — *Test first:* pin toggle and setting stay in sync and persist.
- [x] **F7.6 Visualizer** — *Test first:* FFT of a 1 kHz sine peaks in the 1 kHz band; no frames emitted while paused/hidden/off.
- [x] **F7.7 Equalizer + presets** — *Test first:* Flat = bit-identical output; +6 dB at 1 kHz raises a 1 kHz sine by
  6 dB ±0.5; clipping warning when preamp + max band > 0; user presets save/load.
- [x] **F7.8 Output panel + device picker** — *Test first:* signal-path label = "Bit-perfect" only when EQ off + exclusive.
  Device choice saved; switching keeps song, position, volume, EQ. (Shows shared mode until exclusive mode lands.)
- [ ] **Manual:** listening test with FLAC 16/44.1 and 24/96 through speakers and a USB DAC.

## Phase 5c – Languages and metadata editing
- [ ] **F8.1 Tag encoding detection** — *Test first:* CP1258, GBK, Shift-JIS, CP1251 fixtures decode correctly; folder override wins.
- [ ] **F8.2 Unicode-safe paths** — *Test first:* scan, convert and play a file in `Nhạc Trịnh/日本語/`.
- [ ] **F8.3 Font fallback + RTL** — *Manual:* Arabic/Hebrew titles render right-to-left; CJK has no tofu boxes.
- [ ] **F8.4 Search** — *Test first:* "mua thu" finds "Mùa thu"; "DIEM XUA" finds "Diễm xưa"; CJK substring.
- [ ] **F8.5 Sorting** — *Test first:* Vietnamese order (A Ă Â … D Đ E Ê …), index-bar letters per locale.
- [ ] **F8.6 UI translation** — *Test first (JS):* every key in `en.json` exists in `vi.json`; no hard-coded UI strings (lint test).
- [ ] **F9.1 Single-song editor** — *Test first:* edit → tags written → re-read equals edit, for MP3, M4A, FLAC.
- [ ] **F9.2 Bulk editor** — *Test first:* "Mixed" fields untouched; edited fields applied to all.
- [ ] **F9.3 Bulk tools** — *Test first:* filename pattern parse, auto-number, find/replace, case change.
- [ ] **F9.4 Preview, atomic write, undo** — *Test first:* crash mid-write leaves the original intact; undo restores old tags.
- [ ] **F9.5 Watcher + sync + cache** — *Test first:* own writes don't trigger rescans; edited liked song appears as "update" in the sync plan.

## Phase 6 – Packaging
- [ ] **F6.1 Installer + ffmpeg sidecar** — *Test first:* sidecar path resolution in dev and installed layouts.
- [ ] **F6.2 First-run wizard** — *Test first (JS):* wizard step logic (drivers → phone → folders).
- [ ] **F6.3 Crash-safe journal** — *Test first:* journal replay resumes or rolls back an interrupted sync.
- [ ] **F6.4 Diagnostics export** — *Test first:* export contains logs + device info and **no** `.sqlitedb`, account IDs or paths under `research/`.
- [ ] **Manual:** clean install on a fresh Windows 11 VM works end to end.

## Fallbacks (only if needed)
- [ ] FB.1 Approach B experiment
- [ ] FB.2 Approach C (player-app Documents)
