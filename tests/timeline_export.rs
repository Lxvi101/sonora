//! Cut, ripple-delete and reorder, then export through the public API and
//! read the PCM back. Every source frame stores its own index, so the
//! exported sample order, length and gain are checked exactly.
use sonora::audio::{self, Edit, ExportFormat};
use sonora::timeline::Timeline;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicU32},
};

const RATE: u32 = 48000;
const FRAMES: u64 = 48000;

struct Dir(PathBuf);
impl Dir {
    fn new(name: &str) -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!(
            "sonora-timeline-{name}-{}-{stamp}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn header(frames: u64, channels: u16) -> Vec<u8> {
    let size = (frames * channels as u64 * 3) as u32;
    let mut h = Vec::new();
    h.extend(b"RIFF");
    h.extend((36 + size + size % 2).to_le_bytes());
    h.extend(b"WAVEfmt ");
    h.extend(16u32.to_le_bytes());
    h.extend(1u16.to_le_bytes());
    h.extend(channels.to_le_bytes());
    h.extend(RATE.to_le_bytes());
    h.extend((RATE * channels as u32 * 3).to_le_bytes());
    h.extend((channels * 3).to_le_bytes());
    h.extend(24u16.to_le_bytes());
    h.extend(b"data");
    h.extend(size.to_le_bytes());
    h
}

/// Stereo 24-bit WAV: left holds `+index`, right `-index`, in LSBs.
fn index_wav(path: &Path) {
    let mut bytes = header(FRAMES, 2);
    for i in 0..FRAMES as i32 {
        bytes.extend(&i.to_le_bytes()[..3]);
        bytes.extend(&(-i).to_le_bytes()[..3]);
    }
    fs::File::create(path).unwrap().write_all(&bytes).unwrap();
}

/// The exported WAV's frames as (left, right) integers.
fn read_frames(path: &Path) -> Vec<(i32, i32)> {
    let bytes = fs::read(path).unwrap();
    assert_eq!(&bytes[..4], b"RIFF");
    assert_eq!(u16::from_le_bytes([bytes[22], bytes[23]]), 2);
    assert_eq!(u32::from_le_bytes(bytes[24..28].try_into().unwrap()), RATE);
    assert_eq!(u16::from_le_bytes([bytes[34], bytes[35]]), 24);
    let size = u32::from_le_bytes(bytes[40..44].try_into().unwrap()) as usize;
    let sample = |b: &[u8]| i32::from_le_bytes([b[0], b[1], b[2], 0]) << 8 >> 8;
    bytes[44..44 + size]
        .as_chunks::<6>()
        .0
        .iter()
        .map(|f| (sample(&f[..3]), sample(&f[3..])))
        .collect()
}

fn export(info: &audio::AudioInfo, timeline: &Timeline, edit: Edit, path: &Path) {
    let progress = AtomicU32::new(0);
    audio::export_timeline(
        info,
        timeline,
        edit,
        path,
        ExportFormat::Wav,
        &AtomicBool::new(false),
        &progress,
    )
    .unwrap();
    assert_eq!(progress.into_inner(), 1000);
}

fn seconds(frames: u64) -> f64 {
    frames as f64 / RATE as f64
}

#[test]
fn cut_delete_reorder_export_exact_sample_order() {
    let dir = Dir::new("order");
    let source = dir.0.join("index.wav");
    index_wav(&source);
    let original = fs::read(&source).unwrap();
    let info = audio::probe(&source).unwrap();
    assert_eq!(info.frames, FRAMES);

    // Cut into A[0,10000) B[10000,20000) C[20000,30001) D[30001,48000),
    // at frame-exact positions (30001 is not on any round time).
    let mut timeline = Timeline::new(info.frames);
    for frame in [10_000, 20_000, 30_001] {
        assert!(timeline.split(frame).is_some());
    }
    // Ripple-delete B, then move D to the front: D A C.
    assert!(timeline.delete(&[1]));
    assert_eq!(timeline.move_clips(&[2], 0), Some(0..1));
    let expected: Vec<i32> = (30_001..48_000)
        .chain(0..10_000)
        .chain(20_000..30_001)
        .collect();
    assert_eq!(timeline.frames(), expected.len() as u64);

    // Whole arrangement, unity gain: every frame in order, nothing else.
    let whole = dir.0.join("whole.wav");
    let edit = Edit {
        start: 0.0,
        end: seconds(timeline.frames()),
        gain_db: 0.0,
    };
    export(&info, &timeline, edit, &whole);
    let frames = read_frames(&whole);
    assert_eq!(frames.len(), expected.len());
    for (i, (&(l, r), &want)) in frames.iter().zip(&expected).enumerate() {
        assert_eq!((l, r), (want, -want), "frame {i}");
    }
    assert_eq!(audio::probe(&whole).unwrap().frames, expected.len() as u64);

    // A trim across both joins, in timeline time, still frame exact.
    let (first, last) = (17_000u64, 30_000u64);
    let trimmed = dir.0.join("trimmed.wav");
    let edit = Edit {
        start: seconds(first),
        end: seconds(last),
        gain_db: 0.0,
    };
    export(&info, &timeline, edit, &trimmed);
    let frames = read_frames(&trimmed);
    assert_eq!(frames.len() as u64, last - first);
    for (i, &(l, _)) in frames.iter().enumerate() {
        assert_eq!(l, expected[first as usize + i], "trimmed frame {i}");
    }

    // Gain is applied exactly once: +6.02 dB doubles every sample.
    let louder = dir.0.join("louder.wav");
    let edit = Edit {
        start: 0.0,
        end: seconds(timeline.frames()),
        gain_db: 20.0 * 2f32.log10(),
    };
    export(&info, &timeline, edit, &louder);
    for (i, (&(l, r), &want)) in read_frames(&louder).iter().zip(&expected).enumerate() {
        assert!(
            (l - 2 * want).abs() <= 1,
            "gained frame {i}: {l} vs {}",
            2 * want
        );
        assert!((r + 2 * want).abs() <= 1, "gained frame {i}");
    }

    // Compressed exports follow the same arrangement and length.
    for format in [ExportFormat::Mp3, ExportFormat::M4a] {
        let path = dir.0.join(format!("arranged.{}", format.extension()));
        audio::export_timeline(
            &info,
            &timeline,
            Edit {
                start: 0.0,
                end: seconds(timeline.frames()),
                gain_db: 0.0,
            },
            &path,
            format,
            &AtomicBool::new(false),
            &AtomicU32::new(0),
        )
        .unwrap();
        let decoded = audio::probe(&path).unwrap();
        assert!(
            (decoded.duration() - seconds(timeline.frames())).abs() < 0.08,
            "{format:?}: {}",
            decoded.duration()
        );
    }

    assert_eq!(
        fs::read(&source).unwrap(),
        original,
        "the source is never written"
    );
}

#[test]
fn emptied_or_foreign_timelines_never_export() {
    let dir = Dir::new("empty");
    let source = dir.0.join("index.wav");
    index_wav(&source);
    let info = audio::probe(&source).unwrap();
    let mut timeline = Timeline::new(info.frames);
    timeline.split(24_000);
    let snapshot = timeline.clone();
    assert!(timeline.delete(&[0, 1]));
    assert!(timeline.is_empty());
    let out = dir.0.join("empty.wav");
    let edit = Edit {
        start: 0.0,
        end: 0.5,
        gain_db: 0.0,
    };
    let result = audio::export_timeline(
        &info,
        &timeline,
        edit,
        &out,
        ExportFormat::Wav,
        &AtomicBool::new(false),
        &AtomicU32::new(0),
    );
    assert!(result.is_err());
    assert!(!out.exists());
    // Undo is a snapshot: the restored arrangement exports again.
    export(&info, &snapshot, edit, &out);
    assert_eq!(read_frames(&out).len(), 24_000);
    // An arrangement made for another source is refused.
    let foreign = Timeline::new(info.frames - 1);
    let other = dir.0.join("foreign.wav");
    assert!(
        audio::export_timeline(
            &info,
            &foreign,
            edit,
            &other,
            ExportFormat::Wav,
            &AtomicBool::new(false),
            &AtomicU32::new(0),
        )
        .is_err()
    );
}

#[test]
fn legacy_export_matches_an_uncut_timeline() {
    let dir = Dir::new("legacy");
    let source = dir.0.join("index.wav");
    index_wav(&source);
    let info = audio::probe(&source).unwrap();
    let edit = Edit {
        start: 0.1,
        end: 0.4,
        gain_db: 0.0,
    };
    let legacy = dir.0.join("legacy.wav");
    audio::export_audio(
        &info,
        edit,
        &legacy,
        ExportFormat::Wav,
        &AtomicBool::new(false),
        &AtomicU32::new(0),
    )
    .unwrap();
    let arranged = dir.0.join("arranged.wav");
    export(&info, &Timeline::new(info.frames), edit, &arranged);
    assert_eq!(fs::read(&legacy).unwrap(), fs::read(&arranged).unwrap());
    let frames = read_frames(&legacy);
    assert_eq!(frames.len(), 14_400);
    assert_eq!(frames[0].0, 4_800);
}
