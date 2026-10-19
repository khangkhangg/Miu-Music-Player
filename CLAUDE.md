# Miu Music — project context for Claude Code

(Repo/codename: `litesync`. The product name shown to users is **Miu Music**.)

## What this is
A super-light, modern, beautiful **Windows music player** that syncs your liked songs into the
**native Music app** on an iPhone (first target: iPhone 5s, iOS 12.5.8) — a replacement for
iTunes / 3uTools music sync.

- Watch one or more folders → every audio file is indexed, you like/dislike as you listen, and liked
  songs are converted if needed and appear in the iPhone's Music app.

## Product decisions (agreed 2026-10-19, from UI mockups)
- **Name:** Miu Music. App data lives in `%APPDATA%\MiuMusic\` (logs, backups, local DB).
- **Watched folders:** users add multiple folders; a file watcher keeps Miu's **local** SQLite library
  in step (add/rename/remove). Watching never touches the device — sync is a separate step.
- **Ratings:** every song is *liked*, *not rated* or *disliked*.
- **Sync = liked songs only.** Not rated and disliked songs are skipped; the sync bar shows the counts.
- **Disliked view:** lists disliked songs with undo and delete. Delete removes the song from the local
  library, the drive (to the **Recycle Bin**, never permanent) and the iPhone.
- **Mini player:** separate small window, opens on minimize (setting). Play controls, thumbs-down
  (dislike + skip) and thumbs-up. **Pin button** toggles always-on-top, mirrored by a setting.
- **Sound quality:** native Rust audio engine (symphonia decode + WASAPI output, optional exclusive mode for
  bit-perfect), lossless and hi-res supported. 10-band EQ + preamp with built-in and user presets; EQ affects
  PC playback only, never the synced files. Hi-res sources are converted to ALAC 24/48 (setting) for the iPhone.
- **Visualizer:** frequency bars in the mini player driven by FFT levels from the audio engine;
  can be hidden in settings; styles: bars / mirrored bars / line. Stops drawing when paused, hidden or off.
- **Languages:** any script in tags and paths; legacy tag encodings auto-detected (override per folder);
  accent-insensitive search, ICU sorting, RTL-safe display. UI strings live in `src/i18n/` (en, vi first).
- **Metadata editing:** single and bulk, with preview and undo; edits are written into the files' tags
  (atomic write) and pushed to the iPhone on the next sync.
- **Out of scope:** YouTube / YouTube Music import or download.
- Approach **A**: drive Apple's own `AirTrafficHost.dll` (installed with Apple Mobile Device Support /
  the Apple Devices app / iTunes). Apple's sync protocol fills in the per-track `integrity` signature,
  so we never forge it. This is (most likely) how 3uTools, CopyTrans and iMazing do it.
- Fallback **B**: direct `MediaLibrary.sqlitedb` writes over AFC (see `docs/MEDIALIBRARY_SCHEMA.md`).
- Fallback **C**: copy files into a third-party player app's Documents folder (house_arrest).

Full plan, phases, features, prompts and checklist: **`docs/PLAN.md`**. Read it before starting a phase.
Schema findings from the real device database: **`docs/MEDIALIBRARY_SCHEMA.md`**.

## Stack
- **Shell/UI:** Tauri 2 (Rust core + WebView2). UI in plain **HTML / CSS / vanilla JS** (no framework, no build step beyond Tauri's).
- **Device layer:** Rust FFI → Apple DLLs (`AirTrafficHost.dll`, `iTunesMobileDevice.dll` / `MobileDevice.dll`, `CoreFoundation.dll`).
- **Playback:** Rust audio engine in `src-tauri/src/audio/` (`symphonia`, WASAPI via `cpal`/`wasapi`). Audio never plays through the WebView.
- **Conversion:** bundled `ffmpeg.exe` / `ffprobe.exe` as a Tauri sidecar.
- **Local state:** SQLite (`rusqlite`) — library index, sync history, settings.
- Target: Windows 10/11 x64. Installer < 15 MB (excluding ffmpeg), idle RAM < 60 MB.

## Hard rules
1. **Never modify the device's `MediaLibrary.sqlitedb` without first pulling a timestamped backup** of
   `.sqlitedb`, `-wal`, `-shm` into `%APPDATA%\MiuMusic\backups\`.
2. All Apple-DLL calls go through one module (`src-tauri/src/apple/`). No FFI anywhere else.
3. Every unknown about the DLL API is written down in `docs/AIRTRAFFIC_NOTES.md` as it is discovered
   (export name, signature guess, observed behaviour, confidence). Don't guess silently.
4. The files in `research/` contain the user's Apple account ID — **gitignored, never commit, never upload**.
5. The Music app only plays MP3, AAC/M4A, ALAC, WAV, AIFF. Everything else is converted to AAC 256k
   (or ALAC for lossless sources) before transfer.
6. Keep the UI dependency-free: CSS variables for theming (light + dark), system font stack, no CDN.
7. Small, reviewable commits; each feature ticks its box in **`docs/CHECKLIST.md`** when done.
8. **TDD always:** write the failing test first, watch it fail, then the minimum code to pass, then refactor.
   Run the whole suite (`cargo test` in `src-tauri/` and `node --test` at the root) before every commit.
   Hardware tests are `#[ignore]` and run manually with the iPhone plugged in. No private data in tests.
9. Tools are written in Rust (no Python on this machine).
10. **Never change machine-wide Apple configuration** (HKLM `Apple Inc.` registry keys such as `CoreFP`, Apple
    services, Apple install folders) and never ask the user to. On 2026-10-20 a `CoreFP\LibraryPath` value broke
    device detection for iTunes and Miu until removed. Process-local workarounds only.

## Test device
- iPhone 5s, iOS 12.5.8, currently holds 69 MP3s synced by 3uTools (good reference data).
- Restore path if something breaks: restore DB backup via AFC, or re-sync with 3uTools.
