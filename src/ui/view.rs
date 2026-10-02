//! Rendering for the Sonora window. Top to bottom: a translucent black
//! titlebar carrying the dotted mark and the file, the stage (an empty dot
//! field, or the edit bar over the dot-matrix waveform), the transport, and
//! a one-line status strip. Secondary controls fold away as the window
//! narrows; every one of them stays reachable from the keyboard and menus.

use gpui::{
    AnyElement, Context, Div, ExternalPaths, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, SharedString, Stateful, StatefulInteractiveElement, Styled,
    Window, div, prelude::*, px,
};

use super::logic::{self, FORMATS};
use super::widgets::{
    MARK_LARGE, MARK_SMALL, caps, dot_glyph, icon, icon_button, icon_ink, keycap, link_button,
    play_button, primary_button, tooltip,
};
use super::*;

const TITLEBAR: f32 = 40.0;

#[derive(Clone, Copy, PartialEq)]
enum Clip {
    None,
    /// Gain is above unity but peaks aren't measured yet.
    Possible,
    /// The quick overview says it will clip; the exact scan may refine it.
    Likely,
    Certain,
}

impl Render for Sonora {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Animate only while the engine is playing; idle windows don't redraw.
        // The opening indicator animates too.
        if self.tick_playback() || matches!(self.phase, Phase::Opening(_)) {
            window.request_animation_frame();
        }
        // Nothing else in the window takes focus, so an active window with no
        // focus (after a native panel, say) is a lost keyboard: take it back.
        if window.is_window_active() && window.focused(cx).is_none() {
            self.refocus(window);
        }

        let title = match &self.phase {
            Phase::Ready(doc) => doc.name.clone(),
            _ => "Sonora".to_string(),
        };
        if title != self.window_title {
            window.set_window_title(&title);
            self.window_title = title;
        }
        self.sync_menus(cx);

        let width = f32::from(window.viewport_size().width);

        let root = div()
            .id("sonora")
            .track_focus(&self.focus)
            .key_context("Sonora")
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::surface())
            .font_family(theme::UI_FONT)
            .text_size(px(12.5))
            .text_color(theme::text())
            .on_drop(cx.listener(Self::dropped))
            // Capture phase: runs even when a canvas stops propagation, so
            // any click in the window gives the keyboard back to the editor.
            .capture_any_mouse_down(cx.listener(|this, _, window, _| this.refocus(window)));

        let root = self.register_actions(root, cx);
        let ready = self.doc().is_some();

        root.child(self.render_titlebar(width))
            .child(self.render_stage(width, cx))
            .when(ready, |root| root.child(self.render_transport(width, cx)))
            .child(self.render_footer(width, cx))
            .child(drop_overlay(self.doc().is_some(), cx))
            .when(self.help, |root| root.child(help_sheet(cx)))
    }
}

impl Sonora {
    fn dropped(&mut self, paths: &ExternalPaths, _: &mut Window, cx: &mut Context<Self>) {
        self.open_paths(paths.paths().to_vec(), cx);
    }

    /// Actions are only registered when they can run, which also drives the
    /// enabled state of their menu items.
    fn register_actions(&self, root: Stateful<Div>, cx: &mut Context<Self>) -> Stateful<Div> {
        let doc = self.doc();
        let ready = doc.is_some();
        // A document whose clips were all deleted keeps only history,
        // reset and the tools; nothing plays, trims or exports.
        let audible = doc.is_some_and(Document::has_audio);
        let playable = audible && doc.is_some_and(|d| d.playable);
        let exporting = doc.is_some_and(|d| d.export.is_some());
        let clips_selected = doc.is_some_and(|d| !d.selected.is_empty());
        let can_undo = doc.is_some_and(|d| !d.undo.is_empty());
        let can_redo = doc.is_some_and(|d| !d.redo.is_empty());
        let edited = doc.is_some_and(Document::is_edited);
        let zoomed = doc.is_some_and(|d| !d.view.is_full(d.duration()));
        let opening = matches!(self.phase, Phase::Opening(_));
        let cancellable = self.help
            || opening
            || exporting
            || self.notice.is_some()
            || self.tool == Tool::Razor
            || clips_selected;
        let (wav, mp3, m4a) = (
            self.can_use(ExportFormat::Wav),
            self.can_use(ExportFormat::Mp3),
            self.can_use(ExportFormat::M4a),
        );

        root.on_action(cx.listener(|this, _: &Open, window, cx| this.prompt_open(window, cx)))
            .on_action(cx.listener(|_, _: &Minimize, window, _| window.minimize_window()))
            .on_action(cx.listener(|_, _: &Zoom, window, _| window.zoom_window()))
            .on_action(cx.listener(|this, _: &ToggleHelp, _, cx| this.toggle_help(cx)))
            .on_action(cx.listener(|this, _: &ToggleLoop, _, cx| this.toggle_loop(cx)))
            .when(self.updates.is_ready(), |root| {
                root.on_action(
                    cx.listener(|_, _: &CheckForUpdates, _, _| updater::check_for_updates()),
                )
                .on_action(cx.listener(
                    |this, _: &ToggleAutomaticUpdates, _, cx| this.toggle_automatic_updates(cx),
                ))
            })
            .when(wav, |root| {
                root.on_action(
                    cx.listener(|this, _: &UseWav, _, cx| this.set_format(ExportFormat::Wav, cx)),
                )
            })
            .when(mp3, |root| {
                root.on_action(
                    cx.listener(|this, _: &UseMp3, _, cx| this.set_format(ExportFormat::Mp3, cx)),
                )
            })
            .when(m4a, |root| {
                root.on_action(
                    cx.listener(|this, _: &UseM4a, _, cx| this.set_format(ExportFormat::M4a, cx)),
                )
            })
            .when(ready || opening, |root| {
                root.on_action(cx.listener(|this, _: &CloseFile, _, cx| this.close_file(cx)))
            })
            .when(ready, |root| {
                root.on_action(cx.listener(|this, _: &RevealSource, _, cx| this.reveal_source(cx)))
                    .on_action(
                        cx.listener(|this, _: &SelectTool, _, cx| this.set_tool(Tool::Select, cx)),
                    )
                    .on_action(
                        cx.listener(|this, _: &RazorTool, _, cx| this.set_tool(Tool::Razor, cx)),
                    )
            })
            .when(clips_selected, |root| {
                root.on_action(cx.listener(|this, _: &DeleteClips, _, cx| this.delete_selected(cx)))
                    .on_action(
                        cx.listener(|this, _: &DeselectClips, _, cx| this.deselect_clips(cx)),
                    )
            })
            .when(audible, |root| {
                root.on_action(cx.listener(|this, _: &SelectAll, _, cx| this.select_all_clips(cx)))
                    .on_action(
                        cx.listener(|this, _: &SplitAtPlayhead, _, cx| this.split_at_playhead(cx)),
                    )
                    .on_action(cx.listener(|this, _: &SetIn, _, cx| this.set_in(cx)))
                    .on_action(cx.listener(|this, _: &SetOut, _, cx| this.set_out(cx)))
                    .on_action(
                        cx.listener(|this, _: &ResetStart, _, cx| {
                            this.reset_edge(Handle::Start, cx)
                        }),
                    )
                    .on_action(
                        cx.listener(|this, _: &ResetEnd, _, cx| this.reset_edge(Handle::End, cx)),
                    )
                    .on_action(
                        cx.listener(|this, _: &ToggleOriginal, _, cx| this.toggle_original(cx)),
                    )
                    .on_action(cx.listener(|this, _: &SeekBack, _, cx| this.seek_by(-1.0, cx)))
                    .on_action(cx.listener(|this, _: &SeekForward, _, cx| this.seek_by(1.0, cx)))
                    .on_action(cx.listener(|this, _: &SeekBackFar, _, cx| this.seek_by(-5.0, cx)))
                    .on_action(cx.listener(|this, _: &SeekForwardFar, _, cx| this.seek_by(5.0, cx)))
                    .on_action(cx.listener(|this, _: &NudgeBack, _, cx| this.seek_by(-0.01, cx)))
                    .on_action(cx.listener(|this, _: &NudgeForward, _, cx| this.seek_by(0.01, cx)))
                    .on_action(cx.listener(|this, _: &GoToStart, _, cx| {
                        let start = this.doc().map_or(0.0, |d| d.edit.start);
                        this.seek_to(start, cx);
                    }))
                    .on_action(cx.listener(|this, _: &GoToEnd, _, cx| {
                        let end = this.doc().map_or(0.0, |d| d.edit.end);
                        this.seek_to(end, cx);
                    }))
                    .on_action(
                        cx.listener(|this, _: &GainUp, _, cx| this.nudge_gain(GAIN_STEP, cx)),
                    )
                    .on_action(
                        cx.listener(|this, _: &GainDown, _, cx| this.nudge_gain(-GAIN_STEP, cx)),
                    )
                    .on_action(cx.listener(|this, _: &ResetGain, _, cx| this.reset_gain(cx)))
                    .on_action(
                        cx.listener(|this, _: &ZoomToSelection, _, cx| this.zoom_to_selection(cx)),
                    )
                    .on_action(cx.listener(|this, _: &ZoomIn, _, cx| this.zoom_view(2.0, None, cx)))
            })
            .when(zoomed, |root| {
                root.on_action(
                    cx.listener(|this, _: &ZoomOut, _, cx| this.zoom_view(0.5, None, cx)),
                )
                .on_action(cx.listener(|this, _: &ZoomToFit, _, cx| this.zoom_to_fit(cx)))
            })
            .when(playable, |root| {
                root.on_action(cx.listener(|this, _: &PlayPause, _, cx| this.toggle_playback(cx)))
            })
            .when(audible && !exporting, |root| {
                root.on_action(
                    cx.listener(|this, _: &Export, window, cx| this.prompt_export(window, cx)),
                )
            })
            .when(edited, |root| {
                root.on_action(cx.listener(|this, _: &ResetEdits, _, cx| this.reset_edits(cx)))
            })
            .when(can_undo, |root| {
                root.on_action(cx.listener(|this, _: &Undo, _, cx| this.undo(cx)))
            })
            .when(can_redo, |root| {
                root.on_action(cx.listener(|this, _: &Redo, _, cx| this.redo(cx)))
            })
            .when(cancellable, |root| {
                root.on_action(cx.listener(|this, _: &Cancel, _, cx| this.cancel(cx)))
            })
    }

