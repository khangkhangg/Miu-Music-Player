<p align="center">
  <img src="src-tauri/icons/128x128@2x.png" width="128" height="128" alt="Miu Music">
</p>

<h1 align="center">Miu Music</h1>

<p align="center">
  A light, beautiful music player for Windows that puts the songs you like straight into your iPhone's Music app.<br>
  <i>Trình phát nhạc nhẹ và đẹp cho Windows, đưa những bài bạn thích thẳng vào ứng dụng Nhạc trên iPhone.</i>
</p>

<p align="center">
  <a href="https://github.com/khangkhangg/Miu-Music-Player/releases/latest"><b>Download for Windows</b></a> ·
  <a href="https://musicplayer.vietnamcos.com">Website</a>
</p>

---

## What it does

- **Bit-perfect playback.** Its own audio engine with WASAPI exclusive mode. FLAC, ALAC, WAV and hi-res play exactly as recorded, and the signal path is shown.
- **Like it, and it's on your iPhone.** Thumbs up as you listen; the Sync button sends liked songs to the iPhone's built-in Music app with titles, artists and covers. Mark a whole artist or album "Always sync".
- **Converts for you.** Formats the iPhone can't play (FLAC and others) become Apple Lossless or AAC automatically, with tags and covers kept. Converted once, reused every sync.
- **Safe with the songs already on your phone.** Before every sync Miu backs up the phone's music database and keeps a copy of every song that exists only on the phone; afterwards it checks nothing went missing. It never removes songs it didn't put there.
- **Made for Vietnamese music.** Broken legacy accents (VNI, TCVN3) are repaired, search works without accents, sorting follows Vietnamese order, and the app is fully in Tiếng Việt as well as English.
- **Album covers.** Albums without a cover can be looked up online (iTunes Search, MusicBrainz + Cover Art Archive). Your music files are not changed.
- **Podcasts and audiobooks.** Podcast charts for Vietnam and the US, search, OPML, offline downloads and resume; free public-domain audiobooks from LibriVox, chapter by chapter.
- **The small things.** 10-band EQ with presets (PC playback only), a mini player with a visualizer, play/pause/skip on the taskbar preview, shuffle and repeat.

## Requirements

- Windows 10 or 11 (64-bit).
- For iPhone sync: iTunes for Windows installed, a USB cable, and **"Manually manage music and videos"** turned on for the iPhone in iTunes (Miu shows how on the first sync). Miu is tested on an iPhone 5s with iOS 12.5.8.

The installer isn't code-signed yet, so Windows SmartScreen may warn: choose **More info → Run anyway**.

## How iPhone sync works

Miu drives Apple's own iTunes through its documented COM interface, so iTunes handles the authorisation and the phone receives songs the normal way. With manual management on, Miu adds liked songs, removes songs you un-like (only ones Miu sent), and leaves everything else alone. Liked songs that are already on the phone (from 3uTools, for example) are recognised rather than copied twice.

## Building from source

Stack: Tauri 2 (Rust) with a plain HTML/CSS/JavaScript UI, no framework.

1. Install Rust (stable, MSVC) and Node.js.
2. `npm install`
3. ffmpeg is bundled as a sidecar but is too large for git. Download the LGPL build `ffmpeg-n8.1-latest-win64-lgpl-8.1.zip` from [BtbN/FFmpeg-Builds](https://github.com/BtbN/FFmpeg-Builds/releases), then put `bin/ffmpeg.exe` at `src-tauri/binaries/ffmpeg-x86_64-pc-windows-msvc.exe`.
4. Run it: `npx tauri dev` · build the installer: `npx tauri build`
5. Tests: `cargo test` in `src-tauri/` and `node --test` at the root.

Project notes: [docs/PLAN.md](docs/PLAN.md) (plan and decisions), [docs/CHECKLIST.md](docs/CHECKLIST.md) (feature status).

## Third-party software

- [ffmpeg](https://ffmpeg.org) (LGPL v3 build), bundled for conversion; licence in `src-tauri/binaries/FFMPEG-LICENSE.txt`.
- Apple's iTunes and device libraries are used from your own installation and are never redistributed.

Miu Music is not affiliated with Apple. iPhone and iTunes are trademarks of Apple Inc.
