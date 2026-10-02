//! Sonora's single-document window: open one audio file, cut it into clips,
//! drop or reorder them on one gapless track, trim, raise or lower its gain,
//! preview, and export a new WAV, MP3 or M4A.
//!
//! Clips are non-destructive source ranges (`Timeline`); positions, trim,
//! the view and the playhead are all arranged-timeline seconds. One undo
//! history covers cuts, deletes, moves, trim and gain.
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

use std::cell::Cell;
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
use crate::timeline::Timeline;
use crate::updater;
use logic::{GAIN_STEP, Handle, Tool, View};

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
        DeselectClips,
        SelectTool,
        RazorTool,
        SplitAtPlayhead,
        DeleteClips,
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
        About,
        CheckForUpdates,
        ToggleAutomaticUpdates,
        SourceCode,
        ReportIssue,
    ]
);

const UNDO_LIMIT: usize = 200;
/// Pointer travel before a press on a trim handle becomes a drag, so the
/// first click of a double-click never nudges the edge.
const HANDLE_SLOP: f32 = 2.5;
/// Gain-slider travel (dB) before a grab becomes a drag.
const GAIN_SLOP: f32 = 0.8;
/// Pointer travel before a press on the clip bar becomes a reorder drag.
const CLIP_SLOP: f32 = 3.0;

/// Registers key bindings, menus and app-wide actions.
pub fn init(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &About, _| updater::show_about());
    cx.on_action(|_: &SourceCode, cx| cx.open_url(updater::REPOSITORY));
    cx.on_action(|_: &ReportIssue, cx| cx.open_url(updater::NEW_ISSUE));

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
        KeyBinding::new("cmd-shift-a", DeselectClips, None),
        KeyBinding::new("v", SelectTool, None),
        KeyBinding::new("c", RazorTool, None),
        KeyBinding::new("cmd-b", SplitAtPlayhead, None),
        KeyBinding::new("backspace", DeleteClips, None),
        KeyBinding::new("delete", DeleteClips, None),
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

    cx.set_menus(menus(MenuState::default()));
}

/// What the menu bar shows; it is rebuilt whenever this changes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct MenuState {
    /// The format the Export item names.
    format: ExportFormat,
    /// Automatic update checks, when the updater is running.
    automatic_updates: Option<bool>,
    /// The pointer tool the Clip menu checks.
    tool: Tool,
}

impl Default for MenuState {
    fn default() -> Self {
        Self {
            format: ExportFormat::Wav,
            automatic_updates: None,
            tool: Tool::Select,
        }
    }
}

/// A checkmark-style menu label (GPUI menus have no native check state).
fn checked(on: bool, label: &str) -> String {
    format!("{}{label}", if on { "✓ " } else { "    " })
}

