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
- [x] **F3.4 Conversion policy + Hi-res for iPhone** — *Test first:* table test format/bit depth/rate → action
  (keep / ALAC / AAC 256k / ALAC 24-48); hi-res setting respected. Ticked on tests 2026-10-20: no UI yet (the
  sync engine uses it in Phase 4, the setting appears in F5.5). 88.2/176.4 kHz sources go to 44.1, not 48.
- [x] **F3.7 Find missing album covers** — *Test first:* iTunes Search then MusicBrainz + Cover Art Archive (no keys), accent-insensitive
  matching that ignores "(1984) [Remastered] - Single"-style suffixes; covers saved under %APPDATA%MiuMusiccovers (music files untouched);
  misses remembered for 30 days (migration 13); album tags that are website names ("NhacCuaTui.Com") skipped; fetched covers show in
  lists and pages and go onto converted iPhone copies. "Find artwork" on album pages, "Find missing covers" in Settings (stoppable).
  Built 2026-10-20; live test: Khánh Ly "Diễm Xưa" found on iTunes (600×600). In this library 1 real album lacks a cover.
- [x] **F3.5 Conversion cache** — *Test first:* second conversion of the same audio is a cache hit; tag-only change re-tags, no re-encode.
  Built 2026-10-20 (`library::transcode`): ffmpeg args per action (ALAC 16 → s16p, 24 → s32p + 24 bits; AAC 256k; `-f ipod`, tags
  mapped, artwork re-added by lofty), cache in `%LOCALAPPDATA%\MiuMusic\cache` keyed by audio hash + action, `.tags` key for re-tags,
  failed encodes leave nothing. Machine test: 24/96 FLAC → ALAC 24/48 with tags and cover. `convert-liked`: 51 liked FLACs → ALAC
  16/44.1 in 96 s (1.5 GB), second run all hits in 0.1 s. No cache size limit yet.
- [x] **F3.6 Duplicate detection** — *Test first:* same audio, different names → duplicate; different audio same tags → not.
  Verified in the app 2026-10-20 (Duplicates chip, path tooltip). Fingerprint = first 30 s at 16-bit + length within 1 s.

## Phase 4 – Sync engine
- [ ] **F4.1 Sync plan diff** — *Test first:* local liked set vs device set → add / update metadata / remove.
  Logic built 2026-10-20 (sync/plan.rs, tested): add / update / replace / remove per phone; only Miu-added tracks are ever removed. Sending to the phone waits on Phase 2.
- [ ] **F4.2 Liked-only sync set** — *Test first:* not rated and disliked never in plan; liked → disliked = remove.
  Logic built 2026-10-20 (tested in sync/plan.rs).
- [ ] **F4.3 Queue, pause/resume/cancel** — *Test first:* resume continues from the next unfinished track.
  Logic built 2026-10-20 (sync/queue.rs, tested). Wiring to a real transfer waits on Phase 2.
- [ ] **F4.4 Free-space check** — *Test first:* plan larger than free space (minus margin) → blocked with sizes.
  Logic built 2026-10-20; the device card shows a space warning in the plan preview.
- [ ] **F4.5 Post-sync verification** — *Test first:* mismatch between plan and pulled DB is reported per track.
  Logic built 2026-10-20 (sync/verify.rs reads a pulled MediaLibrary.sqlitedb).
- [ ] **F4.6 Sync history** — *Test first:* each run stored with per-track status; re-run of unchanged library → empty plan.
  Logic built 2026-10-20 (synced / sync_runs / sync_items, migration 8).

