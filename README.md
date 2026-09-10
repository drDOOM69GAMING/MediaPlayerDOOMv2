# MediaPlayerDOOM v2

A retro DOOM-styled media player for Windows, rendered in egui. Tape, CD, and Radio faces on the deck, plus a band/artist catalog, background pulsing speakers, recording, YouTube downloads, and a fullscreen in-app video player with a queue. ffmpeg, ffprobe, and yt-dlp are embedded in the exe, so it runs fully standalone.

## Features

- **Deck modes**: switch the central deck between TAPE / CD / RADIO faces
  - **TAPE**: auto-reverse cassette with counters, winding, and tune slider
  - **CD**: spinning disc tray with track counter, LCDs, and in-order playback
  - **RADIO**: FM/AM dial with click-and-drag tuning, presets, SCAN, and live web streams
- **Bands catalog**: songs grouped by artist folder (A-Z, scrollable), rebuilt from whatever folder you add
- **Play order switch**: flip the retro wall switch under BANDS. ON = random shuffle, OFF = normal in-order playback
- **Smart shuffle**: weighted by your listening history, or plain shuffle
- **Recording**: capture the current track or a live stream (REC on the deck)
- **Video player**: click **Video** to play the matching movie in-app:
  - True fullscreen playback with letterboxing
  - Auto-hiding top bar (mouse to the top of the screen to reveal) with PLAY/PAUSE, prev/next, **+ ADD** (multi-file picker), and show/hide queue
  - Right-side queue panel with per-item play, remove, and CLEAR; auto-advances when a video ends
  - **Automatic intro/credits detection**: when **SKIP: ON**, the app analyzes each video's audio and finds where the intro ends and the credits/outro begins, then auto-skips the intro and jumps to the next video at the credits. Detected times are remembered per file and per folder
  - **Manual markers**: use the top bar **INTRO** / **CREDS** buttons to set the intro-end and credits-start for the current video at the current playback position; values persist per file/folder across runs
  - The top bar and queue stay visible while your mouse is over the queue panel, so you can scroll it and press CLEAR without the bar disappearing
- **Album art**: local `cover/folder` art always wins; online lookup only as a fallback when no local art exists (so pictures never get replaced by wrong web matches)
- **Lyrics**: Artist/Track auto-detected from the filename when tags are unknown
- **Playlist management**: drag-to-reorder, save/load JSON, playlist-only mode, sequential mode, per-CD directory ordering
- **System tray** + global hotkeys (media keys)
- **YouTube**: paste a URL or search term to download audio/video via the embedded yt-dlp

## Requirements

- Windows 10/11 x64
- Rust toolchain to build from source

**The exe is self-contained (single file).** ffmpeg, ffprobe, and yt-dlp are embedded at build time. On first run they self-extract to `%LOCALAPPDATA%\MediaPlayerOFDOOM\tools`, and nothing appears next to the exe. You can move or copy the exe anywhere by itself and everything still works.

## Build

```
cargo build --release
```

The binary lands at `target/release/mediaplayerofdoom.exe`. It finds your music automatically by scanning all drives for the `Music` folder with the most files, or pick any folder with **ADD DIR**.

## Keyboard

`P` play/pause · `S`/`N` skip · `Left` previous · `R` record · `+`/`-` volume · `F` fullscreen · `M` mute · `D` mode · `V` video · `L` lyrics




<img width="3834" height="2155" alt="Screenshot 2026-09-10 133844" src="https://github.com/user-attachments/assets/49e8a167-8b41-4166-9239-08067e05e3c4" />
<img width="3824" height="2155" alt="Screenshot 2026-09-10 133812" src="https://github.com/user-attachments/assets/6a162706-f897-40e7-b048-7eb8326796f1" />
<img width="3834" height="2155" alt="Screenshot 2026-09-10 133734" src="https://github.com/user-attachments/assets/052d9c5d-37c1-475f-bdf8-a1cabd113df7" />


## Notes

- The equalizer is currently **locked to FLAT** and shows a hover hint explaining why. It caused playback problems (videos stuck loading, bogus metadata) when enabled, so it has been disabled until the background EQ encode is reworked in a future build. Band changes / preset cycling / the ON-OFF toggle are disabled.
- If you build from source and have `tools/ffmpeg.exe`, `tools/ffprobe.exe`, `tools/yt-dlp.exe` next to `Cargo.toml`, they are embedded into the exe by `build.rs` (that folder is gitignored). Without them the app falls back to downloading on first run.
- The easter egg only shows when the right panel is pulled out.

