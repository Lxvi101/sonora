//! Sonora's single-document window: open one audio file, trim it, raise or
//! lower its gain, preview, and export a new WAV, MP3 or M4A.
//!
//! All decoding work (probe, waveform, export) runs off the main thread. Each
//! opened file gets a fresh `generation`; results carrying an older generation
//! are discarded, and replacing a document cancels its waveform and export
//! jobs through their `AtomicBool` flags.

pub mod assets;
mod logic;
pub mod mailbox;
mod theme;
mod view;
mod wave;
mod widgets;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::{Duration, Instant};

use gpui::{
    App, AppContext, Bounds, Context, FocusHandle, KeyBinding, Menu, MenuItem, OsAction,
    PathPromptOptions, Subscription, SystemMenuType, Task, TitlebarOptions, Window,
    WindowBackgroundAppearance, WindowBounds, WindowOptions, actions, point, px, size,
};

use crate::audio::{self, AudioInfo, Edit, ExportFormat, Peak, Player, Waveform};
use logic::{GAIN_STEP, Handle, View};

actions!(
    sonora,
    [
        Open,
        Export,
        UseWav,
        UseMp3,
        UseM4a,
        CloseFile,
        RevealSource,
        Quit,
        Hide,
        HideOthers,
        ShowAll,
        Minimize,
        Zoom,
        Undo,
        Redo,
        SelectAll,
        ResetEdits,
        ResetStart,
        ResetEnd,
        SetIn,
        SetOut,
        PlayPause,
        ToggleLoop,
        ToggleOriginal,
        SeekBack,
        SeekForward,
        SeekBackFar,
        SeekForwardFar,
        NudgeBack,
        NudgeForward,
        GoToStart,
        GoToEnd,
        GainUp,
        GainDown,
        ResetGain,
        ZoomToSelection,
        ZoomToFit,
        ZoomIn,
        ZoomOut,
        ToggleHelp,
        Cancel,
    ]
);

const UNDO_LIMIT: usize = 200;
/// Pointer travel before a press on a trim handle becomes a drag, so the
/// first click of a double-click never nudges the edge.
const HANDLE_SLOP: f32 = 2.5;
/// Gain-slider travel (dB) before a grab becomes a drag.
const GAIN_SLOP: f32 = 0.8;

/// Registers key bindings, menus and app-wide actions.
pub fn init(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());

    cx.bind_keys([
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("alt-cmd-h", HideOthers, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("cmd-o", Open, None),
        KeyBinding::new("cmd-shift-s", Export, None),
        KeyBinding::new("cmd-1", UseWav, None),
        KeyBinding::new("cmd-2", UseMp3, None),
        KeyBinding::new("cmd-3", UseM4a, None),
        KeyBinding::new("cmd-w", CloseFile, None),
        KeyBinding::new("cmd-shift-r", RevealSource, None),
        KeyBinding::new("cmd-z", Undo, None),
        KeyBinding::new("cmd-shift-z", Redo, None),
        KeyBinding::new("cmd-a", SelectAll, None),
        KeyBinding::new("cmd-backspace", ResetEdits, None),
        KeyBinding::new("i", SetIn, None),
        KeyBinding::new("o", SetOut, None),
        KeyBinding::new("shift-i", ResetStart, None),
        KeyBinding::new("shift-o", ResetEnd, None),
        KeyBinding::new("space", PlayPause, None),
        KeyBinding::new("l", ToggleLoop, None),
        KeyBinding::new("b", ToggleOriginal, None),
        KeyBinding::new("left", SeekBack, None),
        KeyBinding::new("right", SeekForward, None),
        KeyBinding::new("shift-left", SeekBackFar, None),
        KeyBinding::new("shift-right", SeekForwardFar, None),
        KeyBinding::new("alt-left", NudgeBack, None),
        KeyBinding::new("alt-right", NudgeForward, None),
        KeyBinding::new("home", GoToStart, None),
        KeyBinding::new("up", GoToStart, None),
        KeyBinding::new("end", GoToEnd, None),
        KeyBinding::new("down", GoToEnd, None),
        KeyBinding::new("=", GainUp, None),
        // "+" matches by typed character, so it works for shift-= on US
        // layouts, keypad +, and layouts with a dedicated + key.
        KeyBinding::new("+", GainUp, None),
        KeyBinding::new("-", GainDown, None),
        KeyBinding::new("0", ResetGain, None),
        KeyBinding::new("z", ZoomToSelection, None),
        KeyBinding::new("shift-z", ZoomToFit, None),
        KeyBinding::new("cmd-0", ZoomToFit, None),
        KeyBinding::new("cmd-=", ZoomIn, None),
        KeyBinding::new("cmd-+", ZoomIn, None),
        KeyBinding::new("cmd--", ZoomOut, None),
        KeyBinding::new("?", ToggleHelp, None),
        KeyBinding::new("cmd-/", ToggleHelp, None),
        KeyBinding::new("escape", Cancel, None),
    ]);

    cx.set_menus(menus(ExportFormat::Wav));
}

