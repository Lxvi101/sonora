//! Custom-painted controls: the dot-matrix waveform stage (ruler, clip bar,
//! lit dots, trim brackets, playhead, overview strip), the dotted gain slider
//! and the empty-state dot field. The canvases register their pointer
//! handlers during paint so a drag keeps tracking after it leaves their
//! bounds.
//!
//! The stage draws the arranged timeline: every column asks only the clips
//! it overlaps for their source peaks, so cut and reordered audio reuses the
//! one overview without copying or rescanning it.

use std::sync::Arc;

use gpui::{
    App, BorderStyle, Bounds, CursorStyle, DispatchPhase, FontWeight, Hitbox, HitboxBehavior, Hsla,
    IntoElement, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    ScrollWheelEvent, SharedString, Styled, TextRun, WeakEntity, Window, canvas, fill, point, px,
    quad, size,
};

use super::logic::{GAIN_MAX, GAIN_MIN, Handle, Mapping, Tool, View};
use super::{ClipClick, Sonora, theme};
use crate::audio::Peak;

/// Distance between dot centers, and the dot itself.
const PITCH: f32 = 5.0;
const DOT: f32 = 2.0;
const PAD_X: f32 = 16.0;
const RULER: f32 = 24.0;
/// The clip bar between the ruler and the dots.
const LANE: f32 = 16.0;
const FOOT: f32 = 12.0;
/// Room for the overview strip under the dots. Always reserved, so zooming
/// never changes the waveform's height; the strip itself shows while zoomed.
const STRIP: f32 = 18.0;
/// Half-width of a trim handle's grab zone.
const GRAB: f32 = 10.0;
/// The razor jumps onto the playhead within this many pixels.
const SNAP: f32 = 6.0;

/// A clip as the stage draws it, in timeline seconds.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SceneClip {
    /// 1-based position on the timeline, as labelled.
    pub number: usize,
    pub start: f64,
    pub end: f64,
    /// Source seconds at `start`.
    pub source: f64,
    pub selected: bool,
    /// Picked up by a reorder drag: drawn as an outline left behind.
    pub lifted: bool,
}

/// The selection block following the pointer during a reorder drag.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ghost {
    pub start: f64,
    pub end: f64,
    pub count: usize,
}

/// Everything the waveform needs for one frame.
pub(crate) struct WaveScene {
    pub peaks: Option<Arc<Vec<Peak>>>,
    pub exact: bool,
    /// The arranged timeline's length.
    pub duration: f64,
    /// The source file's length, which the overview's bins span.
    pub source_duration: f64,
    pub view: View,
    pub start: f64,
    pub end: f64,
    /// The gain being previewed (0 dB while comparing with the original).
    pub gain_db: f32,
    pub playhead: f64,
    pub hover: Option<Handle>,
    pub dragging: Option<Handle>,
    pub message: Option<SharedString>,
    pub tool: Tool,
    /// Clips overlapping the view, in timeline order.
    pub clips: Vec<SceneClip>,
    /// Where the razor would cut.
    pub razor: Option<f64>,
    /// The join a reorder drag would drop at.
    pub insertion: Option<f64>,
    pub ghost: Option<Ghost>,
    pub lane_hover: bool,
    /// A press on the clip bar is held (moving or not).
    pub holding_clips: bool,
}

#[derive(Clone, Copy)]
struct Layout {
    map: Mapping,
    /// The whole timeline across the same strip, for the overview.
    full: Mapping,
    cols: usize,
    ruler_top: f32,
    ruler_bottom: f32,
    lane_top: f32,
    lane_bottom: f32,
    top: f32,
    bottom: f32,
    mid: f32,
    rows: i32,
    strip: Option<f32>,
}

impl Layout {
    fn new(bounds: Bounds<Pixels>, view: View, duration: f64) -> Self {
        let ox = f32::from(bounds.origin.x);
        let oy = f32::from(bounds.origin.y);
        let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        let left = (ox + PAD_X).round();
        let cols = (((w - PAD_X * 2.0) / PITCH).floor() as usize).max(1);
        let width = cols as f32 * PITCH;
        let zoomed = !view.is_full(duration);
        let ruler_top = oy;
        let ruler_bottom = oy + RULER;
        let lane_top = ruler_bottom + 3.0;
        let lane_bottom = lane_top + LANE;
        let top = lane_bottom + 8.0;
        let bottom = (oy + h - FOOT - STRIP).max(top + PITCH * 2.0);
        let mid = ((top + bottom) / 2.0).round();
        let rows = (((bottom - top) / 2.0 / PITCH).floor() as i32).max(1);
        Self {
            map: Mapping::new(left, width, view),
            full: Mapping::new(left, width, View::full(duration)),
            cols,
            ruler_top,
            ruler_bottom,
            lane_top,
            lane_bottom,
            top,
            bottom,
            mid,
            rows,
            strip: zoomed.then_some(bottom + 9.0),
        }
    }