    // ---- Titlebar ------------------------------------------------------

    fn render_titlebar(&self, width: f32) -> impl IntoElement {
        let brand = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(9.))
            .child(dot_glyph(
                &MARK_SMALL,
                2.,
                1.5,
                theme::white(0.95),
                theme::white(0.12),
            ))
            .child(caps("Sonora", 10., theme::text()));

        let file: Option<AnyElement> = match &self.phase {
            Phase::Ready(doc) => Some(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .min_w_0()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::text())
                            .child(doc.name.clone()),
                    )
                    .when(doc.is_edited(), |el| {
                        el.child(
                            div()
                                .id("edited")
                                .flex()
                                .flex_none()
                                .items_center()
                                .gap(px(5.))
                                .tooltip(tooltip(
                                    "Edited — the original file is never changed",
                                    None,
                                ))
                                .child(div().size(px(5.)).rounded_full().bg(theme::white(0.9)))
                                .child(caps("Edited", 9., theme::text_faint())),
                        )
                    })
                    .when(doc.info.is_extracted(), |el| {
                        el.child(
                            div()
                                .id("from-video")
                                .flex_none()
                                .h(px(16.))
                                .px(px(5.))
                                .flex()
                                .items_center()
                                .rounded(px(4.))
                                .border_1()
                                .border_color(theme::white(0.2))
                                .tooltip(tooltip(
                                    "Audio from the video's first audio track. Exports are audio only.",
                                    None,
                                ))
                                .child(caps("From video", 8., theme::text_muted())),
                        )
                    })
                    .into_any_element(),
            ),
            Phase::Opening(import) => Some(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(theme::text_muted())
                    .child(import.name.clone())
                    .into_any_element(),
            ),
            Phase::Empty => None,
        };

        let facts = self.doc().filter(|_| width >= 700.0).map(|doc| {
            let clips = doc.timeline.clips().len();
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap(px(10.))
                .font(theme::numeric(FontWeight::MEDIUM))
                .text_size(px(11.))
                .text_color(theme::text_faint())
                .when(clips != 1, |row| {
                    row.child(if clips == 0 {
                        "No clips".to_string()
                    } else {
                        format!("{clips} clips")
                    })
                    .child(separator_dot())
                })
                .child(theme::sample_rate(doc.info.sample_rate))
                .child(separator_dot())
                .child(theme::channels(doc.info.channels))
                .child(separator_dot())
                .child(theme::timecode(doc.duration()))
        });

        div()
            .h(px(TITLEBAR))
            .flex_none()
            .w_full()
            .pl(px(86.))
            .pr(px(16.))
            .flex()
            .items_center()
            .gap(px(12.))
            .bg(theme::chrome())
            .border_b_1()
            .border_color(theme::hairline())
            .on_mouse_down(MouseButton::Left, |event, window, _| {
                if event.click_count == 2 {
                    window.titlebar_double_click();
                }
            })
            .child(brand)
            .when_some(file, |bar, file| {
                bar.child(div().w(px(1.)).h(px(14.)).bg(theme::white(0.14)))
                    .child(file)
            })
            .child(div().flex_1())
            .children(facts)
    }

    // ---- Stage ---------------------------------------------------------

    fn render_stage(&self, width: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let stage = div()
            .flex_1()
            .min_h_0()
            .px(px(12.))
            .pt(px(8.))
            .flex()
            .flex_col();
        match &self.phase {
            Phase::Empty => stage.child(self.render_empty(cx)),
            Phase::Opening(import) => stage.child(render_opening(import, cx)),
            Phase::Ready(doc) if !doc.has_audio() => stage
                .child(self.render_edit_bar(doc, width, cx))
                .child(render_empty_timeline(cx)),
            Phase::Ready(doc) => {
                let dragging = match self.drag {
                    Some(Drag::Handle { handle, .. }) => Some(handle),
                    _ => None,
                };
                // A reorder drag that has moved: the block follows the
                // pointer and the clips it lifted stay as outlines.
                let reorder = match self.drag {
                    Some(Drag::Clips {
                        moved: true,
                        grab,
                        pointer,
                        target,
                        ..
                    }) => Some((pointer - grab, target)),
                    _ => None,
                };
                let ghost = reorder.map(|(start, _)| {
                    let clips = doc.timeline.clips();
                    let frames: u64 = doc.selected.iter().map(|&i| clips[i].frames()).sum();
                    wave::Ghost {
                        start,
                        end: start + doc.seconds(frames),
                        count: doc.selected.len(),
                    }
                });
                let insertion = reorder
                    .and_then(|(_, target)| target)
                    .map(|b| doc.seconds(doc.timeline.boundary_frame(b)));
                let message: Option<SharedString> = if let Some(err) = &doc.wave_error {
                    Some(
                        format!("Waveform unavailable — {err} Preview and export still work.")
                            .into(),
                    )
                } else if doc.peaks.is_none() {
                    Some("Reading waveform…".into())
                } else {
                    None
                };
                let scene = wave::WaveScene {
                    peaks: doc.peaks.clone(),
                    exact: doc.peaks_exact,
                    duration: doc.duration(),
                    source_duration: doc.info.duration(),
                    view: doc.view,
                    start: doc.edit.start,
                    end: doc.edit.end,
                    gain_db: doc.preview_gain(),
                    playhead: self.position(),
                    hover: self.hover,
                    dragging,
                    message,
                    tool: self.tool,
                    clips: scene_clips(doc, reorder.is_some()),
                    razor: self.razor_at.filter(|_| self.drag.is_none()),
                    insertion,
                    ghost,
                    lane_hover: self.lane_hover,
                    holding_clips: matches!(self.drag, Some(Drag::Clips { .. })),
                };
                stage.child(self.render_edit_bar(doc, width, cx)).child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .rounded(px(10.))
                        .bg(theme::well())
                        .border_1()
                        .border_color(theme::hairline())
                        .overflow_hidden()
                        .child(wave::waveform(scene, cx.entity().downgrade())),
                )
            }
        }
    }

    /// The strip above the waveform: trim points and history on the left,
    /// listening mode and zoom on the right.
    fn render_edit_bar(
        &self,
        doc: &Document,
        width: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let edit = doc.edit;
        let duration = doc.duration();
        let can_undo = !doc.undo.is_empty();
        let can_redo = !doc.redo.is_empty();
        let edited = doc.is_edited();
        let audible = doc.has_audio();

        // The two tools, as one compact segmented control.
        let tool = self.tool;
        let tools = div()
            .flex()
            .flex_none()
            .items_center()
            .h(px(28.))
            .p(px(2.))
            .gap(px(2.))
            .rounded(px(8.))
            .bg(theme::white(0.04))
            .border_1()
            .border_color(theme::white(0.12))
            .child(
                tool_button("tool-select", tool == Tool::Select)
                    .tooltip(tooltip(
                        "Select — click a clip to select it, drag its bar to reorder",
                        Some("V"),
                    ))
                    .on_click(cx.listener(|this, _, _, cx| this.set_tool(Tool::Select, cx)))
                    .child(icon(
                        "icons/select.svg",
                        14.,
                        icon_ink(true, tool == Tool::Select),
                    )),
            )
            .child(
                tool_button("tool-razor", tool == Tool::Razor)
                    .tooltip(tooltip(
                        "Razor — click a clip to cut it there · ⌘B cuts at the playhead",
                        Some("C"),
                    ))
                    .on_click(cx.listener(|this, _, _, cx| this.set_tool(Tool::Razor, cx)))
                    .child(icon(
                        "icons/razor.svg",
                        14.,
                        icon_ink(true, tool == Tool::Razor),
                    )),
            );

        let trim = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .child(
                trim_chip("set-in", "In", edit.start)
                    .tooltip(tooltip(
                        "Set start at playhead · ⇧I or double-click the handle resets it",
                        Some("I"),
                    ))
                    .on_click(cx.listener(|this, _, _, cx| this.set_in(cx))),
            )
            .child(div().w(px(10.)).h(px(1.)).bg(theme::white(0.25)))
            .child(
                trim_chip("set-out", "Out", edit.end)
                    .tooltip(tooltip(
                        "Set end at playhead · ⇧O or double-click the handle resets it",
                        Some("O"),
                    ))
                    .on_click(cx.listener(|this, _, _, cx| this.set_out(cx))),
            );

        let history = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .child(
                icon_button("undo", can_undo, false)
                    .tooltip(tooltip("Undo", Some("⌘Z")))
                    .when(can_undo, |b| {
                        b.on_click(cx.listener(|this, _, _, cx| this.undo(cx)))
                    })
                    .child(icon("icons/undo.svg", 15., icon_ink(can_undo, false))),
            )
            .child(
                icon_button("redo", can_redo, false)
                    .tooltip(tooltip("Redo", Some("⇧⌘Z")))
                    .when(can_redo, |b| {
                        b.on_click(cx.listener(|this, _, _, cx| this.redo(cx)))
                    })
                    .child(icon("icons/redo.svg", 15., icon_ink(can_redo, false))),
            )
            .child(
                icon_button("reset", edited, false)
                    .tooltip(tooltip("Reset cuts, clip order, trim and gain", Some("⌘⌫")))
                    .when(edited, |b| {
                        b.on_click(cx.listener(|this, _, _, cx| this.reset_edits(cx)))
                    })
                    .child(icon("icons/reset.svg", 14., icon_ink(edited, false))),
            );

        let view = doc.view;
        let zoomed = !view.is_full(duration);
        let at_max = view.zoom(duration) >= logic::MAX_ZOOM - 1e-6;
        let bins = doc.peaks.as_ref().map_or(0, |p| p.len());
        // Roughly one dot column per 5 px of the well.
        let columns = ((width - 24.0 - 32.0) / 5.0).max(1.0) as f64;
        // The overview's bins span the source, whatever the arrangement.
        let magnified = bins > 0 && logic::visible_bins(bins, view, doc.info.duration()) < columns;

        let readout = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .when(magnified, |el| {
                el.child(
                    div()
                        .id("overview-res")
                        .px(px(6.))
                        .h(px(18.))
                        .flex()
                        .items_center()
                        .rounded(px(4.))
                        .border_1()
                        .border_color(theme::white(0.18))
                        .tooltip(tooltip(
                            format!(
                                "Zoomed past the {bins}-point overview — dots are magnified; audio is untouched"
                            ),
                            None,
                        ))
                        .child(caps("Overview res", 8.5, theme::text_faint())),
                )
            })
            .when(zoomed && width >= 900.0, |el| {
                el.child(
                    div()
                        .font(theme::numeric(FontWeight::MEDIUM))
                        .text_size(px(11.))
                        .text_color(theme::text_muted())
                        .child(format!(
                            "{} – {}",
                            theme::timecode(view.start),
                            theme::timecode(view.end)
                        )),
                )
            })
            .child(
                div()
                    .min_w(px(30.))
                    .font(theme::numeric(FontWeight::SEMIBOLD))
                    .text_size(px(11.))
                    .text_color(if zoomed { theme::text() } else { theme::text_ghost() })
                    .child(format!("{:.1}×", view.zoom(duration))),
            );

        let zoom = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .child(
                icon_button("zoom-out", zoomed, false)
                    .tooltip(tooltip("Zoom out", Some("⌘−")))
                    .when(zoomed, |b| {
                        b.on_click(cx.listener(|this, _, _, cx| this.zoom_view(0.5, None, cx)))
                    })
                    .child(icon("icons/zoom-out.svg", 15., icon_ink(zoomed, false))),
            )
            .child(
                icon_button("zoom-in", !at_max, false)
                    .tooltip(tooltip(
                        "Zoom in · ⌘-scroll zooms at the pointer",
                        Some("⌘="),
                    ))
                    .when(!at_max, |b| {
                        b.on_click(cx.listener(|this, _, _, cx| this.zoom_view(2.0, None, cx)))
                    })
                    .child(icon("icons/zoom-in.svg", 15., icon_ink(!at_max, false))),
            )
            .child(
                icon_button("zoom-sel", true, false)
                    .tooltip(tooltip("Zoom to selection", Some("Z")))
                    .on_click(cx.listener(|this, _, _, cx| this.zoom_to_selection(cx)))
                    .child(icon("icons/zoom-selection.svg", 15., icon_ink(true, false))),
            )
            .child(
                icon_button("zoom-fit", zoomed, false)
                    .tooltip(tooltip("Show entire file", Some("⇧Z")))
                    .when(zoomed, |b| {
                        b.on_click(cx.listener(|this, _, _, cx| this.zoom_to_fit(cx)))
                    })
                    .child(icon("icons/zoom-fit.svg", 15., icon_ink(zoomed, false))),
            );

        let original = doc.original.then(|| {
            div()
                .id("original-badge")
                .flex_none()
                .h(px(20.))
                .px(px(8.))
                .flex()
                .items_center()
                .rounded(px(10.))
                .bg(theme::white(0.94))
                .cursor_pointer()
                .tooltip(tooltip(
                    format!(
                        "Hearing the original level. Export still applies {}.",
                        theme::decibels(edit.gain_db)
                    ),
                    Some("B"),
                ))
                .on_click(cx.listener(|this, _, _, cx| this.toggle_original(cx)))
                .child(caps("A · Original", 9., theme::black(0.9)))
        });

        div()
            .h(px(38.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(10.))
            .child(tools)
            .child(vertical_rule())
            .when(audible, |bar| bar.child(trim))
            .when(width >= 700.0 || !audible, |bar| {
                bar.when(audible, |bar| bar.child(vertical_rule()))
                    .child(history)
            })
            .child(div().flex_1())
            .when(audible, |bar| {
                bar.children(original).child(readout).child(zoom)
            })
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let error = match &self.notice {
            Some(Notice::Error(message)) => Some(message.clone()),
            _ => None,
        };
        // Drops are taken by the whole-window overlay.
        field_zone()
            .child(dot_glyph(
                &MARK_LARGE,
                6.,
                4.,
                theme::white(0.96),
                theme::white(0.07),
            ))
            .child(
                div()
                    .mt(px(26.))
                    .text_size(px(22.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text())
                    .child("Drop audio or video to begin."),
            )
            .child(
                div()
                    .mt(px(8.))
                    .max_w(px(400.))
                    .text_center()
                    .text_color(theme::text_muted())
                    .child(
                        "Trim, set the level, export WAV, MP3 or M4A. From a video, Sonora \
                         edits its first audio track. The original is never touched.",
                    ),
            )
            .child(
                primary_button("open", true)
                    .mt(px(24.))
                    .tooltip(tooltip("Choose an audio or video file", Some("⌘O")))
                    .on_click(cx.listener(|this, _, window, cx| this.prompt_open(window, cx)))
                    .child(icon("icons/open.svg", 14., theme::black(0.9)))
                    .child("Open…"),
            )
            .when_some(error, |zone, message| {
                zone.child(
                    div()
                        .mt(px(18.))
                        .w_full()
                        .max_w(px(440.))
                        .flex()
                        .items_start()
                        .gap(px(8.))
                        .text_color(theme::text())
                        .child(div().flex_none().pt(px(1.)).child(icon(
                            "icons/warning.svg",
                            14.,
                            theme::white(1.0),
                        )))
                        // A sized flex item so long messages wrap inside the row.
                        .child(div().flex_1().min_w_0().whitespace_normal().child(message)),
                )
            })
            .child(div().mt(px(30.)).child(caps(
                "Wav  Aiff  Mp3  M4a  Aac  Flac  Caf  ·  Mov  Mp4  M4v",
                9.,
                theme::text_ghost(),
            )))
    }

    // ---- Transport -----------------------------------------------------

    fn render_transport(&self, width: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(doc) = self.doc() else {
            return div().into_any_element();
        };
        let playing = doc.playing;
        let audible = doc.has_audio();
        let playable = doc.playable && audible;
        let exporting = doc.export.is_some();
        let edit = doc.edit;
        let clip = clip_state(doc);
        let format = self.export_format();
        let fold = TransportFold::for_width(width);

        let to_start = icon_button("to-start", audible, false)
            .tooltip(tooltip("Go to selection start", Some("↑")))
            .when(audible, |b| {
                b.on_click(cx.listener(|this, _, _, cx| {
                    let start = this.doc().map_or(0.0, |d| d.edit.start);
                    this.seek_to(start, cx);
                }))
            })
            .child(icon("icons/to-start.svg", 14., icon_ink(audible, false)));

        // A distinct id per state, so hover and tooltip state never carry a
        // stale "Play" across into playback (or back).
        let play = play_button(if playing { "pause" } else { "play" }, playable)
            .tooltip(if !audible {
                tooltip("Nothing to play — every clip was deleted", None)
            } else if !playable {
                tooltip("Preview unavailable for this file", None)
            } else if playing {
                tooltip("Pause", Some("Space"))
            } else {
                tooltip("Play selection", Some("Space"))
            })
            .when(playable, |b| {
                b.on_click(cx.listener(|this, _, _, cx| this.toggle_playback(cx)))
            })
            .child(icon(
                if playing {
                    "icons/pause.svg"
                } else {
                    "icons/play.svg"
                },
                17.,
                if playable {
                    theme::black(0.92)
                } else {
                    theme::text_ghost()
                },
            ));

        let looping = self.looping;
        let loop_button = icon_button("loop", true, looping)
            .tooltip(tooltip(
                if looping {
                    "Looping the selection — click to play once"
                } else {
                    "Loop the selection"
                },
                Some("L"),
            ))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_loop(cx)))
            .child(icon("icons/loop.svg", 15., icon_ink(true, looping)));

        let clock = div()
            .id("clock")
            .flex_none()
            .min_w(px(92.))
            .whitespace_nowrap()
            .tooltip(tooltip("Playhead · selection length", None))
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                div()
                    .font(theme::numeric(FontWeight::MEDIUM))
                    .text_size(px(19.))
                    .line_height(px(22.))
                    .text_color(theme::text())
                    .child(theme::timecode(self.position())),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(caps("Sel", 8.5, theme::text_faint()))
                    .child(
                        div()
                            .font(theme::numeric(FontWeight::MEDIUM))
                            .text_size(px(11.))
                            .text_color(theme::text_muted())
                            .child(theme::timecode(edit.end - edit.start)),
                    ),
            );

        let original = doc.original;
        let ab = div()
            .id("ab")
            .flex()
            .flex_none()
            .items_center()
            .h(px(24.))
            .p(px(2.))
            .gap(px(2.))
            .rounded(px(7.))
            .border_1()
            .border_color(theme::white(0.14))
            .cursor_pointer()
            .hover(|s| s.border_color(theme::white(0.3)))
            .tooltip(tooltip(
                "A/B — A plays the original level, B the edit. Preview only; export always uses B",
                Some("B"),
            ))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_original(cx)))
            .child(ab_letter("A", original))
            .child(ab_letter("B", !original));

        let gain_readout = div()
            .id("gain-readout")
            // Wide enough for "+24.0 dB"; never wraps the unit.
            .min_w(px(62.))
            .flex_none()
            .whitespace_nowrap()
            .font(theme::numeric(FontWeight::SEMIBOLD))
            .text_size(px(12.5))
            .text_color(if original {
                theme::text_faint()
            } else {
                theme::text()
            })
            .cursor_pointer()
            .tooltip(tooltip("Gain · double-click for 0 dB", Some("+ / −")))
            .on_click(cx.listener(|this, event: &gpui::ClickEvent, _, cx| {
                if event.click_count() == 2 {
                    this.reset_gain(cx);
                }
            }))
            .child(theme::decibels(edit.gain_db));

        let gain = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .when(fold.gain_label, |el| {
                el.child(caps("Gain", 8.5, theme::text_faint()))
            })
            .when_some(fold.slider, |el, slider_width| {
                el.child(
                    div()
                        .id("gain-slider")
                        .tooltip(tooltip(
                            "Drag to set gain · double-click for 0 dB",
                            Some("+ / −"),
                        ))
                        .child(wave::gain_slider(
                            wave::GainScene {
                                gain_db: edit.gain_db,
                                dragging: matches!(self.drag, Some(Drag::Gain { .. })),
                                width: slider_width,
                            },
                            cx.entity().downgrade(),
                        )),
                )
            })
            .child(gain_readout)
            .child(clip_badge(clip))
            .when(fold.ab, |el| el.child(ab));

        let formats = div()
            .flex()
            .flex_none()
            .items_center()
            .h(px(28.))
            .p(px(2.))
            .gap(px(2.))
            .rounded(px(8.))
            .bg(theme::white(0.04))
            .border_1()
            .border_color(theme::white(0.12))
            .children(FORMATS.iter().map(|&f| {
                let usable = self.can_use(f) && !exporting;
                let selected = f == format;
                let keys = match f {
                    ExportFormat::Wav => "⌘1",
                    ExportFormat::Mp3 => "⌘2",
                    ExportFormat::M4a => "⌘3",
                };
                let tip = if self.can_use(f) {
                    format!("{} · {}", f.label(), logic::format_detail(f))
                } else {
                    format!(
                        "{} needs mono or stereo — this file has {} channels",
                        f.label(),
                        doc.info.channels
                    )
                };
                div()
                    .id(SharedString::from(format!("format-{}", f.extension())))
                    .h_full()
                    .px(px(if fold.tight_formats { 6. } else { 8. }))
                    .flex()
                    .items_center()
                    .rounded(px(6.))
                    .font(theme::numeric(FontWeight::SEMIBOLD))
                    .text_size(px(10.5))
                    .tooltip(tooltip(tip, Some(keys)))
                    .map(|el| match (selected, usable) {
                        (true, _) => el.bg(theme::white(0.94)).text_color(theme::black(0.92)),
                        (false, true) => el
                            .text_color(theme::text_muted())
                            .cursor_pointer()
                            .hover(|s| s.bg(theme::white(0.08)).text_color(theme::text())),
                        (false, false) => el.text_color(theme::text_ghost()),
                    })
                    .when(usable && !selected, |el| {
                        el.on_click(cx.listener(move |this, _, _, cx| this.set_format(f, cx)))
                    })
                    .child(if fold.tight_formats {
                        f.label().to_string()
                    } else {
                        theme::tracked(f.label())
                    })
            }));

        let export: AnyElement = match &doc.export {
            Some(job) => {
                let fraction = job.shown as f32 / 1000.0;
                let cancelling = job.cancel.load(Ordering::Relaxed);
                primary_button("export", false)
                    .min_w(px(104.))
                    .justify_center()
                    .text_color(theme::text())
                    .when(!cancelling, |b| {
                        b.cursor_pointer()
                            .hover(|s| s.bg(theme::white(0.14)))
                            .tooltip(tooltip("Cancel export", Some("Esc")))
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx)))
                    })
                    .child(if cancelling {
                        "Cancelling…".to_string()
                    } else {
                        format!("{}%", (fraction * 100.0).floor() as u32)
                    })
                    .into_any_element()
            }
            None if !audible => primary_button("export", false)
                .tooltip(tooltip(
                    "Nothing to export — undo or reset to bring clips back",
                    None,
                ))
                .child(icon("icons/export.svg", 13., theme::text_faint()))
                .child(format!("Export {}", format.label()))
                .into_any_element(),
            None => primary_button("export", true)
                .tooltip(tooltip(
                    format!(
                        "Export the trimmed timeline as {} ({})",
                        format.label(),
                        logic::format_detail(format)
                    ),
                    Some("⇧⌘S"),
                ))
                .on_click(cx.listener(|this, _, window, cx| this.prompt_export(window, cx)))
                .child(icon("icons/export.svg", 13., theme::black(0.9)))
                .child(format!("Export {}", format.label()))
                .into_any_element(),
        };

        // The right-hand group (gain, format, Export) never shrinks; if the
        // estimates in `TransportFold` are ever short, the playback cluster
        // on the left is what gets clipped, never the Export button.
        let playback = div()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .flex()
            .items_center()
            .gap(px(10.))
            .when(fold.to_start, |row| row.child(to_start))
            .child(play)
            .child(loop_button)
            .child(div().flex_none().w(px(4.)))
            .child(clock);
        let output = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(10.))
            .child(gain)
            .child(vertical_rule())
            .child(formats)
            .child(export);

        div()
            .h(px(64.))
            .flex_none()
            .px(px(16.))
            .flex()
            .items_center()
            .gap(px(10.))
            .child(playback)
            .child(output)
            .into_any_element()
    }

    // ---- Status strip --------------------------------------------------

    fn render_footer(&self, width: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let peak = self.doc().filter(|_| width >= 700.0).and_then(|doc| {
            let peak = selection_peak(doc)?;
            let gained = peak * 10f32.powf(doc.edit.gain_db / 20.0);
            let approx = if doc.peaks_exact { "" } else { "≈ " };
            Some(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(7.))
                    .child(caps("Peak", 8.5, theme::text_ghost()))
                    .child(
                        div()
                            .font(theme::numeric(FontWeight::MEDIUM))
                            .text_size(px(11.))
                            .text_color(theme::text_muted())
                            .child(if gained > 0.0 {
                                format!("{approx}{}", dbfs(gained))
                            } else {
                                "Silent".into()
                            }),
                    ),
            )
        });

        div()
            .h(px(30.))
            .flex_none()
            .px(px(16.))
            .flex()
            .items_center()
            .gap(px(12.))
            .border_t_1()
            .border_color(theme::hairline())
            .bg(theme::black(0.22))
            .text_size(px(11.5))
            .text_color(theme::text_muted())
            .children(peak)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(10.))
                    .child(self.render_status(cx)),
            )
            .child(
                icon_button("help", true, self.help)
                    .size(px(22.))
                    .tooltip(tooltip("Keyboard & gestures", Some("?")))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_help(cx)))
                    .child(icon("icons/help.svg", 14., icon_ink(true, self.help))),
            )
    }

    fn render_status(&self, cx: &mut Context<Self>) -> AnyElement {
        let doc = self.doc();

        if let Some(job) = doc.and_then(|d| d.export.as_ref()) {
            let fraction = job.shown as f32 / 1000.0;
            let cancelling = job.cancel.load(Ordering::Relaxed);
            return status_row()
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(theme::text())
                        .child(if cancelling {
                            "Cancelling…".to_string()
                        } else {
                            format!(
                                "Exporting {} · {}",
                                job.format.label(),
                                display_name(&job.destination)
                            )
                        }),
                )
                .child(progress_bar(fraction, 96.))
                .into_any_element();
        }

        // On the empty screen errors are shown in the drop zone itself.
        let notice = if doc.is_none() && matches!(self.notice, Some(Notice::Error(_))) {
            None
        } else {
            self.notice.as_ref()
        };
        if let Some(notice) = notice {
            let dismiss = link_button("dismiss", theme::text_muted())
                .tooltip(tooltip("Dismiss", Some("Esc")))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.notice = None;
                    cx.notify();
                }))
                .child(icon("icons/close.svg", 11., theme::text_muted()));
            return match notice {
                Notice::Error(message) => status_row()
                    .text_color(theme::text())
                    .child(icon("icons/warning.svg", 13., theme::white(1.0)))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_weight(FontWeight::MEDIUM)
                            .child(message.clone()),
                    )
                    .child(dismiss)
                    .into_any_element(),
                Notice::Info(message) => status_row()
                    .child(div().min_w_0().truncate().child(message.clone()))
                    .child(dismiss)
                    .into_any_element(),
                Notice::Saved(path) => {
                    let path = path.clone();
                    status_row()
                        .child(icon("icons/check.svg", 13., theme::white(1.0)))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_color(theme::text())
                                .child(format!("Saved {}", display_name(&path))),
                        )
                        .child(
                            link_button("reveal", theme::text())
                                .tooltip(tooltip("Reveal the exported file in Finder", None))
                                .on_click(move |_, _, _| audio::reveal(&path))
                                .child("Show in Finder"),
                        )
                        .child(dismiss)
                        .into_any_element()
                }
            };
        }

        if let Some(hint) = doc.and_then(|doc| self.clip_hint(doc)) {
            return hint;
        }

        if let Some(doc) = doc
            && doc.wave_error.is_none()
            && !doc.peaks_exact
        {
            let label = if doc.peaks.is_none() {
                "Reading waveform…".to_string()
            } else if doc.wave_progress <= 0.0 {
                "Quick overview · scanning…".to_string()
            } else {
                format!(
                    "Refining overview · {}%",
                    (doc.wave_progress * 100.0).floor() as u32
                )
            };
            return status_row()
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_color(theme::text_faint())
                        .child(label),
                )
                .child(progress_bar(doc.wave_progress, 64.))
                .into_any_element();
        }

        if matches!(self.phase, Phase::Empty) {
            return status_row()
                .text_color(theme::text_faint())
                .child("Open")
                .child(keycap("⌘O"))
                .into_any_element();
        }
        // An untouched file: say how cutting starts.
        if let Some(doc) = doc
            && doc.timeline.clips().len() == 1
        {
            return status_row()
                .text_color(theme::text_ghost())
                .child(div().min_w_0().truncate().child("Cut with the razor"))
                .child(keycap("C"))
                .child(div().flex_none().child("or at the playhead"))
                .child(keycap("⌘B"))
                .into_any_element();
        }
        div().into_any_element()
    }

    /// What the clip tools are doing, and the keys that go with it.
    fn clip_hint(&self, doc: &Document) -> Option<AnyElement> {
        let count = doc.selected.len();
        let clips = |n: usize| {
            if n == 1 {
                "1 clip".to_string()
            } else {
                format!("{n} clips")
            }
        };
        let text = |t: String| div().min_w_0().truncate().child(t);
        if let Some(Drag::Clips {
            moved: true,
            target,
            ..
        }) = self.drag
        {
            return Some(
                status_row()
                    .text_color(theme::text())
                    .child(text(if target.is_some() {
                        format!("Release to move {} to the marked join", clips(count))
                    } else {
                        "Move over another join to reorder".into()
                    }))
                    .child(keycap("Esc"))
                    .into_any_element(),
            );
        }
        if self.tool == Tool::Razor {
            return Some(
                status_row()
                    .text_color(theme::text_muted())
                    .child(text("Razor — click a clip to cut it at the pointer".into()))
                    .child(
                        div()
                            .flex_none()
                            .text_color(theme::text_faint())
                            .child("Select"),
                    )
                    .child(keycap("V"))
                    .into_any_element(),
            );
        }
        (count > 0 && doc.has_audio()).then(|| {
            status_row()
                .text_color(theme::text_muted())
                .child(text(format!(
                    "{} selected · drag the clip bar to reorder",
                    clips(count)
                )))
                .child(
                    div()
                        .flex_none()
                        .text_color(theme::text_faint())
                        .child("Ripple delete"),
                )
                .child(keycap("⌫"))
                .into_any_element()
        })
    }
}

