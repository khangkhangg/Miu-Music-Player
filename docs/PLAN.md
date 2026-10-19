# Miu Music (codename LiteSync) — Project Plan (Approach A: Apple AirTrafficHost)

> Light, beautiful music player for Windows that syncs your liked songs to the native iPhone Music app.
> Product decisions (name, watched folders, ratings, mini player): see "Product decisions" in `CLAUDE.md`.
> First target device: iPhone 5s / iOS 12.5.8. Platform: Windows 10/11 x64.

---

## 1. Research summary

**Problem.** iTunes is heavy and clumsy; 3uTools works but is cluttered and closed. Old iPhones can't
install modern music apps, so the *native* Music app is the only good player — and the only way into it
is a "real" sync.

**What we know (from the device database):**
- The Music app reads `iTunes_Control/iTunes/MediaLibrary.sqlitedb`; audio lives in `iTunes_Control/Music/Fnn/XXXX.ext`.
- Each synced track carries a 57-byte `integrity` blob we can't generate → we must use Apple's sync stack.
- 3uTools successfully wrote 69 tracks with valid integrity → a third-party Windows app *can* do this,
  most likely through `AirTrafficHost.dll` (Apple's "ATC" sync protocol host library).

**Apple components on Windows** (installed by *Apple Devices* app or iTunes → "Apple Mobile Device Support"
+ "Apple Application Support"):
- `AirTrafficHost.dll` — ATC sync protocol (dataclass "Media"), handles the Grappa/signing handshake.
- `iTunesMobileDevice.dll` / `MobileDevice.dll` — device discovery, lockdown, AFC file access.
- `CoreFoundation.dll` — CFString/CFDictionary/CFArray types used by both.
- Exports are undocumented. Names must be confirmed by inspecting the DLL (Phase 1). Community reverse-engineering
  work exists on GitHub (search: `ATHostConnection`, `AirTrafficHost`, `ATHostConnectionSendSyncRequest`);
  reported export names include `ATHostConnectionCreateWithLibrary`, `ATHostConnectionSendHostInfo`,
  `ATHostConnectionReadMessage`, `ATHostConnectionSendSyncRequest`, `ATHostConnectionSendFileBegin`,
  `ATHostConnectionSendAssetCompleted`, `ATHostConnectionGetGrappaSessionId`, `ATHostConnectionInvalidate`,
  `ATHostConnectionRelease`. **Treat all of these as unverified until Phase 1 confirms them.**

**Competitors / references:** iTunes, 3uTools, CopyTrans Manager, iMazing, WALTR — all closed source.
Differentiator: tiny footprint, clean UI, folder-mirror sync, automatic conversion, no account login.

---

## 2. Approaches considered

| # | Method | Pros | Cons | Verdict |
|---|---|---|---|---|
| A | **AirTrafficHost.dll via FFI** | Apple produces `integrity`; same path as 3uTools; native Music app | Windows-only; undocumented API; must reverse the message flow | **Primary** |
| B | Direct DB write over AFC | Cross-platform; full control | `integrity` may be rejected; library-reset risk | Experiment (Phase 0b) |
| C | Copy into a player app's Documents (house_arrest) | Trivial, robust | Not the native app; needs an iOS 12 build of VLC etc. | Fallback |
| D | Automate 3uTools/iTunes UI | No reversing | Brittle, ugly, defeats the purpose | Rejected |

**App-shell options**

| Shell | Install size | Idle RAM | UI in HTML/CSS/JS | FFI to Apple DLLs |
|---|---|---|---|---|
| **Tauri 2 (Rust)** | ~5–10 MB | ~30–60 MB | ✅ | ✅ `libloading` |
| .NET 8 + WebView2 | ~1 MB fw-dependent / ~60 MB self-contained | ~60–90 MB | ✅ | ✅ P/Invoke (most examples online) |
| Electron | ~80 MB+ | 150 MB+ | ✅ | ⚠️ via native addon | 

**Pick: Tauri 2.** If Rust FFI to CoreFoundation becomes painful in Phase 1, switch the device layer to a small
C# helper exe (`litesync-agent.exe`, JSON over stdin/stdout) and keep the Tauri UI — decision gate at end of Phase 1.

---

## 3. Phases

| Phase | Goal | Exit criteria |
|---|---|---|
| **0 – Setup & safety** | Repo, Tauri skeleton, research folder, DB backup/restore tool | Can pull & restore the 3 DB files over AFC |
| **1 – DLL spike** (risk gate) | Load Apple DLLs, list devices, open an ATC connection, log every message | Device listed; ATC messages logged to `docs/AIRTRAFFIC_NOTES.md` |
| **2 – One-track sync** (risk gate) | Push 1 MP3 through ATC so it plays in Music app | Song visible & playable after unplug, no library reset |
| **3 – Library & conversion** | Scan folders, read tags/artwork, convert unsupported formats | 500-file folder scanned < 10 s; FLAC/OGG/WMA converted |
| **4 – Full sync engine** | Diff PC library vs device, add/update/remove, batching, resume | Mirror sync of 500 tracks, re-run = no-op |
| **5 – UI polish** | Beautiful, simple UI, drag-and-drop, progress, dark mode | Usability pass; idle RAM < 60 MB |
| **6 – Packaging** | Installer, prerequisite check, logs, crash-safe | Clean install on fresh Win 11 works end-to-end |

Fallbacks: if Phase 2 fails after a time-box (≈ 1 week), run **Phase 0b** (Approach B experiment) and/or
ship **Approach C** with the same UI.

---

## 4. File structure

```
litesync/
├─ CLAUDE.md
├─ .gitignore                    # research/, *.sqlitedb*, target/, node_modules/
├─ docs/
│  ├─ PLAN.md                    # this file (checklist lives here)
│  ├─ MEDIALIBRARY_SCHEMA.md     # device DB findings
│  └─ AIRTRAFFIC_NOTES.md        # running log of DLL exports, signatures, message flow
├─ research/                     # PRIVATE, gitignored
│  ├─ db-baseline/               # MediaLibrary.sqlitedb(+wal,+shm) as pulled
│  └─ captures/                  # ATC message logs, before/after DB diffs
├─ tools/
│  ├─ dll_exports.py             # dump exports of Apple DLLs (pefile)
│  └─ db_diff.py                 # diff two MediaLibrary snapshots table-by-table
├─ src/                          # UI (vanilla HTML/CSS/JS)
│  ├─ index.html
│  ├─ styles/ tokens.css  app.css
│  └─ js/  app.js  api.js  player.js  visualizer.js  views/{library,disliked,device,sync,settings}.js  components/{track-row,progress,dropzone}.js
│  └─ mini.html                 # mini player window (always-on-top capable)
└─ src-tauri/
   ├─ tauri.conf.json
   ├─ binaries/ ffmpeg-x86_64-pc-windows-msvc.exe  ffprobe-…exe
   └─ src/
      ├─ main.rs                 # Tauri commands + event wiring
      ├─ apple/                  # ALL FFI lives here
      │  ├─ mod.rs  loader.rs    # locate & load DLLs (registry / Program Files / WindowsApps)
      │  ├─ cf.rs                # CoreFoundation helpers (CFString, CFDictionary, CFNumber ↔ Rust)
      │  ├─ device.rs            # discovery, lockdown, device info
      │  ├─ afc.rs               # AFC file ops (pull/push/list)
      │  └─ airtraffic.rs        # ATC connection, message loop, sync session
      ├─ audio/    decode.rs (symphonia)  output.rs (WASAPI shared/exclusive)  eq.rs  fft.rs  queue.rs
      ├─ library/  scan.rs  tags.rs  artwork.rs  db.rs (local SQLite)
      ├─ convert/  ffmpeg.rs  policy.rs
      ├─ sync/     plan.rs (diff)  engine.rs  backup.rs
      └─ util/     log.rs  paths.rs  error.rs
```

---

## 5. Feature list

### F0 Foundation
- F0.1 Tauri 2 skeleton with vanilla JS UI, light/dark tokens
- F0.2 Structured logging to `%APPDATA%\MiuMusic\logs` (rotating)
- F0.3 Prerequisite check: Apple Mobile Device Support present? show friendly install link if not
- F0.4 Local SQLite (settings, library index, sync history)

### F1 Device layer (FFI)
- F1.1 Locate & load Apple DLLs (classic install + Microsoft Store/Apple Devices app paths)
- F1.2 CoreFoundation bridge (create/read CFString, CFNumber, CFDictionary, CFArray, CFData; release safely)
- F1.3 Device discovery + hot-plug events (connect/disconnect, trusted/untrusted)
- F1.4 Device info: name, model, iOS version, free space, track count
- F1.5 AFC: list/pull/push files in media root
- F1.6 Backup & restore of `MediaLibrary.sqlitedb` (+wal, +shm) — mandatory before any write

### F2 ATC sync protocol (the core)
- F2.1 Enumerate & document DLL exports (`tools/dll_exports.py`)
- F2.2 Open ATC connection, send host info, read message loop, log every message (name + params)
- F2.3 Request a "Media" sync session; handle device responses until "ready"
- F2.4 Build & send per-track metadata (title, artist, album, album artist, genre, year, track/disc #, duration, artwork)
- F2.5 Transfer audio file to device path supplied by the session; report progress; mark asset complete
- F2.6 Finish session cleanly; verify via pulled DB that `integrity` was populated
- F2.7 Delete track(s) from device via ATC
- F2.8 Error recovery: cancel, device unplugged mid-sync, locked device, "Trust this computer"

### F3 Library & conversion
- F3.1 **Multiple watched folders** (add/remove in sidebar, per-folder song count) + drag-and-drop;
  file watcher (`notify` crate / ReadDirectoryChangesW) updates the local library on add/rename/remove
- F3.2 Fast scan: detect formats by content (ffprobe), not extension
- F3.3 Tag reading (ID3v2, MP4, Vorbis, FLAC, WMA, APE) incl. embedded artwork; Vietnamese/UTF-8 safe
- F3.4 Conversion policy: keep MP3/AAC/ALAC/WAV/AIFF; FLAC/APE/WV → ALAC (or AAC 256k by setting); OGG/Opus/WMA → AAC 256k.
  **Hi-res for iPhone** setting: sources above 48 kHz / 24-bit are converted to ALAC 24/48 (default) or 16/44.1
  to save device space (iPhone 5s output is ≤ 48 kHz). PC playback always uses the original file
- F3.5 Conversion cache (hash-keyed) so re-syncs don't re-encode
- F3.6 Duplicate detection (tags + duration + audio hash)

### F4 Sync engine
- F4.1 Sync plan: diff local library vs device → add / update metadata / remove
- F4.2 Sync set = **liked songs only** (not rated + disliked skipped); a liked → disliked/unrated change
  removes the song from the device on next sync
- F4.3 Batching, queue, pause/resume, cancel
- F4.4 Storage check before sync; warn if not enough space
- F4.5 Post-sync verification (pull DB, count matches)
- F4.6 Sync history & per-track status

### F5 UI/UX
- F5.1 Device card (photo-style iPhone silhouette, name, iOS, storage bar)
- F5.2 Library view: virtualized track list, search, sort, album grid with artwork
- F5.3 One big "Sync" button + plan preview ("+42 add, 3 remove, 1.2 GB")
- F5.4 Live progress (overall + current track), ETA, cancel
- F5.5 Settings: conversion quality, watched folders, theme, mini player (open on minimize, always on top, visualizer + style)
- F5.6 Empty/error states with plain-language guidance (Trust prompt, missing drivers, cable)
- F5.7 Accessibility: keyboard nav, focus rings, contrast AA

### F6 Packaging
- F6.1 Tauri bundle (MSI/NSIS), bundle ffmpeg sidecar, code-sign optional
- F6.2 First-run wizard (detect drivers, connect phone, choose folder)
- F6.3 Crash-safe: journal of in-progress sync; resume or roll back
- F6.4 Diagnostics export (logs + device info, **no DB/account data**)

### F7 Player, ratings and mini player
- F7.1 **Native Rust audio engine** (`src-tauri/src/audio/`): decode with `symphonia` (FLAC, ALAC, WAV, AIFF,
  MP3, AAC, Vorbis; hi-res up to 24-bit / 192 kHz), output via WASAPI (`cpal` or `wasapi` crate) in shared mode,
  optional **exclusive mode** for bit-perfect output at the file's sample rate. Gapless playback. Controls:
  play/pause/seek/next/prev, shuffle, repeat, volume, queue from current view. UI talks to it via Tauri commands/events
- F7.2 Ratings: liked / not rated / disliked, stored in local SQLite; filter chips in the Songs view
- F7.3 Disliked view: undo dislike, delete one, delete all — removes from library, drive (Recycle Bin only)
  and iPhone; confirm before delete
- F7.4 Mini player window (opens on minimize, setting); controls + thumbs-down (dislike and skip) + thumbs-up
- F7.5 Always on top: pin button on the mini player, mirrored by a setting, persisted
- F7.6 Visualizer: engine computes FFT band levels and emits them to the UI (~30–60/s, only while visible);
  show/hide setting; styles bars / mirrored / line; no work while paused, hidden or off
- F7.7 **Equalizer**: 10-band graphic EQ (31 Hz–16 kHz, ±12 dB, peaking biquads) + preamp, 32-bit float DSP in the engine;
  on/off switch; clipping warning; built-in presets (Flat, Bass boost, Vocal, Rock, Classical, Jazz, Acoustic,
  Treble boost, Loudness); editing switches to Custom; save/rename/delete user presets in local SQLite.
  EQ applies to PC playback only — synced files are never altered
- F7.8 Output panel: show source format (e.g. FLAC 24/96), output device/mode, and signal path
  ("Bit-perfect" when EQ off + exclusive mode, else "EQ on · 32-bit float"); output device picker
- Out of scope: YouTube / YouTube Music import or download

### F8 Languages
- F8.1 Tag text encoding: Unicode tags read as-is; legacy 8-bit tags (ID3v1, Latin-1 ID3v2 frames) run through
  encoding detection (`chardetng`) for Vietnamese (CP1258/VNI/TCVN3), Chinese (GBK/Big5), Japanese (Shift-JIS),
  Korean (EUC-KR), Cyrillic (CP1251) etc.; per-folder encoding override; "Fix encoding" writes Unicode back (F9)
- F8.2 Unicode-safe paths end to end (Rust `OsString`/`PathBuf`, ffmpeg args), tested with Vietnamese and CJK folder names
- F8.3 Display: system font fallback (Segoe UI → Yu Gothic / Microsoft YaHei / Malgun Gothic / Leelawadee),
  `dir="auto"` on every title/artist/album element for RTL scripts
- F8.4 Search: diacritic- and case-insensitive ("mua thu" finds "Mùa thu"), CJK substring match
- F8.5 Sorting: locale-aware collation (`icu_collator`), Vietnamese order (Đ after D), index-bar letters per locale
- F8.6 UI translation: all strings in `src/i18n/{en,vi}.json`, no hard-coded UI text, language setting
  (default = Windows display language); English and Vietnamese first

### F9 Metadata editing
- F9.1 Single-song editor: title, artist, album, album artist, genre, year, track #/total, disc #/total,
  composer, comment, lyrics, artwork (drop/paste image, remove)
- F9.2 Bulk editor for a multi-selection: fields with different values show "Mixed" and are left alone unless
  edited; set artwork for a whole album
- F9.3 Bulk tools: auto-number tracks, find and replace, capitalisation (Title / Sentence case),
  tags from filename pattern (e.g. `%track% - %artist% - %title%`), rename files from tags, fix encoding (F8.1)
- F9.4 Preview before apply (old → new per song), then write tags into the files (`lofty`: ID3v2.4, MP4, FLAC/Vorbis, APE)
  atomically (temp file + replace); **undo** of the last edit batch from a stored copy of the old tags
- F9.5 Watcher ignores Miu's own writes; edited liked songs get a metadata update on the iPhone at next sync
  (F4.1); conversion cache keyed by audio hash so a tag edit re-tags the cached copy instead of re-encoding

### Fallback features (only if gates fail)
- FB.1 Approach B experiment: clone a track row set, NULL vs copied `integrity`, push, reboot, observe
- FB.2 Approach C: house_arrest copy into a chosen player app's Documents

---

## 6. Claude Code prompts (one per feature/step)

Paste these in order. Each assumes Claude Code has read `CLAUDE.md` and this plan.

**P0.1 — Skeleton**
> Read CLAUDE.md and docs/PLAN.md. Scaffold the Tauri 2 project exactly per the file structure in §4, with a vanilla HTML/CSS/JS UI (no framework). Add tokens.css with light/dark themes via CSS variables, a `.gitignore` covering research/ and *.sqlitedb*, and rotating file logging (F0.2). Show me it running with a placeholder "No device connected" screen. Tick F0.1 and F0.2.

**P0.2 — Prerequisites**
> Implement F0.3: detect whether Apple Mobile Device Support / Apple Application Support are installed (registry keys, Program Files, and the Apple Devices Store app path). Expose a Tauri command `check_prereqs()` returning found DLL paths. Show a friendly UI state if missing. Write findings about real paths into docs/AIRTRAFFIC_NOTES.md.

**P1.1 — DLL export map**
> Write tools/dll_exports.py (pefile) that dumps every export of AirTrafficHost.dll, iTunesMobileDevice.dll/MobileDevice.dll and CoreFoundation.dll found by check_prereqs. Run it, save output to research/, and summarize the ATHostConnection* and AMDevice*/AFC* exports in docs/AIRTRAFFIC_NOTES.md. Then search GitHub for existing reverse-engineering of these exports and record signatures you find, each with a confidence level and source link.

**P1.2 — CoreFoundation bridge**
> Implement src-tauri/src/apple/loader.rs and cf.rs (F1.1, F1.2): load the DLLs with libloading (add their directory to the DLL search path first), and wrap CFString/CFNumber/CFDictionary/CFArray/CFData creation, reading and release. Include unit tests that round-trip Rust values through CF types. No FFI outside src/apple/.

**P1.3 — Device discovery & info**
> Implement F1.3 and F1.4 in apple/device.rs: device notification subscription, connect/disconnect events emitted to the UI, and device info (name, ProductType, ProductVersion, free space). Show the device card (F5.1 basic). Test with my iPhone 5s.

**P1.4 — AFC + DB backup**
> Implement F1.5 (AFC list/pull/push on the media root) and F1.6: pull `/iTunes_Control/iTunes/MediaLibrary.sqlitedb` plus -wal and -shm into %APPDATA%\MiuMusic\backups\<timestamp>\, and a restore command. Add tools/db_diff.py that compares two snapshots table by table (row counts + changed rows). Verify a backup opens with sqlite3 and shows 69 items.

**P1.5 — ATC message logger (risk gate)**
> Implement apple/airtraffic.rs F2.2: create an ATHostConnection for my device, send host info, and run a read-message loop that logs each message name and its parameter dictionary as JSON to research/captures/. Don't send any sync request yet. Document the exact call sequence, arguments and every message observed in docs/AIRTRAFFIC_NOTES.md. Stop and report if any call crashes or returns errors — don't guess further without telling me.

**P2.1 — Sync session**
> Implement F2.3: request a Media sync session and handle responses until the device reports it is ready. Log everything. Before running, take a DB backup (F1.6). Report the message flow and what the device expects next.

**P2.2 — One-track sync (risk gate)**
> Implement F2.4–F2.6 for a single MP3 I'll choose: send its metadata, transfer the file, complete the asset and finish the session. Then pull the DB, run tools/db_diff.py against the pre-sync backup, and confirm: a new item row exists, `integrity` is non-null, and the file is in iTunes_Control/Music/Fnn. Tell me to check the Music app on the phone. If it fails, restore the backup and summarize what we learned.

**P2.3 — Delete & recovery**
> Implement F2.7 (remove tracks via ATC) and F2.8 (cancel, unplug mid-sync, locked/untrusted device). Write a test checklist I can run manually with the phone and record results in AIRTRAFFIC_NOTES.md.

**P3.1 — Scanner & tags**
> Implement F3.1–F3.3: folder picker + drag-and-drop, fast parallel scan using ffprobe for format detection, tag + artwork reading (lofty crate or ffprobe), stored in local SQLite. Must handle Vietnamese titles correctly. Benchmark on a 500-file folder and report time.

**P3.2 — Conversion**
> Implement F3.4–F3.5 with the bundled ffmpeg sidecar: conversion policy per CLAUDE.md rule 5, hash-keyed cache in %LOCALAPPDATA%\MiuMusic\cache, preserving tags and artwork. Add F3.6 duplicate detection. Test with FLAC, OGG, WMA, Opus, M4A samples.

**P4.1 — Sync plan & engine**
> Implement F4.1–F4.4: diff liked songs in the local library vs tracks on device (read from pulled DB), a queue with pause/resume/cancel, and a free-space check. Batch many tracks into one ATC session. Running sync twice in a row must produce an empty plan.

**P4.2 — Verification & history**
> Implement F4.5–F4.6: after each sync pull the DB and verify counts/IDs; store sync history and per-track status; surface failures in the UI with retry.

**P5.1 — UI polish**
> Polish the UI (F5.2–F5.7): calm, modern, minimal; virtualized track list, album grid with artwork, one primary Sync button with plan preview, live progress with ETA, settings page, plain-language error states, keyboard navigation, AA contrast in light and dark. Keep idle RAM under 60 MB — measure and report.

**P6.1 — Packaging**
> Implement F6.1–F6.4: NSIS installer with ffmpeg sidecar, first-run wizard, crash-safe sync journal with resume/rollback, and a diagnostics export that excludes any DB or account data. Test a clean install on a fresh Windows 11 VM checklist.

**P7.1 — Player & ratings**
> Implement F7.1–F7.3: the native Rust audio engine in src-tauri/src/audio/ (symphonia decode, WASAPI shared + optional exclusive output, gapless), test with FLAC 16/44.1 and 24/96, ALAC, AIFF, WAV, MP3, AAC; then liked / not rated / disliked ratings with filter chips, and the Disliked view with undo and delete (library + Recycle Bin + device, with confirmation). Make the sync plan use liked songs only (F4.2).

**P7.2 — Mini player**
> Implement F7.4–F7.6: a second Tauri window for the mini player (opens on minimize), like/dislike buttons, a pin button and setting for always-on-top, and a visualizer fed by FFT levels from the audio engine that can be hidden in settings and stops when paused or hidden. Measure idle RAM and CPU with the visualizer on and off.

**P7.3 — Equalizer & output**
> Implement F7.7–F7.8: 10-band EQ + preamp in the audio engine, presets (built-in + user, stored in SQLite), clipping warning, on/off; output device picker, exclusive-mode toggle and the signal-path panel. Verify bit-perfect when EQ is off in exclusive mode (null test or DAC sample-rate display).

**P8.1 — Languages**
> Implement F8.1–F8.6: encoding detection for legacy tags with per-folder override, Unicode-safe paths, RTL-safe display, diacritic-insensitive search, ICU collation, and UI strings moved into src/i18n/en.json and vi.json with a language setting. Test with a folder of old Vietnamese, Chinese, Japanese and Russian MP3s that show garbled text in Windows Explorer.

**P9.1 — Metadata editing**
> Implement F9.1–F9.5: single and bulk tag editor with "Mixed" fields, bulk tools, preview, atomic tag writes with lofty, undo of the last batch, watcher self-write suppression, and metadata update on the iPhone at next sync. Test on copies of files, never on the only copy.

**PB.1 — (fallback) Approach B experiment**
> Only if P2.2 failed: using docs/MEDIALIBRARY_SCHEMA.md, write a script that clones one existing track's full row set (new random pids, new sync_id, new entity_revision rows, container_item row, new file in F00), variant 1 keeping a copied integrity blob, variant 2 with NULL integrity. Back up first, push one variant, have me reboot the phone and report. Restore afterwards.

---

## 7. Checklist

Moved to **`docs/CHECKLIST.md`** — the single working checklist, with the test to write first for every item.