/// The menu bar. Rebuilt when the export format changes so the Export item
/// always names the format it will write.
fn menus(format: ExportFormat) -> Vec<Menu> {
    let choice = |f: ExportFormat| {
        let mark = if f == format { "✓ " } else { "    " };
        format!("{mark}{} — {}", f.label(), logic::format_detail(f))
    };
    vec![
        Menu {
            name: "Sonora".into(),
            items: vec![
                MenuItem::os_submenu("Services", SystemMenuType::Services),
                MenuItem::separator(),
                MenuItem::action("Hide Sonora", Hide),
                MenuItem::action("Hide Others", HideOthers),
                MenuItem::action("Show All", ShowAll),
                MenuItem::separator(),
                MenuItem::action("Quit Sonora", Quit),
            ],
        },
        Menu {
            name: "File".into(),
            items: vec![
                MenuItem::action("Open…", Open),
                MenuItem::separator(),
                MenuItem::action(format!("Export {}…", format.label()), Export),
                MenuItem::submenu(Menu {
                    name: "Export Format".into(),
                    items: vec![
                        MenuItem::action(choice(ExportFormat::Wav), UseWav),
                        MenuItem::action(choice(ExportFormat::Mp3), UseMp3),
                        MenuItem::action(choice(ExportFormat::M4a), UseM4a),
                    ],
                }),
                MenuItem::action("Show Original in Finder", RevealSource),
                MenuItem::separator(),
                MenuItem::action("Close File", CloseFile),
            ],
        },
        Menu {
            name: "Edit".into(),
            items: vec![
                MenuItem::os_action("Undo", Undo, OsAction::Undo),
                MenuItem::os_action("Redo", Redo, OsAction::Redo),
                MenuItem::separator(),
                MenuItem::os_action("Select All", SelectAll, OsAction::SelectAll),
                MenuItem::action("Set Start at Playhead", SetIn),
                MenuItem::action("Set End at Playhead", SetOut),
                MenuItem::action("Reset Start to Beginning", ResetStart),
                MenuItem::action("Reset End to File End", ResetEnd),
                MenuItem::separator(),
                MenuItem::action("Louder", GainUp),
                MenuItem::action("Quieter", GainDown),
                MenuItem::action("Reset Gain to 0 dB", ResetGain),
                MenuItem::separator(),
                MenuItem::action("Reset Trim and Gain", ResetEdits),
            ],
        },
        Menu {
            name: "Playback".into(),
            items: vec![
                MenuItem::action("Play / Pause", PlayPause),
                MenuItem::action("Loop Selection", ToggleLoop),
                MenuItem::action("Compare with Original (A/B)", ToggleOriginal),
                MenuItem::separator(),
                MenuItem::action("Go to Selection Start", GoToStart),
                MenuItem::action("Go to Selection End", GoToEnd),
                MenuItem::action("Back 1 Second", SeekBack),
                MenuItem::action("Forward 1 Second", SeekForward),
                MenuItem::action("Back 5 Seconds", SeekBackFar),
                MenuItem::action("Forward 5 Seconds", SeekForwardFar),
                MenuItem::action("Nudge Back 10 ms", NudgeBack),
                MenuItem::action("Nudge Forward 10 ms", NudgeForward),
            ],
        },
        Menu {
            name: "View".into(),
            items: vec![
                MenuItem::action("Zoom to Selection", ZoomToSelection),
                MenuItem::action("Zoom In", ZoomIn),
                MenuItem::action("Zoom Out", ZoomOut),
                MenuItem::action("Show Entire File", ZoomToFit),
            ],
        },
        Menu {
            name: "Window".into(),
            items: vec![
                MenuItem::action("Minimize", Minimize),
                MenuItem::action("Zoom", Zoom),
            ],
        },
        Menu {
            name: "Help".into(),
            items: vec![MenuItem::action("Keyboard & Gestures", ToggleHelp)],
        },
    ]
}

/// Opens the main window, then feeds it command-line paths and every later
/// Finder "Open With…" request.
pub fn open_main_window(
    cx: &mut App,
    initial: Vec<PathBuf>,
    mut open_requests: mailbox::Receiver<Vec<String>>,
) -> anyhow::Result<()> {
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(940.), px(570.)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some("Sonora".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(14.), px(14.))),
        }),
        window_background: WindowBackgroundAppearance::Blurred,
        window_min_size: Some(size(px(640.), px(420.))),
        focus: true,
        show: true,
        ..Default::default()
    };
    let handle = cx.open_window(options, |window, cx| cx.new(|cx| Sonora::new(window, cx)))?;
    audio::apply_vibrancy();

    // A utility with one window: closing it ends the app.
    cx.on_window_closed(|cx| cx.quit()).detach();

    handle.update(cx, |sonora, window, cx| {
        window.focus(&sonora.focus);
        if !initial.is_empty() {
            sonora.open_paths(initial, cx);
        }
    })?;

    cx.spawn(async move |cx| {
        while let Some(urls) = open_requests.recv().await {
            let paths: Vec<PathBuf> = urls.iter().filter_map(|url| path_from_url(url)).collect();
            if paths.is_empty() {
                eprintln!("Sonora: ignored open request with no usable file path: {urls:?}");
                continue;
            }
            let opened = handle.update(cx, |sonora, window, cx| {
                sonora.open_paths(paths, cx);
                window.activate_window();
                sonora.refocus(window);
            });
            // Keep listening: one failed delivery must not end Finder
            // "Open With" for the rest of the session.
            if let Err(err) = opened {
                eprintln!("Sonora: couldn't deliver open request {urls:?}: {err:#}");
            }
        }
    })
    .detach();
    Ok(())
}

/// Accepts `file://` URLs (percent-encoded) and plain paths.
fn path_from_url(url: &str) -> Option<PathBuf> {
    let Some(rest) = url.strip_prefix("file://") else {
        return (!url.contains("://")).then(|| PathBuf::from(url));
    };
    let path = rest.strip_prefix("localhost").unwrap_or(rest);
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(value) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(value);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).ok().map(PathBuf::from)
}