// ---- Pieces --------------------------------------------------------------

/// Which secondary transport pieces fit at a given window width. Measured
/// widths (tracked caps, timecodes) were rounded up so the essentials — play,
/// loop, clock, gain value, format picker and the full "Export WAV" button —
/// always fit, down to the 640 px minimum window.
#[derive(Clone, Copy, Debug, PartialEq)]
struct TransportFold {
    /// Width of the gain slider, if shown.
    slider: Option<f32>,
    to_start: bool,
    gain_label: bool,
    ab: bool,
    /// Plain (untracked) format labels with less padding.
    tight_formats: bool,
}

impl TransportFold {
    fn for_width(width: f32) -> Self {
        Self {
            slider: if width >= 820.0 {
                Some(120.)
            } else if width >= 780.0 {
                Some(88.)
            } else {
                None
            },
            to_start: width >= 870.0,
            gain_label: width >= 920.0,
            ab: width >= 700.0,
            tight_formats: width < 700.0,
        }
    }

    /// A conservative estimate of the row's width with these pieces.
    #[cfg(test)]
    fn estimate(&self) -> f32 {
        // Padding, play, loop, spacer, clock, gain readout, clip badge,
        // rule, Export, and the gaps between them.
        let mut total = 32. + 40. + 28. + 4. + 92. + 62. + 14. + 1. + 120. + 86.;
        total += if self.tight_formats { 120. } else { 150. };
        total += self.slider.map_or(0., |w| w + 8.);
        if self.ab {
            total += 46. + 8.;
        }
        if self.to_start {
            total += 28. + 10.;
        }
        if self.gain_label {
            total += 32. + 8.;
        }
        total
    }
}

