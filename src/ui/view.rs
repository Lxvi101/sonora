//! Rendering for the Sonora window. Layout, top to bottom: a quiet titlebar
//! strip under the traffic lights, the stage (empty drop target or the
//! waveform), the transport row, and a slim footer for facts and status.

use gpui::{
    AnyElement, Context, Div, ExternalPaths, FontWeight, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window,
    div, prelude::*, px,
};

use super::widgets::{hint, icon, keycap, link_button, pill_button, round_button, tooltip};
use super::*;

const TITLEBAR: f32 = 38.0;
const COMPACT: f32 = 780.0;

#[derive(Clone, Copy, PartialEq)]
enum Clip {
    None,
    /// Gain is above unity but we can't measure yet.
    Possible,
    /// The preview overview says it will clip; the exact scan may refine it.
    Likely,
    Certain,
}

impl Render for Sonora {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Animate only while the engine is playing; idle windows don't redraw.
        if self.tick_playback() {
            window.request_animation_frame();
        }

        let title = match &self.phase {
            Phase::Ready(doc) => doc.name.clone(),
            _ => "Sonora".to_string(),
        };
        if title != self.window_title {
            window.set_window_title(&title);
            self.window_title = title;
        }

        let width = f32::from(window.viewport_size().width);
        let compact = width < COMPACT;

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
            .on_drop(cx.listener(Self::dropped));

        let root = self.register_actions(root, cx);
        let ready = self.doc().is_some();

        root.child(self.render_titlebar())
            .child(self.render_stage(cx))
            .when(ready, |root| {
                root.child(self.render_transport(compact, width, cx))
            })
            .child(self.render_footer(compact, cx))
            .when(!matches!(self.phase, Phase::Empty), |root| {
                root.child(drop_overlay(cx))
            })
    }
}

impl Sonora {
    fn dropped(&mut self, paths: &ExternalPaths, _: &mut Window, cx: &mut Context<Self>) {
        self.open_paths(paths.paths().to_vec(), cx);
    }

