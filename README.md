# Sonora

A small, native macOS audio editor: open a recording, trim it, adjust its volume, preview, and export a new file. Rust + GPUI, rendered with Metal. Native macOS glass, file dialogs, audio decoding and playback. No browser, server, account, or network connection at runtime.

The complete interface and icon were authored through **Claude Code using `claude-opus-5-5`**, as requested. The independent audio engine uses Apple's AVAudioEngine and AudioToolbox through a small Objective-C bridge. Zeron was a visual and architectural reference; its source is not bundled or copied.

## Run and build

Requires macOS 13+, Rust, and Xcode Command Line Tools. This build was verified on Apple Silicon; Intel is not yet verified.

```sh
cargo run --release -- /path/to/recording.wav
./scripts/bundle.sh
open dist/Sonora.app
```

Dependencies are pinned in `Cargo.lock`. On a fresh machine, run `cargo fetch --locked` first; the bundle script builds offline. The `runtime_shaders` GPUI feature avoids requiring a full Xcode installation. Metal compiles shaders at runtime, so first launch can cost more than later launches.

To install in your user Applications folder, run `./scripts/install.sh` (it refuses to replace an existing installation).

The app bundle declares common audio types for Finder's **Open With** menu. To make double-click opening permanent, use Finder → Get Info on an audio file → Open with → Sonora → Change All. Repeat for the formats you use. This is an ordinary editor window, not a Quick Look extension replacing the Space-bar preview.

## Editing

- Open with ⌘O, drop an audio file, or use Finder's Open With.
- Drag waveform trim handles to keep a range. Click to seek.
- Space plays or pauses the selected range. I and O set its start and end at the playhead.
- Left/right seek one second; Shift-left/right seek five seconds; Option-left/right seek 10 milliseconds for precise I/O marks.
- Adjust gain from −24 to +24 dB. `+` / `−` change it, and `0` resets it.
- ⌘Z / ⇧⌘Z undo and redo edits. ⌘A selects the whole recording.
- ⇧⌘S exports a new 24-bit PCM WAV at the original sample rate and channel count.

The source is never modified. Existing output files are refused, even if the native save panel offers Replace: choose a fresh name. Gain above available headroom hard-clips at full scale; this is not a limiter or loudness-normalization tool. WAV exports above the RIFF 4 GB limit are rejected; export a shorter selection.

Opening/decoding supports the audio formats available through the installed macOS codecs, including WAV, AIFF, MP3, M4A/AAC, CAF and FLAC. WAV, M4A and AIFF were exercised locally. This first version exports WAV only.

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

```sh
cargo test --lib --locked --offline
cargo run --release --example benchmark -- /path/to/recording.wav
# Run only with a silent file longer than two hours:
cargo run --release --example playback_check -- /path/to/silent.wav
```

Eight engine tests cover native decoding, exact trim/gain round trips, clipping, invalid edits, WAV sizing, cancelled-export cleanup, source preservation, cached waveforms and multi-hour metadata access. The silent playback check verifies output scheduling, seeking, selection completion, pause/resume and gain changes against Core Audio.

Local engine-only measurements on a synthetic sparse three-hour, 48 kHz stereo 24-bit WAV (3.11 GB logical size): metadata **6.1 ms**, first sparse waveform update **0.47 ms**, exact waveform **5.51 s**, maximum resident size **12.0 MB**. Cached waveform lookup **0.20 ms**. A generated three-hour AAC/M4A (32 kHz stereo, silence) opened metadata in **49 ms**, produced its first sparse update in **37 ms**, and completed its exact scan in **4.28 s**, with **17.4 MB** maximum resident size. These are not whole-app startup/RAM figures or guarantees for compressed files, slower disks, network storage, or uncached physical data.

The actual application was also exercised through its native window: Finder opening while already running, both trim handles, gain adjustment, playback/pause, menu Undo, and a native save-panel export. The 24-bit export was compared against the source and matched the selected samples and +6.1 dB gain within 0.000001 amplitude. A three-hour M4A opened in the UI with bounded memory (about 111 MiB RSS in the observed session). Drag-and-drop is implemented but was not exercised by automation. GPUI 0.2.2 does not expose the custom controls to VoiceOver; keyboard controls and tooltips are provided.

The bundle is locally ad-hoc signed, not Developer ID signed or notarized for public distribution.
