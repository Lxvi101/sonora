//! Sonora's single-document window: open one audio file, trim it, raise or
//! lower its gain, preview, and export a new WAV.
//!
//! All decoding work (probe, waveform, export) runs off the main thread. Each
//! opened file gets a fresh `generation`; results carrying an older generation
//! are discarded, and replacing a document cancels its waveform and export
//! jobs through their `AtomicBool` flags.

pub mod assets;
pub mod mailbox;
mod theme;
mod view;
mod wave;
mod widgets;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::time::Duration;

use gpui::{
    App, AppContext, Bounds, Context, FocusHandle, KeyBinding, Menu, MenuItem, OsAction,
    PathPromptOptions, SystemMenuType, Task, TitlebarOptions, Window, WindowBackgroundAppearance,
    WindowBounds, WindowOptions, actions, point, px, size,
};

use crate::audio::{self, AudioInfo, Edit, Peak, Player, Waveform};

actions!(
    sonora,
    [
        Open,
        Export,
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
        SetIn,
        SetOut,
        PlayPause,
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
        Cancel,
    ]
);

const GAIN_MIN: f32 = -24.0;
const GAIN_MAX: f32 = 24.0;
const GAIN_STEP: f32 = 0.5;
const MIN_SELECTION: f64 = 0.02;
const UNDO_LIMIT: usize = 200;

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
        KeyBinding::new("cmd-w", CloseFile, None),
        KeyBinding::new("cmd-shift-r", RevealSource, None),
        KeyBinding::new("cmd-z", Undo, None),
        KeyBinding::new("cmd-shift-z", Redo, None),
        KeyBinding::new("cmd-a", SelectAll, None),
        KeyBinding::new("cmd-backspace", ResetEdits, None),
        KeyBinding::new("i", SetIn, None),
        KeyBinding::new("o", SetOut, None),
        KeyBinding::new("space", PlayPause, None),
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
        KeyBinding::new("escape", Cancel, None),
    ]);

    cx.set_menus(vec![
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
                MenuItem::action("Export WAV…", Export),
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
                MenuItem::separator(),
                MenuItem::action("Reset Trim and Gain", ResetEdits),
            ],
        },
        Menu {
            name: "Playback".into(),
            items: vec![
                MenuItem::action("Play / Pause", PlayPause),
                MenuItem::separator(),
                MenuItem::action("Back 1 Second", SeekBack),
                MenuItem::action("Forward 1 Second", SeekForward),
                MenuItem::action("Back 5 Seconds", SeekBackFar),
                MenuItem::action("Forward 5 Seconds", SeekForwardFar),
                MenuItem::action("Nudge Back 10 ms", NudgeBack),
                MenuItem::action("Nudge Forward 10 ms", NudgeForward),
                MenuItem::action("Go to Start", GoToStart),
                MenuItem::action("Go to End", GoToEnd),
                MenuItem::separator(),
                MenuItem::action("Louder", GainUp),
                MenuItem::action("Quieter", GainDown),
                MenuItem::action("Reset Gain", ResetGain),
            ],
        },
        Menu {
            name: "Window".into(),
            items: vec![
                MenuItem::action("Minimize", Minimize),
                MenuItem::action("Zoom", Zoom),
            ],
        },
    ]);
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
            size(px(900.), px(540.)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some("Sonora".into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(14.), px(14.))),
        }),
        window_background: WindowBackgroundAppearance::Blurred,
        window_min_size: Some(size(px(600.), px(380.))),
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
            let opened = handle.update(cx, |sonora, window, cx| {
                window.activate_window();
                sonora.open_paths(paths, cx);
            });
            if opened.is_err() {
                break;
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
}

enum Phase {
    Empty,
    Opening { name: String, _task: Task<()> },
    Ready(Box<Document>),
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
    peaks: Option<Arc<Vec<Peak>>>,
    /// Loudest absolute sample per waveform bin's neighbourhood is in `peaks`;
    /// this marks whether they are the exact scan or the sparse preview.
    peaks_exact: bool,
    wave_progress: f32,
    wave_error: Option<String>,
    wave_cancel: Arc<AtomicBool>,
    _wave_task: Task<()>,
    export: Option<ExportJob>,
}

impl Drop for Document {
    fn drop(&mut self) {
        self.wave_cancel.store(true, Ordering::Relaxed);
    }
}

struct ExportJob {
    destination: PathBuf,
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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Handle {
    Start,
    End,
}

#[derive(Clone, Copy, Debug)]
enum Drag {
    Handle {
        handle: Handle,
        origin: Edit,
    },
    Range {
        origin: Edit,
        anchor: f64,
        anchor_x: f32,
        moved: bool,
    },
    Gain {
        origin: Edit,
    },
}

impl Sonora {
    fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
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
        }
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

    // ---- Opening -------------------------------------------------------