    /// Actions are only registered when they can run, which also drives the
    /// enabled state of their menu items.
    fn register_actions(
        &self,
        root: gpui::Stateful<Div>,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<Div> {
        let doc = self.doc();
        let ready = doc.is_some();
        let playable = doc.is_some_and(|d| d.playable);
        let exporting = doc.is_some_and(|d| d.export.is_some());
        let can_undo = doc.is_some_and(|d| !d.undo.is_empty());
        let can_redo = doc.is_some_and(|d| !d.redo.is_empty());
        let edited = doc.is_some_and(Self::is_edited);
        let cancellable = exporting || self.notice.is_some();

        root.on_action(cx.listener(|this, _: &Open, _, cx| this.prompt_open(cx)))
            .on_action(cx.listener(|_, _: &Minimize, window, _| window.minimize_window()))
            .on_action(cx.listener(|_, _: &Zoom, window, _| window.zoom_window()))
            .when(ready, |root| {
                root.on_action(cx.listener(|this, _: &CloseFile, _, cx| this.close_file(cx)))
                    .on_action(cx.listener(|this, _: &RevealSource, _, cx| this.reveal_source(cx)))
                    .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.select_all(cx)))
                    .on_action(cx.listener(|this, _: &SetIn, _, cx| this.set_in(cx)))
                    .on_action(cx.listener(|this, _: &SetOut, _, cx| this.set_out(cx)))
                    .on_action(cx.listener(|this, _: &SeekBack, _, cx| this.seek_by(-1.0, cx)))
                    .on_action(cx.listener(|this, _: &SeekForward, _, cx| this.seek_by(1.0, cx)))
                    .on_action(cx.listener(|this, _: &SeekBackFar, _, cx| this.seek_by(-5.0, cx)))
                    .on_action(cx.listener(|this, _: &NudgeBack, _, cx| this.seek_by(-0.01, cx)))
                    .on_action(cx.listener(|this, _: &NudgeForward, _, cx| this.seek_by(0.01, cx)))
                    .on_action(cx.listener(|this, _: &SeekForwardFar, _, cx| this.seek_by(5.0, cx)))
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
                    .on_action(cx.listener(|this, _: &ResetGain, _, cx| {
                        if let Some(doc) = this.doc() {
                            let edit = Edit {
                                gain_db: 0.0,
                                ..doc.edit
                            };
                            this.apply_edit(edit, cx);
                        }
                    }))
            })
            .when(playable, |root| {
                root.on_action(cx.listener(|this, _: &PlayPause, _, cx| this.toggle_playback(cx)))
            })
            .when(ready && !exporting, |root| {
                root.on_action(cx.listener(|this, _: &Export, _, cx| this.prompt_export(cx)))
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

    fn render_titlebar(&self) -> impl IntoElement {
        let content: AnyElement = match &self.phase {
            Phase::Ready(doc) => div()
                .flex()
                .items_center()
                .gap(px(7.))
                .min_w_0()
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme::ivory(0.86))
                        .child(doc.name.clone()),
                )
                .when(Self::is_edited(doc), |el| {
                    el.child(
                        div()
                            .id("edited")
                            .flex_none()
                            .size(px(6.))
                            .rounded_full()
                            .bg(theme::accent(0.85))
                            .tooltip(tooltip("Edited — the original stays untouched", None)),
                    )
                })
                .into_any_element(),
            Phase::Opening { name, .. } => div()
                .min_w_0()
                .truncate()
                .text_color(theme::text_muted())
                .child(format!("Opening {name}…"))
                .into_any_element(),
            Phase::Empty => div()
                .text_color(theme::text_faint())
                .font_weight(FontWeight::MEDIUM)
                .child("Sonora")
                .into_any_element(),
        };

        div()
            .h(px(TITLEBAR))
            .flex_none()
            .w_full()
            .px(px(88.))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(12.5))
            .on_mouse_down(MouseButton::Left, |event, window, _| {
                if event.click_count == 2 {
                    window.titlebar_double_click();
                }
            })
            .child(content)
    }

    fn render_stage(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let stage = div().flex_1().min_h_0().px(px(14.)).flex().flex_col();
        match &self.phase {
            Phase::Empty => stage.child(self.render_empty(cx)),
            Phase::Opening { name, .. } => stage.child(
                drop_zone()
                    .child(glyph(0.45))
                    .child(
                        div()
                            .mt(px(18.))
                            .text_size(px(15.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(name.clone()),
                    )
                    .child(
                        div()
                            .mt(px(6.))
                            .text_color(theme::text_muted())
                            .child("Reading audio…"),
                    ),
            ),
            Phase::Ready(doc) => {
                let playhead = self.position();
                let dragging = match self.drag {
                    Some(Drag::Handle { handle, .. }) => Some(handle),
                    _ => None,
                };
                let message = if let Some(err) = &doc.wave_error {
                    Some((
                        SharedString::from(format!(
                            "Waveform unavailable — {err} Preview and export still work."
                        )),
                        theme::coral(0.85),
                    ))
                } else if doc.peaks.is_none() {
                    Some(("Reading waveform…".into(), theme::text_faint()))
                } else {
                    None
                };
                let scene = wave::WaveScene {
                    peaks: doc.peaks.clone(),
                    exact: doc.peaks_exact,
                    duration: doc.info.duration(),
                    start: doc.edit.start,
                    end: doc.edit.end,
                    gain_db: doc.edit.gain_db,
                    playhead,
                    hover: self.hover,
                    dragging,
                    message,
                };
                stage.child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .rounded(px(12.))
                        .bg(theme::well())
                        .border_1()
                        .border_color(theme::hairline())
                        .overflow_hidden()
                        .id("waveform")
                        .tooltip(tooltip(
                            "Click to place the playhead, drag to select · ⌥← → nudges 10 ms · I / O set trim",
                            None,
                        ))
                        .child(wave::waveform(scene, cx.entity().downgrade())),
                )
            }
        }
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let error = match &self.notice {
            Some(Notice::Error(message)) => Some(message.clone()),
            _ => None,
        };
        drop_zone()
            .id("drop-zone")
            .drag_over::<ExternalPaths>(|style, _, _, _| {
                style.border_color(theme::accent(0.55)).bg(theme::accent(0.06))
            })
            .on_drop(cx.listener(Self::dropped))
            .child(glyph(1.0))
            .child(
                div()
                    .mt(px(20.))
                    .text_size(px(18.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme::text())
                    .child("Drop an audio file"),
            )
            .child(
                div()
                    .mt(px(6.))
                    .max_w(px(360.))
                    .text_center()
                    .text_color(theme::text_muted())
                    .child("Trim it, make it louder, and export a fresh WAV. Your original is never changed."),
            )
            .child(
                pill_button("open", false, true)
                    .mt(px(22.))
                    .tooltip(tooltip("Choose an audio file", Some("⌘O")))
                    .on_click(cx.listener(|this, _, _, cx| this.prompt_open(cx)))
                    .child(icon("icons/open.svg", 14., theme::text_muted()))
                    .child("Open Audio…")
                    .child(keycap("⌘O")),
            )
            .when_some(error, |zone, message| {
                zone.child(
                    div()
                        .mt(px(16.))
                        .w_full()
                        .max_w(px(420.))
                        .flex()
                        .items_start()
                        .gap(px(7.))
                        .text_color(theme::coral(0.95))
                        .child(
                            div()
                                .flex_none()
                                .pt(px(1.))
                                .child(icon("icons/warning.svg", 13., theme::coral(0.95))),
                        )
                        // A sized flex item so long messages wrap inside the row.
                        .child(div().flex_1().min_w_0().whitespace_normal().child(message)),
                )
            })
            .child(
                div()
                    .mt(px(26.))
                    .text_size(px(11.))
                    .text_color(theme::text_faint())
                    .child("WAV · AIFF · MP3 · M4A · AAC · FLAC · CAF"),
            )
    }

    fn render_transport(
        &self,
        compact: bool,
        width: f32,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let Some(doc) = self.doc() else {
            return div().into_any_element();
        };
        let playing = doc.playing;
        let playable = doc.playable;
        let exporting = doc.export.is_some();
        let edit = doc.edit;
        let clip = clip_state(doc);
        let trim_min_width = if doc.info.duration() >= 3600.0 {
            860.0
        } else {
            COMPACT
        };
        let gain_color = match clip {
            Clip::Certain | Clip::Likely => theme::amber(1.0),
            _ => theme::text(),
        };

        let play = round_button("play", playable)
            .tooltip(if !playable {
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
                16.,
                if playable {
                    theme::shade(0.92)
                } else {
                    theme::text_faint()
                },
            ));

        let clock = div()
            .id("clock")
            .flex_none()
            .min_w(px(96.))
            .whitespace_nowrap()
            .tooltip(tooltip("Playhead · selected duration", None))
            .flex()
            .flex_col()
            .gap(px(1.))
            .child(
                div()
                    .font(theme::numeric(FontWeight::MEDIUM))
                    .text_size(px(17.))
                    .line_height(px(21.))
                    .text_color(theme::text())
                    .child(theme::timecode(self.position())),
            )
            .child(
                div()
                    .font(theme::numeric(FontWeight::NORMAL))
                    .text_size(px(10.5))
                    .text_color(theme::text_faint())
                    .child(format!("Sel {}", theme::timecode(edit.end - edit.start))),
            );

        let trim = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(2.))
            .child(
                trim_chip("set-in", "Start", edit.start)
                    .tooltip(tooltip("Set start at playhead", Some("I")))
                    .on_click(cx.listener(|this, _, _, cx| this.set_in(cx))),
            )
            .child(div().w(px(10.)).h(px(1.)).bg(theme::ivory(0.26)))
            .child(
                trim_chip("set-out", "End", edit.end)
                    .tooltip(tooltip("Set end at playhead", Some("O")))
                    .on_click(cx.listener(|this, _, _, cx| this.set_out(cx))),
            )
            .when(Self::is_edited(doc), |el| {
                el.child(
                    div()
                        .id("reset")
                        .ml(px(4.))
                        .size(px(26.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(7.))
                        .cursor_pointer()
                        .hover(|s| s.bg(theme::ivory(0.07)))
                        .active(|s| s.bg(theme::ivory(0.11)))
                        .tooltip(tooltip("Reset trim and gain", Some("⌘⌫")))
                        .on_click(cx.listener(|this, _, _, cx| this.reset_edits(cx)))
                        .child(icon("icons/reset.svg", 14., theme::text_muted())),
                )
            });

        let gain = div()
            .flex()
            .flex_none()
            .items_center()
            .gap(px(8.))
            .child(
                div()
                    .id("gain-icon")
                    .tooltip(tooltip(
                        "Gain — double-click the slider for 0 dB",
                        Some("+ / −"),
                    ))
                    .child(icon("icons/volume.svg", 15., theme::text_muted())),
            )
            .child(wave::gain_slider(
                wave::GainScene {
                    gain_db: edit.gain_db,
                    enabled: true,
                    clipping: matches!(clip, Clip::Certain | Clip::Likely),
                    dragging: matches!(self.drag, Some(Drag::Gain { .. })),
                },
                cx.entity().downgrade(),
            ))
            .child(
                div()
                    .w(px(54.))
                    .flex_none()
                    .font(theme::numeric(FontWeight::MEDIUM))
                    .text_size(px(12.5))
                    .text_color(gain_color)
                    .child(theme::decibels(edit.gain_db)),
            )
            .child(clip_badge(clip));

        let export = pill_button("export", true, !exporting)
            .tooltip(tooltip(
                if exporting {
                    "Exporting…"
                } else {
                    "Export the selection as a new WAV"
                },
                Some("⇧⌘S"),
            ))
            .when(!exporting, |b| {
                b.on_click(cx.listener(|this, _, _, cx| this.prompt_export(cx)))
            })
            .child(icon(
                "icons/export.svg",
                14.,
                if exporting {
                    theme::text_faint()
                } else {
                    theme::accent(1.0)
                },
            ))
            .child(if compact { "Export" } else { "Export WAV" });

        div()
            .h(px(64.))
            .flex_none()
            .px(px(18.))
            .flex()
            .items_center()
            .gap(px(12.))
            .child(play)
            .child(clock)
            // Multi-hour timecodes are wider; drop the trim chips sooner
            // rather than squeezing them (I / O and the handles still work).
            .when(!compact && width >= trim_min_width, |row| row.child(trim))
            .child(div().flex_1())
            .child(gain)
            .child(export)
            .into_any_element()
    }

    fn render_footer(&self, compact: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let facts: Option<String> = self.doc().map(|doc| {
            let mut parts = vec![
                theme::sample_rate(doc.info.sample_rate),
                theme::channels(doc.info.channels),
                theme::timecode(doc.info.duration()),
            ];
            if !compact && let Some(peak) = selection_peak(doc) {
                let gained = peak * 10f32.powf(doc.edit.gain_db / 20.0);
                let approx = if doc.peaks_exact { "" } else { "≈ " };
                parts.push(if gained > 0.0 {
                    format!("Peak {approx}{}", dbfs(gained))
                } else {
                    "Silent".into()
                });
            }
            parts.join("  ·  ")
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
            .bg(theme::shade(0.10))
            .text_size(px(11.))
            .text_color(theme::text_muted())
            .children(facts.map(|facts| {
                div()
                    .flex_none()
                    .font(theme::numeric(FontWeight::NORMAL))
                    .text_size(px(11.))
                    .child(facts)
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(10.))
                    .child(self.render_status(compact, cx)),
            )
    }

    fn render_status(&self, compact: bool, cx: &mut Context<Self>) -> AnyElement {
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
                            format!("Exporting {}", display_name(&job.destination))
                        }),
                )
                .child(progress_bar(fraction, 88.))
                .child(
                    div()
                        .w(px(30.))
                        .flex_none()
                        .font(theme::numeric(FontWeight::NORMAL))
                        .child(format!("{}%", (fraction * 100.0).floor() as u32)),
                )
                .when(!cancelling, |row| {
                    row.child(
                        link_button("cancel-export", theme::text())
                            .tooltip(tooltip("Stop exporting", Some("Esc")))
                            .on_click(cx.listener(|this, _, _, cx| this.cancel(cx)))
                            .child("Cancel"),
                    )
                })
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
                    .text_color(theme::coral(0.95))
                    .child(icon("icons/warning.svg", 12., theme::coral(0.95)))
                    .child(div().min_w_0().truncate().child(message.clone()))
                    .child(dismiss)
                    .into_any_element(),
                Notice::Info(message) => status_row()
                    .child(div().min_w_0().truncate().child(message.clone()))
                    .child(dismiss)
                    .into_any_element(),
                Notice::Saved(path) => {
                    let path = path.clone();
                    status_row()
                        .child(icon("icons/check.svg", 12., theme::accent(1.0)))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_color(theme::text())
                                .child(format!("Saved {}", display_name(&path))),
                        )
                        .child(
                            link_button("reveal", theme::accent(1.0))
                                .tooltip(tooltip("Reveal the exported file in Finder", None))
                                .on_click(move |_, _, _| audio::reveal(&path))
                                .child("Show in Finder"),
                        )
                        .child(dismiss)
                        .into_any_element()
                }
            };
        }

        if let Some(doc) = doc
            && doc.wave_error.is_none()
            && !doc.peaks_exact
        {
            let label = if doc.peaks.is_none() {
                "Reading waveform…".to_string()
            } else if doc.wave_progress <= 0.0 {
                "Quick preview · scanning…".to_string()
            } else {
                format!(
                    "Preview · refining {}%",
                    (doc.wave_progress * 100.0).floor() as u32
                )
            };
            return status_row()
                .child(div().min_w_0().truncate().child(label))
                .child(progress_bar(doc.wave_progress, 56.))
                .into_any_element();
        }

        let hints = if doc.is_some() {
            if compact {
                vec![hint("Space", "Play"), hint("I O", "Trim")]
            } else {
                vec![
                    hint("Space", "Play"),
                    hint("I O", "Trim at playhead"),
                    hint("← →", "Seek"),
                    hint("⇧⌘S", "Export"),
                ]
            }
        } else if matches!(self.phase, Phase::Empty) {
            vec![hint("⌘O", "Open"), hint("Drop", "Any audio file")]
        } else {
            vec![]
        };
        status_row().gap(px(14.)).children(hints).into_any_element()
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

