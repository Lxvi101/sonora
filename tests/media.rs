use sonora::audio::{self, Edit, ExportFormat};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicU32},
};
fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}
#[test]
fn movie_audio_import_export_and_lifetime() {
    for name in ["aac.mp4", "pcm.mov"] {
        let source = fixture(name);
        let original = std::fs::read(&source).unwrap();
        let info = audio::prepare_media(&source, &AtomicBool::new(false)).unwrap();
        assert!(info.is_extracted());
        assert_eq!(info.source_path(), source.canonicalize().unwrap());
        assert!((info.duration() - 0.5).abs() < 0.05);
        assert_eq!(info.channels, 1);
        let backing = info.path.clone();
        let export = backing.parent().unwrap().join("export.wav");
        audio::export_audio(
            &info,
            Edit {
                start: 0.1,
                end: 0.3,
                gain_db: 6.,
            },
            &export,
            ExportFormat::Wav,
            &AtomicBool::new(false),
            &AtomicU32::new(0),
        )
        .unwrap();
        assert!((audio::probe(&export).unwrap().duration() - 0.2).abs() < 0.001);
        assert_eq!(std::fs::read(&source).unwrap(), original);
        let retained = info.clone();
        drop(info);
        assert!(backing.exists());
        drop(retained);
        assert!(!backing.parent().unwrap().exists());
    }
}
#[test]
fn video_without_audio_is_reported() {
    let err = audio::prepare_media(&fixture("no-audio.mp4"), &AtomicBool::new(false)).unwrap_err();
    assert!(err.to_string().contains("no readable audio track"), "{err}");
}
#[test]
fn import_can_be_cancelled() {
    let err = audio::prepare_media(&fixture("aac.mp4"), &AtomicBool::new(true)).unwrap_err();
    assert!(err.to_string().contains("cancelled"));
}
