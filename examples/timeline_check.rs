//! Exercise native, contiguous clip playback. Pass a silent WAV of >= 2 seconds.
use sonora::{
    audio::{self, Player},
    timeline::Timeline,
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
fn main() -> anyhow::Result<()> {
    let path = PathBuf::from(std::env::args_os().nth(1).expect("silent audio path"));
    let info = audio::probe(&path)?;
    anyhow::ensure!(info.duration() >= 2.0, "Use >= 2 seconds of silence");
    let rate = info.sample_rate as u64;
    let mut timeline = Timeline::new(info.frames);
    for n in 1..8 {
        timeline.split(n * rate / 4);
    }
    timeline.delete(&[1, 3, 5]);
    timeline.move_clip(2, 0);
    let mut player = Player::new()?;
    player.load(&path)?;
    player.play_timeline(&info, &timeline, 0.1, 0.9, -24.0)?;
    let begin = Instant::now();
    let mut previous = 0.1;
    let mut crossed = false;
    while player.is_playing() && begin.elapsed() < Duration::from_secs(3) {
        let at = player.position();
        anyhow::ensure!(at + 0.001 >= previous, "Timeline playhead went backwards");
        crossed |= at > 0.55;
        previous = at;
        std::thread::sleep(Duration::from_millis(10));
    }
    anyhow::ensure!(
        crossed && !player.is_playing(),
        "Clip sequence did not complete"
    );
    anyhow::ensure!(
        (player.position() - 0.9).abs() < 0.001,
        "Wrong timeline end"
    );
    player.play_timeline(&info, &timeline, 0.2, 0.8, 0.0)?;
    std::thread::sleep(Duration::from_millis(120));
    player.pause();
    let paused = player.position();
    anyhow::ensure!(
        paused > 0.2 && paused < 0.8,
        "Pause position outside timeline"
    );
    player.play_timeline(&info, &timeline, paused, 0.8, 0.0)?;
    player.stop();
    println!("Native clip sequence, timeline clock, seek, pause/resume: OK");
    Ok(())
}