    fn prompt_open(&mut self, cx: &mut Context<Self>) {
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
        cx.spawn(async move |this, cx| {
            let chosen = paths.await;
            this.update(cx, |this, cx| {
                this.dialog_open = false;
                match chosen {
                    Ok(Ok(Some(paths))) => this.open_paths(paths, cx),
                    Ok(Err(err)) => this.error(format!("Couldn't show the Open panel: {err}"), cx),
                    _ => {}
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
            self.error("That's a folder. Choose an audio file to open.", cx);
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

        let name = display_name(&path);
        let probe_path = path.clone();
        let probe = cx
            .background_executor()
            .spawn(async move { audio::probe(&probe_path) });
        let task = cx.spawn(async move |this, cx| {
            let result = probe.await;
            this.update(cx, |this, cx| this.probed(generation, path, result, cx))
                .ok();
        });
        // Replacing the phase drops the previous document, cancelling its jobs.
        self.phase = Phase::Opening { name, _task: task };
        cx.notify();
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
        let info = match result {
            Ok(info) if info.frames > 0 && info.duration() > 0.0 => info,
            Ok(_) => {
                self.phase = Phase::Empty;
                self.error(format!("“{}” contains no audio.", display_name(&path)), cx);
                return;
            }
            Err(err) => {
                self.phase = Phase::Empty;
                self.error(
                    format!(
                        "Couldn't open “{}”. {}",
                        display_name(&path),
                        sentence(&err)
                    ),
                    cx,
                );
                return;
            }
        };

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
            name: display_name(&info.path),
            edit: Edit::full(&info),
            info,
            undo: Vec::new(),
            redo: Vec::new(),
            cursor: 0.0,
            playing: false,
            playable,
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
        self.stop_playback();
        self.generation += 1;
        self.phase = Phase::Empty;
        self.notice = None;
        self.drag = None;
        self.hover = None;
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
            let Edit {
                start,
                end,
                gain_db,
            } = doc.edit;
            let from = if position >= start && position < end - 0.01 {
                position
            } else {
                start
            };
            match player.play(from, end, gain_db) {
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
        let time = time.clamp(0.0, doc.info.duration());
        if doc.playing
            && let Some(player) = self.player.as_mut()
        {
            let Edit {
                start,
                end,
                gain_db,
            } = doc.edit;
            let from = time.clamp(start, (end - 0.01).max(start));
            doc.cursor = from;
            if let Err(err) = player.play(from, end, gain_db) {
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
        let Edit {
            start,
            end,
            gain_db,
        } = doc.edit;
        if gain_db != before.gain_db {
            player.set_gain(gain_db);
        }
        if !doc.playing || (start == before.start && end == before.end) {
            return;
        }
        let at = player.position();
        if at >= start && at < end - 0.01 {
            if player.play(at, end, gain_db).is_err() {
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

    /// Called every frame while playing (from render) to follow the engine
    /// and notice when it reaches the end of the selection.
    fn tick_playback(&mut self) -> bool {
        let Phase::Ready(doc) = &mut self.phase else {
            return false;
        };
        if !doc.playing {
            return false;
        }
        match &self.player {
            Some(player) if player.is_playing() => true,
            _ => {
                doc.playing = false;
                doc.cursor = doc.edit.start;
                false
            }
        }
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
        let duration = doc.info.duration();
        let mut edit = doc.edit;
        edit.start = at.clamp(0.0, (duration - MIN_SELECTION).max(0.0));
        if edit.end - edit.start < MIN_SELECTION {
            edit.end = duration;
        }
        self.apply_edit(edit, cx);
    }

    fn set_out(&mut self, cx: &mut Context<Self>) {
        let at = self.position();
        let Some(doc) = self.doc() else { return };
        let duration = doc.info.duration();
        let mut edit = doc.edit;
        edit.end = at.clamp(MIN_SELECTION.min(duration), duration);
        if edit.end - edit.start < MIN_SELECTION {
            edit.start = 0.0;
        }
        self.apply_edit(edit, cx);
    }

    fn select_all(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.doc() else { return };
        let full = Edit::full(&doc.info);
        let edit = Edit {
            gain_db: doc.edit.gain_db,
            ..full
        };
        self.apply_edit(edit, cx);
    }

    fn reset_edits(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.doc() else { return };
        let full = Edit::full(&doc.info);
        self.apply_edit(full, cx);
    }

    fn nudge_gain(&mut self, delta: f32, cx: &mut Context<Self>) {
        let Some(doc) = self.doc() else { return };
        let mut edit = doc.edit;
        edit.gain_db = ((edit.gain_db + delta) / GAIN_STEP).round() * GAIN_STEP;
        edit.gain_db = edit.gain_db.clamp(GAIN_MIN, GAIN_MAX);
        self.apply_edit(edit, cx);
    }

    fn set_gain(&mut self, db: f32, cx: &mut Context<Self>) {
        let Some(doc) = self.doc_mut() else { return };
        doc.edit.gain_db = db.clamp(GAIN_MIN, GAIN_MAX);
        let db = doc.edit.gain_db;
        if let Some(player) = &mut self.player {
            player.set_gain(db);
        }
        cx.notify();
    }

    fn is_edited(doc: &Document) -> bool {
        doc.edit != Edit::full(&doc.info)
    }

    // ---- Pointer gestures (driven by the canvases) ---------------------

    fn begin_handle_drag(&mut self, handle: Handle, cx: &mut Context<Self>) {
        let Some(doc) = self.doc() else { return };
        self.drag = Some(Drag::Handle {
            handle,
            origin: doc.edit,
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

    fn begin_gain_drag(&mut self, db: f32, cx: &mut Context<Self>) {
        let Some(doc) = self.doc() else { return };
        self.drag = Some(Drag::Gain { origin: doc.edit });
        self.set_gain(snap_gain(db), cx);
    }

    fn drag_wave(&mut self, time: f64, x: f32, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.as_mut() else {
            return;
        };
        let Phase::Ready(doc) = &mut self.phase else {
            return;
        };
        let duration = doc.info.duration();
        let time = time.clamp(0.0, duration);
        match drag {
            Drag::Handle {
                handle: Handle::Start,
                ..
            } => {
                doc.edit.start = time.min(doc.edit.end - MIN_SELECTION).max(0.0);
            }
            Drag::Handle {
                handle: Handle::End,
                ..
            } => {
                doc.edit.end = time.max(doc.edit.start + MIN_SELECTION).min(duration);
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
                let (a, b) = if time < *anchor {
                    (time, *anchor)
                } else {
                    (*anchor, time)
                };
                if b - a >= MIN_SELECTION {
                    doc.edit.start = a;
                    doc.edit.end = b;
                }
            }
            Drag::Gain { .. } => return,
        }
        cx.notify();
    }

    fn drag_gain(&mut self, db: f32, cx: &mut Context<Self>) {
        if matches!(self.drag, Some(Drag::Gain { .. })) {
            self.set_gain(snap_gain(db), cx);
        }
    }

    fn end_drag(&mut self, cx: &mut Context<Self>) {
        let Some(drag) = self.drag.take() else { return };
        let origin = match drag {
            Drag::Handle { origin, .. } | Drag::Range { origin, .. } | Drag::Gain { origin } => {
                origin
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
        }
        // Gain already followed live; this reschedules a changed selection.
        self.sync_playback(Edit {
            gain_db: self.doc().map_or(origin.gain_db, |d| d.edit.gain_db),
            ..origin
        });
        cx.notify();
    }

    fn set_hover(&mut self, hover: Option<Handle>, cx: &mut Context<Self>) {
        if self.hover != hover {
            self.hover = hover;
            cx.notify();
        }
    }

    // ---- Export --------------------------------------------------------

    fn prompt_export(&mut self, cx: &mut Context<Self>) {
        if self.dialog_open {
            return;
        }
        let Some(doc) = self.doc() else { return };
        if doc.export.is_some() {
            return;
        }
        let source = doc.info.path.clone();
        let directory = source.parent().map(Path::to_path_buf).unwrap_or_default();
        let stem = source
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Audio".into());
        let suggested = format!("{stem} edit.wav");
        let generation = self.generation;

        self.dialog_open = true;
        let chosen = cx.prompt_for_new_path(&directory, Some(&suggested));
        cx.spawn(async move |this, cx| {
            let chosen = chosen.await;
            this.update(cx, |this, cx| {
                this.dialog_open = false;
                match chosen {
                    Ok(Ok(Some(path))) if generation == this.generation => {
                        this.start_export(path, cx)
                    }
                    Ok(Err(err)) => this.error(format!("Couldn't show the Save panel: {err}"), cx),
                    _ => {}
                }
            })
            .ok();
        })
        .detach();
    }

    fn start_export(&mut self, destination: PathBuf, cx: &mut Context<Self>) {
        let destination = with_wav_extension(destination);
        let generation = self.generation;
        let Some(doc) = self.doc() else { return };
        let (info, edit) = (doc.info.clone(), doc.edit);

        if same_file(&destination, &info.path) {
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
                let result = audio::export_wav(&info, edit, &destination, &cancel, &progress);
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
        if let Some(job) = self.doc().and_then(|doc| doc.export.as_ref()) {
            job.cancel.store(true, Ordering::Relaxed);
            cx.notify();
        } else if self.notice.is_some() {
            self.notice = None;
            cx.notify();
        }
    }

    fn reveal_source(&mut self, _: &mut Context<Self>) {
        if let Some(doc) = self.doc() {
            audio::reveal(&doc.info.path);
        }
    }
}

fn snap_gain(db: f32) -> f32 {
    let db = (db * 10.0).round() / 10.0;
    if db.abs() < 0.45 {
        0.0
    } else {
        db.clamp(GAIN_MIN, GAIN_MAX)
    }
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

fn with_wav_extension(path: PathBuf) -> PathBuf {
    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("wav") => path,
        Some(_) => {
            let mut name = path.file_name().unwrap_or_default().to_os_string();
            name.push(".wav");
            path.with_file_name(name)
        }
        None => path.with_extension("wav"),
    }
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