fn drop_zone() -> Div {
    div()
        .flex_1()
        .min_h_0()
        .mb(px(14.))
        .rounded(px(14.))
        .border_1()
        .border_color(theme::ivory(0.06))
        .bg(theme::ivory(0.015))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .px(px(24.))
}

/// A decorative mark: five seafoam bars in a soft disc.
fn glyph(strength: f32) -> impl IntoElement {
    div()
        .size(px(64.))
        .rounded_full()
        .bg(theme::accent(0.09 * strength))
        .border_1()
        .border_color(theme::accent(0.22 * strength))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(3.))
        .children([12., 22., 32., 20., 10.].map(|h| {
            div()
                .w(px(3.))
                .h(px(h))
                .rounded(px(1.5))
                .bg(theme::accent(0.9 * strength))
        }))
}

/// Shown over a loaded document while a file is dragged over the window.
/// GPUI only hit-tests an element with a listener, so the overlay takes the
/// drop itself.
fn drop_overlay(cx: &mut Context<Sonora>) -> impl IntoElement {
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
                .border_color(theme::accent(0.6))
                .bg(theme::shade(0.62))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(12.))
                .child(glyph(1.0))
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(FontWeight::MEDIUM)
                        .child("Release to open"),
                )
                .child(
                    div()
                        .text_color(theme::text_muted())
                        .child("The current file closes; nothing is saved over."),
                ),
        )
}