fn status_row() -> Div {
    div()
        .flex()
        .items_center()
        .justify_end()
        .gap(px(8.))
        .min_w_0()
        .overflow_hidden()
}

fn separator_dot() -> Div {
    div().size(px(3.)).rounded_full().bg(theme::white(0.25))
}

fn vertical_rule() -> Div {
    div().flex_none().w(px(1.)).h(px(22.)).bg(theme::hairline())
}

/// The empty / opening stage: a fading dot field behind centered content.
fn field_zone() -> Div {
    div()
        .relative()
        .flex_1()
        .min_h_0()
        .mb(px(12.))
        .rounded(px(12.))
        .border_1()
        .border_color(theme::hairline())
        .bg(theme::well())
        .overflow_hidden()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .px(px(24.))
        .child(wave::dot_field())
}

fn trim_chip(id: &'static str, label: &'static str, time: f64) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .h(px(28.))
        .px(px(8.))
        .rounded(px(7.))
        .flex()
        .items_center()
        .gap(px(7.))
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(|s| s.bg(theme::white(0.07)))
        .active(|s| s.bg(theme::white(0.12)))
        .child(caps(label, 8.5, theme::text_faint()))
        .child(
            div()
                .font(theme::numeric(FontWeight::MEDIUM))
                .text_size(px(12.5))
                .text_color(theme::text())
                .child(theme::timecode(time)),
        )
}