pub struct Sonora {
    focus: FocusHandle,
    player: Option<Player>,
    generation: u64,
    phase: Phase,
    notice: Option<Notice>,
    drag: Option<Drag>,
    hover: Option<Handle>,
    dialog_open: bool,
    window_title: String,
    /// Export format chosen this session; survives opening other files.
    format: ExportFormat,
    /// The format the menu bar currently names.
    menu_format: ExportFormat,
    /// Loop the selection during preview. A session preference.
    looping: bool,
    help: bool,
    /// A gain-track click that jumped the thumb: (before, after). A
    /// double-click that follows folds it into one "reset to 0 dB" step.
    gain_click: Option<(Edit, Edit)>,
    _subscriptions: Vec<Subscription>,
}

enum Phase {
    Empty,
    Opening(Import),
    Ready(Box<Document>),
}

/// A file being probed or, for video, having its audio extracted. Dropping
/// it (replaced, cancelled, window closed) stops the background work.
struct Import {
    name: String,
    /// A known video container, so the caption can say what's happening.
    video: bool,
    started: Instant,
    cancel: Arc<AtomicBool>,
    _task: Task<()>,
}

impl Drop for Import {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub(crate) struct Document {
    info: AudioInfo,
    name: String,
    edit: Edit,
    undo: Vec<Edit>,
    redo: Vec<Edit>,
    /// Playhead while paused, absolute seconds.
    cursor: f64,
    playing: bool,
    playable: bool,
    /// Visible time range of the waveform.
    view: View,
    /// A/B: preview (and draw) the original level. Never touches the edit.
    original: bool,
    peaks: Option<Arc<Vec<Peak>>>,
    /// Whether `peaks` are the exact scan or the sparse preview.
    peaks_exact: bool,
    wave_progress: f32,
    wave_error: Option<String>,
    wave_cancel: Arc<AtomicBool>,
    _wave_task: Task<()>,
    export: Option<ExportJob>,
}

impl Document {
    fn duration(&self) -> f64 {
        self.info.duration()
    }

    fn preview_gain(&self) -> f32 {
        logic::preview_gain(self.edit, self.original)
    }

    fn is_edited(&self) -> bool {
        self.edit != Edit::full(&self.info)
    }
}

impl Drop for Document {
    fn drop(&mut self) {
        self.wave_cancel.store(true, Ordering::Relaxed);
    }
}

struct ExportJob {
    destination: PathBuf,
    format: ExportFormat,
    cancel: Arc<AtomicBool>,
    progress: Arc<AtomicU32>,
    shown: u32,
    _task: Task<()>,
}

impl Drop for ExportJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

enum Notice {
    Error(String),
    Info(String),
    Saved(PathBuf),
}

#[derive(Clone, Copy, Debug)]
enum Drag {
    Handle {
        handle: Handle,
        origin: Edit,
        anchor_x: f32,
        /// Edge time minus pointer time at the press, so the edge doesn't
        /// jump to the pointer when grabbed off-center.
        offset: f64,
        moved: bool,
    },
    Range {
        origin: Edit,
        anchor: f64,
        anchor_x: f32,
        moved: bool,
    },
    Gain {
        origin: Edit,
        /// Thumb dB minus pointer dB at the press.
        offset: f32,
        pointer: f32,
        jumped: bool,
        moved: bool,
    },
    /// Dragging in the overview strip.
    Pan,
}

impl Sonora {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        // Coming back from the Open/Save panels, another app or a Finder
        // "Open With", the window must own keyboard focus again or no
        // shortcut reaches the editor.
        let activation = cx.observe_window_activation(window, |this, window, _| {
            if window.is_window_active() {
                window.focus(&this.focus);
            }
        });
        Self {
            focus: cx.focus_handle(),
            player: None,
            generation: 0,
            phase: Phase::Empty,
            notice: None,
            drag: None,
            hover: None,
            dialog_open: false,
            window_title: String::new(),
            format: ExportFormat::Wav,
            menu_format: ExportFormat::Wav,
            looping: false,
            help: false,
            gain_click: None,
            _subscriptions: vec![activation],
        }
    }

    /// Hands keyboard focus back to the editor root.
    fn refocus(&self, window: &mut Window) {
        window.focus(&self.focus);
    }

    fn doc(&self) -> Option<&Document> {
        match &self.phase {
            Phase::Ready(doc) => Some(doc),
            _ => None,
        }
    }

    fn doc_mut(&mut self) -> Option<&mut Document> {
        match &mut self.phase {
            Phase::Ready(doc) => Some(doc),
            _ => None,
        }
    }

    fn error(&mut self, message: impl Into<String>, cx: &mut Context<Self>) {
        self.notice = Some(Notice::Error(message.into()));
        cx.notify();
    }

    /// The format an export would write right now.
    fn export_format(&self) -> ExportFormat {
        match self.doc() {
            Some(doc) => logic::effective_format(self.format, doc.info.channels),
            None => self.format,
        }
    }

    fn can_use(&self, format: ExportFormat) -> bool {
        self.doc()
            .is_none_or(|doc| logic::supports(format, doc.info.channels))
    }

    fn set_format(&mut self, format: ExportFormat, cx: &mut Context<Self>) {
        if self.can_use(format) && self.format != format {
            self.format = format;
            cx.notify();
        }
    }

