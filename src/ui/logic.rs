//! Pure editing, view and export-format math. Nothing here touches GPUI or
//! the audio engine, so every rule the pointer gestures rely on is unit
//! tested below.

use std::path::{Path, PathBuf};

use crate::audio::{Edit, ExportFormat};
use crate::timeline::Timeline;

pub const GAIN_MIN: f32 = -24.0;
pub const GAIN_MAX: f32 = 24.0;
pub const GAIN_STEP: f32 = 0.5;
pub const MIN_SELECTION: f64 = 0.02;
/// Deepest zoom relative to the whole file. The overview has a fixed number
/// of bins, so zooming further would only magnify the same blocks.
pub const MAX_ZOOM: f64 = 32.0;

pub const FORMATS: [ExportFormat; 3] = [ExportFormat::Wav, ExportFormat::Mp3, ExportFormat::M4a];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Handle {
    Start,
    End,
}

/// The two pointer tools. Select (V) picks and reorders clips and keeps the
/// trim gestures; Razor (C) cuts the clip under the pointer.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Tool {
    #[default]
    Select,
    Razor,
}

// ---- Clip selection ------------------------------------------------------------

/// How a click changes the clip selection: plain picks one clip, ⌘ toggles
/// it, ⇧ extends from the anchor. Returns the new selection and anchor.
pub fn click_selection(
    selected: &[usize],
    anchor: Option<usize>,
    index: usize,
    toggle: bool,
    extend: bool,
) -> (Vec<usize>, Option<usize>) {
    if extend {
        let from = anchor.unwrap_or(index);
        let mut out: Vec<usize> = selected.to_vec();
        out.extend(from.min(index)..=from.max(index));
        out.sort_unstable();
        out.dedup();
        (out, Some(from))
    } else if toggle {
        let mut out: Vec<usize> = selected.iter().copied().filter(|&i| i != index).collect();
        if out.len() == selected.len() {
            out.push(index);
            out.sort_unstable();
        }
        (out, Some(index))
    } else {
        (vec![index], Some(index))
    }
}

/// A timeline time after ripple-deleting `deleted` from `before`: the same
/// audio's new time, or the join that replaced it.
pub fn ripple_time(before: &Timeline, deleted: &[usize], time: f64, sample_rate: f64) -> f64 {
    let frame = (time.max(0.0) * sample_rate).round() as u64;
    before.ripple(deleted, frame) as f64 / sample_rate
}

/// The trim after a ripple delete. Its edges follow their audio; if the
/// trimmed audio itself is gone, everything left is kept; if nothing is
/// left, an empty edit (no playback, no export) until undo or reset.
pub fn edit_after_delete(
    edit: Edit,
    before: &Timeline,
    deleted: &[usize],
    sample_rate: f64,
) -> Edit {
    let remaining = before.ripple(deleted, before.frames());
    let whole = remaining as f64 / sample_rate;
    let kept = Edit {
        start: ripple_time(before, deleted, edit.start, sample_rate),
        end: ripple_time(before, deleted, edit.end, sample_rate),
        ..edit
    };
    if remaining == 0 {
        Edit {
            start: 0.0,
            end: 0.0,
            ..edit
        }
    } else if kept.end - kept.start >= MIN_SELECTION
        && kept.validate_frames(remaining, sample_rate).is_ok()
    {
        kept
    } else {
        Edit {
            start: 0.0,
            end: whole,
            ..edit
        }
    }
}

/// Selection indices after a cut whose right-hand piece landed at `right`:
/// later clips shift by one, and a selected clip stays selected as two halves.
pub fn selection_after_split(selected: &[usize], right: usize) -> Vec<usize> {
    let mut out = Vec::with_capacity(selected.len() + 1);
    for &i in selected {
        if i >= right {
            out.push(i + 1);
        } else {
            out.push(i);
            if i + 1 == right {
                out.push(right);
            }
        }
    }
    out.sort_unstable();
    out
}

// ---- Edits -----------------------------------------------------------------

/// Double-clicking a trim handle sends only that edge back to the file
/// boundary; the other edge and the gain are kept.
pub fn reset_edge(edit: Edit, handle: Handle, duration: f64) -> Edit {
    match handle {
        Handle::Start => Edit { start: 0.0, ..edit },
        Handle::End => Edit {
            end: duration,
            ..edit
        },
    }
}