fn ab_letter(letter: &'static str, on: bool) -> Div {
    div()
        .size(px(18.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(5.))
        .font_weight(FontWeight::BOLD)
        .text_size(px(10.5))
        .map(|el| {
            if on {
                el.bg(theme::white(0.94)).text_color(theme::black(0.92))
            } else {
                el.text_color(theme::text_faint())
            }
        })
        .child(letter)
}

fn progress_bar(fraction: f32, width: f32) -> impl IntoElement {
    let fraction = fraction.clamp(0.0, 1.0);
    div()
        .flex_none()
        .w(px(width))
        .h(px(3.))
        .rounded(px(1.5))
        .bg(theme::white(0.12))
        .child(
            div()
                .h_full()
                .w(px(width * fraction))
                .rounded(px(1.5))
                .bg(theme::white(0.95)),
        )
}

fn clip_badge(clip: Clip) -> impl IntoElement {
    let (label, alpha) = match clip {
        Clip::None => return div().w(px(14.)).flex_none().into_any_element(),
        Clip::Possible => ("Gain above 0 dB may clip once peaks are measured", 0.45),
        Clip::Likely => (
            "The quick overview suggests peaks will clip; export hard-limits them",
            0.9,
        ),
        Clip::Certain => (
            "Peaks will clip — export hard-limits samples to full scale",
            1.0,
        ),
    };
    div()
        .id("clip")
        .flex_none()
        .tooltip(tooltip(label, None))
        .child(icon("icons/warning.svg", 14., theme::white(alpha)))
        .into_any_element()
}

/// Shown over a loaded document while a file is dragged over the window.
/// GPUI only hit-tests an element with a listener, so the overlay takes the
/// drop itself.
fn drop_overlay(loaded: bool, cx: &mut Context<Sonora>) -> impl IntoElement {
    div()
        .id("drop-overlay")
        .absolute()
        .inset_0()
        .opacity(0.)
        .drag_over::<ExternalPaths>(|style, _, _, _| style.opacity(1.))
        .on_drop(cx.listener(Sonora::dropped))
        .p(px(10.))
        .child(
            div()
                .size_full()
                .rounded(px(14.))
                .border_1()
                .border_color(theme::white(0.6))
                .bg(theme::black(0.82))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(14.))
                .child(dot_glyph(
                    &MARK_LARGE,
                    6.,
                    4.,
                    theme::white(0.96),
                    theme::white(0.07),
                ))
                .child(
                    div()
                        .text_size(px(16.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("Release to open audio or video"),
                )
                .child(
                    div()
                        .max_w(px(420.))
                        .text_center()
                        .text_color(theme::text_muted())
                        .child(if loaded {
                            "From a video, its first audio track. The current file closes; \
                             nothing is saved over."
                        } else {
                            "From a video, Sonora edits its first audio track."
                        }),
                ),
        )
}

/// The stage while a file is probed or its audio extracted.
fn render_opening(import: &Import, cx: &mut Context<Sonora>) -> impl IntoElement {
    let caption = if import.video {
        "Extracting audio from video"
    } else {
        "Opening"
    };
    field_zone()
        .child(dot_glyph(
            &MARK_LARGE,
            6.,
            4.,
            theme::white(0.5),
            theme::white(0.06),
        ))
        .child(
            div()
                .mt(px(22.))
                .max_w_full()
                .truncate()
                .text_size(px(15.))
                .font_weight(FontWeight::MEDIUM)
                .child(import.name.clone()),
        )
        .child(
            div()
                .mt(px(10.))
                .flex()
                .items_center()
                .gap(px(12.))
                .child(caps(caption, 9.5, theme::text_faint()))
                .child(scanner(import.started.elapsed().as_secs_f32())),
        )
        .when(import.video, |zone| {
            zone.child(
                div()
                    .mt(px(10.))
                    .text_color(theme::text_muted())
                    .child("Sonora uses the first audio track. Only audio is exported."),
            )
        })
        .child(
            link_button("cancel-open", theme::text_muted())
                .mt(px(18.))
                .tooltip(tooltip("Stop opening this file", Some("Esc")))
                .on_click(cx.listener(|this, _, _, cx| this.cancel_open(cx)))
                .child("Cancel")
                .child(keycap("Esc")),
        )
}

/// An indeterminate progress line: a bright dot sweeping back and forth.
fn scanner(seconds: f32) -> Div {
    const DOTS: usize = 9;
    let travel = (DOTS - 1) as f32;
    let phase = (seconds * 7.0) % (travel * 2.0);
    let head = if phase > travel {
        travel * 2.0 - phase
    } else {
        phase
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.))
        .children((0..DOTS).map(|i| {
            let distance = (i as f32 - head).abs();
            div()
                .size(px(4.))
                .rounded_full()
                .bg(theme::white((1.0 - distance * 0.28).max(0.12)))
        }))
}

/// The keyboard and gesture reference, toggled with `?`.
fn help_sheet(cx: &mut Context<Sonora>) -> impl IntoElement {
    const KEYS: &[(&str, &str)] = &[
        ("Space", "Play / pause"),
        ("L", "Loop selection"),
        ("B", "A/B original level"),
        ("I  O", "Start / end at playhead"),
        ("⇧I  ⇧O", "Reset start / end"),
        ("← →", "Seek 1 s  ·  ⇧ 5 s"),
        ("⌥← →", "Nudge 10 ms"),
        ("↑  ↓", "Selection start / end"),
        ("+  −  0", "Gain up / down / 0 dB"),
        ("Z  ⇧Z", "Zoom to selection / fit"),
        ("⌘=  ⌘−", "Zoom in / out"),
        ("⌘1 2 3", "WAV · MP3 · M4A"),
        ("⌘Z  ⇧⌘Z", "Undo / redo"),
        ("⇧⌘S", "Export"),
    ];
    const CLIPS: &[(&str, &str)] = &[
        ("V", "Select tool"),
        ("C", "Razor tool"),
        ("Razor click", "Cut at the pointer"),
        ("⌘B", "Cut at the playhead"),
        ("Click clip", "Select it"),
        ("⌘ / ⇧ click", "Add · extend"),
        ("⌘A", "Select all clips"),
        ("Drag clip bar", "Reorder, snaps to joins"),
        ("⌫", "Delete, closing the gap"),
        ("Esc", "Deselect · Select tool"),
        ("⌘Z  ⌘⌫", "Undo · reset all"),
    ];
    const GESTURES: &[(&str, &str)] = &[
        ("Click", "Move the playhead"),
        ("Drag", "Select a range"),
        ("Drag handle", "Trim one edge"),
        ("Double-click handle", "Reset that edge"),
        ("Double-click gain", "Back to 0 dB"),
        ("Scroll", "Pan while zoomed"),
        ("⌘ Scroll", "Zoom at the pointer"),
        ("Overview strip", "Click or drag to jump"),
    ];

    let row = |keys: &'static str, label: &'static str| {
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(10.))
            .h(px(22.))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(theme::text_muted())
                    .child(label),
            )
            .child(keycap(keys))
    };
    let column = |title: &'static str, rows: &'static [(&'static str, &'static str)]| {
        div()
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(div().mb(px(8.)).child(caps(title, 9., theme::text_faint())))
            .children(rows.iter().map(|(k, l)| row(k, l)))
    };

    div()
        .id("help-sheet")
        .absolute()
        .inset_0()
        .occlude()
        .bg(theme::black(0.62))
        .flex()
        .items_center()
        .justify_center()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.help = false;
                cx.notify();
            }),
        )
        .child(
            div()
                .id("help-panel")
                .w(px(780.))
                .max_w_full()
                .max_h_full()
                .overflow_y_scroll()
                .mx(px(20.))
                .p(px(22.))
                .rounded(px(14.))
                .bg(theme::panel())
                .border_1()
                .border_color(theme::white(0.12))
                .text_size(px(12.))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .mb(px(18.))
                        .child(dot_glyph(
                            &MARK_SMALL,
                            2.,
                            1.5,
                            theme::white(0.95),
                            theme::white(0.12),
                        ))
                        .child(caps("Editing, keyboard & gestures", 10., theme::text()))
                        .child(div().flex_1())
                        .child(
                            link_button("close-help", theme::text_muted())
                                .on_click(cx.listener(|this, _, _, cx| this.toggle_help(cx)))
                                .child("Close")
                                .child(keycap("Esc")),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(24.))
                        .child(column("Keys", KEYS))
                        .child(column("Clips", CLIPS))
                        .child(column("Pointer", GESTURES)),
                ),
        )
}

