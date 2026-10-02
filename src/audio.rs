//! Native streaming audio. No full PCM buffers live in the application.
use anyhow::{Context, Result, bail, ensure};
use std::{
    ffi::{CString, c_char, c_void},
    fs::{self, File, OpenOptions},
    io::{BufWriter, Read, Write},
    marker::PhantomData,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    rc::Rc,
    sync::atomic::{AtomicBool, AtomicU32, Ordering},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

unsafe extern "C" {
    fn sonora_reader_open(
        path: *const c_char,
        rate: *mut f64,
        channels: *mut u32,
        frames: *mut u64,
        err: *mut c_char,
        cap: usize,
    ) -> *mut c_void;
    fn sonora_reader_seek(reader: *mut c_void, frame: u64) -> i32;
    fn sonora_reader_read(
        reader: *mut c_void,
        samples: *mut f32,
        capacity: u32,
        read: *mut u32,
    ) -> i32;
    fn sonora_reader_close(reader: *mut c_void);
    fn sonora_player_new() -> *mut c_void;
    fn sonora_player_load(
        player: *mut c_void,
        path: *const c_char,
        err: *mut c_char,
        cap: usize,
    ) -> i32;
    fn sonora_player_play(
        player: *mut c_void,
        start: f64,
        end: f64,
        db: f32,
        err: *mut c_char,
        cap: usize,
    ) -> i32;
    fn sonora_player_pause(player: *mut c_void);
    fn sonora_player_stop(player: *mut c_void);
    fn sonora_player_gain(player: *mut c_void, db: f32);
    fn sonora_player_position(player: *mut c_void) -> f64;
    fn sonora_player_is_playing(player: *mut c_void) -> i32;
    fn sonora_player_free(player: *mut c_void);
    fn sonora_reveal(path: *const c_char);
    fn sonora_apply_vibrancy();
}
fn cpath(path: &Path) -> Result<CString> {
    Ok(CString::new(path.as_os_str().as_bytes())?)
}
fn message(err: &[c_char]) -> String {
    let bytes: Vec<_> = err
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}
#[derive(Clone, Debug)]
pub struct AudioInfo {
    pub path: PathBuf,
    pub frames: u64,
    pub sample_rate: f64,
    pub channels: u32,
}
impl AudioInfo {
    pub fn duration(&self) -> f64 {
        self.frames as f64 / self.sample_rate
    }
}
struct Reader {
    ptr: *mut c_void,
    info: AudioInfo,
}
impl Reader {
    fn open(path: &Path) -> Result<Self> {
        let mut info = AudioInfo {
            path: fs::canonicalize(path).context("Audio file is unavailable")?,
            frames: 0,
            sample_rate: 0.,
            channels: 0,
        };
        let path = cpath(&info.path)?;
        let mut err = [0; 1024];
        let ptr = unsafe {
            sonora_reader_open(
                path.as_ptr(),
                &mut info.sample_rate,
                &mut info.channels,
                &mut info.frames,
                err.as_mut_ptr(),
                err.len(),
            )
        };
        ensure!(!ptr.is_null(), "{}", message(&err));
        Ok(Self { ptr, info })
    }
    fn seek(&mut self, frame: u64) -> Result<()> {
        ensure!(
            unsafe { sonora_reader_seek(self.ptr, frame) } == 0,
            "Could not seek audio"
        );
        Ok(())
    }
    fn read(&mut self, samples: &mut [f32]) -> Result<usize> {
        let capacity = (samples.len() / self.info.channels as usize) as u32;
        let mut read = 0;
        let status =
            unsafe { sonora_reader_read(self.ptr, samples.as_mut_ptr(), capacity, &mut read) };
        ensure!(status == 0, "Audio decoding failed (Core Audio {status})");
        Ok(read as usize)
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        unsafe { sonora_reader_close(self.ptr) }
    }
}
pub fn probe(path: &Path) -> Result<AudioInfo> {
    Ok(Reader::open(path)?.info.clone())
}
#[derive(Clone, Copy, Debug, Default)]
pub struct Peak {
    pub min: f32,
    pub max: f32,
}
#[derive(Clone, Debug)]
pub struct Waveform {
    pub peaks: Vec<Peak>,
    pub complete: bool,
    pub progress: f32,
}
const BINS: usize = 2048;
const CHUNK: usize = 16384;
fn accumulate(peak: &mut Peak, sample: f32) {
    if sample.is_finite() {
        peak.min = peak.min.min(sample);
        peak.max = peak.max.max(sample);
    }
}
fn cache_path(info: &AudioInfo) -> Option<PathBuf> {
    use std::hash::{Hash, Hasher};
    let meta = fs::metadata(&info.path).ok()?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    info.path.hash(&mut hash);
    meta.len().hash(&mut hash);
    meta.modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos()
        .hash(&mut hash);
    info.frames.hash(&mut hash);
    info.sample_rate.to_bits().hash(&mut hash);
    let root = if cfg!(test) {
        std::env::temp_dir().join("sonora-test-wave-cache")
    } else {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Caches/io.sonora.editor")
    };
    Some(root.join(format!("wave-v1-{:016x}.bin", hash.finish())))
}
fn cached(path: &Path) -> Option<Vec<Peak>> {
    let mut bytes = vec![0u8; BINS * 8];
    let mut file = File::open(path).ok()?;
    if file.metadata().ok()?.len() != bytes.len() as u64 {
        return None;
    }
    file.read_exact(&mut bytes).ok()?;
    let peaks: Vec<_> = bytes
        .as_chunks::<8>()
        .0
        .iter()
        .map(|x| Peak {
            min: f32::from_le_bytes(x[..4].try_into().unwrap()),
            max: f32::from_le_bytes(x[4..].try_into().unwrap()),
        })
        .collect();
    peaks
        .iter()
        .all(|p| p.min.is_finite() && p.max.is_finite())
        .then_some(peaks)
}
fn write_cache(path: &Path, peaks: &[Peak]) {
    // Bound disk cache to 128 overviews (~2 MiB); caches are expendable.
    if let Some(parent) = path.parent() {
        if fs::create_dir_all(parent).is_err() {
            return;
        }
        if let Ok(entries) = fs::read_dir(parent) {
            let mut files: Vec<_> = entries
                .flatten()
                .filter(|e| e.file_name().to_string_lossy().starts_with("wave-v1-"))
                .collect();
            files.sort_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
            let remove = files.len().saturating_sub(127);
            for entry in files.into_iter().take(remove) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    let mut bytes = Vec::with_capacity(BINS * 8);
    for p in peaks {
        bytes.extend(p.min.to_le_bytes());
        bytes.extend(p.max.to_le_bytes());
    }
    let _ = fs::write(path, bytes);
}
pub fn waveform(
    info: &AudioInfo,
    cancel: &AtomicBool,
    mut publish: impl FnMut(Waveform),
) -> Result<()> {
    if let Some(peaks) = cache_path(info).as_deref().and_then(cached) {
        publish(Waveform {
            peaks,
            complete: true,
            progress: 1.,
        });
        return Ok(());
    }
    let mut reader = Reader::open(&info.path)?;
    let channels = info.channels as usize;
    let mut peaks = vec![Peak::default(); BINS];
    let mut sample = vec![0.; 256 * channels];
    // Sparse overview: bounded work independent of source length, explicitly marked incomplete.
    for i in 0..128usize {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        reader.seek((info.frames.saturating_sub(256)) * i as u64 / 128)?;
        let n = reader.read(&mut sample)?;
        let mut peak = Peak::default();
        for &s in &sample[..n * channels] {
            accumulate(&mut peak, s)
        }
        for p in &mut peaks[i * 16..(i + 1) * 16] {
            *p = peak
        }
        if i % 32 == 31 {
            publish(Waveform {
                peaks: peaks.clone(),
                complete: false,
                progress: 0.,
            });
        }
    }
    reader.seek(0)?;
    let mut exact = vec![Peak::default(); BINS];
    let mut buffer = vec![0.; CHUNK * channels];
    let mut offset = 0u64;
    let mut last = Instant::now();
    while offset < info.frames {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        let mut local = 0usize;
        while local < n {
            let position = offset + local as u64;
            let bin =
                ((position as u128 * BINS as u128 / info.frames as u128) as usize).min(BINS - 1);
            let next = ((bin as u128 + 1) * info.frames as u128).div_ceil(BINS as u128) as u64;
            let count = ((next.saturating_sub(position)) as usize)
                .max(1)
                .min(n - local);
            for &s in &buffer[local * channels..(local + count) * channels] {
                accumulate(&mut exact[bin], s)
            }
            local += count;
        }
        offset += n as u64;
        if last.elapsed().as_millis() > 100 {
            let done = ((offset as u128 * BINS as u128 / info.frames as u128) as usize).min(BINS);
            peaks[..done].copy_from_slice(&exact[..done]);
            publish(Waveform {
                peaks: peaks.clone(),
                complete: false,
                progress: (offset as f64 / info.frames as f64).min(1.) as f32,
            });
            last = Instant::now();
        }
    }
    ensure!(
        offset >= info.frames,
        "Audio ended before its declared duration; waveform is incomplete"
    );
    if cancel.load(Ordering::Relaxed) {
        return Ok(());
    }
    if let Some(path) = cache_path(info) {
        write_cache(&path, &exact)
    }
    publish(Waveform {
        peaks: exact,
        complete: true,
        progress: 1.,
    });
    Ok(())
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Edit {
    pub start: f64,
    pub end: f64,
    pub gain_db: f32,
}
impl Edit {
    pub fn full(info: &AudioInfo) -> Self {
        Self {
            start: 0.,
            end: info.duration(),
            gain_db: 0.,
        }
    }
    pub fn validate(&self, info: &AudioInfo) -> Result<()> {
        ensure!(
            self.start.is_finite() && self.end.is_finite() && self.gain_db.is_finite(),
            "Invalid edit values"
        );
        ensure!(
            self.start >= 0. && self.end <= info.duration() + 1e-6 && self.end > self.start,
            "Choose a non-empty range within this recording"
        );
        ensure!(
            (-24.0..=24.0).contains(&self.gain_db),
            "Gain must be between -24 and +24 dB"
        );
        ensure!(
            (self.end * info.sample_rate).round() > (self.start * info.sample_rate).round(),
            "Selection must contain at least one sample"
        );
        Ok(())
    }
}
pub struct Player {
    ptr: *mut c_void,
    _main_thread: PhantomData<Rc<()>>,
}
impl Player {
    pub fn new() -> Result<Self> {
        let ptr = unsafe { sonora_player_new() };
        ensure!(!ptr.is_null(), "Could not initialize audio engine");
        Ok(Self {
            ptr,
            _main_thread: PhantomData,
        })
    }
    pub fn load(&mut self, path: &Path) -> Result<()> {
        let path = cpath(path)?;
        let mut err = [0; 1024];
        ensure!(
            unsafe { sonora_player_load(self.ptr, path.as_ptr(), err.as_mut_ptr(), err.len()) }
                == 0,
            "{}",
            message(&err)
        );
        Ok(())
    }
    pub fn play(&mut self, start: f64, end: f64, gain_db: f32) -> Result<()> {
        ensure!(
            start.is_finite() && end.is_finite() && gain_db.is_finite(),
            "Invalid playback values"
        );
        let mut err = [0; 1024];
        ensure!(
            unsafe {
                sonora_player_play(
                    self.ptr,
                    start,
                    end,
                    gain_db.clamp(-24., 24.),
                    err.as_mut_ptr(),
                    err.len(),
                )
            } == 0,
            "{}",
            message(&err)
        );
        Ok(())
    }
    pub fn pause(&mut self) {
        unsafe { sonora_player_pause(self.ptr) }
    }
    pub fn stop(&mut self) {
        unsafe { sonora_player_stop(self.ptr) }
    }
    pub fn set_gain(&mut self, db: f32) {
        if db.is_finite() {
            unsafe { sonora_player_gain(self.ptr, db.clamp(-24., 24.)) }
        }
    }
    pub fn position(&self) -> f64 {
        unsafe { sonora_player_position(self.ptr) }
    }
    pub fn is_playing(&self) -> bool {
        unsafe { sonora_player_is_playing(self.ptr) != 0 }
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        unsafe { sonora_player_free(self.ptr) }
    }
}
pub fn reveal(path: &Path) {
    if let Ok(path) = cpath(path) {
        unsafe { sonora_reveal(path.as_ptr()) }
    }
}
pub fn apply_vibrancy() {
    unsafe { sonora_apply_vibrancy() }
}

struct TempOutput(PathBuf);
impl Drop for TempOutput {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn wav_header(frames: u64, rate: u32, channels: u16) -> Result<Vec<u8>> {
    let size = frames
        .checked_mul(channels as u64 * 3)
        .context("Output is too large")?;
    ensure!(
        size <= u32::MAX as u64 - 37,
        "WAV export exceeds 4 GB. Export a shorter selection."
    );
    let padding = (size % 2) as u32;
    let mut h = Vec::with_capacity(44);
    h.extend(b"RIFF");
    h.extend((36 + size as u32 + padding).to_le_bytes());
    h.extend(b"WAVEfmt ");
    h.extend(16u32.to_le_bytes());
    h.extend(1u16.to_le_bytes());
    h.extend(channels.to_le_bytes());
    h.extend(rate.to_le_bytes());
    h.extend((rate * channels as u32 * 3).to_le_bytes());
    h.extend((channels * 3).to_le_bytes());
    h.extend(24u16.to_le_bytes());
    h.extend(b"data");
    h.extend((size as u32).to_le_bytes());
    Ok(h)
}
fn pcm24(sample: f32, gain: f32) -> [u8; 3] {
    let v = if sample.is_finite() {
        (sample * gain).clamp(-1., 1.)
    } else {
        0.
    };
    let n = (v as f64 * 8388608.).round().clamp(-8388608., 8388607.) as i32;
    let b = n.to_le_bytes();
    [b[0], b[1], b[2]]
}
pub fn export_wav(
    info: &AudioInfo,
    edit: Edit,
    destination: &Path,
    cancel: &AtomicBool,
    progress: &AtomicU32,
) -> Result<()> {
    edit.validate(info)?;
    ensure!(
        !destination.exists(),
        "This destination already exists. Choose a new name; originals are never overwritten."
    );
    let parent = fs::canonicalize(
        destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;
    let destination = parent.join(
        destination
            .file_name()
            .context("Choose an output filename")?,
    );
    ensure!(
        destination != fs::canonicalize(&info.path)?,
        "The source cannot be overwritten"
    );
    let first = (edit.start * info.sample_rate).round() as u64;
    let end = ((edit.end * info.sample_rate).round() as u64).min(info.frames);
    let frames = end - first;
    let header = wav_header(
        frames,
        info.sample_rate.round() as u32,
        info.channels as u16,
    )?;
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let temp = TempOutput(parent.join(format!(".sonora-{}-{stamp}.wav", std::process::id())));
    let mut out = BufWriter::with_capacity(
        256 * 1024,
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp.0)?,
    );
    out.write_all(&header)?;
    let mut reader = Reader::open(&info.path)?;
    ensure!(
        reader.info.frames == info.frames
            && reader.info.channels == info.channels
            && reader.info.sample_rate == info.sample_rate,
        "Source changed; reopen it before exporting"
    );
    reader.seek(first)?;
    let channels = info.channels as usize;
    let mut buffer = vec![0.; CHUNK * channels];
    let mut encoded = Vec::with_capacity(CHUNK * channels * 3);
    let gain = 10f32.powf(edit.gain_db / 20.);
    let mut done = 0;
    progress.store(0, Ordering::Relaxed);
    while done < frames {
        if cancel.load(Ordering::Relaxed) {
            bail!("Export cancelled")
        }
        let want = ((frames - done) as usize).min(CHUNK);
        let n = reader.read(&mut buffer[..want * channels])?;
        ensure!(n > 0, "Source ended early; no output was saved");
        encoded.clear();
        for &s in &buffer[..n * channels] {
            encoded.extend(pcm24(s, gain))
        }
        out.write_all(&encoded)?;
        done += n as u64;
        progress.store((done * 1000 / frames) as u32, Ordering::Relaxed);
    }
    if frames * channels as u64 * 3 % 2 == 1 {
        out.write_all(&[0])?
    }
    out.flush()?;
    out.get_ref().sync_all()?;
    drop(out);
    if cancel.load(Ordering::Relaxed) {
        bail!("Export cancelled")
    }
    // Same-directory hard link atomically publishes without ever replacing an existing file.
    fs::hard_link(&temp.0, &destination)
        .context("Could not save export (destination may already exist)")?;
    progress.store(1000, Ordering::Relaxed);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gain_and_clipping() {
        assert_eq!(pcm24(1., 2.), [255, 255, 127]);
        assert_eq!(pcm24(-1., 2.), [0, 0, 128]);
        assert_eq!(pcm24(f32::NAN, 1.), [0, 0, 0]);
        assert_eq!(pcm24(0.5, 1.), [0, 0, 64]);
    }
    #[test]
    fn rejects_invalid_edits() {
        let i = AudioInfo {
            path: PathBuf::new(),
            frames: 48000,
            sample_rate: 48000.,
            channels: 2,
        };
        assert!(Edit::full(&i).validate(&i).is_ok());
        for e in [
            Edit {
                start: 0.,
                end: 0.,
                gain_db: 0.,
            },
            Edit {
                start: f64::NAN,
                end: 1.,
                gain_db: 0.,
            },
            Edit {
                start: 0.,
                end: 2.,
                gain_db: 0.,
            },
            Edit {
                start: 0.,
                end: 1.,
                gain_db: 25.,
            },
        ] {
            assert!(e.validate(&i).is_err())
        }
    }
    #[test]
    fn wav_size_and_padding() {
        let h = wav_header(1, 48000, 1).unwrap();
        assert_eq!(h.len(), 44);
        assert_eq!(u32::from_le_bytes(h[4..8].try_into().unwrap()), 40);
        assert_eq!(u32::from_le_bytes(h[40..44].try_into().unwrap()), 3);
        assert!(wav_header(u32::MAX as u64, 48000, 2).is_err());
    }
}

#[cfg(test)]
mod pipeline_tests {
    use super::*;
    struct Fixture {
        dir: PathBuf,
        source: PathBuf,
    }
    impl Fixture {
        fn new(frames: u64) -> Self {
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir =
                std::env::temp_dir().join(format!("sonora-test-{}-{stamp}", std::process::id()));
            fs::create_dir_all(&dir).unwrap();
            let source = dir.join("tone.wav");
            let mut out = BufWriter::new(File::create(&source).unwrap());
            out.write_all(&wav_header(frames, 48000, 2).unwrap())
                .unwrap();
            for i in 0..frames {
                let s = (i as f32 * std::f32::consts::TAU * 440. / 48000.).sin() * 0.2;
                out.write_all(&pcm24(s, 1.)).unwrap();
                out.write_all(&pcm24(-s * 0.5, 1.)).unwrap();
            }
            out.flush().unwrap();
            Self { dir, source }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }
    #[test]
    fn native_trim_gain_and_source_preservation() {
        let fixture = Fixture::new(48000);
        let original = fs::read(&fixture.source).unwrap();
        let info = probe(&fixture.source).unwrap();
        assert_eq!(info.frames, 48000);
        assert_eq!(info.channels, 2);
        assert_eq!(info.duration(), 1.);
        let output = fixture.dir.join("trim.wav");
        let edit = Edit {
            start: 0.123,
            end: 0.456,
            gain_db: 6.,
        };
        let progress = AtomicU32::new(0);
        export_wav(&info, edit, &output, &AtomicBool::new(false), &progress).unwrap();
        assert_eq!(progress.load(Ordering::Relaxed), 1000);
        let result = probe(&output).unwrap();
        assert_eq!(result.frames, 15984);
        assert_eq!(result.sample_rate, 48000.);
        assert_eq!(result.channels, 2);
        let mut reader = Reader::open(&output).unwrap();
        let mut samples = vec![0.; 1024];
        let n = reader.read(&mut samples).unwrap();
        assert_eq!(n, 512);
        let gain = 10f32.powf(6. / 20.);
        for (i, f) in samples.chunks_exact(2).enumerate() {
            let expected =
                ((5904 + i) as f32 * std::f32::consts::TAU * 440. / 48000.).sin() * 0.2 * gain;
            assert!((f[0] - expected).abs() < 0.000001);
            assert!((f[1] + expected * 0.5).abs() < 0.000001);
        }
        assert_eq!(fs::read(&fixture.source).unwrap(), original);
        assert!(
            export_wav(
                &info,
                edit,
                &fixture.source,
                &AtomicBool::new(false),
                &progress
            )
            .is_err()
        );
        assert!(export_wav(&info, edit, &output, &AtomicBool::new(false), &progress).is_err());
    }
    #[test]
    fn cancellation_cleans_partial_export() {
        let f = Fixture::new(1000);
        let info = probe(&f.source).unwrap();
        let out = f.dir.join("cancel.wav");
        assert!(
            export_wav(
                &info,
                Edit::full(&info),
                &out,
                &AtomicBool::new(true),
                &AtomicU32::new(0)
            )
            .is_err()
        );
        assert!(!out.exists());
        assert_eq!(fs::read_dir(&f.dir).unwrap().count(), 1);
    }
    #[test]
    fn waveform_is_bounded_and_exact() {
        let f = Fixture::new(48000);
        let info = probe(&f.source).unwrap();
        let mut result = None;
        waveform(&info, &AtomicBool::new(false), |w| {
            assert_eq!(w.peaks.len(), 2048);
            if w.complete {
                result = Some(w)
            }
        })
        .unwrap();
        let result = result.unwrap();
        assert_eq!(result.progress, 1.);
        let peak = result
            .peaks
            .iter()
            .map(|p| p.max.max(-p.min))
            .fold(0., f32::max);
        assert!((peak - 0.2).abs() < 0.000001);
        let mut count = 0;
        waveform(&info, &AtomicBool::new(false), |w| {
            assert!(w.complete);
            count += 1
        })
        .unwrap();
        assert_eq!(count, 1);
        if let Some(path) = cache_path(&info) {
            let _ = fs::remove_file(path);
        }
    }
    #[test]
    fn multihour_metadata_does_not_decode_pcm() {
        let f = Fixture::new(1);
        let frames = 48000 * 60 * 60 * 3;
        let mut file = File::create(&f.source).unwrap();
        file.write_all(&wav_header(frames, 48000, 2).unwrap())
            .unwrap();
        file.set_len(44 + frames * 6).unwrap();
        drop(file);
        let info = probe(&f.source).unwrap();
        assert_eq!(info.duration(), 10800.);
        assert_eq!(info.frames, frames);
        let cancel = AtomicBool::new(false);
        let mut calls = 0;
        waveform(&info, &cancel, |w| {
            calls += 1;
            assert!(!w.complete);
            assert_eq!(w.peaks.len(), 2048);
            cancel.store(true, Ordering::Relaxed)
        })
        .unwrap();
        assert_eq!(calls, 1);
    }
    #[test]
    fn invalid_audio_is_rejected() {
        let f = Fixture::new(1);
        let path = f.dir.join("broken.wav");
        fs::write(&path, b"not audio").unwrap();
        assert!(probe(&path).is_err());
    }
}
