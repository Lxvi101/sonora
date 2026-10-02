use sonora::audio::{self, Edit, ExportFormat};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicU32},
    time::Instant,
};
fn main() -> anyhow::Result<()> {
    let source = PathBuf::from(std::env::args_os().nth(1).expect("input media"));
    let start = Instant::now();
    let info = audio::prepare_media(&source, &AtomicBool::new(false))?;
    println!(
        "Opened {:.3}s, {} channels, {} Hz in {:?}; extracted={}, backing={}",
        info.duration(),
        info.channels,
        info.sample_rate,
        start.elapsed(),
        info.is_extracted(),
        info.path.display()
    );
    let mut player = audio::Player::new()?;
    player.load(&info.path)?;
    if let Some(output) = std::env::args_os().nth(2) {
        audio::export_audio(
            &info,
            Edit {
                start: 0.,
                end: info.duration().min(1.),
                gain_db: 0.,
            },
            &PathBuf::from(output),
            ExportFormat::Wav,
            &AtomicBool::new(false),
            &AtomicU32::new(0),
        )?;
    }
    Ok(())
}
