//! Usage: cargo run --release --example benchmark -- /path/to/audio.wav
use sonora::audio::{probe, waveform};
use std::{path::PathBuf, sync::atomic::AtomicBool, time::Instant};
fn main() -> anyhow::Result<()> {
    let path = PathBuf::from(std::env::args_os().nth(1).expect("audio path"));
    let start = Instant::now();
    let info = probe(&path)?;
    println!(
        "metadata_ms={:.3} duration_s={} channels={} sample_rate={}",
        start.elapsed().as_secs_f64() * 1000.,
        info.duration(),
        info.channels,
        info.sample_rate
    );
    let start = Instant::now();
    let mut first = true;
    waveform(&info, &AtomicBool::new(false), |wave| {
        if first {
            println!(
                "first_waveform_ms={:.3} bins={} cached={}",
                start.elapsed().as_secs_f64() * 1000.,
                wave.peaks.len(),
                wave.complete
            );
            first = false
        }
        if wave.complete {
            println!(
                "complete_waveform_ms={:.3}",
                start.elapsed().as_secs_f64() * 1000.
            )
        }
    })?;
    Ok(())
}