## Phase 4b – Sync through iTunes (spec option A)
Decided 2026-10-20 from the user's sync spec (static analysis of iTunes 12139): our own ATC engine
(option B) stays blocked by Grappa and the `.cig` plan signature, which must not be lifted or
forged (CLAUDE.md rules; the spec's legal note). Option A lets Apple's iTunes do authentication and
signing: Miu only drives iTunes through its documented COM automation interface.
- [x] **S.1 COM probe** — *Machine test (#[ignore]):* connect to `iTunes.Application`, list sources, find the iPhone source. Read-only.
  2026-10-20: COM works by CLSID (packaged server, not in HKCR). With the 5s on USB iTunes lists it: 'khang’s iPhone', kind 2, 29.8 GB (23.3 free), 69 tracks, UpdateIPod available.
- [x] **S.1b Sync safeguards** — *Test first:* read the phone's tracks from a backup; find phone songs with no PC copy (size, or title
  and length within 2 s) and copy them off read-only; before a sync flag phone songs missing from iTunes' view and IDs with a leading
  zero byte; after a sync list songs added and removed. Tool: `sync-guard before` / `after` (iTunes' view read over COM since S.2).
  Built 2026-10-20; first run on the 5s: 65 phone-only songs copied to `backups\e51e30a5\media` (770 MB), iTunes' view matches all
  69, `after` with no sync reports no change.
- [x] **S.2 Mirror liked songs through iTunes (manual management)** — *Test first:* against a fake iTunes, the plan's steps become
  adds (library AddFile, then device AddTrack), removals (device track Delete; already gone counts as done), and replace / tag
  update as remove + add; songs needing conversion fail with a clear reason until ffmpeg is set up; liked songs already on the phone
  (3uTools) are adopted (migration 12) — not sent twice, never removed by Miu. Real iTunes: `apple::itunes_com` (late-bound COM,
  persistent ID = phone sync_id). Built 2026-10-20; machine test reads the 5s's 69 tracks with IDs matching the phone. Nothing sent yet.
- [x] **S.3 Guarded sync from the Sync button** — *Test first (fakes for phone and iTunes):* backup → copy off phone-only songs →
  stop if iTunes' view misses a phone song (nothing changed) → adopt → plan → apply through iTunes → `UpdateIPod` only when something
  changed → wait until the phone shows the changes (unconfirmed after 5 min is reported) → backup, diff (unplanned removals reported) →
  record the run. Leading-zero IDs only warn (a copy is kept). `sync_now` runs on its own DB connection with `sync-progress` events.
  Built 2026-10-20. Conversion is in (F3.5): liked FLACs go as cached ALAC copies; a first run sends the 51 liked FLACs (~1.4 GB).
- [x] **S.4 First-time guide** — iTunes' COM has no "Manually manage music" flag, so: the sync box shows a three-step guide until the
  phone's first sync, and when iTunes refuses every new song the result says to turn the setting on (tested with a fake). Built 2026-10-20.
- [x] **S.5 Verify** — every sync checks which songs Miu sent (or adopted) are still on the phone; ones deleted there are forgotten
  and sent again (tested). The sync box shows the last sync ("Last sync: 10 min ago · 51 songs"). Built 2026-10-20.
- [x] **Manual:** liked songs play in the iPhone 5s Music app after an iTunes-driven sync. Verified 2026-10-20: 54 songs (3 MP3, 51 ALAC) play with titles and covers.

## Phase 5 – UI
- [x] **F5.1 Device card** — *Test first (JS):* storage bar percentages and labels from device info.
  Done by F1.3/F1.4 (device.test.js), verified then.
- [x] **F5.2 Library view** — *Test first (JS):* virtual list window math (visible range for scroll offset).
  Done by the F5.2 song list (visibleRange in songs.test.js), plus search, sorting, index bar and Format column.
- [ ] **F5.3 Sync button + plan preview** — *Test first (JS):* plan → "+42 add · 3 remove · 1.2 GB" text.
  Built 2026-10-20, tests green, **waiting for your check in the app**: the device card shows "+N add · N remove · size" for a trusted iPhone; the Sync button stays disabled (Phase 2).
- [ ] **F5.4 Live progress + ETA** — *Test first (JS):* ETA from bytes/sec, smoothed, never negative.
  Text helpers built 2026-10-20 (sync.js); live progress needs a real transfer (Phase 2).
- [ ] **F5.5 Settings** — *Test first:* settings schema defaults + validation.
  Built 2026-10-20, tests green, **waiting for your check in the app**: sidebar → Settings (theme, language, mini player, visualizer, iPhone conversion, exclusive mode, diagnostics).
- [ ] **F5.6 Error/empty states** — *Test first (JS):* each error code maps to a message and an action.
  Built 2026-10-20, tests green, **waiting for your check in the app**: the driver notice has an "Open Microsoft Store" button; the other messages are ready for the states that use them.
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
- [x] **F7.2b Always sync an artist or album** — *Test first:* a rule likes every not-rated song of the artist
  (or album + album artist, as the Albums view groups them) now and after later scans or tag edits; disliked songs stay
  disliked; removing the rule keeps existing likes. Verified in the app 2026-10-20.
- [x] **F7.3 Disliked view + delete** — *Test first:* delete moves the file to Recycle Bin (temp-dir stub of the
  trash API), removes it from the library and queues device removal; undo dislike restores "not rated".
- [x] **F7.4 Mini player window** — *Test first:* open-on-minimize setting decides mini vs taskbar.
- [x] **F7.5 Always on top** — *Test first:* pin toggle and setting stay in sync and persist.
- [x] **F7.6 Visualizer** — *Test first:* FFT of a 1 kHz sine peaks in the 1 kHz band; no frames emitted while paused/hidden/off.
- [x] **F7.7 Equalizer + presets** — *Test first:* Flat = bit-identical output; +6 dB at 1 kHz raises a 1 kHz sine by
  6 dB ±0.5; clipping warning when preamp + max band > 0; user presets save/load.
- [x] **F7.8 Output panel + device picker** — *Test first:* signal-path label = "Bit-perfect" only when EQ off + exclusive.
  Device choice saved; switching keeps song, position, volume, EQ. (Shows shared mode until exclusive mode lands.)
- [x] **F7.9 Second output (experimental)** — *Test first:* stereo split routes left/right, delay line inserts and
  skips frames, drift controller keeps two simulated clocks (±300 ppm, other rate, callback jitter) within 2 ms and
  learns the clock difference, variable resampler follows its correction, click track pattern; settings remember
  delays per device pair. Shared mode only. `#[ignore]` machine test plays a tone on two real outputs.
  Verified 2026-10-21 in the app on Klipsch The One (Bluetooth, 44.1 kHz) + LG HDR 4K (HDMI, 48 kHz): same sound and
  stereo split, delays, alignment clicks, offset held within ±2 ms over a minute. Machine test on two Bluetooth
  speakers (Klipsch The One + Bose SoundLink Flex, `MIU_TEST_OUTPUTS`): within ±0.1 ms over 15 s, clocks ~2 ppm apart.
  Alignment UI: one signed offset slider (±500 ms) with ±1/±10 ms nudges; clicks are low on the first output, high on
  the second. Balance slider (±24 dB, remembered per pair) turns the louder speaker down. A speaker that starts late
  (Bose waking up ~20 ms after the Klipsch) is stepped into line within a second instead of being learnt as drift.
  Lining up: nothing is judged for 3 s after a start, seek or pause (Bluetooth takes audio in bursts while it
  fills its buffer); a gap over 5 ms seen for 2 s (median, so bursts do not count) is closed in one jump, sized
  from the last half second. Big gaps are only jumped once they have stopped moving (quarter-second
  medians within 4 ms); the speed correction only follows small gaps and the clock difference. Jumps are blamed on the output that fell behind. The second output only counts as gone
  if the first kept playing. Hands-off app runs on Klipsch + Bose: median 0.1–0.5 ms apart; jumps only after real
  dropouts.
  Connection row: per-output health over the last minute (late device callbacks, a second of audio taken clearly slower than real time (how a
  struggling Bluetooth link shows: it calls back on time but takes less), jumps blamed on the output that fell behind: steady / a few
  dropouts / keeps dropping out) and battery level where Windows has one (Bose: 40 %). No signal strength: Windows
  does not expose RSSI for connected Bluetooth audio devices.
  Auto-calibrate (microphone at the listening spot): clicks for timing, short noise bursts for loudness (headset
  mics gate out steady sound), sets and saves delay and balance; a speaker's own Bluetooth mic is never offered.
  On Klipsch + Bose with a USB headset mic: run 1 heard +2.5 ms / +4.0 dB and corrected, run 2 heard +2 ms / -0.4 dB.
- [ ] **Manual:** line up the two Bluetooth speakers by ear with the clicks, then stereo split on music; switch the
  second speaker off mid-song (should drop to one output after ~2 s); walk a speaker out of range and watch the
  connection row turn amber/red. Not tried yet.
- [ ] **Manual:** listening test with FLAC 16/44.1 and 24/96 through speakers and a USB DAC.

## Phase 5c – Languages and metadata editing
- [ ] **F8.1 Tag encoding detection** — *Test first:* CP1258, GBK, Shift-JIS, CP1251 fixtures decode correctly; folder override wins.
  Built 2026-10-20, tests green, **waiting for your check in the app**: old ID3 tags in Windows-1258 / GBK / Shift-JIS / Windows-1251 read correctly; "Aa" next to a folder forces an encoding.
- [ ] **F8.2 Unicode-safe paths** — *Test first:* scan, convert and play a file in `Nhạc Trịnh/日本語/`.
  Scan, tags, play and fingerprint covered by a test under Nhạc Trịnh/日本語/ (passed first run). Conversion waits for F3.5 (ffmpeg).
- [ ] **F8.3 Font fallback + RTL** — *Manual:* Arabic/Hebrew titles render right-to-left; CJK has no tofu boxes.
- [x] **F8.4 Search** — *Test first:* "mua thu" finds "Mùa thu"; "DIEM XUA" finds "Diễm xưa"; CJK substring.
  Verified in the app 2026-10-20 (title/artist/album, every word must match, Ctrl+F / Esc).
- [x] **F8.5 Sorting** — *Test first:* Vietnamese order (A Ă Â … D Đ E Ê …), index-bar letters per locale.
  Verified in the app 2026-10-20 (ICU collation in the WebView; English/Vietnamese systems use vi rules).
- [x] **F8.6 UI translation** — *Test first (JS):* every key in `en.json` exists in `vi.json`; no hard-coded UI strings (lint test).
  Verified in the app 2026-10-20 (language picker; both windows reload). Left: error messages from the Rust side are English only.
- [ ] **F9.1 Single-song editor** — *Test first:* edit → tags written → re-read equals edit, for MP3, M4A, FLAC.
  Built 2026-10-20, tests green, **waiting for your check in the app**: select a song, then "Edit tags", right-click or F2.
- [ ] **F9.2 Bulk editor** — *Test first:* "Mixed" fields untouched; edited fields applied to all.
  Built 2026-10-20, tests green, **waiting for your check in the app**: Ctrl/Shift-click several songs; differing fields show "Mixed" and are left alone.
- [ ] **F9.3 Bulk tools** — *Test first:* filename pattern parse, auto-number, find/replace, case change.
  Built 2026-10-20, tests green, **waiting for your check in the app**: file-name pattern, auto-number, find/replace, case change (in the editor's Bulk tools).
- [ ] **F9.4 Preview, atomic write, undo** — *Test first:* crash mid-write leaves the original intact; undo restores old tags.
  Built 2026-10-20, tests green, **waiting for your check in the app**: preview list in the editor; Undo in the toast after saving.
- [ ] **F9.5 Watcher + sync + cache** — *Test first:* own writes don't trigger rescans; edited liked song appears as "update" in the sync plan.
  Built 2026-10-20: our own writes refresh the row so the rescan finds nothing; an edited liked song plans an update (tested). Cache part waits for F3.5.

## Phase 7 – Podcasts
Added 2026-10-20. Default Discover charts: Vietnam and US.
- [x] **P.1 Feeds and subscriptions** — *Test first:* RSS fixtures (iTunes tags, Vietnamese titles, missing fields) parse into a show and episodes; subscribing stores them; refreshing adds new episodes only.
  Verified in the app 2026-10-20 (subscribe by address, artwork, episodes, check for new episodes).
- [x] **P.2 OPML import / export** — *Test first:* an OPML file from another app subscribes to every feed; export round-trips.
  Verified in the app 2026-10-20 (import from another app with summary; export re-imports).
- [x] **P.3 Offline downloads** — *Test first:* resumable download (Range), auto-download newest N per show, cleanup of played episodes after X days.
  Verified in the app 2026-10-20 (download with progress, remove, auto-download newest episodes).
- [x] **P.4 Discover** — *Test first:* Apple top charts (vn, us) and Apple search results parse into subscribable shows.
  Verified in the app 2026-10-20 (Vietnam and US charts, search, subscribe; subscribed ones marked).
- [x] **P.5 Podcast playback** — *Test first:* resume position per episode, played state, skip back 15 s / forward 30 s.
  Verified in the app 2026-10-20 (play, resume, skip 15/30, now playing and mini player show the episode).
  Fix 2026-10-20: the play button pauses a playing episode instead of starting all songs (verified in the app).
- [ ] **P.6 Playback speed** (optional) — time-stretch without pitch change.
  Skipped 2026-10-20 at the user's request.

## Phase 8 – Audiobooks (LibriVox)
Added 2026-10-20. LibriVox only: free public-domain recordings with a public JSON API. Audible, Libby/OverDrive, Storytel,
Google Play Books, Spotify and Kobo have no public download API and sell DRM files, so they stay out.
A book is a `podcasts` row with kind 'audiobook' (migration 14); its chapters are episodes, read from the book's RSS feed,
so downloads, progress, the now-playing column and the mini player are the podcast ones. Books are not refreshed or cleaned up.
- [x] **A.1 Find books** — *Test first:* saved LibriVox answers (title and author search, recent books, not found) parse into
  books; search merges title and author matches; browse lists newest first, by language. Machine test (`#[ignore]`) hits the real API.
  Built 2026-10-20. Verified in the app 2026-10-20: found a book on LibriVox and added it.
  LibriVox has no Vietnamese recordings yet (checked 2026-10-20): choosing Vietnamese says so.
- [x] **A.2 Add to library, offline** — *Test first:* a book and its chapters are stored in reading order, adding again keeps progress,
  books are not podcasts, removing takes the chapters; a passing server error is retried, a 404 is not.
  Built 2026-10-20. Verified in the app 2026-10-20: added a book and downloaded a chapter. Not tried yet: Remove download, Remove from library.
- [x] **A.3 Listen** — *Test first:* resume after the furthest finished chapter; chapters play in order whatever shuffle and repeat say;
  the play queue is the downloaded chapters from the chosen one on.
  Built 2026-10-20. Verified in the app 2026-10-20: a chapter played and resumed. Not tried yet: skip 15/30, on to the next chapter, mini player.

## Phase 9 – Other music players
Added 2026-10-21. Code in `src-tauri/src/player/`; MTP assessment in `docs/MTP.md`.
- [x] **M.1 Find USB mass-storage players** — *Test first:* removable drives with a `Music` folder are offered (the folder keeps
  its real name), fixed disks and drives without one are not; drives the user set up are players whatever they hold; a player is
  known by its volume serial (`vol:XXXXXXXX`), not its letter; a picked folder splits into drive + folder; per-player settings
  (folder, convert-to MP3 V0 / MP3 320 / AAC 256, formats copied as they are) are remembered and folders off the drive refused.
  Built 2026-10-21, tests green. Drive listing checked on this PC (`player::volumes`, ignored test). Verified in the app 2026-10-21:
  the USB stick showed up as a player.
- [x] **M.2 Sync to a mass-storage player** — *Test first (fake drive):* liked songs land as `Artist\Album\NN Title.ext`; names FAT
  refuses are made safe; a second sync does nothing; unliking removes only Miu's file and the folders it emptied; a file changed
  on the player, or one Miu did not put there, is never removed or overwritten; deleted songs are copied again; a tag edit moves
  the file; a new music folder moves the songs; formats the player cannot play go converted (MP3 via LAME, cached like the
  iPhone's); a copy of the wrong size is deleted and retried next time; a sync that does not fit stops before copying; progress.
  Built 2026-10-21, tests green; FLAC → MP3 V0 with ID3 tags checked with the real ffmpeg (ignored test); the whole sync ran
  against a real drive letter (`subst`, ignored test with `MIU_PLAYER_DRIVE`). Verified in the app 2026-10-21: liked songs copied to a USB stick and play on the player.
- [ ] **M.3 MTP (Android, Walkman)** — *Test first:* each MTP storage is a player known by serial + storage id (iPhone and
  USB disks left out); without rename, songs go straight to their name, a short copy is deleted, a tag edit still moves the file.
  Built 2026-10-21 (`player::wpd_fs`, `mtp::players`), tests green. **Not tried on a real device yet**: plug in a phone (unlocked,
  File transfer) or Walkman, run `cargo test --lib wpd_fs -- --ignored --nocapture`, then sync from its card. See docs/MTP.md.

## Phase 6 – Packaging
- [ ] **F6.1 Installer + ffmpeg sidecar** — *Test first:* sidecar path resolution in dev and installed layouts.
  2026-10-20: path resolution tested (next to the app, or `src-tauri/binaries` from `target/<profile>`); `externalBin` + licence
  resource set in tauri.conf.json; the build copies `ffmpeg.exe` next to the app. ffmpeg is the BtbN LGPL v3 build
  `ffmpeg-n8.1-latest-win64-lgpl-8.1.zip` (2026-10-19), 128 MB, **not in git** (GitHub's 100 MB limit): put `bin/ffmpeg.exe` at
  `src-tauri/binaries/ffmpeg-x86_64-pc-windows-msvc.exe` and `LICENSE.txt` at `src-tauri/binaries/FFMPEG-LICENSE.txt` before building.
  2026-10-20: `npx tauri build` on this PC → `Miu Music_0.1.0_x64-setup.exe`, 48.2 MB (ffmpeg is most of it). Still to do: a clean install.
- [ ] **F6.2 First-run wizard** — *Test first (JS):* wizard step logic (drivers → phone → folders).
  Built 2026-10-20, tests green, **waiting for your check in the app**: welcome panel (drivers → iPhone, skippable → folders), hideable.
- [ ] **F6.3 Crash-safe journal** — *Test first:* journal replay resumes or rolls back an interrupted sync.
  Logic built 2026-10-20 (sync/journal.rs, tested); used once transfers exist (Phase 2).
- [ ] **F6.4 Diagnostics export** — *Test first:* export contains logs + device info and **no** `.sqlitedb`, account IDs or paths under `research/`.
  Built 2026-10-20, tests green, **waiting for your check in the app**: Settings → Help → Export diagnostics….
- [ ] **F6.5 In-app updates** — *Test first:* latest.json shape (`tools/latest-json.mjs`), updater config (endpoint, public
  key, passive install, updater artifacts), "autoUpdate" on by default and checked at most once a day, version compare, banner,
  download bar and Check now texts (JS).
  Built 2026-10-22 with tauri-plugin-updater, tests green. The app checks ~20 s after start and then daily, shows a card with the
  release notes; "Update and restart" downloads, verifies the signature, runs the NSIS installer passively (per-user, no admin)
  and restarts. Release: `.\tools\build-release.ps1` asks for the key password and writes the installer, `.sig` and
  `latest.json` to `src-tauri/target/release/github/<version>/`; upload all three to the GitHub release `v<version>`.
  The private key is `%USERPROFILE%\.tauri\miu-music.key` (on this PC `C:\Users\duykh\.tauri`, made with
  `tools/new-updater-key.ps1`, not in git): without it and its password, installed copies cannot
  be updated. **Still to verify:** an installed 0.3.1 finds and installs a later test version.
- [ ] **Manual:** clean install on a fresh Windows 11 VM works end to end.

## Fallbacks (only if needed)
- [ ] FB.1 Approach B experiment
- [ ] FB.2 Approach C (player-app Documents)
