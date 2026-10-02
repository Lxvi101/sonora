//! Silent playback integration check. Use a silent fixture to avoid audible output.
use sonora::audio::Player;
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};
fn main() -> anyhow::Result<()> {
    let path = PathBuf::from(std::env::args_os().nth(1).expect("silent audio path"));
    let mut player = Player::new()?;
    player.load(&path)?;
    player.play(3600., 3600.4, -24.)?;
    let start = Instant::now();
    let mut advanced = false;
    while start.elapsed() < Duration::from_secs(3) {
        let playing = player.is_playing();
        let pos = player.position();
        if pos > 3600.01 {
            advanced = true;
        }
        if !playing {
            assert!(advanced, "playhead did not advance");
            assert!((pos - 3600.4).abs() < 0.001);
            println!("playback, seek, automatic completion: OK");
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    anyhow::ensure!(
        !player.is_playing(),
        "Playback did not stop at selection end"
    );
    player.play(7200., 7201., 0.)?;
    std::thread::sleep(Duration::from_millis(150));
    player.pause();
    let pos = player.position();
    assert!(pos >= 7200. && pos < 7201.);
    std::thread::sleep(Duration::from_millis(50));
    assert_eq!(pos, player.position());
    assert!(!player.is_playing());
    player.play(pos, 7201., -6.)?;
    assert!(player.is_playing());
    player.set_gain(6.);
    player.stop();
    assert!(!player.is_playing());
    println!("pause, resume, gain, stop: OK");
    Ok(())
}