    fn right(&self) -> f32 {
        self.map.left + self.map.width
    }

    fn in_strip(&self, y: f32) -> bool {
        self.strip.is_some_and(|top| y >= top - 5.0)
    }

    fn in_ruler(&self, y: f32) -> bool {
        y < self.ruler_bottom
    }

    fn in_lane(&self, y: f32) -> bool {
        y >= self.lane_top - 2.0 && y <= self.lane_bottom + 3.0
    }
}

pub(crate) fn waveform(scene: WaveScene, entity: WeakEntity<Sonora>) -> impl IntoElement {
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        move |bounds, hitbox, window, cx| {
            let layout = Layout::new(bounds, scene.view, scene.duration);
            paint_waveform(&scene, layout, bounds, window, cx);

            let cursor = if scene.tool == Tool::Razor {
                CursorStyle::Crosshair
            } else if scene.holding_clips {
                CursorStyle::ClosedHand
            } else if scene.dragging.is_some() || scene.hover.is_some() {
                CursorStyle::ResizeLeftRight
            } else if scene.lane_hover {
                CursorStyle::OpenHand
            } else {
                CursorStyle::IBeam
            };
            window.set_cursor_style(cursor, &hitbox);
            register_wave_events(&scene, layout, hitbox, entity, window);
        },
    )
    .size_full()
}

fn register_wave_events(
    scene: &WaveScene,
    layout: Layout,
    hitbox: Hitbox,
    entity: WeakEntity<Sonora>,
    window: &mut Window,
) {
    let (start, end, tool) = (scene.start, scene.end, scene.tool);
    // The razor cuts exactly at the playhead when it's this close to it.
    let playhead = scene.playhead;
    let razor_time = move |x: f32| {
        let map = layout.map;
        if map.visible(playhead) && (x - map.x_at(playhead)).abs() <= SNAP {
            playhead
        } else {
            map.time_at(x)
        }
    };
    window.on_mouse_event({
        let (entity, hitbox) = (entity.clone(), hitbox.clone());
        move |event: &MouseDownEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble
                || event.button != MouseButton::Left
                || !hitbox.is_hovered(window)
            {
                return;
            }
            let x = f32::from(event.position.x);
            let y = f32::from(event.position.y);
            let time = layout.map.time_at(x);
            let (toggle, extend) = (event.modifiers.platform, event.modifiers.shift);
            entity
                .update(cx, |this, cx| {
                    if layout.in_strip(y) {
                        this.begin_pan(layout.full.time_at(x), cx);
                        return;
                    }
                    // The ruler always moves the playhead, whatever the tool.
                    if tool == Tool::Razor && !layout.in_ruler(y) {
                        this.split_at(razor_time(x), cx);
                        return;
                    }
                    if tool == Tool::Select && layout.in_lane(y) {
                        this.press_clip(time, x, toggle, extend, cx);
                        return;
                    }
                    match layout.map.handle_at(x, start, end, GRAB) {
                        Some(handle) if event.click_count >= 2 => this.reset_edge(handle, cx),
                        Some(handle) => this.begin_handle_drag(handle, x, time, cx),
                        None => {
                            let click = (!layout.in_ruler(y))
                                .then(|| this.clip_at(time))
                                .flatten()
                                .map(|index| ClipClick {
                                    index,
                                    toggle,
                                    extend,
                                });
                            this.begin_range_drag(time, x, click, cx)
                        }
                    }
                })
                .ok();
            cx.stop_propagation();
        }
    });
    window.on_mouse_event({
        let (entity, hitbox) = (entity.clone(), hitbox.clone());
        move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let x = f32::from(event.position.x);
            let y = f32::from(event.position.y);
            let over = hitbox.is_hovered(window) && !layout.in_strip(y);
            let lane = over && layout.in_lane(y);
            let hover = (over && !lane && tool == Tool::Select)
                .then(|| layout.map.handle_at(x, start, end, GRAB))
                .flatten();
            let razor = (over && !layout.in_ruler(y)).then(|| razor_time(x));
            entity
                .update(cx, |this, cx| {
                    if event.dragging() {
                        this.drag_wave(layout.map.time_at(x), layout.full.time_at(x), x, cx);
                    } else {
                        // A release outside the window never reached us.
                        this.end_drag(cx);
                    }
                    if this.drag.is_none() {
                        this.set_hover(hover, cx);
                        this.set_pointer(razor, lane, cx);
                    }
                })
                .ok();
        }
    });
    window.on_mouse_event({
        let entity = entity.clone();
        move |event: &ScrollWheelEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble || !hitbox.is_hovered(window) {
                return;
            }
            let delta = event.delta.pixel_delta(px(16.));
            let (dx, dy) = (f32::from(delta.x), f32::from(delta.y));
            let x = f32::from(event.position.x);
            entity
                .update(cx, |this, cx| {
                    if event.modifiers.platform {
                        let factor = (dy as f64 * 0.01).exp();
                        this.zoom_view(factor, Some(layout.map.time_at(x)), cx);
                    } else {
                        let d = if dx.abs() > dy.abs() { dx } else { dy };
                        let seconds = -(d / layout.map.width) as f64 * layout.map.view.span();
                        this.pan_view(seconds, cx);
                    }
                })
                .ok();
            cx.stop_propagation();
        }
    });
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
        if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
            entity.update(cx, |this, cx| this.end_drag(cx)).ok();
        }
    });
}

