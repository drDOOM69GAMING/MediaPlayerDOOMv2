# MediaPlayerDOOM v2

A retro DOOM-styled media player for Windows, rendered in egui. Tape, CD, and Radio faces on the deck — plus EQ, a band/artist catalog, background pulsing speakers, recording, and an in-app ffmpeg video player.

## Features

- **Deck modes** — switch the central deck between TAPE / CD / RADIO faces
  - **TAPE**: auto-reverse cassette with counters, winding, and tune slider
  - **CD**: spinning disc tray with track counter, LCDs, and in-order playback
  - **RADIO**: FM/AM dial with click-and-drag tuning, presets, SCAN, and live web streams
- **Bands catalog** — songs grouped by artist folder (A–Z, scrollable), rebuilt from whatever folder you add
- **Play order switch** — flip the retro wall switch under BANDS: ON = random shuffle, OFF = normal in-order playback
- **Equalizer** — 10-band, with presets and optional ffmpeg transcode
- **Smart shuffle** — weighted by your listening history, or plain shuffle
- **Recording** — capture the current track or a live stream (REC on the deck)
- **Video player** — click **Video** to play the matching movie in-app (ffmpeg decode): drag to move, double-click to pause
- **Album art** — local `cover/folder` art first, online lookup as fallback
- **Playlist management** — drag-to-reorder, save/load JSON, playlist-only mode, sequential mode
- **System tray** + global hotkeys (media keys)
- Credentials-optional web downloads via yt-dlp (downloads its own ffmpeg tooling on first use)

## Requirements

- Windows 10/11 x64
- Rust toolchain to build from source
- ffmpeg/ffprobe/yt-dlp are downloaded automatically into `tools/` on first run (web access required once)

## Build

```
cargo build --release
```

The binary lands at `target/release/mediaplayerofdoom.exe`. It finds your music automatically by scanning all drives for the `Music` folder with the most files — or pick any folder with **ADD DIR**.

## Keyboard

`P` play/pause · `S`/`N` skip · `Left` previous · `R` record · `+`/`-` volume · `F` fullscreen · `M` mute · `D` mode · `V` video · `L` lyrics

## Notes

- Song metadata needs ffmpeg — see `tools/ffmpeg.exe` (auto-downloaded).
- The easter egg only shows when the right panel is pulled out.