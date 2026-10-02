//! Native arranged playback over a silent file: hundreds of short clips in
//! shuffled order must play back to back, with the playhead in timeline
//! time, finishing on schedule (no gaps at joins) and stopping on its own.
//! Needs an audio output device, so it is opt-in:
//! `cargo test --test arranged_playback -- --ignored`
use sonora::audio::{self, Player};
use sonora::timeline::Timeline;
use std::{
    fs,
    io::Write,
    time::{Duration, Instant},
};

#[test]
#[ignore = "needs an audio output device"]
fn many_short_clips_play_gaplessly_in_timeline_time() {
    let rate = 48000u32;
    let frames = rate as u64 * 4;
    let dir = std::env::temp_dir().join(format!("sonora-playback-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("silence.wav");
    let size = (frames * 2) as u32;
    let mut bytes = Vec::new();
    bytes.extend(b"RIFF");
    bytes.extend((36 + size).to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16u32.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(1u16.to_le_bytes());
    bytes.extend(rate.to_le_bytes());
    bytes.extend((rate * 2).to_le_bytes());
    bytes.extend(2u16.to_le_bytes());
    bytes.extend(16u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(size.to_le_bytes());
    bytes.resize(bytes.len() + size as usize, 0);
    fs::File::create(&path).unwrap().write_all(&bytes).unwrap();

    let info = audio::probe(&path).unwrap();
    // 25 ms clips over the first 3.5 s (140 joins), every other one moved
    // to the end, then the last half second deleted: more clips than the
    // engine's look-ahead window, so refills are exercised.
    let mut timeline = Timeline::new(info.frames);
    let step = rate as u64 / 40;
    let mut at = step;
    while at < rate as u64 * 7 / 2 {
        timeline.split(at);
        at += step;
    }
    let odd: Vec<usize> = (0..timeline.clips().len()).filter(|i| i % 2 == 1).collect();
    timeline.move_clips(&odd, timeline.clips().len()).unwrap();
    let last = timeline.clips().len() - 1;
    timeline.delete(&[last]);
    let duration = timeline.frames() as f64 / rate as f64;
    assert!(duration > 3.0);

    let mut player = Player::new().unwrap();
    player.load(&info.path).unwrap();
    let start = Instant::now();
    player
        .play_timeline(&info, &timeline, 0.5, duration, -24.0)
        .unwrap();
    let mut previous = 0.5;
    while player.is_playing() {
        let position = player.position();
        assert!(position + 1e-9 >= previous, "{position} < {previous}");
        assert!(position <= duration + 1e-6);
        previous = position;
        assert!(start.elapsed() < Duration::from_secs(10), "never finished");
        std::thread::sleep(Duration::from_millis(5));
    }
    let elapsed = start.elapsed().as_secs_f64();
    let expected = duration - 0.5;
    eprintln!("played {expected:.3}s of arranged audio in {elapsed:.3}s");
    assert!((player.position() - duration).abs() < 1e-6);
    // Gaps at joins would make playback run long.
    assert!(
        (elapsed - expected).abs() < 0.2,
        "played {expected:.3}s of audio in {elapsed:.3}s"
    );

    // Pause and resume in timeline time.
    player
        .play_timeline(&info, &timeline, 1.0, duration, -24.0)
        .unwrap();
    std::thread::sleep(Duration::from_millis(200));
    player.pause();
    let paused = player.position();
    assert!((1.0..duration).contains(&paused));
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(paused, player.position());
    player.stop();
    assert!(!player.is_playing());
    let _ = fs::remove_dir_all(&dir);
}