fn dot(window: &mut Window, cx: f32, cy: f32, size: f32, color: Hsla) {
    window.paint_quad(quad(
        rect(cx - size / 2.0, cy - size / 2.0, size, size),
        px(size / 2.0),
        color,
        px(0.),
        gpui::transparent_black(),
        Default::default(),
    ));
}

/// Loudest absolute sample in source seconds `[t0, t1)` per the overview.
fn amplitude(peaks: &[Peak], t0: f64, t1: f64, duration: f64) -> f32 {
    let n = peaks.len();
    let scale = n as f64 / duration.max(1e-9);
    let b0 = ((t0 * scale).floor().max(0.0) as usize).min(n - 1);
    let b1 = ((t1 * scale).ceil().max(0.0) as usize).clamp(b0 + 1, n);
    peaks[b0..b1]
        .iter()
        .fold(0f32, |acc, p| acc.max(p.min.abs()).max(p.max.abs()))
}

/// Loudest sample in timeline seconds `[t0, t1)`, read through the clips
/// that cover it. `from` is a cursor into `clips` that only moves forward,
/// so a left-to-right sweep of columns costs one pass over the visible clips.
fn arranged_amplitude(
    peaks: &[Peak],
    clips: &[SceneClip],
    from: &mut usize,
    t0: f64,
    t1: f64,
    source_duration: f64,
) -> f32 {
    while *from < clips.len() && clips[*from].end <= t0 {
        *from += 1;
    }
    let mut level = 0f32;
    for clip in clips[*from..].iter().take_while(|c| c.start < t1) {
        let (a, b) = (t0.max(clip.start), t1.min(clip.end));
        if b > a {
            let offset = clip.source - clip.start;
            level = level.max(amplitude(peaks, a + offset, b + offset, source_duration));
        }
    }
    level
}