/// Moves one edge to `time`, never letting the selection collapse.
pub fn drag_edge(edit: Edit, handle: Handle, time: f64, duration: f64) -> Edit {
    let time = time.clamp(0.0, duration);
    match handle {
        Handle::Start => Edit {
            start: time.min(edit.end - MIN_SELECTION).max(0.0),
            ..edit
        },
        Handle::End => Edit {
            end: time.max(edit.start + MIN_SELECTION).min(duration),
            ..edit
        },
    }
}

/// The selection spanned by a range drag, if it is long enough to keep.
pub fn range_between(anchor: f64, time: f64, duration: f64) -> Option<(f64, f64)> {
    let (a, b) = (anchor.clamp(0.0, duration), time.clamp(0.0, duration));
    let (a, b) = if b < a { (b, a) } else { (a, b) };
    (b - a >= MIN_SELECTION).then_some((a, b))
}

/// `I`: start at the playhead; if that leaves too little, release the end.
pub fn set_in(edit: Edit, at: f64, duration: f64) -> Edit {
    let mut edit = edit;
    edit.start = at.clamp(0.0, (duration - MIN_SELECTION).max(0.0));
    if edit.end - edit.start < MIN_SELECTION {
        edit.end = duration;
    }
    edit
}

/// `O`: end at the playhead; if that leaves too little, release the start.
pub fn set_out(edit: Edit, at: f64, duration: f64) -> Edit {
    let mut edit = edit;
    edit.end = at.clamp(MIN_SELECTION.min(duration), duration);
    if edit.end - edit.start < MIN_SELECTION {
        edit.start = 0.0;
    }
    edit
}

pub fn nudge_gain(db: f32, delta: f32) -> f32 {
    (((db + delta) / GAIN_STEP).round() * GAIN_STEP).clamp(GAIN_MIN, GAIN_MAX)
}

/// Pointer gain: tenths of a dB, with a small detent at unity.
pub fn snap_gain(db: f32) -> f32 {
    let db = (db * 10.0).round() / 10.0;
    if db.abs() < 0.45 {
        0.0
    } else {
        db.clamp(GAIN_MIN, GAIN_MAX)
    }
}

/// The gain the preview plays and draws. A/B "original" bypasses gain for
/// listening only; the edit (and so the export) keeps it.
pub fn preview_gain(edit: Edit, original: bool) -> f32 {
    if original { 0.0 } else { edit.gain_db }
}

// ---- View --------------------------------------------------------------------

/// The visible time range of the waveform, absolute seconds.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct View {
    pub start: f64,
    pub end: f64,
}

impl View {
    pub fn full(duration: f64) -> Self {
        Self {
            start: 0.0,
            end: duration.max(0.0),
        }
    }

    pub fn span(&self) -> f64 {
        (self.end - self.start).max(1e-9)
    }

    pub fn is_full(&self, duration: f64) -> bool {
        self.start <= 1e-9 && self.end >= duration - 1e-9
    }

    /// How many times closer than the whole file.
    pub fn zoom(&self, duration: f64) -> f64 {
        duration.max(1e-9) / self.span()
    }

    /// Fits `[start, end]` inside the file, honouring the zoom limit.
    /// A range that would go past one end slides back rather than shrinking.
    pub fn fitted(start: f64, end: f64, duration: f64) -> Self {
        if duration <= 0.0 {
            return Self::full(0.0);
        }
        let span = (end - start).clamp(min_span(duration), duration);
        let center = (start + end) / 2.0;
        let start = (center - span / 2.0).clamp(0.0, duration - span);
        Self {
            start,
            end: start + span,
        }
    }

    /// Frames a selection with a little air on both sides.
    pub fn around(sel_start: f64, sel_end: f64, duration: f64) -> Self {
        let pad = (sel_end - sel_start).max(0.0) * 0.06;
        Self::fitted(sel_start - pad, sel_end + pad, duration)
    }

    /// Zooms by `factor` (>1 = closer), keeping `anchor` fixed on screen.
    pub fn zoomed(&self, factor: f64, anchor: f64, duration: f64) -> Self {
        let anchor = anchor.clamp(self.start, self.end);
        let ratio = (anchor - self.start) / self.span();
        let span = (self.span() / factor.max(1e-6)).clamp(min_span(duration), duration);
        let start = (anchor - ratio * span).clamp(0.0, (duration - span).max(0.0));
        Self {
            start,
            end: start + span,
        }
    }