/// Loudest absolute sample inside the trimmed arrangement, per the current
/// overview. Remembered until the trim, the clips or the overview change,
/// so the per-frame redraw during playback never rescans it.
fn selection_peak(doc: &Document) -> Option<f32> {
    let peaks = doc.peaks.as_ref().filter(|p| !p.is_empty())?;
    let key = (
        doc.edit.start.to_bits(),
        doc.edit.end.to_bits(),
        doc.revision,
        Arc::as_ptr(peaks) as usize,
    );
    if let Some((cached, peak)) = doc.peak_memo.get()
        && cached == key
    {
        return peak;
    }
    let rate = doc.info.sample_rate;
    let frames = doc.edit.frame_range(rate, doc.timeline.frames());
    let n = peaks.len();
    let scale = n as f64 / doc.info.frames.max(1) as f64;
    let mut found = None;
    for range in doc.timeline.ranges(frames) {
        let b0 = ((range.start as f64 * scale).floor() as usize).min(n - 1);
        let b1 = ((range.end as f64 * scale).ceil() as usize).clamp(b0 + 1, n);
        let peak = peaks[b0..b1]
            .iter()
            .fold(0f32, |acc, p| acc.max(p.min.abs()).max(p.max.abs()));
        found = Some(found.map_or(peak, |f: f32| f.max(peak)));
    }
    doc.peak_memo.set(Some((key, found)));
    found
}