fn paint_waveform(
    scene: &WaveScene,
    l: Layout,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let map = l.map;
    let (left, right) = (map.left, l.right());
    let xs = map.x_at(scene.start);
    let xe = map.x_at(scene.end);
    let (sel_l, sel_r) = (xs.max(left), xe.min(right));

    // The kept region gets a faint lift behind its dots.
    if sel_r > sel_l {
        window.paint_quad(fill(
            rect(
                sel_l,
                l.ruler_bottom,
                sel_r - sel_l,
                l.bottom - l.ruler_bottom + 6.0,
            ),
            theme::white(0.028),
        ));
    }

    paint_ruler(&l, window, cx);

    // Selection band along the ruler's foot.
    if sel_r > sel_l {
        window.paint_quad(fill(
            rect(sel_l, l.ruler_bottom - 2.0, sel_r - sel_l, 2.0),
            theme::white(0.85),
        ));
        // Trailing dots where the selection continues off screen.
        for (i, alpha) in [0.6, 0.35, 0.15].into_iter().enumerate() {
            let step = 5.0 * (i as f32 + 1.0);
            if scene.start < map.view.start {
                dot(
                    window,
                    left - step,
                    l.ruler_bottom - 1.0,
                    2.0,
                    theme::white(alpha),
                );
            }
            if scene.end > map.view.end {
                dot(
                    window,
                    right + step,
                    l.ruler_bottom - 1.0,
                    2.0,
                    theme::white(alpha),
                );
            }
        }
    }

    // Selected clips: a hairline frame around their dots.
    for clip in scene.clips.iter().filter(|c| c.selected && !c.lifted) {
        let (x0, x1) = (
            map.x_at(clip.start).max(left - 4.0),
            map.x_at(clip.end).min(right + 4.0),
        );
        if x1 - x0 >= 2.0 {
            window.paint_quad(quad(
                rect(
                    x0 + 1.0,
                    l.top - 5.0,
                    x1 - x0 - 2.0,
                    l.bottom - l.top + 10.0,
                ),
                px(6.),
                theme::white(0.035),
                px(1.),
                theme::white(0.34),
                BorderStyle::Solid,
            ));
        }
    }

    // The matrix: lit dots for signal, a sparse dim grid for the rest.
    let peaks = scene.peaks.as_deref().filter(|p| !p.is_empty());
    let gain = 10f32.powf(scene.gain_db / 20.0);
    let (body_in, tip_in) = if scene.exact {
        (0.80, 1.0)
    } else {
        (0.46, 0.62)
    };
    let (body_out, tip_out) = (0.16, 0.26);
    let span = map.view.span();
    let mut cursor = 0;
    for i in 0..l.cols {
        let x0 = left + i as f32 * PITCH;
        let cx_ = x0 + PITCH / 2.0;
        let lit = peaks.map_or(-1, |peaks| {
            let t0 = map.view.start + (i as f64 * PITCH as f64 / map.width as f64) * span;
            let t1 = map.view.start + ((i + 1) as f64 * PITCH as f64 / map.width as f64) * span;
            let level = arranged_amplitude(
                peaks,
                &scene.clips,
                &mut cursor,
                t0,
                t1,
                scene.source_duration,
            ) * gain;
            if level > 1.0 {
                l.rows + 1
            } else {
                (level * l.rows as f32).round() as i32
            }
        });
        let inside = cx_ >= xs && cx_ <= xe;
        let (body, tip) = if inside {
            (body_in, tip_in)
        } else {
            (body_out, tip_out)
        };
        let clipped = lit > l.rows;
        let lit = lit.min(l.rows);
        for k in -l.rows..=l.rows {
            let cy = l.mid + k as f32 * PITCH;
            let a = k.abs();
            if a <= lit {
                let alpha = if a == lit && lit > 0 {
                    tip
                } else if lit == 0 {
                    body * 0.55
                } else {
                    body
                };
                if clipped && a == lit && inside {
                    // Clipping reads as solid squares at full scale.
                    window.paint_quad(fill(rect(cx_ - 1.5, cy - 1.5, 3.0, 3.0), theme::white(1.0)));
                } else {
                    dot(window, cx_, cy, DOT, theme::white(alpha));
                }
            } else if i % 2 == 0 && k % 2 == 0 {
                dot(window, cx_, cy, 1.5, theme::white(0.075));
            }
        }
    }

    paint_joins(scene, &l, window);
    paint_lane(scene, &l, window, cx);

    if let Some(message) = &scene.message {
        let line = shape(
            window,
            message.clone(),
            11.5,
            theme::text_muted(),
            FontWeight::MEDIUM,
        );
        let w = f32::from(line.width);
        let x = left + (map.width - w) / 2.0;
        window.paint_quad(quad(
            rect(x - 12.0, l.mid - 30.0, w + 24.0, 24.0),
            px(12.),
            theme::black(0.85),
            px(1.),
            theme::white(0.10),
            Default::default(),
        ));
        line.paint(point(px(x), px(l.mid - 25.5)), px(15.), window, cx)
            .ok();
    }

    if let Some(top) = l.strip {
        paint_strip(scene, &l, top, window);
    }

    // Trim brackets, only where they are actually on screen.
    for (handle, time, x) in [
        (Handle::Start, scene.start, xs),
        (Handle::End, scene.end, xe),
    ] {
        if !map.visible(time) {
            continue;
        }
        let active = scene.dragging == Some(handle)
            || (scene.dragging.is_none() && scene.hover == Some(handle));
        let ink = theme::white(if active { 1.0 } else { 0.88 });
        let line_w = if active { 1.5 } else { 1.0 };
        let y0 = l.ruler_bottom - 7.0;
        let y1 = l.bottom + 6.0;
        window.paint_quad(fill(rect(x - line_w / 2.0, y0, line_w, y1 - y0), ink));
        let inward = if handle == Handle::Start { 0.0 } else { -6.0 };
        window.paint_quad(fill(rect(x + inward, y0, 6.0, 1.5), ink));
        window.paint_quad(fill(rect(x + inward, y1 - 1.5, 6.0, 1.5), ink));
        let (gw, gh) = if active { (8.0, 34.0) } else { (6.0, 26.0) };
        window.paint_quad(quad(
            rect(x - gw / 2.0, l.mid - gh / 2.0, gw, gh),
            px(gw / 2.0),
            ink,
            px(0.),
            gpui::transparent_black(),
            Default::default(),
        ));
        for k in -1..=1 {
            dot(window, x, l.mid + k as f32 * 4.0, 1.5, theme::black(0.75));
        }
    }

    paint_reorder(scene, &l, window, cx);

    // The playhead: a hairline with a dot head in the ruler.
    if map.visible(scene.playhead) {
        let xp = map.x_at(scene.playhead);
        window.paint_quad(fill(
            rect(
                xp - 0.5,
                l.ruler_bottom - 6.0,
                1.0,
                l.bottom - l.ruler_bottom + 12.0,
            ),
            theme::white(1.0),
        ));
        dot(window, xp, l.ruler_bottom - 7.0, 7.0, theme::white(1.0));
    }

    // The razor: a dashed cut line and the exact time it will cut at.
    if let Some(time) = scene
        .razor
        .filter(|&t| map.visible(t) && t < scene.duration)
    {
        let x = map.x_at(time);
        let mut y = l.lane_top - 2.0;
        while y < l.bottom + 6.0 {
            window.paint_quad(fill(rect(x - 0.5, y, 1.0, 4.0), theme::white(0.95)));
            y += 7.0;
        }
        dot(window, x, l.lane_top + LANE / 2.0, 5.0, theme::white(1.0));
        paint_flag(
            format!("Cut {}", theme::timecode(time)),
            x,
            l.top + 2.0,
            bounds,
            window,
            cx,
        );
    }

    if let Some(handle) = scene.dragging {
        let (x, time) = match handle {
            Handle::Start => (xs, scene.start),
            Handle::End => (xe, scene.end),
        };
        paint_flag(theme::timecode(time), x, l.top + 2.0, bounds, window, cx);
    } else if let Some(handle) = scene.hover {
        // Hovering a handle teaches its double-click.
        let (x, text) = match handle {
            Handle::Start => (xs, "Drag to trim · double-click resets to 0:00".to_string()),
            Handle::End => (
                xe,
                format!(
                    "Drag to trim · double-click resets to {}",
                    theme::timecode(scene.duration)
                ),
            ),
        };
        paint_hint(text, x, l.bottom - 20.0, bounds, window, cx);
    }
}