    pub fn panned(&self, seconds: f64, duration: f64) -> Self {
        let span = self.span().min(duration);
        let start = (self.start + seconds).clamp(0.0, (duration - span).max(0.0));
        Self {
            start,
            end: start + span,
        }
    }

    /// Centers the view on `time` without changing its span.
    pub fn centered_on(&self, time: f64, duration: f64) -> Self {
        self.panned(time - (self.start + self.end) / 2.0, duration)
    }

    /// Keeps a moving playhead on screen by paging forward (or back).
    pub fn following(&self, playhead: f64, duration: f64) -> Self {
        if playhead > self.end || playhead < self.start {
            self.panned(playhead - self.start - self.span() * 0.05, duration)
        } else {
            *self
        }
    }
}

pub fn min_span(duration: f64) -> f64 {
    duration.max(0.0) / MAX_ZOOM
}

/// Pixel ↔ time mapping for the plotted strip `[left, left + width]`.
#[derive(Clone, Copy, Debug)]
pub struct Mapping {
    pub left: f32,
    pub width: f32,
    pub view: View,
}

impl Mapping {
    pub fn new(left: f32, width: f32, view: View) -> Self {
        Self {
            left,
            width: width.max(1.0),
            view,
        }
    }

    /// Unclamped: off-screen times map outside the strip.
    pub fn x_at(&self, time: f64) -> f32 {
        self.left + ((time - self.view.start) / self.view.span()) as f32 * self.width
    }

    /// Clamped to the visible range.
    pub fn time_at(&self, x: f32) -> f64 {
        let fraction = ((x - self.left) / self.width).clamp(0.0, 1.0) as f64;
        self.view.start + fraction * self.view.span()
    }

    pub fn visible(&self, time: f64) -> bool {
        let slack = self.view.span() * 1e-9;
        time >= self.view.start - slack && time <= self.view.end + slack
    }

    /// The trim edge under the pointer. Edges scrolled out of view are never
    /// hit, even though their (clamped) position would sit at the border.
    pub fn handle_at(&self, x: f32, start: f64, end: f64, grab: f32) -> Option<Handle> {
        let near = |time: f64| {
            self.visible(time)
                .then(|| (x - self.x_at(time)).abs())
                .filter(|d| *d <= grab)
        };
        match (near(start), near(end)) {
            (Some(ds), Some(de)) => Some(if de < ds || x > self.x_at(end) {
                Handle::End
            } else {
                Handle::Start
            }),
            (Some(_), None) => Some(Handle::Start),
            (None, Some(_)) => Some(Handle::End),
            (None, None) => None,
        }
    }
}

/// Overview bins that fall inside the view; fewer than the dot columns means
/// the picture is magnified overview data, not more detail.
pub fn visible_bins(bins: usize, view: View, duration: f64) -> f64 {
    bins as f64 * view.span() / duration.max(1e-9)
}

// ---- Export formats -----------------------------------------------------------

/// The engine's MP3 and AAC encoders take mono or stereo only.
pub fn supports(format: ExportFormat, channels: u32) -> bool {
    format == ExportFormat::Wav || channels <= 2
}

/// The chosen format, or WAV when this file can't be encoded that way. The
/// session choice itself is kept for the next file.
pub fn effective_format(chosen: ExportFormat, channels: u32) -> ExportFormat {
    if supports(chosen, channels) {
        chosen
    } else {
        ExportFormat::Wav
    }
}

/// Containers that usually carry video; opening one extracts its audio.
const VIDEO_EXTENSIONS: [&str; 14] = [
    "mov", "mp4", "m4v", "qt", "avi", "mkv", "webm", "3gp", "3g2", "mts", "m2ts", "mpg", "mpeg",
    "wmv",
];

/// Whether `path` looks like a video file. Only used for wording while it
/// opens; the engine decides what the file really holds.
pub fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| VIDEO_EXTENSIONS.iter().any(|v| ext.eq_ignore_ascii_case(v)))
}

pub fn format_detail(format: ExportFormat) -> &'static str {
    match format {
        ExportFormat::Wav => "Lossless · 24-bit",
        ExportFormat::Mp3 => "192 kbps",
        // AAC is variable bitrate; 192 kbps is the encoder's target.
        ExportFormat::M4a => "AAC · 192 kbps target",
    }
}