/// The clips overlapping the view, for the stage. `lifting`: a reorder drag
/// is under way, so the selected clips draw as lifted outlines.
fn scene_clips(doc: &Document, lifting: bool) -> Vec<wave::SceneClip> {
    let view = doc.view;
    let mut out = Vec::new();
    for (index, (offset, clip)) in doc.timeline.spans().enumerate() {
        let start = doc.seconds(offset);
        let end = doc.seconds(offset + clip.frames());
        if end < view.start {
            continue;
        }
        if start > view.end {
            break;
        }
        let selected = doc.selected.binary_search(&index).is_ok();
        out.push(wave::SceneClip {
            number: index + 1,
            start,
            end,
            source: doc.seconds(clip.start),
            selected,
            lifted: selected && lifting,
        });
    }
    out
}

/// One segment of the tool picker.
fn tool_button(id: &'static str, on: bool) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .size(px(24.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .cursor_pointer()
        .map(|el| {
            if on {
                el.bg(theme::white(0.94))
            } else {
                el.hover(|s| s.bg(theme::white(0.08)))
                    .active(|s| s.bg(theme::white(0.13)))
            }
        })
}

/// The stage once every clip is gone: nothing to draw, two ways back.
fn render_empty_timeline(cx: &mut Context<Sonora>) -> impl IntoElement {
    field_zone()
        .child(dot_glyph(
            &[1, 1, 1, 1, 1, 1, 1, 1, 1],
            6.,
            4.,
            theme::white(0.3),
            theme::white(0.06),
        ))
        .child(
            div()
                .mt(px(22.))
                .text_size(px(16.))
                .font_weight(FontWeight::SEMIBOLD)
                .child("Every clip was deleted."),
        )
        .child(
            div()
                .mt(px(8.))
                .max_w(px(400.))
                .text_center()
                .text_color(theme::text_muted())
                .child("The original file is untouched. Undo brings back the last clips; reset restores the whole recording."),
        )
        .child(
            div()
                .mt(px(20.))
                .flex()
                .items_center()
                .gap(px(10.))
                .child(
                    primary_button("empty-undo", true)
                        .tooltip(tooltip("Undo the delete", Some("⌘Z")))
                        .on_click(cx.listener(|this, _, _, cx| this.undo(cx)))
                        .child(icon("icons/undo.svg", 14., theme::black(0.9)))
                        .child("Undo"),
                )
                .child(
                    link_button("empty-reset", theme::text_muted())
                        .tooltip(tooltip("Back to the untouched recording", Some("⌘⌫")))
                        .on_click(cx.listener(|this, _, _, cx| this.reset_edits(cx)))
                        .child("Reset all edits")
                        .child(keycap("⌘⌫")),
                ),
        )
}