/// Cuts show in the waveform as dotted seams between clips. Seams closer
/// than a few pixels merge, so a zoomed-out view of many cuts stays cheap.
fn paint_joins(scene: &WaveScene, l: &Layout, window: &mut Window) {
    let map = l.map;
    let mut last = f32::NEG_INFINITY;
    for clip in scene.clips.iter().filter(|c| c.number > 1) {
        let x = map.x_at(clip.start);
        if x < map.left || x > l.right() || x - last < 4.0 {
            continue;
        }
        last = x;
        let mut y = l.top - 4.0;
        while y <= l.bottom + 4.0 {
            dot(window, x, y, 1.5, theme::white(0.5));
            y += 3.0;
        }
    }
}

/// The clip bar: one rounded block per clip, white when selected, an
/// outline where a dragged clip was lifted from.
fn paint_lane(scene: &WaveScene, l: &Layout, window: &mut Window, cx: &mut App) {
    let map = l.map;
    for clip in &scene.clips {
        let x0 = map.x_at(clip.start).max(map.left - 3.0);
        let x1 = map.x_at(clip.end).min(l.right() + 3.0);
        let w = x1 - x0 - 2.0;
        if w < 1.0 {
            // Too narrow for a block: a tick keeps it visible.
            window.paint_quad(fill(
                rect(x0, l.lane_top + 3.0, 1.0, LANE - 6.0),
                theme::white(if clip.selected { 0.9 } else { 0.3 }),
            ));
            continue;
        }
        let (bg, border, style, ink) = match (clip.lifted, clip.selected) {
            (true, _) => (
                gpui::transparent_black(),
                theme::white(0.3),
                BorderStyle::Dashed,
                theme::text_ghost(),
            ),
            (false, true) => (
                theme::white(0.92),
                theme::white(0.92),
                BorderStyle::Solid,
                theme::black(0.85),
            ),
            (false, false) => (
                theme::white(0.06),
                theme::white(0.18),
                BorderStyle::Solid,
                theme::text_faint(),
            ),
        };
        window.paint_quad(quad(
            rect(x0 + 1.0, l.lane_top, w, LANE),
            px(4.),
            bg,
            px(1.),
            border,
            style,
        ));
        // Label from the visible edge so long clips stay named while zoomed.
        let label_x = (x0 + 1.0).max(map.left) + 6.0;
        let room = x0 + 1.0 + w - label_x - 4.0;
        if room < 10.0 {
            continue;
        }
        let long = format!(
            "{}  ·  {}",
            clip.number,
            theme::timecode(clip.end - clip.start)
        );
        for text in [long, clip.number.to_string()] {
            let line = shape(window, text.into(), 9.5, ink, FontWeight::SEMIBOLD);
            if f32::from(line.width) <= room {
                line.paint(
                    point(px(label_x), px(l.lane_top + 1.5)),
                    px(13.),
                    window,
                    cx,
                )
                .ok();
                break;
            }
        }
    }
}

