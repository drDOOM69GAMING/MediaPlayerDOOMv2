# MediaPlayerDOOM v2

A retro DOOM-styled media player for Windows, rendered in egui. Tape, CD, and Radio faces on the deck — plus a band/artist catalog, background pulsing speakers, recording, YouTube downloads, and a fullscreen in-app video player with a queue. ffmpeg, ffprobe, and yt-dlp are embedded in the exe, so it runs fully standalone.

## Features

- **Deck modes** — switch the central deck between TAPE / CD / RADIO faces
  - **TAPE**: auto-reverse cassette with counters, winding, and tune slider
  - **CD**: spinning disc tray with track counter, LCDs, and in-order playback
  - **RADIO**: FM/AM dial with click-and-drag tuning, presets, SCAN, and live web streams
- **Bands catalog** — songs grouped by artist folder (A–Z, scrollable), rebuilt from whatever folder you add
- **Play order switch** — flip the retro wall switch under BANDS: ON = random shuffle, OFF = normal in-order playback
- **Smart shuffle** — weighted by your listening history, or plain shuffle
- **Recording** — capture the current track or a live stream (REC on the deck)
- **Video player** — click **Video** to play the matching movie in-app:
  - True fullscreen playback with letterboxing
  - Auto-hiding top bar (mouse to the top of the screen to reveal) with PLAY/PAUSE, prev/next, **+ ADD** (multi-file picker), and show/hide queue
  - Right-side queue panel with per-item play, remove, and CLEAR; auto-advances when a video ends
- **Album art** — local `cover/folder` art always wins; online lookup only as a fallback when no local art exists (so pictures never get replaced by wrong web matches)
- **Lyrics** — Artist/Track auto-detected from the filename when tags are unknown
- **Playlist management** — drag-to-reorder, save/load JSON, playlist-only mode, sequential mode, per-CD directory ordering
- **System tray** + global hotkeys (media keys)
- **YouTube** — paste a URL or search term to download audio/video via the embedded yt-dlp

## Requirements

- Windows 10/11 x64
- Rust toolchain to build from source

**The exe is self-contained (single file).** ffmpeg, ffprobe, and yt-dlp are embedded at build time. On first run they self-extract to `%LOCALAPPDATA%\MediaPlayerOFDOOM\tools` — nothing appears next to the exe. You can move or copy the exe anywhere by itself and everything still works.

## Build

```
cargo build --release
```

The binary lands at `target/release/mediaplayerofdoom.exe`. It finds your music automatically by scanning all drives for the `Music` folder with the most files — or pick any folder with **ADD DIR**.

## Keyboard

`P` play/pause · `S`/`N` skip · `Left` previous · `R` record · `+`/`-` volume · `F` fullscreen · `M` mute · `D` mode · `V` video · `L` lyrics

## Notes

- The equalizer is currently **locked to FLAT** and shows a hover hint explaining why. It caused playback problems (videos stuck loading, bogus metadata) when enabled, so it has been disabled until the background EQ encode is reworked in a future build. Band changes / preset cycling / the ON-OFF toggle are disabled.
- If you build from source and have `tools/ffmpeg.exe`, `tools/ffprobe.exe`, `tools/yt-dlp.exe` next to `Cargo.toml`, they are embedded into the exe by `build.rs` (that folder is gitignored). Without them the app falls back to downloading on first run.
- The easter egg only shows when the right panel is pulled out.

## v2.1 changelog

- **EQ disabled (forced Flat).** Using the equalizer could wedge the background thread that handles metadata and video decode, causing video to get stuck on "LOADING..." and metadata/pictures to go wrong. A hover hint now explains it; re-enable in a future build after the encode is moved off the shared worker.
- **Album art fix.** Web album-art lookup could overwrite a correct local `cover/folder` picture with a wrong match. Local art now always wins; web art is only used when no local art exists.
- **Video player overhaul** — fullscreen playback with letterboxing, auto-hiding top control bar, multi-file **+ ADD**, right-side queue (per-item play/remove/CLEAR), auto-advance at end of video, and stale-closed-event protection so manually closing never swallows the queue.
- **Background crash-proofing** — the library worker (metadata, album art, video decode, EQ encode) and the network worker (lyrics, art lookup, downloads) run every message inside a panic guard; any failure surfaces "Background task crashed…" in the status bar instead of killing the app.
- **EQ redesign (background render)** — when it returns, EQ renders in the background and swaps in the processed file without stopping the currently-playing song. (Currently disabled per above.)
- **Self-contained tools** — ffmpeg / ffprobe / yt-dlp are embedded in the exe and extract on first run to `%LOCALAPPDATA%\MediaPlayerOFDOOM\tools` (was: a folder next to the exe). The exe now works standalone from anywhere.
- **Radio fix** — Rock 107.1 (KJML) stream URL corrected to `https://ice42.securenetsystems.net/KJML` in both the code and the on-disk radio prefs.
- **Lyrics fix** — title/artist parsed from the filename when tags are unknown; lyrics lookups no longer crash the network worker.
- **Play order fixes** — disc/CD and playlist-only-sequential playback always advance in order even with the random switch on; random skip no longer replays the current song immediately.
- **Crash logging** — any panic writes a trace to `%APPDATA%\MediaPlayerOFDOOM\crash.log`.