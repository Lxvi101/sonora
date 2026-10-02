//! Export the first two seconds with +3 dB gain in each supported format.
//! Usage: cargo run --release --example export_check -- input.wav output-directory
use sonora::audio::{Edit, ExportFormat, export_audio, probe};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicU32},
};
fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let source = PathBuf::from(args.next().expect("source path"));
    let folder = PathBuf::from(args.next().expect("output directory"));
    std::fs::create_dir_all(&folder)?;
    let info = probe(&source)?;
    let edit = Edit {
        start: 0.,
        end: info.duration().min(2.),
        gain_db: 3.,
    };
    for format in [ExportFormat::Wav, ExportFormat::Mp3, ExportFormat::M4a] {
        let path = folder.join(format!("export-check.{}", format.extension()));
        export_audio(
            &info,
            edit,
            &path,
            format,
            &AtomicBool::new(false),
            &AtomicU32::new(0),
        )?;
        let decoded = probe(&path)?;
        println!(
            "{}: {:.4}s / {} Hz / {} channels",
            path.display(),
            decoded.duration(),
            decoded.sample_rate,
            decoded.channels
        );
    }
    Ok(())
}