/// During a reorder drag: the gathered block under the pointer and a bright
/// insertion bar at the join it will drop into.
fn paint_reorder(scene: &WaveScene, l: &Layout, window: &mut Window, cx: &mut App) {
    let map = l.map;
    if let Some(time) = scene.insertion.filter(|&t| map.visible(t)) {
        let x = map.x_at(time).clamp(map.left, l.right());
        let (y0, y1) = (l.lane_top - 5.0, l.bottom + 6.0);
        window.paint_quad(fill(rect(x - 1.0, y0, 2.0, y1 - y0), theme::white(1.0)));
        dot(window, x, y0, 7.0, theme::white(1.0));
        dot(window, x, y1, 7.0, theme::white(1.0));
    }
    if let Some(ghost) = scene.ghost {
        let x0 = map.x_at(ghost.start);
        let x1 = map.x_at(ghost.end).max(x0 + 6.0);
        let (x0, x1) = (x0.max(map.left - 8.0), x1.min(l.right() + 8.0));
        if x1 <= x0 {
            return;
        }
        window.paint_quad(quad(
            rect(x0, l.lane_top - 3.0, x1 - x0, LANE + 6.0),
            px(5.),
            theme::white(0.22),
            px(1.),
            theme::white(0.95),
            BorderStyle::Solid,
        ));
        let text = if ghost.count == 1 {
            "1 clip".to_string()
        } else {
            format!("{} clips", ghost.count)
        };
        let line = shape(
            window,
            text.into(),
            9.5,
            theme::text(),
            FontWeight::SEMIBOLD,
        );
        if f32::from(line.width) + 12.0 <= x1 - x0 {
            line.paint(
                point(px(x0 + 6.0), px(l.lane_top + 1.5)),
                px(13.),
                window,
                cx,
            )
            .ok();
        }
    }
}

fn paint_hint(
    text: String,
    x: f32,
    y: f32,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let line = shape(window, text.into(), 10.5, theme::text(), FontWeight::MEDIUM);
    let w = f32::from(line.width) + 16.0;
    let min = f32::from(bounds.origin.x) + 4.0;
    let max = f32::from(bounds.origin.x) + f32::from(bounds.size.width) - w - 4.0;
    let left = (x + 10.0).min(max).max(min);
    window.paint_quad(quad(
        rect(left, y, w, 20.0),
        px(6.),
        theme::panel(),
        px(1.),
        theme::white(0.16),
        Default::default(),
    ));
    line.paint(point(px(left + 8.0), px(y + 3.5)), px(13.), window, cx)
        .ok();
}

/// Ruler steps and how many dots subdivide each.
const STEPS: [(f64, u32); 23] = [
    (0.01, 5),
    (0.02, 4),
    (0.05, 5),
    (0.1, 5),
    (0.2, 4),
    (0.5, 5),
    (1., 5),
    (2., 4),
    (5., 5),
    (10., 5),
    (15., 3),
    (30., 3),
    (60., 4),
    (120., 4),
    (300., 5),
    (600., 5),
    (900., 3),
    (1800., 3),
    (3600., 4),
    (7200., 4),
    (14400., 4),
    (28800., 4),
    (86400., 4),
];