    /// Keeps the menu bar's Export item naming the real format.
    fn sync_menus(&mut self, cx: &mut Context<Self>) {
        let format = self.export_format();
        if format != self.menu_format {
            self.menu_format = format;
            cx.set_menus(menus(format));
        }
    }

    // ---- Opening -------------------------------------------------------

    fn prompt_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog_open {
            return;
        }
        self.dialog_open = true;
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some("Open".into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let chosen = paths.await;
            this.update_in(cx, |this, window, cx| {
                this.dialog_open = false;
                window.activate_window();
                this.refocus(window);
                match chosen {
                    Ok(Ok(Some(paths))) => this.open_paths(paths, cx),
                    Ok(Err(err)) => this.error(format!("Couldn't show the Open panel: {err}"), cx),
                    _ => cx.notify(),
                }
            })
            .ok();
        })
        .detach();
    }

    /// Single-document: the first path wins.
    pub fn open_paths(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let Some(path) = paths.into_iter().next() else {
            return;
        };
        if path.is_dir() {
            self.error("That's a folder. Choose an audio or video file.", cx);
            return;
        }
        self.open(path, cx);
    }

    fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.stop_playback();
        self.generation += 1;
        let generation = self.generation;
        self.drag = None;
        self.hover = None;
        self.notice = None;
        self.gain_click = None;

        let cancel = Arc::new(AtomicBool::new(false));
        let job = cx.background_executor().spawn({
            let (path, cancel) = (path.clone(), cancel.clone());
            async move { audio::prepare_media(&path, &cancel) }
        });
        let import = Import {
            name: display_name(&path),
            video: logic::is_video(&path),
            started: Instant::now(),
            cancel,
            _task: cx.spawn(async move |this, cx| {
                let result = job.await;
                this.update(cx, |this, cx| this.probed(generation, path, result, cx))
                    .ok();
            }),
        };
        // Replacing the phase drops the previous document or import,
        // cancelling its jobs.
        self.phase = Phase::Opening(import);
        cx.notify();
    }

    /// Escape (or Cancel) while a file is opening: stop and go back to empty.
    fn cancel_open(&mut self, cx: &mut Context<Self>) {
        if matches!(self.phase, Phase::Opening(_)) {
            self.generation += 1;
            self.phase = Phase::Empty;
            self.notice = None;
            cx.notify();
        }
    }

    fn probed(
        &mut self,
        generation: u64,
        path: PathBuf,
        result: anyhow::Result<AudioInfo>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.generation {
            return;
        }
        let video = logic::is_video(&path);
        let info = match result {
            Ok(info) if info.frames > 0 && info.duration() > 0.0 => info,
            Ok(_) => {
                self.phase = Phase::Empty;
                let what = if video { "no audio track" } else { "no audio" };
                self.error(format!("“{}” has {what}.", display_name(&path)), cx);
                return;
            }
            Err(err) => {
                self.phase = Phase::Empty;
                let verb = if video { "get audio from" } else { "open" };
                self.error(
                    format!(
                        "Couldn't {verb} “{}”. {}",
                        display_name(&path),
                        sentence(&err)
                    ),
                    cx,
                );
                return;
            }
        };
        if info.is_extracted() {
            self.notice = Some(Notice::Info(format!(
                "Using the first audio track of “{}”. Exports are audio only.",
                display_name(info.source_path())
            )));
        }

        // Preview becomes available as soon as metadata is in; the waveform
        // streams in behind it.
        let playable = match self.ensure_player() {
            Ok(player) => match player.load(&info.path) {
                Ok(()) => true,
                Err(err) => {
                    self.notice = Some(Notice::Error(format!(
                        "Preview unavailable. {}",
                        sentence(&err)
                    )));
                    false
                }
            },
            Err(err) => {
                self.notice = Some(Notice::Error(format!(
                    "No audio output. {}",
                    sentence(&err)
                )));
                false
            }
        };

        let wave_cancel = Arc::new(AtomicBool::new(false));
        let wave_task = self.spawn_waveform(generation, info.clone(), wave_cancel.clone(), cx);
        self.phase = Phase::Ready(Box::new(Document {
            name: display_name(info.source_path()),
            edit: Edit::full(&info),
            view: View::full(info.duration()),
            info,
            undo: Vec::new(),
            redo: Vec::new(),
            cursor: 0.0,
            playing: false,
            playable,
            original: false,
            peaks: None,
            peaks_exact: false,
            wave_progress: 0.0,
            wave_error: None,
            wave_cancel,
            _wave_task: wave_task,
            export: None,
        }));
        cx.notify();
    }

    fn ensure_player(&mut self) -> anyhow::Result<&mut Player> {
        if self.player.is_none() {
            self.player = Some(Player::new()?);
        }
        Ok(self.player.as_mut().expect("player was just created"))
    }

    fn spawn_waveform(
        &self,
        generation: u64,
        info: AudioInfo,
        cancel: Arc<AtomicBool>,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let (tx, mut rx) = mailbox::channel::<Waveform>();
        let job = cx.background_executor().spawn(async move {
            // `tx` moves into the closure and drops when the scan returns,
            // which closes the channel below.
            audio::waveform(&info, &cancel, move |snapshot| tx.send(snapshot))
        });
        cx.spawn(async move |this, cx| {
            while let Some(snapshot) = rx.recv_latest().await {
                let alive = this.update(cx, |this, cx| {
                    this.waveform_arrived(generation, snapshot, cx)
                });
                if alive.is_err() {
                    return;
                }
            }
            let result = job.await;
            this.update(cx, |this, cx| {
                this.waveform_finished(generation, result, cx)
            })
            .ok();
        })
    }

    fn waveform_arrived(&mut self, generation: u64, snapshot: Waveform, cx: &mut Context<Self>) {
        if generation != self.generation {
            return;
        }
        if let Some(doc) = self.doc_mut() {
            doc.wave_progress = snapshot.progress.clamp(0.0, 1.0);
            doc.peaks_exact = snapshot.complete;
            doc.peaks = Some(Arc::new(snapshot.peaks));
            cx.notify();
        }
    }

    fn waveform_finished(
        &mut self,
        generation: u64,
        result: anyhow::Result<()>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.generation {
            return;
        }
        if let Some(doc) = self.doc_mut() {
            match result {
                Ok(()) if doc.peaks.is_some() => {
                    doc.peaks_exact = true;
                    doc.wave_progress = 1.0;
                }
                Ok(()) => doc.wave_error = Some("No waveform data was produced.".into()),
                Err(err) if !doc.wave_cancel.load(Ordering::Relaxed) => {
                    doc.wave_error = Some(sentence(&err));
                }
                Err(_) => {}
            }
            cx.notify();
        }
    }

    fn close_file(&mut self, cx: &mut Context<Self>) {
        // Closing also abandons a file that is still opening.
        self.stop_playback();
        self.generation += 1;
        self.phase = Phase::Empty;
        self.notice = None;
        self.drag = None;
        self.hover = None;
        self.gain_click = None;
        cx.notify();
    }

    // ---- Playback ------------------------------------------------------

    /// Current playhead, absolute seconds.
    fn position(&self) -> f64 {
        match (self.doc(), &self.player) {
            (Some(doc), Some(player)) if doc.playing => player.position(),
            (Some(doc), _) => doc.cursor,
            _ => 0.0,
        }
    }

    fn stop_playback(&mut self) {
        if let Some(player) = &mut self.player {
            player.stop();
        }
        if let Some(doc) = self.doc_mut() {
            doc.playing = false;
        }
    }

    fn toggle_playback(&mut self, cx: &mut Context<Self>) {
        let position = self.position();
        let Phase::Ready(doc) = &mut self.phase else {
            return;
        };
        let Some(player) = self.player.as_mut().filter(|_| doc.playable) else {
            return;
        };
        if doc.playing {
            doc.cursor = player.position();
            player.pause();
            doc.playing = false;
        } else {
            let Edit { start, end, .. } = doc.edit;
            let from = if position >= start && position < end - 0.01 {
                position
            } else {
                start
            };
            match player.play(from, end, doc.preview_gain()) {
                Ok(()) => {
                    doc.playing = true;
                    doc.cursor = from;
                }
                Err(err) => {
                    self.notice = Some(Notice::Error(format!("Couldn't play. {}", sentence(&err))));
                }
            }
        }
        cx.notify();
    }

    /// Moves the playhead; keeps playing from there when playing.
    fn seek_to(&mut self, time: f64, cx: &mut Context<Self>) {
        let Phase::Ready(doc) = &mut self.phase else {
            return;
        };
        let time = time.clamp(0.0, doc.duration());
        if doc.playing
            && let Some(player) = self.player.as_mut()
        {
            let Edit { start, end, .. } = doc.edit;
            let from = time.clamp(start, (end - 0.01).max(start));
            doc.cursor = from;
            if let Err(err) = player.play(from, end, doc.preview_gain()) {
                doc.playing = false;
                self.notice = Some(Notice::Error(format!("Couldn't play. {}", sentence(&err))));
            }
        } else {
            doc.cursor = time;
        }
        cx.notify();
    }

    fn seek_by(&mut self, delta: f64, cx: &mut Context<Self>) {
        let target = self.position() + delta;
        self.seek_to(target, cx);
    }

    /// After an edit, follow the new gain and keep playback inside the
    /// (possibly new) selection.
    fn sync_playback(&mut self, before: Edit) {
        let Phase::Ready(doc) = &mut self.phase else {
            return;
        };
        let Some(player) = self.player.as_mut() else {
            return;
        };
        let Edit { start, end, .. } = doc.edit;
        let gain = doc.preview_gain();
        player.set_gain(gain);
        if !doc.playing || (start == before.start && end == before.end) {
            return;
        }
        let at = player.position();
        if at >= start && at < end - 0.01 {
            if player.play(at, end, gain).is_err() {
                player.pause();
                doc.playing = false;
                doc.cursor = at;
            }
        } else {
            player.pause();
            doc.playing = false;
            doc.cursor = start;
        }
    }

    /// Called every frame while playing (from render) to follow the engine,
    /// loop or stop at the end of the selection, and keep a zoomed view on
    /// the playhead.
    fn tick_playback(&mut self) -> bool {
        let (looping, dragging) = (self.looping, self.drag.is_some());
        let Phase::Ready(doc) = &mut self.phase else {
            return false;
        };
        if !doc.playing {
            return false;
        }
        let Some(player) = self.player.as_mut() else {
            doc.playing = false;
            return false;
        };
        let mut alive = player.is_playing();
        if !alive && looping {
            alive = player
                .play(doc.edit.start, doc.edit.end, doc.preview_gain())
                .is_ok();
        }
        if !alive {
            doc.playing = false;
            doc.cursor = doc.edit.start;
            return false;
        }
        if !dragging {
            doc.view = doc.view.following(player.position(), doc.duration());
        }
        true
    }

    fn toggle_loop(&mut self, cx: &mut Context<Self>) {
        self.looping = !self.looping;
        cx.notify();
    }

    fn toggle_original(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.doc_mut() else { return };
        doc.original = !doc.original;
        let gain = doc.preview_gain();
        if let Some(player) = &mut self.player {
            player.set_gain(gain);
        }
        cx.notify();
    }

    // ---- Edits ---------------------------------------------------------

    fn record(doc: &mut Document, before: Edit) {
        if doc.edit != before {
            doc.undo.push(before);
            if doc.undo.len() > UNDO_LIMIT {
                doc.undo.remove(0);
            }
            doc.redo.clear();
        }
    }

    /// Applies a discrete edit as one undo step.
    fn apply_edit(&mut self, edit: Edit, cx: &mut Context<Self>) {
        let Some(doc) = self.doc_mut() else { return };
        if edit.validate(&doc.info).is_err() {
            return;
        }
        let before = doc.edit;
        doc.edit = edit;
        Self::record(doc, before);
        self.sync_playback(before);
        cx.notify();
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.doc_mut() else { return };
        let Some(previous) = doc.undo.pop() else {
            return;
        };
        let before = doc.edit;
        doc.redo.push(before);
        doc.edit = previous;
        self.sync_playback(before);
        cx.notify();
    }

    fn redo(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.doc_mut() else { return };
        let Some(next) = doc.redo.pop() else { return };
        let before = doc.edit;
        doc.undo.push(before);
        doc.edit = next;
        self.sync_playback(before);
        cx.notify();
    }

    fn set_in(&mut self, cx: &mut Context<Self>) {
        let at = self.position();
        let Some(doc) = self.doc() else { return };
        let edit = logic::set_in(doc.edit, at, doc.duration());
        self.apply_edit(edit, cx);
    }

    fn set_out(&mut self, cx: &mut Context<Self>) {
        let at = self.position();
        let Some(doc) = self.doc() else { return };
        let edit = logic::set_out(doc.edit, at, doc.duration());
        self.apply_edit(edit, cx);
    }

    /// Sends one trim edge back to the file boundary (double-click on a
    /// handle, ⇧I, ⇧O). The other edge and the gain are untouched.
    fn reset_edge(&mut self, handle: Handle, cx: &mut Context<Self>) {
        self.drag = None;
        let Some(doc) = self.doc() else { return };
        let edit = logic::reset_edge(doc.edit, handle, doc.duration());
        self.apply_edit(edit, cx);
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.doc() else { return };
        let edit = Edit {
            gain_db: doc.edit.gain_db,
            ..Edit::full(&doc.info)
        };
        self.apply_edit(edit, cx);
    }

    fn reset_edits(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.doc() else { return };
        let full = Edit::full(&doc.info);
        self.apply_edit(full, cx);
    }

    fn reset_gain(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.doc() else { return };
        let edit = Edit {
            gain_db: 0.0,
            ..doc.edit
        };
        self.apply_edit(edit, cx);
    }

    fn nudge_gain(&mut self, delta: f32, cx: &mut Context<Self>) {
        let Some(doc) = self.doc() else { return };
        let edit = Edit {
            gain_db: logic::nudge_gain(doc.edit.gain_db, delta),
            ..doc.edit
        };
        self.apply_edit(edit, cx);
    }

    /// Live gain while dragging; recorded as one step when the drag ends.
    fn set_gain(&mut self, db: f32, cx: &mut Context<Self>) {
        let Some(doc) = self.doc_mut() else { return };
        doc.edit.gain_db = db.clamp(logic::GAIN_MIN, logic::GAIN_MAX);
        let gain = doc.preview_gain();
        if let Some(player) = &mut self.player {
            player.set_gain(gain);
        }
        cx.notify();
    }

    // ---- View ----------------------------------------------------------

    fn set_view(&mut self, view: View, cx: &mut Context<Self>) {
        if let Some(doc) = self.doc_mut()
            && doc.view != view
        {
            doc.view = view;
            cx.notify();
        }
    }

    fn zoom_to_selection(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.doc() else { return };
        let view = View::around(doc.edit.start, doc.edit.end, doc.duration());
        self.set_view(view, cx);
    }

    fn zoom_to_fit(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.doc() else { return };
        let view = View::full(doc.duration());
        self.set_view(view, cx);
    }

    /// Zooms around `anchor`, or the playhead when it's on screen.
    fn zoom_view(&mut self, factor: f64, anchor: Option<f64>, cx: &mut Context<Self>) {
        let playhead = self.position();
        let Some(doc) = self.doc() else { return };
        let anchor = anchor.unwrap_or(if playhead >= doc.view.start && playhead <= doc.view.end {
            playhead
        } else {
            (doc.view.start + doc.view.end) / 2.0
        });
        let view = doc.view.zoomed(factor, anchor, doc.duration());
        self.set_view(view, cx);
    }

    fn pan_view(&mut self, seconds: f64, cx: &mut Context<Self>) {
        let Some(doc) = self.doc() else { return };
        let view = doc.view.panned(seconds, doc.duration());
        self.set_view(view, cx);
    }

    fn toggle_help(&mut self, cx: &mut Context<Self>) {
        self.help = !self.help;
        cx.notify();
    }

    // ---- Pointer gestures (driven by the canvases) ---------------------

    fn begin_handle_drag(&mut self, handle: Handle, x: f32, time: f64, cx: &mut Context<Self>) {
        let Some(doc) = self.doc() else { return };
        let edge = match handle {
            Handle::Start => doc.edit.start,
            Handle::End => doc.edit.end,
        };
        self.drag = Some(Drag::Handle {
            handle,
            origin: doc.edit,
            anchor_x: x,
            offset: edge - time,
            moved: false,
        });
        cx.notify();
    }

    fn begin_range_drag(&mut self, time: f64, x: f32, cx: &mut Context<Self>) {
        let Some(doc) = self.doc() else { return };
        self.drag = Some(Drag::Range {
            origin: doc.edit,
            anchor: time,
            anchor_x: x,
            moved: false,
        });
        self.seek_to(time, cx);
    }

    fn begin_pan(&mut self, time: f64, cx: &mut Context<Self>) {
        let Some(doc) = self.doc() else { return };
        let view = doc.view.centered_on(time, doc.duration());
        self.drag = Some(Drag::Pan);
        self.set_view(view, cx);
    }

    /// `jump`: the track was pressed away from the thumb, so the gain moves
    /// there at once. Otherwise the thumb was grabbed and keeps its offset.
    fn begin_gain_drag(&mut self, jump: Option<f32>, pointer: f32, cx: &mut Context<Self>) {
        let Some(doc) = self.doc() else { return };
        let origin = doc.edit;
        self.gain_click = None;
        match jump {
            Some(db) => {
                self.drag = Some(Drag::Gain {
                    origin,
                    offset: 0.0,
                    pointer,
                    jumped: true,
                    moved: false,
                });
                self.set_gain(logic::snap_gain(db), cx);
            }
            None => {
                self.drag = Some(Drag::Gain {
                    origin,
                    offset: origin.gain_db - pointer,
                    pointer,
                    jumped: false,
                    moved: false,
                });
                cx.notify();
            }
        }
    }

    fn drag_gain(&mut self, pointer: f32, cx: &mut Context<Self>) {
        let Some(Drag::Gain {
            offset,
            pointer: start,
            jumped,
            moved,
            ..
        }) = self.drag.as_mut()
        else {
            return;
        };
        if !*moved && (pointer - *start).abs() < GAIN_SLOP {
            return;
        }
        *moved = true;
        let db = if *jumped { pointer } else { pointer + *offset };
        self.set_gain(logic::snap_gain(db), cx);
    }

    /// Double-click on the slider: 0 dB as a single undo step, folding in
    /// the jump the first click may have made.
    fn reset_gain_by_double_click(&mut self, cx: &mut Context<Self>) {
        self.drag = None;
        let click = self.gain_click.take();
        let Some(doc) = self.doc_mut() else { return };
        if let Some((before, after)) = click
            && doc.edit == after
            && doc.undo.last() == Some(&before)
        {
            doc.undo.pop();
            doc.edit = before;
        }
        // `apply_edit` re-syncs the engine, which may still be at the
        // folded-away gain.
        self.reset_gain(cx);
    }

    fn drag_wave(&mut self, time: f64, full_time: f64, x: f32, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        let Phase::Ready(doc) = &mut self.phase else {
            return;
        };
        let duration = doc.duration();
        match drag {
            Drag::Handle {
                handle,
                anchor_x,
                offset,
                moved,
                ..
            } => {
                if !*moved && (x - *anchor_x).abs() < HANDLE_SLOP {
                    return;
                }
                *moved = true;
                doc.edit = logic::drag_edge(doc.edit, *handle, time + *offset, duration);
            }
            Drag::Range {
                anchor,
                anchor_x,
                moved,
                ..
            } => {
                if !*moved && (x - *anchor_x).abs() < 3.0 {
                    return;
                }
                *moved = true;
                if let Some((a, b)) = logic::range_between(*anchor, time, duration) {
                    doc.edit.start = a;
                    doc.edit.end = b;
                }
            }
            Drag::Pan => {
                doc.view = doc.view.centered_on(full_time, duration);
            }
            Drag::Gain { .. } => return,
        }
        cx.notify();
    }

    fn end_drag(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else { return };
        let origin = match drag {
            Drag::Handle { origin, .. }
            | Drag::Range { origin, .. }
            | Drag::Gain { origin, .. } => origin,
            Drag::Pan => {
                cx.notify();
                return;
            }
        };
        if let Some(doc) = self.doc_mut() {
            if doc.edit.validate(&doc.info).is_err() {
                doc.edit = origin;
            }
            Self::record(doc, origin);
            // Keep a fresh range's playhead at its start.
            if let Drag::Range { moved: true, .. } = drag
                && !doc.playing
            {
                doc.cursor = doc.edit.start;
            }
            if let Drag::Gain {
                jumped: true,
                moved: false,
                ..
            } = drag
            {
                self.gain_click = Some((origin, doc.edit));
            }
        }
        // Gain already followed live; this reschedules a changed selection.
        self.sync_playback(origin);
        cx.notify();
    }

    fn set_hover(&mut self, hover: Option<Handle>, cx: &mut Context<Self>) {
        if self.hover != hover {
            self.hover = hover;
            cx.notify();
        }
    }

    // ---- Export --------------------------------------------------------

    fn prompt_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog_open {
            return;
        }
        let format = self.export_format();
        let Some(doc) = self.doc() else { return };
        if doc.export.is_some() {
            return;
        }
        let source = doc.info.source_path().to_path_buf();
        let directory = source.parent().map(Path::to_path_buf).unwrap_or_default();
        let stem = source
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Audio".into());
        let suggested = format!("{stem} edit.{}", format.extension());
        let generation = self.generation;

        self.dialog_open = true;
        let chosen = cx.prompt_for_new_path(&directory, Some(&suggested));
        cx.spawn_in(window, async move |this, cx| {
            let chosen = chosen.await;
            this.update_in(cx, |this, window, cx| {
                this.dialog_open = false;
                window.activate_window();
                this.refocus(window);
                match chosen {
                    Ok(Ok(Some(path))) if generation == this.generation => {
                        this.start_export(path, format, cx)
                    }
                    Ok(Err(err)) => this.error(format!("Couldn't show the Save panel: {err}"), cx),
                    _ => cx.notify(),
                }
            })
            .ok();
        })
        .detach();
    }

    fn start_export(&mut self, destination: PathBuf, format: ExportFormat, cx: &mut Context<Self>) {
        let destination = logic::with_format_extension(destination, format);
        let generation = self.generation;
        let Some(doc) = self.doc() else { return };
        let (info, edit) = (doc.info.clone(), doc.edit);

        // Never write over the dropped original, nor over the extracted
        // audio that backs a video import.
        if same_file(&destination, info.source_path()) || same_file(&destination, &info.path) {
            self.error(
                "That's the original file. Sonora never overwrites it — choose a new name.",
                cx,
            );
            return;
        }
        if destination.exists() {
            self.error(
                format!(
                    "“{}” already exists. Sonora won't replace files — choose a new name.",
                    display_name(&destination)
                ),
                cx,
            );
            return;
        }
        if !logic::supports(format, info.channels) {
            self.error(
                format!(
                    "{} takes mono or stereo; this file has {} channels. Choose WAV.",
                    format.label(),
                    info.channels
                ),
                cx,
            );
            return;
        }
        if let Err(err) = edit.validate(&info) {
            self.error(
                format!("Can't export this selection. {}", sentence(&err)),
                cx,
            );
            return;
        }

        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(AtomicU32::new(0));
        let finished = Arc::new(AtomicBool::new(false));
        let job = cx.background_executor().spawn({
            let (cancel, progress, finished) = (cancel.clone(), progress.clone(), finished.clone());
            let destination = destination.clone();
            async move {
                let result =
                    audio::export_audio(&info, edit, &destination, format, &cancel, &progress);
                finished.store(true, Ordering::Release);
                result
            }
        });
        // Progress is an atomic, so poll it — but only while this job lives.
        let task = cx.spawn(async move |this, cx| {
            while !finished.load(Ordering::Acquire) {
                cx.background_executor()
                    .timer(Duration::from_millis(50))
                    .await;
                let alive = this.update(cx, |this, cx| this.export_progressed(generation, cx));
                if alive.is_err() {
                    return;
                }
            }
            let result = job.await;
            this.update(cx, |this, cx| this.export_finished(generation, result, cx))
                .ok();
        });

        if let Some(doc) = self.doc_mut() {
            doc.export = Some(ExportJob {
                destination,
                format,
                cancel,
                progress,
                shown: 0,
                _task: task,
            });
        }
        self.notice = None;
        cx.notify();
    }

    fn export_progressed(&mut self, generation: u64, cx: &mut Context<Self>) {
        if generation != self.generation {
            return;
        }
        if let Some(job) = self.doc_mut().and_then(|doc| doc.export.as_mut()) {
            let now = job.progress.load(Ordering::Relaxed).min(1000);
            if now != job.shown {
                job.shown = now;
                cx.notify();
            }
        }
    }

    fn export_finished(
        &mut self,
        generation: u64,
        result: anyhow::Result<()>,
        cx: &mut Context<Self>,
    ) {
        if generation != self.generation {
            return;
        }
        let Some(job) = self.doc_mut().and_then(|doc| doc.export.take()) else {
            return;
        };
        let cancelled = job.cancel.load(Ordering::Relaxed);
        self.notice = Some(match result {
            Ok(()) => Notice::Saved(job.destination.clone()),
            Err(_) if cancelled => Notice::Info("Export cancelled. Nothing was written.".into()),
            Err(err) => Notice::Error(format!("Export failed. {}", sentence(&err))),
        });
        cx.notify();
    }

    fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.help {
            self.help = false;
            cx.notify();
        } else if matches!(self.phase, Phase::Opening(_)) {
            self.cancel_open(cx);
        } else if let Some(job) = self.doc().and_then(|doc| doc.export.as_ref()) {
            job.cancel.store(true, Ordering::Relaxed);
            cx.notify();
        } else if self.notice.is_some() {
            self.notice = None;
            cx.notify();
        }
    }

    fn reveal_source(&mut self, _: &mut Context<Self>) {
        if let Some(doc) = self.doc() {
            audio::reveal(doc.info.source_path());
        }
    }
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn same_file(destination: &Path, source: &Path) -> bool {
    let canonical = |path: &Path| -> Option<PathBuf> {
        if let Ok(path) = path.canonicalize() {
            return Some(path);
        }
        let parent = path.parent()?.canonicalize().ok()?;
        Some(parent.join(path.file_name()?))
    };
    match (canonical(destination), canonical(source)) {
        (Some(a), Some(b)) => a == b,
        _ => destination == source,
    }
}

/// An error as a short sentence ending in a period.
fn sentence(err: &anyhow::Error) -> String {
    let text = err.to_string();
    let text = text.trim();
    if text.is_empty() {
        return "Something went wrong.".into();
    }
    let mut chars = text.chars();
    let first = chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>())
        .unwrap_or_default();
    let mut out = first + chars.as_str();
    if !out.ends_with(['.', '!', '?']) {
        out.push('.');
    }
    out
}
