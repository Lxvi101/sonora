# Sonora

<img src="assets/icon.svg" width="80" alt="Sonora dotted waveform icon">

[Download for macOS](https://github.com/Lxvi101/sonora/releases/latest) · [Contributing](CONTRIBUTING.md) · [Security](SECURITY.md) · [Changelog](CHANGELOG.md)

[![CI](https://github.com/Lxvi101/sonora/actions/workflows/ci.yml/badge.svg)](https://github.com/Lxvi101/sonora/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-white.svg)](LICENSE)


A small, native macOS audio editor: drop an audio recording or video, trim its audio, adjust its volume, preview, and export a new audio file. Rust + GPUI, rendered with Metal. A monochrome dot-matrix interface, native macOS file dialogs, audio decoding and playback. No browser runtime, account, or analytics. Editing stays on your Mac; the optional updater checks GitHub for signed releases.

The audio engine uses Apple's AVAudioEngine and AudioToolbox through a small Objective-C bridge. The interface uses GPUI and Metal. [Zeron](https://github.com/zeronsh/zeron) inspired the native approach; its source is not bundled or copied. Sonora was developed with AI assistance, including the Opus-authored interface.

## Install

Download the latest **Apple Silicon** ZIP from [Releases](https://github.com/Lxvi101/sonora/releases/latest), unzip it, and move Sonora to Applications. Official releases are Developer ID signed and notarized. macOS 13 or newer is required; Intel release binaries are not currently provided.

Sonora checks for updates approximately daily and asks before installing. Use **Sonora → Check for Updates…** for a manual check, or turn automatic checks off in the same menu. Updates use [Sparkle](https://sparkle-project.org/) with signed feeds and archives; recordings are never uploaded. Versions before 0.3.0 need this first updater-enabled release installed manually.

## Run and build

Requires macOS 13+, Rust, and Xcode Command Line Tools. This build was verified on Apple Silicon; Intel is not yet verified.

```sh
cargo run --release -- /path/to/recording.wav
./scripts/bundle.sh
open dist/Sonora.app
```

Dependencies are pinned in `Cargo.lock`. On a fresh machine, run `cargo fetch --locked` first; the bundle script builds offline. The `runtime_shaders` GPUI feature avoids requiring a full Xcode installation. Metal compiles shaders at runtime, so first launch can cost more than later launches.

To install in your user Applications folder, run `./scripts/install.sh`. For an update, close Sonora and run `./scripts/install.sh --replace`; the previous bundle is backed up under `target/install-backup/`.

The app bundle declares common audio types for Finder's **Open With** menu. To make double-click opening permanent, use Finder → Get Info on an audio file → Open with → Sonora → Change All. Repeat for the formats you use. This is an ordinary editor window, not a Quick Look extension replacing the Space-bar preview.

## Editing

- **Drop audio or video anywhere in the window**, including over an existing recording. MP4, MOV and M4V imports use the first audio track, starting at that track's beginning. Files with no readable audio show an error. Exports contain audio only; the source video is unchanged.
- **Drag an edge** to trim. **Double-click the left edge** to reset its start to zero; **double-click the right edge** to reset its end to the full recording. The other edge and gain stay unchanged; undo restores the prior edge.
- **Click the waveform** to seek; **drag across it** to select a range.
- **Space** previews/pauses. The transport also offers return-to-start and loop selection (**L**).
- **Original / Edited** (**B**) compares the gain change while listening. This only changes preview gain; the exported edit is unchanged, and the selected trim still applies.
- **Z** zooms to the selection; **Shift-Z** or **⌘0** fits the whole file. The overview is limited to 32× zoom because its resolution is finite.
- **I / O** set the start/end at the playhead; **Shift-I / Shift-O** restore those edges.
- **Left/right** seek 1 second; **Shift-left/right** seek 5 seconds; **Option-left/right** seek 10 milliseconds.
- Gain spans **−24 to +24 dB**. **+ / −** nudge it; **0** or a **double-click on the gain slider** resets it.
- **⌘Z / ⇧⌘Z** undo/redo. **⌘A** keeps the full recording; **⌘⌫** resets trim and gain.
- Choose **WAV / MP3 / M4A** before exporting (**⌘1 / ⌘2 / ⌘3**). **⇧⌘S** opens the native save dialog with the selected extension. The format choice persists while opening other files during the session.
- **?** opens the control guide. **⌘O** opens a file; **⌘W** closes it.

### Export formats

| Format | Encoding | Channels and sample rate |
| --- | --- | --- |
| WAV | Lossless, 24-bit PCM | Original channel count and sample rate |
| MP3 | LAME, 192 kbps | Mono/stereo; 32, 44.1 or 48 kHz |
| M4A | Apple AAC, 192 kbps target | Mono/stereo; 44.1 or 48 kHz |

Compressed exports resample when necessary and may contain codec priming/padding, which players account for differently. Actual AAC bitrate can vary with the audio. MP3 uses a bundled, dynamically linked LAME encoder built from the included source; M4A uses Apple's system encoder. **Neither requires FFmpeg, Homebrew, or a network connection.** The LAME library, license and corresponding source are included in the app; see `vendor/README.md`.

The source is never modified. Existing output files are refused: choose a fresh name. Gain above available headroom hard-clips at full scale; this is not a limiter or loudness-normalization tool. WAV exports above the RIFF 4 GB limit are rejected; export a shorter selection or choose a compressed format. Use WAV for audio with more than two channels.

Opening supports the audio formats available through the installed macOS codecs, including WAV, AIFF, MP3, M4A/AAC, CAF and FLAC. WAV, MP3, M4A and AIFF were exercised locally.

Video import uses AVFoundation. AAC/ALAC soundtracks are copied into a private M4A without re-encoding when supported. Other supported soundtracks are decoded to a lossless float CAF on a background worker with bounded buffers. This fallback needs temporary disk space proportional to audio duration; it never decodes video frames. Backing files remain alive while waveform or export jobs need them and are removed after the last owner releases them. MKV/WebM and codecs absent from macOS are not guaranteed. Multiple soundtrack selection and video output are not included.

## Performance design

- Opening probes metadata; it does not decode the recording.
- Playback schedules a native AVAudioFile segment, without keeping a whole-file PCM array.
- Waveform analysis runs off the UI thread. It publishes a sparse preview first, then an exact sequential scan.
- The overview is always 2,048 min/max bins. Decode buffers contain 16,384 frames regardless of recording duration.
- Waveforms are cached in `~/Library/Caches/io.sonora.editor`, keyed by path, size, modification time, sample rate and frame count. The cache retains at most 128 small overviews (about 2 MiB).
- New files cancel old analysis/export jobs. Generation IDs prevent stale results replacing the current file.
- Exports stream into a temporary sibling file, then publish atomically without overwriting any existing file.
- The UI sleeps when idle. Playback and export progress drive updates only while active.

## Verification

All 41 tests pass, along with formatting and all-target Clippy. The redesigned window's mouse controls, edge resets, gain reset, A/B, loop playback, zoom and native MP3/M4A exports were exercised. Keyboard playback, format selection and Undo were checked, along with cold and warm Launch Services video opening, no-audio errors, and a trimmed/amplified MP3 export from MOV. The installed app's 841-pixel tiled layout was checked with positive gain and the full format/export controls visible. Cross-application dragging could not be completed by the automation tool; the whole-window drop handler is implemented but the physical Finder-to-window gesture remains a manual verification item.

```sh
cargo test --locked --offline
cargo run --release --example benchmark -- /path/to/recording.wav
# Run only with a silent file longer than two hours:
cargo run --release --example playback_check -- /path/to/silent.wav
```

Twelve engine tests cover native decoding, exact trim/gain round trips, clipping, invalid edits, WAV sizing, cancelled-export cleanup, source preservation, cached waveforms, multi-hour metadata access, real MP3/AAC round trips, mono resampling, compressed-export cancellation and multichannel rejection. UI logic tests additionally cover edge resets, zoom mappings, format extensions and preview-gain independence. The silent playback check verifies output scheduling, seeking, selection completion, pause/resume and gain changes against Core Audio.

Local engine-only measurements on a synthetic sparse three-hour, 48 kHz stereo 24-bit WAV (3.11 GB logical size): metadata **6.1 ms**, first sparse waveform update **0.47 ms**, exact waveform **5.51 s**, maximum resident size **12.0 MB**. Cached waveform lookup **0.20 ms**. A generated three-hour AAC/M4A (32 kHz stereo, silence) opened metadata in **49 ms**, produced its first sparse update in **37 ms**, and completed its exact scan in **4.28 s**, with **17.4 MB** maximum resident size. These are not whole-app startup/RAM figures or guarantees for compressed files, slower disks, network storage, or uncached physical data.

The original application was exercised through its native window: Finder opening while already running, both trim handles, gain adjustment, playback/pause, menu Undo, and a native save-panel export. The 24-bit export was compared against the source and matched the selected samples and +6.1 dB gain within 0.000001 amplitude. A three-hour M4A opened in the UI with bounded memory (about 111 MiB RSS in the observed session). Drag-and-drop is implemented but was not exercised by automation. GPUI 0.2.2 does not expose the custom controls to VoiceOver; keyboard controls and tooltips are provided.

Export verification: a two-second fixture was independently inspected with ffprobe and identified as PCM24 WAV, 192 kbps MP3, and AAC in an M4A container. FFmpeg is used only for verification, never by the app.

Video tests cover AAC-in-MP4 and PCM-in-MOV import, trimmed WAV export, source preservation, shared backing-file lifetime, cleanup, cancellation and missing audio. A synthetic three-hour MP4 with a small silent AAC soundtrack imported in **531 ms** in a release engine check; a repeat took **530 ms** with **27 MB** maximum resident size. This measures audio extraction and metadata, not window startup or waveform completion; import time depends on soundtrack size and codecs.

## Open source

Sonora's original code is [MIT licensed](LICENSE). Dependencies retain their own licenses: GPUI is Apache-2.0, Sparkle is MIT with bundled third-party notices, and the dynamically linked LAME encoder is LGPL-2.0-or-later. LAME's corresponding source and rebuild script are included in the repository and app. See [dependency notes](docs/DEPENDENCIES.md).

Contributions are welcome; see [CONTRIBUTING.md](CONTRIBUTING.md). Report reproducible bugs through [Issues](https://github.com/Lxvi101/sonora/issues), and security vulnerabilities through [private reporting](https://github.com/Lxvi101/sonora/security/advisories/new). Maintainer release instructions are in [docs/RELEASING.md](docs/RELEASING.md).