/// The menu bar. Rebuilt when the export format or the update preference
/// changes, so labels always say what will happen.
fn menus(state: MenuState) -> Vec<Menu> {
    let format = state.format;
    let choice = |f: ExportFormat| {
        checked(
            f == format,
            &format!("{} — {}", f.label(), logic::format_detail(f)),
        )
    };
    vec![
        Menu {
            name: "Sonora".into(),
            items: vec![
                MenuItem::action("About Sonora", About),
                MenuItem::separator(),
                // Disabled (no handler registered) unless the updater runs.
                MenuItem::action("Check for Updates…", CheckForUpdates),
                MenuItem::action(
                    checked(
                        state.automatic_updates == Some(true),
                        "Automatically Check for Updates",
                    ),
                    ToggleAutomaticUpdates,
                ),
                MenuItem::separator(),
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
                MenuItem::os_action("Select All Clips", SelectAll, OsAction::SelectAll),
                MenuItem::action("Deselect All Clips", DeselectClips),
                MenuItem::separator(),
                MenuItem::action("Set Start at Playhead", SetIn),
                MenuItem::action("Set End at Playhead", SetOut),
                MenuItem::action("Reset Start to Beginning", ResetStart),
                MenuItem::action("Reset End to Timeline End", ResetEnd),
                MenuItem::separator(),
                MenuItem::action("Louder", GainUp),
                MenuItem::action("Quieter", GainDown),
                MenuItem::action("Reset Gain to 0 dB", ResetGain),
                MenuItem::separator(),
                MenuItem::action("Reset All Edits", ResetEdits),
            ],
        },
        // The editing workflow in one place: pick a tool, cut, remove.
        // Reordering is a drag on the clip bar (see Help).
        Menu {
            name: "Clip".into(),
            items: vec![
                MenuItem::action(
                    checked(
                        state.tool == Tool::Select,
                        "Select Tool — Pick & Drag Clips",
                    ),
                    SelectTool,
                ),
                MenuItem::action(
                    checked(state.tool == Tool::Razor, "Razor Tool — Click to Cut"),
                    RazorTool,
                ),
                MenuItem::separator(),
                MenuItem::action("Cut at Playhead", SplitAtPlayhead),
                MenuItem::action("Ripple Delete Selected Clips", DeleteClips),
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
                MenuItem::action("Show Entire Timeline", ZoomToFit),
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
            items: vec![
                MenuItem::action("Editing Clips, Keyboard & Gestures", ToggleHelp),
                MenuItem::separator(),
                MenuItem::action("Sonora Source Code", SourceCode),
                MenuItem::action("Report an Issue…", ReportIssue),
            ],
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

    // Sparkle starts on the main thread once the window exists; it is a
    // no-op in development runs and unconfigured source builds.
    let updates = updater::start();
    handle.update(cx, |sonora, window, cx| {
        sonora.updates = updates;
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
    menu_state: MenuState,
    /// Whether the updater is running (only in configured release bundles).
    updates: updater::Status,
    /// Loop the selection during preview. A session preference.
    looping: bool,
    /// The pointer tool. A session preference, like the format.
    tool: Tool,
    /// Where the razor would cut (timeline seconds) while it hovers the stage.
    razor_at: Option<f64>,
    /// The pointer is over the clip bar (grab cursor with the Select tool).
    lane_hover: bool,
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

/// One undo step: the arrangement and the trim/gain edit. The clip selection
/// rides along so undo brings back what was selected, but selecting alone
/// never creates a step.
#[derive(Clone)]
struct Snapshot {
    edit: Edit,
    timeline: Timeline,
    selected: Vec<usize>,
}

pub(crate) struct Document {
    info: AudioInfo,
    name: String,
    /// Trim and gain, in arranged-timeline seconds.
    edit: Edit,
    /// The clips: source ranges of `info`, played back to back.
    timeline: Timeline,
    /// Bumped on every arrangement change; keys cached measurements.
    revision: u64,
    /// Selected clip indices, sorted and unique.
    selected: Vec<usize>,
    /// Where a ⇧-click range starts.
    anchor: Option<usize>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// Playhead while paused, timeline seconds.
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
    /// The trim range's peak, keyed by what it depends on, so painting
    /// never rescans the overview.
    peak_memo: Cell<Option<(PeakKey, Option<f32>)>>,
}

/// (trim start bits, trim end bits, arrangement revision, overview identity).
type PeakKey = (u64, u64, u64, usize);

impl Document {
    fn new(info: AudioInfo, playable: bool, wave_cancel: Arc<AtomicBool>, task: Task<()>) -> Self {
        Self {
            name: display_name(info.source_path()),
            edit: Edit::full(&info),
            view: View::full(info.duration()),
            timeline: Timeline::new(info.frames),
            revision: 0,
            selected: Vec::new(),
            anchor: None,
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
            _wave_task: task,
            export: None,
            peak_memo: Cell::new(None),
        }
    }

    /// Length of the arranged timeline.
    fn duration(&self) -> f64 {
        self.seconds(self.timeline.frames())
    }

    fn seconds(&self, frame: u64) -> f64 {
        frame as f64 / self.info.sample_rate
    }

    /// The timeline frame nearest to `time`, inside the timeline.
    fn frame_at(&self, time: f64) -> u64 {
        ((time.max(0.0) * self.info.sample_rate).round() as u64).min(self.timeline.frames())
    }

    /// Whether any clip is left to play or export.
    fn has_audio(&self) -> bool {
        !self.timeline.is_empty()
    }

    fn preview_gain(&self) -> f32 {
        logic::preview_gain(self.edit, self.original)
    }

    fn is_edited(&self) -> bool {
        self.edit != Edit::full(&self.info) || !self.timeline.is_original()
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            edit: self.edit,
            timeline: self.timeline.clone(),
            selected: self.selected.clone(),
        }
    }

    /// After the arrangement's length changed from `old_duration`: keep a
    /// whole-file view whole, keep the playhead inside, re-key caches.
    fn rearranged(&mut self, old_duration: f64) {
        self.revision += 1;
        let duration = self.duration();
        self.view = if self.view.is_full(old_duration) || duration <= 0.0 {
            View::full(duration)
        } else {
            View::fitted(self.view.start, self.view.end, duration)
        };
        self.cursor = self.cursor.clamp(0.0, duration);
        self.anchor = self.anchor.filter(|&i| i < self.timeline.clips().len());
    }

    fn restore(&mut self, state: Snapshot) {
        let old_duration = self.duration();
        let moved = state.timeline != self.timeline;
        self.edit = state.edit;
        self.timeline = state.timeline;
        self.selected = state.selected;
        if moved {
            self.rearranged(old_duration);
        }
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
        /// A press that stays a click selects the clip under it.
        click: Option<ClipClick>,
    },
    /// Dragging the selected clips along the clip bar.
    Clips {
        anchor_x: f32,
        /// Pointer time minus the dragged block's start at the press.
        grab: f64,
        /// Pointer, timeline seconds.
        pointer: f64,
        moved: bool,
        /// The boundary the block would drop at, when that changes anything.
        target: Option<usize>,
        /// A press on one clip of a multi-selection that never moves
        /// narrows the selection to that clip, as in Finder.
        collapse: Option<usize>,
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

/// A click on a clip, with the modifiers that shape the selection.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ClipClick {
    pub index: usize,
    /// ⌘: add or remove this clip.
    pub toggle: bool,
    /// ⇧: extend from the last clicked clip.
    pub extend: bool,
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
            menu_state: MenuState::default(),
            updates: updater::Status::Development,
            looping: false,
            tool: Tool::Select,
            razor_at: None,
            lane_hover: false,
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
        let state = MenuState {
            format: self.export_format(),
            automatic_updates: self.updates.is_ready().then(updater::automatic_checks),
            tool: self.tool,
        };
        if state != self.menu_state {
            self.menu_state = state;
            cx.set_menus(menus(state));
        }
    }

    fn toggle_automatic_updates(&mut self, cx: &mut Context<Self>) {
        if self.updates.is_ready() {
            updater::set_automatic_checks(!updater::automatic_checks());
            cx.notify();
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
        self.razor_at = None;
        self.lane_hover = false;
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
        self.phase = Phase::Ready(Box::new(Document::new(
            info,
            playable,
            wave_cancel,
            wave_task,
        )));
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
        self.razor_at = None;
        self.lane_hover = false;
        self.gain_click = None;
        cx.notify();
    }

    // ---- Playback ------------------------------------------------------

    /// Current playhead, timeline seconds.
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

    /// Plays the arrangement from `from` to the trim end. The engine queues
    /// every join itself, so clips follow each other without a gap.
    fn play_from(player: &mut Player, doc: &Document, from: f64) -> anyhow::Result<()> {
        player.play_timeline(
            &doc.info,
            &doc.timeline,
            from,
            doc.edit.end,
            doc.preview_gain(),
        )
    }

    fn toggle_playback(&mut self, cx: &mut Context<Self>) {
        let position = self.position();
        let Phase::Ready(doc) = &mut self.phase else {
            return;
        };
        let Some(player) = self
            .player
            .as_mut()
            .filter(|_| doc.playable && doc.has_audio())
        else {
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
            match Self::play_from(player, doc, from) {
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
            if let Err(err) = Self::play_from(player, doc, from) {
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
    /// (possibly new) selection. `resume` is set when the arrangement itself
    /// changed: the engine's queue is rebuilt from that timeline position.
    fn sync_playback(&mut self, before: Edit, resume: Option<f64>) {
        let Phase::Ready(doc) = &mut self.phase else {
            return;
        };
        let Some(player) = self.player.as_mut() else {
            return;
        };
        let Edit { start, end, .. } = doc.edit;
        player.set_gain(doc.preview_gain());
        if !doc.playing || (resume.is_none() && start == before.start && end == before.end) {
            return;
        }
        let at = resume.unwrap_or_else(|| player.position());
        if doc.has_audio() && at >= start && at < end - 0.01 {
            if Self::play_from(player, doc, at).is_err() {
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
        if !alive && looping && doc.has_audio() {
            alive = Self::play_from(player, doc, doc.edit.start).is_ok();
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

    /// Pushes `before` as one undo step if the edit or the arrangement
    /// changed since.
    fn record(doc: &mut Document, before: Snapshot) {
        if doc.edit != before.edit || doc.timeline != before.timeline {
            doc.undo.push(before);
            if doc.undo.len() > UNDO_LIMIT {
                doc.undo.remove(0);
            }
            doc.redo.clear();
        }
    }

    /// Records an arrangement change made since `before` and rebuilds
    /// playback from `resume` when the audio under the playhead changed.
    fn commit(&mut self, before: Snapshot, resume: Option<f64>, cx: &mut Context<Self>) {
        let edit = before.edit;
        let Some(doc) = self.doc_mut() else { return };
        Self::record(doc, before);
        self.sync_playback(edit, resume);
        cx.notify();
    }

    /// Applies a discrete trim or gain edit as one undo step.
    fn apply_edit(&mut self, edit: Edit, cx: &mut Context<Self>) {
        let Some(doc) = self.doc_mut() else { return };
        if edit
            .validate_frames(doc.timeline.frames(), doc.info.sample_rate)
            .is_err()
        {
            return;
        }
        let before = doc.edit;
        // Even an unchanged edit re-syncs the engine below (the gain
        // double-click fold relies on it).
        let snapshot = doc.snapshot();
        doc.edit = edit;
        Self::record(doc, snapshot);
        self.sync_playback(before, None);
        cx.notify();
    }

    /// Undo (`forward == false`) or redo one step: trim, gain, cuts,
    /// deletes and moves alike.
    fn step_history(&mut self, forward: bool, cx: &mut Context<Self>) {
        let position = self.position();
        let Some(doc) = self.doc_mut() else { return };
        let Some(target) = (if forward {
            doc.redo.pop()
        } else {
            doc.undo.pop()
        }) else {
            return;
        };
        let current = doc.snapshot();
        let before = current.edit;
        let rearranged = target.timeline != doc.timeline;
        if forward {
            doc.undo.push(current);
        } else {
            doc.redo.push(current);
        }
        doc.restore(target);
        let resume = rearranged.then(|| position.min(doc.duration()));
        self.sync_playback(before, resume);
        cx.notify();
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        self.step_history(false, cx);
    }

    fn redo(&mut self, cx: &mut Context<Self>) {
        self.step_history(true, cx);
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

    /// Sends one trim edge back to the timeline boundary (double-click on a
    /// handle, ⇧I, ⇧O). The other edge and the gain are untouched.
    fn reset_edge(&mut self, handle: Handle, cx: &mut Context<Self>) {
        self.drag = None;
        let Some(doc) = self.doc() else { return };
        let edit = logic::reset_edge(doc.edit, handle, doc.duration());
        self.apply_edit(edit, cx);
    }

    /// Back to the untouched source: one clip, whole-file trim, 0 dB. One
    /// undo step, so even an emptied timeline is a click away from either.
    fn reset_edits(&mut self, cx: &mut Context<Self>) {
        let position = self.position();
        let Some(doc) = self.doc_mut().filter(|d| d.is_edited()) else {
            return;
        };
        let before = doc.snapshot();
        let old_duration = doc.duration();
        doc.timeline.reset();
        doc.selected.clear();
        doc.anchor = None;
        doc.rearranged(old_duration);
        doc.edit = Edit::full(&doc.info);
        let resume = position.min(doc.duration());
        self.commit(before, Some(resume), cx);
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

    // ---- Clips ---------------------------------------------------------

    fn set_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        if self.tool != tool {
            self.tool = tool;
            self.razor_at = None;
            self.hover = None;
            cx.notify();
        }
    }

    /// Cuts the clip under `time` (timeline seconds) at the nearest sample
    /// frame. Nothing happens on an existing join or past the end.
    fn split_at(&mut self, time: f64, cx: &mut Context<Self>) {
        let Some(doc) = self.doc_mut().filter(|d| d.has_audio()) else {
            return;
        };
        let before = doc.snapshot();
        let Some(right) = doc.timeline.split(doc.frame_at(time)) else {
            return;
        };
        doc.selected = logic::selection_after_split(&doc.selected, right);
        doc.anchor = doc.anchor.map(|a| if a >= right { a + 1 } else { a });
        doc.revision += 1;
        // The audio is unchanged, so playback carries on untouched.
        self.commit(before, None, cx);
    }

    fn split_at_playhead(&mut self, cx: &mut Context<Self>) {
        let at = self.position();
        self.split_at(at, cx);
    }

    /// Ripple delete: the selected clips go and later ones close the gap.
    /// Trim edges and the playhead keep pointing at the same audio.
    fn delete_selected(&mut self, cx: &mut Context<Self>) {
        let position = self.position();
        let Some(doc) = self.doc_mut().filter(|d| !d.selected.is_empty()) else {
            return;
        };
        let before = doc.snapshot();
        let old_duration = doc.duration();
        let deleted = std::mem::take(&mut doc.selected);
        let rate = doc.info.sample_rate;
        let ripple = |time: f64| logic::ripple_time(&before.timeline, &deleted, time, rate);
        let (cursor, resume) = (ripple(doc.cursor), ripple(position));
        let edit = logic::edit_after_delete(doc.edit, &before.timeline, &deleted, rate);
        doc.timeline.delete(&deleted);
        doc.anchor = None;
        doc.rearranged(old_duration);
        doc.cursor = cursor.min(doc.duration());
        doc.edit = edit;
        self.commit(before, Some(resume), cx);
    }

    /// Moves the selected clips, in order, to a boundary of the arrangement.
    fn move_selection(&mut self, boundary: usize, cx: &mut Context<Self>) {
        let position = self.position();
        let Some(doc) = self.doc_mut() else { return };
        let before = doc.snapshot();
        let picked = doc.selected.clone();
        let Some(moved) = doc.timeline.move_clips(&picked, boundary) else {
            return;
        };
        doc.anchor = Some(moved.start);
        doc.selected = moved.collect();
        doc.revision += 1;
        // Same length; the playhead stays put while the audio under it moves.
        self.commit(before, Some(position), cx);
    }

    fn click_clip(&mut self, click: ClipClick, cx: &mut Context<Self>) {
        let Some(doc) = self.doc_mut() else { return };
        let (selected, anchor) = logic::click_selection(
            &doc.selected,
            doc.anchor,
            click.index,
            click.toggle,
            click.extend,
        );
        doc.selected = selected;
        doc.anchor = anchor;
        cx.notify();
    }

    fn select_all_clips(&mut self, cx: &mut Context<Self>) {
        let Some(doc) = self.doc_mut().filter(|d| d.has_audio()) else {
            return;
        };
        doc.selected = (0..doc.timeline.clips().len()).collect();
        doc.anchor = Some(0);
        cx.notify();
    }

    fn deselect_clips(&mut self, cx: &mut Context<Self>) {
        if let Some(doc) = self.doc_mut() {
            doc.selected.clear();
            doc.anchor = None;
            cx.notify();
        }
    }

    /// The clip under a timeline time.
    fn clip_at(&self, time: f64) -> Option<usize> {
        let doc = self.doc()?;
        doc.timeline.clip_at(doc.frame_at(time))
    }

    // ---- View ----------------------------------------------------------

    fn set_view(&mut self, view: View, cx: &mut Context<Self>) {
        if let Some(doc) = self.doc_mut()
            && doc.has_audio()
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

    /// `click`: the clip to select if this press never becomes a drag.
    fn begin_range_drag(
        &mut self,
        time: f64,
        x: f32,
        click: Option<ClipClick>,
        cx: &mut Context<Self>,
    ) {
        let Some(doc) = self.doc() else { return };
        self.drag = Some(Drag::Range {
            origin: doc.edit,
            anchor: time,
            anchor_x: x,
            moved: false,
            click,
        });
        self.seek_to(time, cx);
    }

    /// A press on the clip bar with the Select tool: ⌘/⇧ adjust the
    /// selection; a plain press selects the clip and arms a reorder drag of
    /// everything selected.
    fn press_clip(
        &mut self,
        time: f64,
        x: f32,
        toggle: bool,
        extend: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.clip_at(time) else {
            return;
        };
        if toggle || extend {
            self.click_clip(
                ClipClick {
                    index,
                    toggle,
                    extend,
                },
                cx,
            );
            return;
        }
        let Some(doc) = self.doc_mut() else { return };
        let collapse = (doc.selected.len() > 1 && doc.selected.contains(&index)).then_some(index);
        if !doc.selected.contains(&index) {
            doc.selected = vec![index];
        }
        doc.anchor = Some(index);
        // The dragged block is the selection gathered in order; keep the
        // pointer where it grabbed the pressed clip within that block.
        let clips = doc.timeline.clips();
        let ahead: u64 = doc
            .selected
            .iter()
            .filter(|&&i| i < index)
            .map(|&i| clips[i].frames())
            .sum();
        let grab = time - doc.seconds(doc.timeline.boundary_frame(index)) + doc.seconds(ahead);
        self.drag = Some(Drag::Clips {
            anchor_x: x,
            grab,
            pointer: time,
            moved: false,
            target: None,
            collapse,
        });
        cx.notify();
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
        // An emptied timeline has no valid edit to change the gain of.
        let Some(doc) = self.doc().filter(|d| d.has_audio()) else {
            return;
        };
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
            && doc
                .undo
                .last()
                .is_some_and(|s| s.edit == before && s.timeline == doc.timeline)
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
            Drag::Clips {
                anchor_x,
                pointer,
                moved,
                target,
                ..
            } => {
                if !*moved && (x - *anchor_x).abs() < CLIP_SLOP {
                    return;
                }
                *moved = true;
                *pointer = time;
                // Snap to the join nearest the pointer; joins that would
                // leave the order as it is show no indicator.
                let boundary = doc.timeline.nearest_boundary(doc.frame_at(time));
                let mut trial = doc.timeline.clone();
                *target = trial.move_clips(&doc.selected, boundary).map(|_| boundary);
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
            Drag::Clips {
                moved,
                target,
                collapse,
                ..
            } => {
                if let Some(boundary) = target.filter(|_| moved) {
                    self.move_selection(boundary, cx);
                } else if let Some(index) = collapse.filter(|_| !moved) {
                    self.click_clip(
                        ClipClick {
                            index,
                            toggle: false,
                            extend: false,
                        },
                        cx,
                    );
                }
                cx.notify();
                return;
            }
            Drag::Pan => {
                cx.notify();
                return;
            }
        };
        if let Some(doc) = self.doc_mut() {
            if doc
                .edit
                .validate_frames(doc.timeline.frames(), doc.info.sample_rate)
                .is_err()
            {
                doc.edit = origin;
            }
            if doc.edit != origin {
                let before = Snapshot {
                    edit: origin,
                    ..doc.snapshot()
                };
                Self::record(doc, before);
            }
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
        // A click (not a drag) in the waveform also selects the clip under it.
        if let Drag::Range {
            moved: false,
            click: Some(click),
            ..
        } = drag
        {
            self.click_clip(click, cx);
        }
        // Gain already followed live; this reschedules a changed selection.
        self.sync_playback(origin, None);
        cx.notify();
    }

    fn set_hover(&mut self, hover: Option<Handle>, cx: &mut Context<Self>) {
        if self.hover != hover {
            self.hover = hover;
            cx.notify();
        }
    }

    /// Pointer feedback over the stage: the razor line (Razor tool only) and
    /// whether the clip bar is under the pointer.
    fn set_pointer(&mut self, razor: Option<f64>, lane: bool, cx: &mut Context<Self>) {
        let razor = razor.filter(|_| self.tool == Tool::Razor);
        if self.razor_at != razor || self.lane_hover != lane {
            self.razor_at = razor;
            self.lane_hover = lane;
            cx.notify();
        }
    }

    // ---- Export --------------------------------------------------------

    fn prompt_export(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog_open {
            return;
        }
        let format = self.export_format();
        let Some(doc) = self.doc().filter(|d| d.has_audio()) else {
            return;
        };
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
        // The job owns its copy of the arrangement (source ranges only), so
        // edits made while it runs never change what it writes.
        let (info, edit, timeline) = (doc.info.clone(), doc.edit, doc.timeline.clone());

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
        if timeline.is_empty() {
            self.error(
                "Nothing to export — every clip was deleted. Undo or reset.",
                cx,
            );
            return;
        }
        if let Err(err) = edit.validate_frames(timeline.frames(), info.sample_rate) {
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
                let result = audio::export_timeline(
                    &info,
                    &timeline,
                    edit,
                    &destination,
                    format,
                    &cancel,
                    &progress,
                );
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
        } else if matches!(self.drag, Some(Drag::Clips { .. })) {
            self.drag = None;
            cx.notify();
        } else if self.tool == Tool::Razor || self.doc().is_some_and(|d| !d.selected.is_empty()) {
            // Back to a neutral state: Select tool, nothing selected.
            self.set_tool(Tool::Select, cx);
            self.deselect_clips(cx);
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