fn trim_chip(id: &'static str, label: &'static str, time: f64) -> gpui::Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .h(px(38.))
        .px(px(9.))
        .rounded(px(8.))
        .flex()
        .flex_col()
        .justify_center()
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(|s| s.bg(theme::ivory(0.06)))
        .active(|s| s.bg(theme::ivory(0.10)))
        .child(
            div()
                .text_size(px(10.))
                .text_color(theme::text_faint())
                .font_weight(FontWeight::MEDIUM)
                .child(label),
        )
        .child(
            div()
                .font(theme::numeric(FontWeight::NORMAL))
                .text_size(px(12.5))
                .text_color(theme::text())
                .child(theme::timecode(time)),
        )
}

fn progress_bar(fraction: f32, width: f32) -> impl IntoElement {
    let fraction = fraction.clamp(0.0, 1.0);
    div()
        .flex_none()
        .w(px(width))
        .h(px(3.))
        .rounded(px(1.5))
        .bg(theme::ivory(0.10))
        .child(
            div()
                .h_full()
                .w(px(width * fraction))
                .rounded(px(1.5))
                .bg(theme::accent(0.95)),
        )
}

fn clip_badge(clip: Clip) -> impl IntoElement {
    let (label, color) = match clip {
        Clip::None => return div().w(px(14.)).flex_none().into_any_element(),
        Clip::Possible => (
            "Gain above 0 dB may clip once peaks are measured",
            theme::amber(0.55),
        ),
        Clip::Likely => (
            "The overview suggests peaks will clip; export hard-limits them",
            theme::amber(0.95),
        ),
        Clip::Certain => (
            "Peaks will clip — export hard-limits samples to full scale",
            theme::amber(1.0),
        ),
    };
    div()
        .id("clip")
        .flex_none()
        .tooltip(tooltip(label, None))
        .child(icon("icons/warning.svg", 14., color))
        .into_any_element()
}

/// Loudest absolute sample inside the selection, per the current overview.
fn selection_peak(doc: &Document) -> Option<f32> {
    let peaks = doc.peaks.as_deref().filter(|p| !p.is_empty())?;
    let duration = doc.info.duration().max(1e-9);
    let n = peaks.len();
    let b0 = ((doc.edit.start / duration) * n as f64)
        .floor()
        .clamp(0.0, (n - 1) as f64) as usize;
    let b1 = ((doc.edit.end / duration) * n as f64)
        .ceil()
        .clamp((b0 + 1) as f64, n as f64) as usize;
    Some(
        peaks[b0..b1]
            .iter()
            .fold(0f32, |acc, p| acc.max(p.min.abs()).max(p.max.abs())),
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
