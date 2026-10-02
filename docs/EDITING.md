# Basic clip editing

Sonora edits one recording on one track. Open audio, or drop a video to use its first audio track. The original file is never changed.

## Cut, select, delete, move

1. Press **C** to choose the cut tool, then click where you want a cut. Cut on both sides of a section to isolate it. **⌘B** cuts at the playhead.
2. Press **V** to return to the selection tool and click a clip.
3. Press **Delete / Backspace** to remove the selected clip. Following clips close the gap automatically.
4. Drag a clip’s bar above the waveform to a boundary between clips to change its order. The insertion marker shows where it will land.
5. Press **Space** to preview the arrangement. Export WAV, MP3 or M4A when it sounds right.

**⌘-click** toggles individual clips; **Shift-click** extends a selection. Dragging selected clip bars moves the selection together.

**⌘Z** undoes edits; **⇧⌘Z** redoes them. **⌘A** selects all clips. **Escape** clears the selection and returns to the selection tool. Deleting everything leaves an empty track; undo or reset restores audio.

The track remains contiguous: moving reorders clips, and deleting removes time. There are no extra tracks, overlapping clips, mixing or free-floating gaps. Cuts reference source sample frames, so they do not copy audio or create intermediate files.

## Trim and volume

The outer trim handles choose the part of the arranged track to preview/export. **I / O** place the trim boundaries at the playhead. Double-click an outer edge to reset it to the corresponding timeline boundary. Gain applies to the exported arrangement; Original/Edited bypasses gain for preview only.

Existing zoom, loop, gain and format controls remain available. The in-app **?** guide and native menus list shortcuts. Export writes a new file and refuses to overwrite an existing destination.

## Implementation and verification

`src/timeline.rs` holds ordered source-frame ranges. Splitting, deletion and reordering manipulate only those ranges. Undo restores editing snapshots. Waveforms reuse the source overview; cuts do not trigger another full decode. Native playback queues source segments on one audio player node, and export streams the ranges through the selected encoder with a bounded PCM buffer.

Engine tests check frame preservation, edge cuts, deletion, reordering, empty/reset behavior and exact exported WAV sample order with gain. Compressed export tests cover MP3 and M4A. To exercise native playback across joins, use a silent file of at least two seconds:

```sh
cargo run --locked --offline --example timeline_check -- /path/to/silence.wav
```

This checks timeline position, completion, seeking and pause/resume. Use silence to avoid unexpected audible output. UI verification should additionally cover cut clicks at zoom, clip insertion markers, deletion/undo, empty-track recovery and the native export dialog.

## 0.4.0 verification

All 57 tests, formatting and all-target Clippy passed. The opt-in silent playback test also passed across many short reordered clips. In the native window, C created two cuts, V selected the middle clip, Delete reduced 12 seconds to 9, Undo restored it, and dragging moved the middle clip to the end. Select All/Delete showed the empty state with disabled playback/export; Undo recovered the arrangement. A WAV exported through the native save dialog matched the reordered source samples exactly at unity gain.
