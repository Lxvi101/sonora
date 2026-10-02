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
    fn sonora_extract_audio(
        source: *const c_char,
        directory: *const c_char,
        cancelled: extern "C" fn(*mut c_void) -> i32,
        context: *mut c_void,
        kind: *mut i32,
        error: *mut c_char,
        cap: usize,
    ) -> i32;
    fn sonora_writer_open(
        path: *const c_char,
        kind: i32,
        rate: f64,
        channels: u32,
        frames: u64,
        err: *mut c_char,
        cap: usize,
    ) -> *mut c_void;
    fn sonora_writer_write(writer: *mut c_void, samples: *const f32, frames: u32) -> i32;
    fn sonora_writer_close(writer: *mut c_void, finish: i32) -> i32;

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
    fn sonora_player_play_ranges(
        player: *mut c_void,
        starts: *const u64,
        ends: *const u64,
        count: usize,
        timeline_start: f64,
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
    source: Option<PathBuf>,
    temporary: Option<std::sync::Arc<TemporaryMedia>>,
}
#[derive(Debug)]
struct TemporaryMedia(PathBuf);
impl Drop for TemporaryMedia {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
impl AudioInfo {
    pub fn source_path(&self) -> &Path {
        self.source.as_deref().unwrap_or(&self.path)
    }
    pub fn is_extracted(&self) -> bool {
        self.temporary.is_some()
    }
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
            source: None,
            temporary: None,
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

/// Open audio directly, or extract the first audio track of a movie on a worker.
/// Clones keep the private backing file alive for waveform/export jobs.
pub fn prepare_media(path: &Path, cancel: &AtomicBool) -> Result<AudioInfo> {
    ensure!(!cancel.load(Ordering::Relaxed), "Import cancelled");
    let source = fs::canonicalize(path).context("Media file is unavailable")?;
    let extension = source
        .extension()
        .and_then(|x| x.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let movie = matches!(
        extension.as_str(),
        "mp4" | "mov" | "m4v" | "avi" | "mkv" | "webm" | "mts" | "m2ts"
    );
    if !movie && let Ok(info) = probe(&source) {
        return Ok(info);
    }
    static NEXT_IMPORT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let root = std::env::temp_dir();
    let temporary = loop {
        let id = NEXT_IMPORT.fetch_add(1, Ordering::Relaxed);
        let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let dir = root.join(format!("sonora-media-{}-{stamp}-{id}", std::process::id()));
        match fs::create_dir(&dir) {
            Ok(()) => break std::sync::Arc::new(TemporaryMedia(dir)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    };
    extern "C" fn cancelled(context: *mut c_void) -> i32 {
        unsafe { (&*(context as *const AtomicBool)).load(Ordering::Relaxed) as i32 }
    }
    let mut kind = 0;
    let mut error = [0; 1024];
    let status = unsafe {
        sonora_extract_audio(
            cpath(&source)?.as_ptr(),
            cpath(&temporary.0)?.as_ptr(),
            cancelled,
            cancel as *const AtomicBool as *mut c_void,
            &mut kind,
            error.as_mut_ptr(),
            error.len(),
        )
    };
    ensure!(status == 0, "{}", message(&error));
    ensure!(!cancel.load(Ordering::Relaxed), "Import cancelled");
    let mut info = probe(
        &temporary
            .0
            .join(if kind == 0 { "audio.m4a" } else { "audio.caf" }),
    )?;
    info.source = Some(source);
    info.temporary = Some(temporary);
    Ok(info)
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
        self.validate_frames(info.frames, info.sample_rate)
    }
    /// Validates against a length in frames, such as an arranged timeline.
    pub fn validate_frames(&self, frames: u64, sample_rate: f64) -> Result<()> {
        ensure!(
            self.start.is_finite() && self.end.is_finite() && self.gain_db.is_finite(),
            "Invalid edit values"
        );
        ensure!(
            self.start >= 0.
                && self.end <= frames as f64 / sample_rate + 1e-6
                && self.end > self.start,
            "Choose a non-empty range within this recording"
        );
        ensure!(
            (-24.0..=24.0).contains(&self.gain_db),
            "Gain must be between -24 and +24 dB"
        );
        ensure!(
            (self.end * sample_rate).round() > (self.start * sample_rate).round(),
            "Selection must contain at least one sample"
        );
        Ok(())
    }
    /// The selected frames, rounded to the nearest sample and clamped to `frames`.
    pub fn frame_range(&self, sample_rate: f64, frames: u64) -> std::ops::Range<u64> {
        let first = (self.start * sample_rate).round() as u64;
        first..((self.end * sample_rate).round() as u64).min(frames)
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
    /// Play arranged source clips continuously, reporting position in timeline seconds.
    pub fn play_timeline(
        &mut self,
        info: &AudioInfo,
        timeline: &crate::timeline::Timeline,
        start: f64,
        end: f64,
        gain_db: f32,
    ) -> Result<()> {
        ensure!(
            timeline.source_frames() == info.frames,
            "Arrangement belongs to a different source"
        );
        let edit = Edit {
            start,
            end,
            gain_db,
        };
        edit.validate_frames(timeline.frames(), info.sample_rate)?;
        let frames = edit.frame_range(info.sample_rate, timeline.frames());
        let first = frames.start;
        let ranges = timeline.ranges(frames);
        let starts: Vec<_> = ranges.iter().map(|r| r.start).collect();
        let ends: Vec<_> = ranges.iter().map(|r| r.end).collect();
        let mut err = [0; 1024];
        ensure!(
            unsafe {
                sonora_player_play_ranges(
                    self.ptr,
                    starts.as_ptr(),
                    ends.as_ptr(),
                    starts.len(),
                    first as f64 / info.sample_rate,
                    gain_db,
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

/// The actual output codec, independent from the source filename.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    Wav,
    Mp3,
    M4a,
}
impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Mp3 => "mp3",
            Self::M4a => "m4a",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Wav => "WAV",
            Self::Mp3 => "MP3",
            Self::M4a => "M4A",
        }
    }
}
struct CompressedWriter(*mut c_void);
impl CompressedWriter {
    fn new(path: &Path, info: &AudioInfo, frames: u64, format: ExportFormat) -> Result<Self> {
        let path = cpath(path)?;
        let mut err = [0; 1024];
        let kind = match format {
            ExportFormat::Mp3 => 1,
            ExportFormat::M4a => 2,
            ExportFormat::Wav => unreachable!(),
        };
        let ptr = unsafe {
            sonora_writer_open(
                path.as_ptr(),
                kind,
                info.sample_rate,
                info.channels,
                frames,
                err.as_mut_ptr(),
                err.len(),
            )
        };
        ensure!(!ptr.is_null(), "{}", message(&err));
        Ok(Self(ptr))
    }
    fn write(&mut self, samples: &[f32], frames: usize) -> Result<()> {
        let status = unsafe { sonora_writer_write(self.0, samples.as_ptr(), frames as u32) };
        ensure!(
            status == 0,
            "Audio encoding failed ({status}); no output was saved"
        );
        Ok(())
    }
    fn finish(mut self) -> Result<()> {
        let ptr = std::mem::replace(&mut self.0, std::ptr::null_mut());
        let status = unsafe { sonora_writer_close(ptr, 1) };
        ensure!(
            status == 0,
            "Could not finish the encoded audio ({status}); no output was saved"
        );
        Ok(())
    }
}
impl Drop for CompressedWriter {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                sonora_writer_close(self.0, 0);
            }
        }
    }
}
/// Streams the trimmed arrangement in clip order; editing never materializes PCM.
pub fn export_timeline(
    info: &AudioInfo,
    timeline: &crate::timeline::Timeline,
    edit: Edit,
    destination: &Path,
    format: ExportFormat,
    cancel: &AtomicBool,
    progress: &AtomicU32,
) -> Result<()> {
    ensure!(
        timeline.source_frames() == info.frames,
        "Arrangement belongs to a different source"
    );
    edit.validate_frames(timeline.frames(), info.sample_rate)?;
    let ranges = timeline.ranges(edit.frame_range(info.sample_rate, timeline.frames()));
    export_ranges(
        info,
        &ranges,
        edit.gain_db,
        destination,
        format,
        cancel,
        progress,
    )
}

/// Streams edited PCM into a real WAV, MP3 or AAC/M4A encoder. No external
/// process, full-file intermediate, or unbounded PCM allocation is used.
pub fn export_audio(
    info: &AudioInfo,
    edit: Edit,
    destination: &Path,
    format: ExportFormat,
    cancel: &AtomicBool,
    progress: &AtomicU32,
) -> Result<()> {
    edit.validate(info)?;
    let frames = edit.frame_range(info.sample_rate, info.frames);
    let ranges = [crate::timeline::Clip {
        start: frames.start,
        end: frames.end,
    }];
    export_ranges(
        info,
        &ranges,
        edit.gain_db,
        destination,
        format,
        cancel,
        progress,
    )
}

fn export_ranges(
    info: &AudioInfo,
    ranges: &[crate::timeline::Clip],
    gain_db: f32,
    destination: &Path,
    format: ExportFormat,
    cancel: &AtomicBool,
    progress: &AtomicU32,
) -> Result<()> {
    if format == ExportFormat::Wav {
        return export_wav_ranges(info, ranges, gain_db, destination, cancel, progress);
    }
    ensure!(
        info.channels <= 2,
        "MP3 and M4A support mono or stereo. Choose WAV for multichannel audio."
    );
    ensure!(
        !destination.exists(),
        "This destination already exists. Choose a new name; originals are never overwritten."
    );
    if cancel.load(Ordering::Relaxed) {
        bail!("Export cancelled");
    }
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
    let frames: u64 = ranges.iter().map(|r| r.frames()).sum();
    let mut reader = Reader::open(&info.path)?;
    ensure!(
        reader.info.frames == info.frames
            && reader.info.channels == info.channels
            && reader.info.sample_rate == info.sample_rate,
        "Source changed; reopen it before exporting"
    );
    let stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let temp = TempOutput(parent.join(format!(
        ".sonora-{}-{stamp}.{}",
        std::process::id(),
        format.extension()
    )));
    // Reserve exclusively before the native encoder opens it. Only this private
    // sibling file can be replaced by the AAC container writer.
    drop(
        OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&temp.0)?,
    );
    let mut writer = CompressedWriter::new(&temp.0, info, frames, format)?;
    let channels = info.channels as usize;
    let mut buffer = vec![0.; CHUNK * channels];
    let gain = 10f32.powf(gain_db / 20.);
    let mut done = 0u64;
    progress.store(0, Ordering::Relaxed);
    for range in ranges {
        reader.seek(range.start)?;
        let stop = done + range.frames();
        while done < stop {
            if cancel.load(Ordering::Relaxed) {
                bail!("Export cancelled");
            }
            let want = (stop - done).min(CHUNK as u64) as usize;
            let n = reader.read(&mut buffer[..want * channels])?;
            ensure!(n > 0, "Source ended early; no output was saved");
            for value in &mut buffer[..n * channels] {
                *value = if value.is_finite() {
                    (*value * gain).clamp(-1., 1.)
                } else {
                    0.
                };
            }
            writer.write(&buffer[..n * channels], n)?;
            done += n as u64;
            progress.store((done * 990 / frames) as u32, Ordering::Relaxed);
        }
    }
    writer.finish()?;
    File::open(&temp.0)?.sync_all()?;
    if cancel.load(Ordering::Relaxed) {
        bail!("Export cancelled");
    }
    fs::hard_link(&temp.0, &destination)
        .context("Could not save export (destination may already exist)")?;
    progress.store(1000, Ordering::Relaxed);
    Ok(())
}

pub fn export_wav(
    info: &AudioInfo,
    edit: Edit,
    destination: &Path,
    cancel: &AtomicBool,
    progress: &AtomicU32,
) -> Result<()> {
    export_audio(info, edit, destination, ExportFormat::Wav, cancel, progress)
}

fn export_wav_ranges(
    info: &AudioInfo,
    ranges: &[crate::timeline::Clip],
    gain_db: f32,
    destination: &Path,
    cancel: &AtomicBool,
    progress: &AtomicU32,
) -> Result<()> {
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
    let frames: u64 = ranges.iter().map(|r| r.frames()).sum();
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
    let channels = info.channels as usize;
    let mut buffer = vec![0.; CHUNK * channels];
    let mut encoded = Vec::with_capacity(CHUNK * channels * 3);
    let gain = 10f32.powf(gain_db / 20.);
    let mut done = 0;
    progress.store(0, Ordering::Relaxed);
    for range in ranges {
        reader.seek(range.start)?;
        let stop = done + range.frames();
        while done < stop {
            if cancel.load(Ordering::Relaxed) {
                bail!("Export cancelled")
            }
            let want = ((stop - done) as usize).min(CHUNK);
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
            source: None,
            temporary: None,
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
            static NEXT_FIXTURE: std::sync::atomic::AtomicU64 =
                std::sync::atomic::AtomicU64::new(0);
            let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let dir = std::env::temp_dir().join(format!(
                "sonora-test-{}-{stamp}-{sequence}",
                std::process::id()
            ));
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
    fn arranged_export_matches_reordered_source_samples() {
        let fixture = Fixture::new(48000);
        let original = fs::read(&fixture.source).unwrap();
        let info = probe(&fixture.source).unwrap();
        let mut timeline = crate::timeline::Timeline::new(info.frames);
        timeline.split(12000);
        timeline.split(24000);
        timeline.split(36000);
        timeline.delete(&[1]);
        timeline.move_clip(2, 0);
        // Arrangement: source [36000..48000, 0..12000, 24000..36000].
        let edit = Edit {
            start: 0.125,
            end: 0.625,
            gain_db: 6.0,
        };
        let output = fixture.dir.join("arranged.wav");
        export_timeline(
            &info,
            &timeline,
            edit,
            &output,
            ExportFormat::Wav,
            &AtomicBool::new(false),
            &AtomicU32::new(0),
        )
        .unwrap();
        let result = probe(&output).unwrap();
        assert_eq!(result.frames, 24000);
        let mut reader = Reader::open(&output).unwrap();
        let mut samples = vec![0.0; 48000];
        assert_eq!(reader.read(&mut samples).unwrap(), 24000);
        let gain = 10f32.powf(6.0 / 20.0);
        for (i, pair) in samples.as_chunks::<2>().0.iter().enumerate() {
            let source = timeline.source_frame(6000 + i as u64).unwrap();
            let expected =
                (source as f32 * std::f32::consts::TAU * 440.0 / 48000.0).sin() * 0.2 * gain;
            assert!((pair[0] - expected).abs() < 0.000001, "frame {i}");
            assert!((pair[1] + expected * 0.5).abs() < 0.000001, "frame {i}");
        }
        assert_eq!(fs::read(&fixture.source).unwrap(), original);
        for format in [ExportFormat::Mp3, ExportFormat::M4a] {
            let path = fixture.dir.join(format!("arranged.{}", format.extension()));
            export_timeline(
                &info,
                &timeline,
                edit,
                &path,
                format,
                &AtomicBool::new(false),
                &AtomicU32::new(0),
            )
            .unwrap();
            let result = probe(&path).unwrap();
            assert!((result.duration() - 0.5).abs() < 0.1);
        }
        timeline.delete(&[0, 1, 2]);
        assert!(
            export_timeline(
                &info,
                &timeline,
                edit,
                &fixture.dir.join("empty.wav"),
                ExportFormat::Wav,
                &AtomicBool::new(false),
                &AtomicU32::new(0)
            )
            .is_err()
        );
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
        for (i, f) in samples.as_chunks::<2>().0.iter().enumerate() {
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
    fn compressed_exports_are_real_decodable_files() {
        let f = Fixture::new(96000);
        let before = fs::read(&f.source).unwrap();
        let info = probe(&f.source).unwrap();
        for format in [ExportFormat::Mp3, ExportFormat::M4a] {
            let path = f.dir.join(format!("trim.{}", format.extension()));
            let edit = Edit {
                start: 0.2,
                end: 1.4,
                gain_db: 6.0,
            };
            let progress = AtomicU32::new(0);
            export_audio(
                &info,
                edit,
                &path,
                format,
                &AtomicBool::new(false),
                &progress,
            )
            .unwrap();
            assert_eq!(progress.load(Ordering::Relaxed), 1000);
            let bytes = fs::read(&path).unwrap();
            match format {
                ExportFormat::Mp3 => assert!(bytes[0] == 0xff || &bytes[..3] == b"ID3"),
                ExportFormat::M4a => assert_eq!(&bytes[4..8], b"ftyp"),
                _ => unreachable!(),
            }
            let result = probe(&path).unwrap();
            assert!(
                (result.duration() - 1.2).abs() < 0.08,
                "{format:?}: {}",
                result.duration()
            );
            assert_eq!(result.channels, 2);
            let mut reader = Reader::open(&path).unwrap();
            let mut buffer = vec![0.; CHUNK * 2];
            let mut peak = 0f32;
            loop {
                let n = reader.read(&mut buffer).unwrap();
                if n == 0 {
                    break;
                }
                for s in &buffer[..n * 2] {
                    assert!(s.is_finite());
                    peak = peak.max(s.abs());
                }
            }
            assert!(
                peak > 0.35 && peak < 0.46,
                "{format:?} gain round-trip peak {peak}"
            );
            assert!(
                export_audio(
                    &info,
                    edit,
                    &path,
                    format,
                    &AtomicBool::new(false),
                    &progress
                )
                .is_err()
            );
        }
        assert_eq!(before, fs::read(&f.source).unwrap());
    }
    #[test]
    fn mono_compressed_export_resamples_supported_rates() {
        let f = Fixture::new(1);
        let mut out = File::create(&f.source).unwrap();
        out.write_all(&wav_header(22050, 22050, 1).unwrap())
            .unwrap();
        for i in 0..22050 {
            out.write_all(&pcm24((i as f32 * 0.125).sin() * 0.15, 1.))
                .unwrap();
        }
        drop(out);
        let info = probe(&f.source).unwrap();
        for format in [ExportFormat::Mp3, ExportFormat::M4a] {
            let path = f.dir.join(format!("mono.{}", format.extension()));
            export_audio(
                &info,
                Edit::full(&info),
                &path,
                format,
                &AtomicBool::new(false),
                &AtomicU32::new(0),
            )
            .unwrap();
            let result = probe(&path).unwrap();
            assert_eq!(result.channels, 1);
            assert_eq!(
                result.sample_rate,
                if format == ExportFormat::M4a {
                    44100.
                } else {
                    32000.
                }
            );
            assert!((result.duration() - 1.).abs() < 0.1);
        }
    }
    #[test]
    fn cancelled_compressed_export_never_publishes() {
        let f = Fixture::new(48000 * 10);
        let info = probe(&f.source).unwrap();
        for format in [ExportFormat::Mp3, ExportFormat::M4a] {
            let path = f.dir.join(format!("cancelled.{}", format.extension()));
            let cancel = AtomicBool::new(false);
            let progress = AtomicU32::new(0);
            std::thread::scope(|scope| {
                let job = scope.spawn(|| {
                    export_audio(&info, Edit::full(&info), &path, format, &cancel, &progress)
                });
                let start = Instant::now();
                while progress.load(Ordering::Relaxed) == 0 && start.elapsed().as_secs() < 5 {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
                cancel.store(true, Ordering::Relaxed);
                assert!(job.join().unwrap().is_err());
            });
            assert!(!path.exists());
            assert_eq!(fs::read_dir(&f.dir).unwrap().count(), 1);
        }
    }
    #[test]
    fn compressed_export_rejects_surround_without_writing() {
        let f = Fixture::new(1000);
        let mut info = probe(&f.source).unwrap();
        info.channels = 6;
        let path = f.dir.join("surround.mp3");
        let error = export_audio(
            &info,
            Edit::full(&info),
            &path,
            ExportFormat::Mp3,
            &AtomicBool::new(false),
            &AtomicU32::new(0),
        )
        .unwrap_err();
        assert!(error.to_string().contains("mono or stereo"));
        assert!(!path.exists());
    }
    #[test]
    fn invalid_audio_is_rejected() {
        let f = Fixture::new(1);
        let path = f.dir.join("broken.wav");
        fs::write(&path, b"not audio").unwrap();
        assert!(probe(&path).is_err());
    }
}