fn paint_ruler(l: &Layout, window: &mut Window, cx: &mut App) {
    let map = l.map;
    let px_per_second = map.width as f64 / map.view.span();
    let (step, subdiv) = STEPS
        .iter()
        .copied()
        .find(|(s, _)| s * px_per_second >= 84.0)
        .unwrap_or(STEPS[STEPS.len() - 1]);
    let minor = step / subdiv as f64;
    let first = (map.view.start / minor).ceil() as i64;
    let last = (map.view.end / minor).floor() as i64;
    let y = l.ruler_bottom - 7.0;
    for j in first..=last {
        let t = j as f64 * minor;
        let x = map.x_at(t);
        let major = j % subdiv as i64 == 0;
        if major {
            dot(window, x, y, 2.0, theme::white(0.5));
            let line = shape(
                window,
                theme::tick_label(t, step).into(),
                9.5,
                theme::text_faint(),
                FontWeight::MEDIUM,
            );
            if x + 5.0 + f32::from(line.width) <= l.right() + PAD_X - 2.0 {
                line.paint(
                    point(px(x + 5.0), px(l.ruler_top + 3.0)),
                    px(12.),
                    window,
                    cx,
                )
                .ok();
            }
        } else {
            dot(window, x, y, 1.5, theme::white(0.2));
        }
    }
}

/// The whole file in one dotted line, with the selection and the visible
/// window marked. Shown only while zoomed.
fn paint_strip(scene: &WaveScene, l: &Layout, top: f32, window: &mut Window) {
    let full = l.full;
    let y = top + 4.0;
    let mut x = full.left + 1.0;
    let (ss, se) = (full.x_at(scene.start), full.x_at(scene.end));
    while x <= full.left + full.width {
        let alpha = if x >= ss && x <= se { 0.55 } else { 0.18 };
        dot(window, x, y, 1.5, theme::white(alpha));
        x += 4.0;
    }
    let (v0, v1) = (full.x_at(l.map.view.start), full.x_at(l.map.view.end));
    let w = (v1 - v0).max(6.0);
    window.paint_quad(quad(
        rect(v0, top, w, 8.0),
        px(4.),
        theme::white(0.10),
        px(1.),
        theme::white(0.75),
        Default::default(),
    ));
    let xp = full.x_at(scene.playhead);
    window.paint_quad(fill(
        rect(xp - 0.5, top - 1.0, 1.0, 10.0),
        theme::white(0.9),
    ));
}

fn paint_flag(
    text: String,
    x: f32,
    y: f32,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let line = shape(
        window,
        text.into(),
        10.5,
        theme::black(0.94),
        FontWeight::SEMIBOLD,
    );
    let w = f32::from(line.width) + 14.0;
    let min = f32::from(bounds.origin.x) + 4.0;
    let max = f32::from(bounds.origin.x) + f32::from(bounds.size.width) - w - 4.0;
    let left = (x - w / 2.0).clamp(min, max.max(min));
    window.paint_quad(quad(
        rect(left, y, w, 18.0),
        px(9.),
        theme::white(0.96),
        px(0.),
        gpui::transparent_black(),
        Default::default(),
    ));
    line.paint(point(px(left + 7.0), px(y + 2.5)), px(13.), window, cx)
        .ok();
}

// ---- Gain slider -----------------------------------------------------------

pub(crate) struct GainScene {
    pub gain_db: f32,
    pub dragging: bool,
    pub width: f32,
}