/// Makes the saved name carry the real format's extension. A typed extension
/// of another export format is replaced; anything else is kept and appended to.
pub fn with_format_extension(path: PathBuf, format: ExportFormat) -> PathBuf {
    let wanted = format.extension();
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case(wanted) => path,
        Some(ext)
            if FORMATS
                .iter()
                .any(|f| ext.eq_ignore_ascii_case(f.extension())) =>
        {
            path.with_extension(wanted)
        }
        Some(_) => {
            let mut name = path.file_name().unwrap_or_default().to_os_string();
            name.push(".");
            name.push(wanted);
            path.with_file_name(name)
        }
        None => path.with_extension(wanted),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: f64 = 120.0;

    fn edit(start: f64, end: f64, gain_db: f32) -> Edit {
        Edit {
            start,
            end,
            gain_db,
        }
    }

    #[test]
    fn double_click_start_resets_only_start() {
        let e = reset_edge(edit(12.5, 80.0, 4.5), Handle::Start, D);
        assert_eq!(e, edit(0.0, 80.0, 4.5));
    }

    #[test]
    fn double_click_end_resets_only_end() {
        let e = reset_edge(edit(12.5, 80.0, -3.0), Handle::End, D);
        assert_eq!(e, edit(12.5, D, -3.0));
    }

    #[test]
    fn resetting_an_edge_already_at_the_boundary_is_a_no_op() {
        let e = edit(0.0, D, 2.0);
        assert_eq!(reset_edge(e, Handle::Start, D), e);
        assert_eq!(reset_edge(e, Handle::End, D), e);
    }

    #[test]
    fn edge_drag_keeps_a_minimum_selection() {
        let e = edit(10.0, 20.0, 0.0);
        assert_eq!(
            drag_edge(e, Handle::Start, 25.0, D).start,
            20.0 - MIN_SELECTION
        );
        assert_eq!(drag_edge(e, Handle::End, 5.0, D).end, 10.0 + MIN_SELECTION);
        assert_eq!(drag_edge(e, Handle::Start, -4.0, D).start, 0.0);
        assert_eq!(drag_edge(e, Handle::End, 500.0, D).end, D);
        assert_eq!(drag_edge(e, Handle::End, 50.0, D).start, 10.0);
    }

    #[test]
    fn range_drag_orders_and_rejects_tiny_ranges() {
        assert_eq!(range_between(30.0, 10.0, D), Some((10.0, 30.0)));
        assert_eq!(range_between(10.0, 10.01, D), None);
        assert_eq!(range_between(110.0, 900.0, D), Some((110.0, D)));
    }

    #[test]
    fn in_and_out_preserve_existing_behaviour() {
        let e = edit(10.0, 20.0, 1.0);
        assert_eq!(set_in(e, 15.0, D), edit(15.0, 20.0, 1.0));
        assert_eq!(set_in(e, 30.0, D), edit(30.0, D, 1.0));
        assert_eq!(set_out(e, 15.0, D), edit(10.0, 15.0, 1.0));
        assert_eq!(set_out(e, 5.0, D), edit(0.0, 5.0, 1.0));
    }

    #[test]
    fn clicks_pick_toggle_and_extend_clips() {
        assert_eq!(
            click_selection(&[0, 2], Some(0), 3, false, false),
            (vec![3], Some(3))
        );
        assert_eq!(
            click_selection(&[0, 2], Some(0), 3, true, false),
            (vec![0, 2, 3], Some(3))
        );
        assert_eq!(
            click_selection(&[0, 2], Some(0), 2, true, false),
            (vec![0], Some(2))
        );
        assert_eq!(
            click_selection(&[1], Some(1), 4, false, true),
            (vec![1, 2, 3, 4], Some(1))
        );
        assert_eq!(
            click_selection(&[5], Some(5), 3, false, true),
            (vec![3, 4, 5], Some(5))
        );
        assert_eq!(
            click_selection(&[], None, 2, false, true),
            (vec![2], Some(2))
        );
    }

    #[test]
    fn cuts_keep_the_selection_on_the_same_audio() {
        // Clips 0..4, cutting clip 1 (its right half becomes clip 2).
        assert_eq!(selection_after_split(&[1], 2), vec![1, 2]);
        assert_eq!(selection_after_split(&[0, 3], 2), vec![0, 4]);
        assert_eq!(selection_after_split(&[2], 2), vec![3]);
        assert!(selection_after_split(&[], 1).is_empty());
    }

    #[test]
    fn ripple_delete_keeps_the_trim_on_its_audio() {
        // 100 frames at 10 Hz: clips 0–3 s, 3–7 s, 7–10 s.
        let mut t = Timeline::new(100);
        t.split(30);
        t.split(70);
        // Trim 2–8 s; deleting the middle clip leaves 2–4 s of what remains.
        let e = edit(2.0, 8.0, 3.0);
        assert_eq!(edit_after_delete(e, &t, &[1], 10.0), edit(2.0, 4.0, 3.0));
        // A whole-timeline trim stays whole.
        let full = edit(0.0, 10.0, 0.0);
        assert_eq!(edit_after_delete(full, &t, &[0], 10.0), edit(0.0, 7.0, 0.0));
        // Deleting exactly the trimmed audio keeps everything left.
        assert_eq!(
            edit_after_delete(edit(3.0, 7.0, 1.0), &t, &[1], 10.0),
            edit(0.0, 6.0, 1.0)
        );
        // Nothing left: an empty edit that keeps the gain.
        let empty = edit_after_delete(e, &t, &[0, 1, 2], 10.0);
        assert_eq!(empty, edit(0.0, 0.0, 3.0));
        assert!(empty.validate_frames(0, 10.0).is_err());
        assert_eq!(ripple_time(&t, &[1], 5.0, 10.0), 3.0);
        assert_eq!(ripple_time(&t, &[1], 9.0, 10.0), 5.0);
    }

    #[test]
    fn undo_snapshots_restore_arrangement_and_trim_together() {
        // The document's history stores (edit, timeline) pairs; restoring one
        // must give back a consistent, exportable state, even from empty.
        let mut t = Timeline::new(100);
        t.split(40);
        let e = edit(1.0, 9.0, 2.0);
        let snapshot = (e, t.clone());
        let after = edit_after_delete(e, &t, &[0, 1], 10.0);
        assert!(t.delete(&[0, 1]));
        assert!(after.validate_frames(t.frames(), 10.0).is_err());
        let (e, t) = snapshot;
        assert_eq!(t.frames(), 100);
        assert!(e.validate_frames(t.frames(), 10.0).is_ok());
        assert_eq!(t.clips().len(), 2);
    }

    #[test]
    fn gain_snapping() {
        assert_eq!(snap_gain(0.3), 0.0);
        assert_eq!(snap_gain(3.04), 3.0);
        assert_eq!(snap_gain(99.0), GAIN_MAX);
        assert_eq!(nudge_gain(0.2, GAIN_STEP), 0.5);
        assert_eq!(nudge_gain(GAIN_MAX, GAIN_STEP), GAIN_MAX);
    }

    #[test]
    fn original_preview_bypasses_gain_without_touching_the_edit() {
        let e = edit(0.0, D, 6.0);
        assert_eq!(preview_gain(e, true), 0.0);
        assert_eq!(preview_gain(e, false), 6.0);
        assert_eq!(e.gain_db, 6.0);
    }

    #[test]
    fn full_view_maps_edges_to_strip_edges() {
        let m = Mapping::new(100.0, 600.0, View::full(D));
        assert_eq!(m.x_at(0.0), 100.0);
        assert_eq!(m.x_at(D), 700.0);
        assert_eq!(m.time_at(400.0), 60.0);
        assert_eq!(m.time_at(0.0), 0.0);
        assert_eq!(m.time_at(10_000.0), D);
    }

    #[test]
    fn zoomed_view_maps_pointer_into_view_range() {
        let m = Mapping::new(
            0.0,
            500.0,
            View {
                start: 40.0,
                end: 50.0,
            },
        );
        assert_eq!(m.time_at(0.0), 40.0);
        assert_eq!(m.time_at(250.0), 45.0);
        assert_eq!(m.time_at(-30.0), 40.0);
        assert_eq!(m.time_at(900.0), 50.0);
        assert_eq!(m.x_at(45.0), 250.0);
        assert!(m.x_at(10.0) < 0.0);
    }

    #[test]
    fn offscreen_edges_are_not_hittable_at_the_border() {
        // Selection 10–90 s, viewing 40–50 s: both edges are off screen.
        let m = Mapping::new(
            16.0,
            500.0,
            View {
                start: 40.0,
                end: 50.0,
            },
        );
        assert_eq!(m.handle_at(16.0, 10.0, 90.0, 10.0), None);
        assert_eq!(m.handle_at(10.0, 10.0, 90.0, 10.0), None);
        assert_eq!(m.handle_at(516.0, 10.0, 90.0, 10.0), None);
        assert_eq!(m.handle_at(522.0, 10.0, 90.0, 10.0), None);
        // Once an edge is inside the view it is grabbable, even from the pad.
        assert_eq!(m.handle_at(10.0, 40.0, 90.0, 10.0), Some(Handle::Start));
        assert_eq!(m.handle_at(266.0, 10.0, 45.0, 10.0), Some(Handle::End));
    }

    #[test]
    fn overlapping_handles_prefer_the_side_of_the_pointer() {
        let m = Mapping::new(0.0, 1200.0, View::full(D));
        // 10 px per second: a 0.5 s selection puts both grab zones together.
        assert_eq!(m.handle_at(98.0, 10.0, 10.5, 10.0), Some(Handle::Start));
        assert_eq!(m.handle_at(107.0, 10.0, 10.5, 10.0), Some(Handle::End));
    }

    #[test]
    fn zoom_to_selection_frames_it_and_respects_limits() {
        let v = View::around(30.0, 60.0, D);
        assert!(v.start < 30.0 && v.end > 60.0);
        assert!((v.span() - 33.6).abs() < 1e-9);
        // A tiny selection stops at the zoom limit.
        let v = View::around(50.0, 50.05, D);
        assert!((v.span() - D / MAX_ZOOM).abs() < 1e-9);
        // Near the end the window slides back inside the file.
        let v = View::around(110.0, 120.0, D);
        assert_eq!(v.end, D);
        assert!(v.start >= 0.0);
    }

    #[test]
    fn zoom_keeps_anchor_fixed_and_pan_stays_inside() {
        let v = View::full(D).zoomed(4.0, 60.0, D);
        assert!((v.span() - 30.0).abs() < 1e-9);
        assert!((v.start - 45.0).abs() < 1e-9);
        let v = v.panned(1000.0, D);
        assert_eq!(v.end, D);
        let v = v.panned(-1000.0, D);
        assert_eq!(v.start, 0.0);
        assert!(View::full(D).zoomed(0.1, 10.0, D).is_full(D));
        let deepest = View::full(D).zoomed(1e6, 10.0, D);
        assert!((deepest.zoom(D) - MAX_ZOOM).abs() < 1e-6);
    }

    #[test]
    fn follow_pages_the_view_with_the_playhead() {
        let v = View {
            start: 0.0,
            end: 10.0,
        };
        assert_eq!(v.following(5.0, D), v);
        let next = v.following(10.5, D);
        assert!(next.start < 10.5 && next.end > 10.5);
        assert!((next.span() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn compressed_formats_require_mono_or_stereo() {
        assert!(supports(ExportFormat::Mp3, 2));
        assert!(!supports(ExportFormat::M4a, 6));
        assert!(supports(ExportFormat::Wav, 6));
        assert_eq!(effective_format(ExportFormat::Mp3, 6), ExportFormat::Wav);
        assert_eq!(effective_format(ExportFormat::M4a, 1), ExportFormat::M4a);
    }

    #[test]
    fn video_containers_are_recognised_by_extension() {
        assert!(is_video(Path::new("/a/Clip.MOV")));
        assert!(is_video(Path::new("/a/clip.mp4")));
        assert!(!is_video(Path::new("/a/song.m4a")));
        assert!(!is_video(Path::new("/a/song.wav")));
        assert!(!is_video(Path::new("/a/noext")));
    }

    #[test]
    fn m4a_is_described_as_a_bitrate_target() {
        assert!(format_detail(ExportFormat::M4a).contains("192 kbps target"));
    }

    #[test]
    fn saved_name_carries_the_chosen_extension() {
        let p = |s: &str| PathBuf::from(s);
        let mp3 = ExportFormat::Mp3;
        assert_eq!(with_format_extension(p("/a/b.mp3"), mp3), p("/a/b.mp3"));
        assert_eq!(with_format_extension(p("/a/b.MP3"), mp3), p("/a/b.MP3"));
        assert_eq!(with_format_extension(p("/a/b.wav"), mp3), p("/a/b.mp3"));
        assert_eq!(with_format_extension(p("/a/b"), mp3), p("/a/b.mp3"));
        assert_eq!(
            with_format_extension(p("/a/take.1"), mp3),
            p("/a/take.1.mp3")
        );
        let wav = ExportFormat::Wav;
        assert_eq!(with_format_extension(p("/a/b.m4a"), wav), p("/a/b.wav"));
    }
}
