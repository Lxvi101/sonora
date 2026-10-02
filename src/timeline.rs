//! Non-destructive, contiguous single-source clip arrangement.
//! Positions are sample frames; cutting and moving never decode or copy PCM.
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clip {
    pub start: u64,
    pub end: u64,
}
impl Clip {
    pub fn frames(self) -> u64 {
        self.end - self.start
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Timeline {
    source_frames: u64,
    clips: Vec<Clip>,
}
impl Timeline {
    pub fn new(source_frames: u64) -> Self {
        Self {
            source_frames,
            clips: if source_frames == 0 {
                vec![]
            } else {
                vec![Clip {
                    start: 0,
                    end: source_frames,
                }]
            },
        }
    }
    pub fn clips(&self) -> &[Clip] {
        &self.clips
    }
    pub fn frames(&self) -> u64 {
        self.clips.iter().map(|c| c.frames()).sum()
    }
    pub fn source_frames(&self) -> u64 {
        self.source_frames
    }
    pub fn is_empty(&self) -> bool {
        self.clips.is_empty()
    }
    /// Still the untouched source as one clip.
    pub fn is_original(&self) -> bool {
        match self.clips.as_slice() {
            [] => self.source_frames == 0,
            [only] => only.start == 0 && only.end == self.source_frames,
            _ => false,
        }
    }
    pub fn reset(&mut self) {
        *self = Self::new(self.source_frames);
    }
    /// Returns clip index and its offset on the arranged timeline. Boundaries
    /// belong to the following clip, and the exclusive end hits no clip.
    pub fn locate(&self, frame: u64) -> Option<(usize, u64)> {
        let mut offset = 0;
        for (index, clip) in self.clips.iter().enumerate() {
            if frame < offset + clip.frames() {
                return Some((index, offset));
            }
            offset += clip.frames();
        }
        None
    }
    pub fn source_frame(&self, frame: u64) -> Option<u64> {
        self.locate(frame)
            .map(|(i, offset)| self.clips[i].start + frame - offset)
    }
    /// Each clip with its start on the arranged timeline, in playback order.
    pub fn spans(&self) -> impl Iterator<Item = (u64, Clip)> + '_ {
        self.clips.iter().scan(0, |offset, &clip| {
            let start = *offset;
            *offset += clip.frames();
            Some((start, clip))
        })
    }
    /// The clip under a pointer: like `locate`, but the timeline's very end
    /// still belongs to the last clip.
    pub fn clip_at(&self, frame: u64) -> Option<usize> {
        self.locate(frame)
            .map(|(index, _)| index)
            .or_else(|| self.clips.len().checked_sub(1))
    }
    /// Timeline frame of a boundary (0 = start, `len` = end).
    pub fn boundary_frame(&self, boundary: usize) -> u64 {
        self.clips.iter().take(boundary).map(|c| c.frames()).sum()
    }
    /// The clip boundary nearest to `frame`; ties go to the earlier one.
    pub fn nearest_boundary(&self, frame: u64) -> usize {
        let mut best = (0, frame);
        let mut offset = 0;
        for (index, clip) in self.clips.iter().enumerate() {
            offset += clip.frames();
            let distance = offset.abs_diff(frame);
            if distance < best.1 {
                best = (index + 1, distance);
            }
        }
        best.0
    }
    /// Moves every listed clip, keeping their relative order, to a boundary
    /// of the current arrangement. Returns where the moved block now sits, or
    /// `None` when nothing would change.
    pub fn move_clips(&mut self, indices: &[usize], boundary: usize) -> Option<Range<usize>> {
        let len = self.clips.len();
        let mut picked = vec![false; len];
        for &i in indices {
            *picked.get_mut(i)? = true;
        }
        if boundary > len || !picked.contains(&true) {
            return None;
        }
        let at = (0..boundary).filter(|&i| !picked[i]).count();
        let (mut moved, mut rest) = (Vec::new(), Vec::new());
        for (clip, picked) in self.clips.iter().zip(&picked) {
            if *picked { &mut moved } else { &mut rest }.push(*clip);
        }
        let count = moved.len();
        rest.splice(at..at, moved);
        if rest == self.clips {
            return None;
        }
        self.clips = rest;
        Some(at..at + count)
    }
    /// Where a timeline frame lands once the listed clips are ripple-deleted.
    /// Frames inside a deleted clip collapse onto the join that replaces it.
    pub fn ripple(&self, indices: &[usize], frame: u64) -> u64 {
        let removed: u64 = self
            .spans()
            .enumerate()
            .filter(|(i, _)| indices.contains(i))
            .map(|(_, (start, clip))| frame.clamp(start, start + clip.frames()) - start)
            .sum();
        frame.min(self.frames()) - removed
    }
    /// Split strictly inside a clip. Returns the right-hand clip index.
    pub fn split(&mut self, frame: u64) -> Option<usize> {
        let (index, offset) = self.locate(frame)?;
        if frame == offset {
            return None;
        }
        let original = self.clips[index];
        let source = original.start + frame - offset;
        self.clips[index].end = source;
        self.clips.insert(
            index + 1,
            Clip {
                start: source,
                end: original.end,
            },
        );
        Some(index + 1)
    }
    /// Remove selected indices, automatically closing gaps. Invalid/duplicate
    /// indices are harmless, including when the selection contains every clip.
    pub fn delete(&mut self, indices: &[usize]) -> bool {
        let mut selected = vec![false; self.clips.len()];
        for &i in indices {
            if let Some(value) = selected.get_mut(i) {
                *value = true;
            }
        }
        let mut index = 0;
        let before = self.clips.len();
        self.clips.retain(|_| {
            let keep = !selected[index];
            index += 1;
            keep
        });
        before != self.clips.len()
    }
    /// Move one clip to a boundary indexed in the original arrangement (0 is
    /// before the first clip, len is after the last). No gaps or overlaps.
    pub fn move_clip(&mut self, from: usize, boundary: usize) -> bool {
        if from >= self.clips.len()
            || boundary > self.clips.len()
            || boundary == from
            || boundary == from + 1
        {
            return false;
        }
        let clip = self.clips.remove(from);
        self.clips.insert(
            if boundary > from {
                boundary - 1
            } else {
                boundary
            },
            clip,
        );
        true
    }
    /// Source ranges needed for a trimmed timeline interval. The output is in
    /// playback order and clips are never merged, preserving editable cuts.
    pub fn ranges(&self, range: Range<u64>) -> Vec<Clip> {
        let end = range.end.min(self.frames());
        if range.start >= end {
            return vec![];
        }
        let mut offset = 0;
        let mut result = Vec::new();
        for clip in &self.clips {
            let next = offset + clip.frames();
            let start = range.start.max(offset);
            let stop = end.min(next);
            if start < stop {
                result.push(Clip {
                    start: clip.start + start - offset,
                    end: clip.start + stop - offset,
                });
            }
            offset = next;
            if offset >= end {
                break;
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn splits_preserve_every_source_frame() {
        let mut t = Timeline::new(100);
        assert_eq!(t.split(0), None);
        assert_eq!(t.split(100), None);
        assert_eq!(t.split(30), Some(1));
        assert_eq!(t.split(30), None);
        assert_eq!(t.split(31), Some(2));
        assert_eq!(t.frames(), 100);
        for i in 0..100 {
            assert_eq!(t.source_frame(i), Some(i));
        }
        assert_eq!(t.source_frame(100), None);
    }
    #[test]
    fn delete_ripples_and_handles_duplicates() {
        let mut t = Timeline::new(100);
        t.split(30);
        t.split(70);
        assert!(t.delete(&[1, 1, 999]));
        assert_eq!(t.frames(), 60);
        assert_eq!(t.source_frame(29), Some(29));
        assert_eq!(t.source_frame(30), Some(70));
        assert!(!t.delete(&[9]));
    }
    #[test]
    fn reorder_maps_trim_to_source() {
        let mut t = Timeline::new(100);
        t.split(30);
        t.split(70);
        assert!(t.move_clip(2, 0));
        assert_eq!(
            t.ranges(20..50),
            vec![
                Clip {
                    start: 90,
                    end: 100
                },
                Clip { start: 0, end: 20 }
            ]
        );
        assert!(t.move_clip(0, 3));
        assert_eq!(t.source_frame(80), Some(80));
        assert!(!t.move_clip(1, 2));
        assert!(!t.move_clip(99, 0));
    }
    #[test]
    fn split_bounds_and_spans() {
        let mut t = Timeline::new(10);
        assert_eq!(t.split(1), Some(1));
        assert_eq!(t.split(9), Some(2));
        let spans: Vec<_> = t.spans().collect();
        assert_eq!(
            spans,
            vec![
                (0, Clip { start: 0, end: 1 }),
                (1, Clip { start: 1, end: 9 }),
                (9, Clip { start: 9, end: 10 }),
            ]
        );
        assert_eq!(t.clip_at(0), Some(0));
        assert_eq!(t.clip_at(9), Some(2));
        assert_eq!(t.clip_at(10), Some(2), "the end belongs to the last clip");
        assert_eq!(t.boundary_frame(0), 0);
        assert_eq!(t.boundary_frame(2), 9);
        assert_eq!(t.boundary_frame(3), 10);
        assert_eq!(Timeline::new(0).clip_at(0), None);
    }
    #[test]
    fn nearest_boundary_snaps_to_joins() {
        let mut t = Timeline::new(100);
        t.split(30);
        t.split(70);
        assert_eq!(t.nearest_boundary(0), 0);
        assert_eq!(t.nearest_boundary(14), 0);
        assert_eq!(t.nearest_boundary(15), 0, "ties go to the earlier boundary");
        assert_eq!(t.nearest_boundary(16), 1);
        assert_eq!(t.nearest_boundary(69), 2);
        assert_eq!(t.nearest_boundary(99), 3);
        assert_eq!(t.nearest_boundary(500), 3);
        assert_eq!(Timeline::new(0).nearest_boundary(7), 0);
    }
    #[test]
    fn moving_a_selection_keeps_order_and_reports_noops() {
        let mut t = Timeline::new(40);
        for f in [10, 20, 30] {
            t.split(f);
        }
        let [a, b, c, d] = [t.clips()[0], t.clips()[1], t.clips()[2], t.clips()[3]];
        // Boundaries touching the clip itself change nothing.
        assert_eq!(t.move_clips(&[1], 1), None);
        assert_eq!(t.move_clips(&[1], 2), None);
        assert_eq!(t.move_clips(&[0, 1], 0), None);
        assert_eq!(t.move_clips(&[9], 0), None);
        assert_eq!(t.move_clips(&[], 0), None);
        assert_eq!(t.move_clips(&[0], 5), None);
        // A scattered selection gathers as one block in its original order.
        assert_eq!(t.move_clips(&[3, 0], 4), Some(2..4));
        assert_eq!(t.clips(), &[b, c, a, d]);
        assert_eq!(t.move_clips(&[2, 3], 0), Some(0..2));
        assert_eq!(t.clips(), &[a, d, b, c]);
        assert_eq!(t.frames(), 40);
        // Single-clip moves agree with `move_clip`.
        let mut single = t.clone();
        assert!(single.move_clip(3, 1));
        assert_eq!(t.move_clips(&[3], 1), Some(1..2));
        assert_eq!(t, single);
    }
    #[test]
    fn ripple_maps_frames_across_deleted_clips() {
        let mut t = Timeline::new(100);
        t.split(30);
        t.split(70);
        let deleted = [1];
        assert_eq!(t.ripple(&deleted, 0), 0);
        assert_eq!(t.ripple(&deleted, 29), 29);
        assert_eq!(t.ripple(&deleted, 30), 30);
        assert_eq!(t.ripple(&deleted, 50), 30, "inside the gap: the join");
        assert_eq!(t.ripple(&deleted, 70), 30);
        assert_eq!(t.ripple(&deleted, 85), 45);
        assert_eq!(t.ripple(&deleted, 100), 60);
        assert_eq!(t.ripple(&[0, 2], 100), 40);
        assert_eq!(t.ripple(&[0, 1, 2], 64), 0);
        let mut after = t.clone();
        after.delete(&deleted);
        assert_eq!(after.frames(), t.ripple(&deleted, t.frames()));
        for frame in [0, 10, 29, 70, 99] {
            assert_eq!(
                after.source_frame(t.ripple(&deleted, frame)),
                t.source_frame(frame)
            );
        }
    }
    #[test]
    fn empty_can_be_restored_from_snapshot_or_reset() {
        let mut t = Timeline::new(100);
        t.split(50);
        let snapshot = t.clone();
        assert!(t.delete(&[0, 1]));
        assert!(t.is_empty());
        assert_eq!(t.locate(0), None);
        assert!(t.ranges(0..100).is_empty());
        t = snapshot;
        assert_eq!(t.clips().len(), 2);
        assert!(!t.is_original());
        t.reset();
        assert_eq!(t, Timeline::new(100));
        assert!(t.is_original());
        assert!(Timeline::new(0).is_empty());
    }
}