## v2.4.0 changelog

- **Automatic intro/credits detection**: videos with **SKIP: ON** are analyzed on play — the app measures the audio front/back, finds where the intro ends and credits/outro begins, then auto-skips the intro and hops to the next video when credits roll.
- **Manual INTRO / CREDS markers** in the video top bar: click during playback to mark intro-end or credits-start at the current position for that file/folder; persisted in settings.
- **Queue stays open while you use it**: the bar and queue show whenever the mouse is over the top strip or the queue panel, and hide the moment it leaves — so you can scroll, play rows, and hit CLEAR without them vanishing mid-use. The old SHOW QUEUE pin toggle is removed.
- **Per-file + per-folder skip memory**: detected and manual bound times are stored both for the exact file and for its folder, so the whole series skips consistently.
- **Auto-hide controls**: the fullscreen video top bar and queue appear only while the mouse is over the top strip or the queue panel, and disappear when the mouse leaves — nothing stays stuck on screen.

## v2.3.0 changelog

- **REW/FWD buttons now work with a click** (10 second skip) in addition to press-and-hold winding.
- **CD last-folder memory**: inserting a disc remembers the last folder used.
- **All-band A-Z ordering**: the ALL button sorts bands alphabetically, songs within each band by title.
- **CD to Tape restore**: switching from CD mode to Tape always restores the full A-Z library playlist.
- **Currently-playing playlist index**: the app tracks which playlist row is playing for reliable highlight and playback.
- **Resume tape state fix**: resuming a tape now correctly sets the current song and playing state before playback begins.
- **Random/shuffle fallback**: shuffle and dir-sequential modes always find a valid playlist index so playback doesn't lose track.
- **Help window expanded**: full documentation of keyboard shortcuts, deck controls (Tape/CD/Radio), tune slider, playlist controls, and extras.
- **Duplicate Radio button removed** from the button bar.
- **Scroll-follow removed**: playlist scrolls freely without locking to the playing song.
- **Playlist highlight**: the currently playing song is marked with a filled triangle and colored text in the playlist.
- **EQ disabled (forced Flat)**: carried forward from v2.1; the equalizer is locked until the background encode is reworked.

## v2.1 changelog

- **EQ disabled (forced Flat).** Using the equalizer could wedge the background thread that handles metadata and video decode, causing video to get stuck on "LOADING..." and metadata/pictures to go wrong. A hover hint now explains it; re-enable in a future build after the EQ encode is moved off the shared worker.
- **Album art fix.** Web album-art lookup could overwrite a correct local `cover/folder` picture with a wrong match. Local art now always wins; web art is only used when no local art exists.
- **Video player overhaul**: fullscreen playback with letterboxing, auto-hiding top control bar, multi-file **+ ADD**, right-side queue (per-item play/remove/CLEAR), auto-advance at end of video, and stale-closed-event protection so manually closing never swallows the queue.
- **Video queue skip fix**: skipping forward/backwards (and the queue's per-item selection) no longer kills the video window while the audio keeps playing. Close events from the *previous* video are ignored once a new one has started. Adding videos while nothing is playing now starts playback immediately instead of silently queuing. Video ffmpeg also launches with a hidden window, so no console box flashes when a movie loads.
- **Background crash-proofing**: the library worker (metadata, album art, video decode, EQ encode) and the network worker (lyrics, art lookup, downloads) run every message inside a panic guard; any failure surfaces "Background task crashed..." in the status bar instead of killing the app.
- **EQ redesign (background render)**: when it returns, EQ renders in the background and swaps in the processed file without stopping the currently-playing song. (Currently disabled per above.)
- **Self-contained tools**: ffmpeg / ffprobe / yt-dlp are embedded in the exe and extract on first run to `%LOCALAPPDATA%\MediaPlayerOFDOOM\tools` (was: a folder next to the exe). The exe now works standalone from anywhere.
- **Radio fix**: Rock 107.1 (KJML) stream URL corrected to `https://ice42.securenetsystems.net/KJML` in both the code and the on-disk radio prefs.
- **Lyrics fix**: title/artist parsed from the filename when tags are unknown; lyrics lookups no longer crash the network worker.
- **Play order fixes**: disc/CD and playlist-only-sequential playback always advance in order even with the random switch on; random skip no longer replays the current song immediately.
- **Crash logging**: any panic writes a trace to `%APPDATA%\MediaPlayerOFDOOM\crash.log`.