fn clip_state(doc: &Document) -> Clip {
    let gain = 10f32.powf(doc.edit.gain_db / 20.0);
    match selection_peak(doc) {
        Some(peak) if peak * gain > 1.0 => {
            if doc.peaks_exact {
                Clip::Certain
            } else {
                Clip::Likely
            }
        }
        Some(_) if doc.peaks_exact => Clip::None,
        _ if doc.edit.gain_db > 0.0 => Clip::Possible,
        _ => Clip::None,
    }
}

fn dbfs(amplitude: f32) -> String {
    let db = 20.0 * amplitude.log10();
    let rounded = (db * 10.0).round() / 10.0;
    if rounded > 0.0 {
        format!("+{rounded:.1} dBFS")
    } else if rounded < 0.0 {
        format!("\u{2212}{:.1} dBFS", -rounded)
    } else {
        "0.0 dBFS".into()
    }
}

#[cfg(test)]
mod tests {
    use super::TransportFold;

    #[test]
    fn transport_fits_every_supported_width() {
        // 640 is the minimum window; 841 is macOS "Move & Resize › Right".
        for width in (640..=1400).step_by(1) {
            let width = width as f32;
            let fold = TransportFold::for_width(width);
            assert!(
                fold.estimate() <= width,
                "{width}px needs {} for {fold:?}",
                fold.estimate()
            );
        }
    }

    #[test]
    fn half_screen_keeps_the_slider_and_drops_low_priority_pieces() {
        let fold = TransportFold::for_width(841.0);
        assert_eq!(fold.slider, Some(120.));
        assert!(fold.ab && !fold.to_start && !fold.gain_label && !fold.tight_formats);
        let full = TransportFold::for_width(940.0);
        assert!(full.to_start && full.gain_label);
    }
}