pub(crate) fn gain_slider(scene: GainScene, entity: WeakEntity<Sonora>) -> impl IntoElement {
    let width = scene.width;
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        move |bounds, hitbox, window, _cx| {
            let left = f32::from(bounds.origin.x) + 6.0;
            let width = (f32::from(bounds.size.width) - 12.0).max(1.0);
            let mid = (f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.0).round();
            let x_at = move |db: f32| left + (db - GAIN_MIN) / (GAIN_MAX - GAIN_MIN) * width;
            let db_at = move |x: f32| {
                GAIN_MIN + ((x - left) / width).clamp(0.0, 1.0) * (GAIN_MAX - GAIN_MIN)
            };

            let zero = x_at(0.0);
            let thumb = x_at(scene.gain_db);
            let (lo, hi) = if thumb < zero {
                (thumb, zero)
            } else {
                (zero, thumb)
            };
            // One dot per 2 dB; those between unity and the gain are lit.
            let steps = ((GAIN_MAX - GAIN_MIN) / 2.0) as i32;
            for i in 0..=steps {
                let x = x_at(GAIN_MIN + i as f32 * 2.0);
                let on = x >= lo - 0.5 && x <= hi + 0.5 && hi - lo > 0.5;
                dot(
                    window,
                    x,
                    mid,
                    2.0,
                    theme::white(if on { 0.92 } else { 0.22 }),
                );
            }
            window.paint_quad(fill(
                rect(zero - 0.5, mid - 6.0, 1.0, 12.0),
                theme::white(0.5),
            ));

            let (tw, th) = if scene.dragging {
                (5.0, 18.0)
            } else {
                (4.0, 15.0)
            };
            window.paint_quad(quad(
                rect(thumb - tw / 2.0, mid - th / 2.0, tw, th),
                px(tw / 2.0),
                theme::white(1.0),
                px(1.),
                theme::black(0.6),
                Default::default(),
            ));

            window.set_cursor_style(CursorStyle::ResizeLeftRight, &hitbox);
            window.on_mouse_event({
                let entity = entity.clone();
                move |event: &MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble
                        || event.button != MouseButton::Left
                        || !hitbox.is_hovered(window)
                    {
                        return;
                    }
                    let x = f32::from(event.position.x);
                    entity
                        .update(cx, |this, cx| {
                            if event.click_count >= 2 {
                                this.reset_gain_by_double_click(cx);
                            } else if (x - thumb).abs() <= 8.0 {
                                // Grabbing the thumb never jumps it.
                                this.begin_gain_drag(None, db_at(x), cx);
                            } else {
                                this.begin_gain_drag(Some(db_at(x)), db_at(x), cx);
                            }
                        })
                        .ok();
                    cx.stop_propagation();
                }
            });
            window.on_mouse_event({
                let entity = entity.clone();
                move |event: &MouseMoveEvent, phase, _, cx| {
                    if phase == DispatchPhase::Bubble && event.dragging() {
                        entity
                            .update(cx, |this, cx| {
                                this.drag_gain(db_at(f32::from(event.position.x)), cx)
                            })
                            .ok();
                    }
                }
            });
            window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
                if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
                    entity.update(cx, |this, cx| this.end_drag(cx)).ok();
                }
            });
        },
    )
    .w(px(width))
    .h(px(24.))
    .flex_none()
}

// ---- Empty-state field -----------------------------------------------------

/// A static dot matrix that fades out from the middle of the stage.
pub(crate) fn dot_field() -> impl IntoElement {
    canvas(
        |_, _, _| (),
        |bounds, _, window, _| {
            const P: f32 = 14.0;
            let ox = f32::from(bounds.origin.x);
            let oy = f32::from(bounds.origin.y);
            let (w, h) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
            let cols = (w / P).floor() as i32;
            let rows = (h / P).floor() as i32;
            let x0 = (ox + (w - (cols - 1) as f32 * P) / 2.0).round();
            let y0 = (oy + (h - (rows - 1) as f32 * P) / 2.0).round();
            let (cx_, cy_) = (ox + w / 2.0, oy + h / 2.0);
            let reach = (w.max(h) / 2.0).max(1.0);
            for r in 0..rows {
                for c in 0..cols {
                    let (x, y) = (x0 + c as f32 * P, y0 + r as f32 * P);
                    let d = (((x - cx_) / reach).powi(2) + ((y - cy_) / reach).powi(2)).sqrt();
                    let alpha = 0.03 + 0.09 * (1.0 - d).max(0.0).powf(1.6);
                    dot(window, x, y, 2.0, theme::white(alpha));
                }
            }
        },
    )
    .absolute()
    .inset_0()
}

fn rect(x: f32, y: f32, w: f32, h: f32) -> Bounds<Pixels> {
    Bounds::new(point(px(x), px(y)), size(px(w.max(0.0)), px(h.max(0.0))))
}

fn shape(
    window: &mut Window,
    text: SharedString,
    font_size: f32,
    color: Hsla,
    weight: FontWeight,
) -> gpui::ShapedLine {
    let run = TextRun {
        len: text.len(),
        font: theme::numeric(weight),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(text, px(font_size), &[run], None)
}
