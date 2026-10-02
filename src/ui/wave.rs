//! Custom-painted controls: the waveform stage (bars, trim handles, playhead,
//! ruler) and the gain slider. Both register their pointer handlers during
//! paint so they can keep tracking a drag that leaves their bounds.

use std::sync::Arc;

use gpui::{
    App, Bounds, CursorStyle, DispatchPhase, FontWeight, Hitbox, HitboxBehavior, Hsla, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, SharedString, Styled,
    TextRun, WeakEntity, Window, canvas, fill, point, px, quad, size,
};

use super::{Handle, Sonora, theme};
use crate::audio::Peak;

const PAD_X: f32 = 14.0;
const PAD_TOP: f32 = 16.0;
const RULER: f32 = 22.0;
const BAR: f32 = 2.0;
const STEP: f32 = 3.0;
const GRAB: f32 = 8.0;

/// Everything the waveform needs for one frame.
pub(crate) struct WaveScene {
    pub peaks: Option<Arc<Vec<Peak>>>,
    pub exact: bool,
    pub duration: f64,
    pub start: f64,
    pub end: f64,
    pub gain_db: f32,
    pub playhead: f64,
    pub hover: Option<Handle>,
    pub dragging: Option<Handle>,
    pub message: Option<(SharedString, Hsla)>,
}

#[derive(Clone, Copy)]
struct Geometry {
    left: f32,
    width: f32,
    top: f32,
    bottom: f32,
    duration: f64,
}

impl Geometry {
    fn new(bounds: Bounds<Pixels>, duration: f64) -> Self {
        let left = f32::from(bounds.origin.x) + PAD_X;
        let width = (f32::from(bounds.size.width) - PAD_X * 2.0).max(1.0);
        let top = f32::from(bounds.origin.y) + PAD_TOP;
        let bottom =
            (f32::from(bounds.origin.y) + f32::from(bounds.size.height) - RULER).max(top + 1.0);
        Self {
            left,
            width,
            top,
            bottom,
            duration: duration.max(1e-9),
        }
    }

    fn x_at(&self, time: f64) -> f32 {
        self.left + (time / self.duration).clamp(0.0, 1.0) as f32 * self.width
    }

    fn time_at(&self, x: f32) -> f64 {
        (((x - self.left) / self.width).clamp(0.0, 1.0) as f64) * self.duration
    }

    fn handle_at(&self, x: f32, start: f64, end: f64) -> Option<Handle> {
        let (ds, de) = ((x - self.x_at(start)).abs(), (x - self.x_at(end)).abs());
        match (ds <= GRAB, de <= GRAB) {
            (true, true) => Some(if de < ds || x > self.x_at(end) {
                Handle::End
            } else {
                Handle::Start
            }),
            (true, false) => Some(Handle::Start),
            (false, true) => Some(Handle::End),
            _ => None,
        }
    }
}

pub(crate) fn waveform(scene: WaveScene, entity: WeakEntity<Sonora>) -> impl IntoElement {
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        move |bounds, hitbox, window, cx| {
            let geo = Geometry::new(bounds, scene.duration);
            paint_waveform(&scene, geo, bounds, window, cx);

            let cursor = if scene.dragging.is_some() || scene.hover.is_some() {
                CursorStyle::ResizeLeftRight
            } else {
                CursorStyle::IBeam
            };
            window.set_cursor_style(cursor, &hitbox);
            register_wave_events(&scene, geo, hitbox, entity, window);
        },
    )
    .size_full()
}

fn register_wave_events(
    scene: &WaveScene,
    geo: Geometry,
    hitbox: Hitbox,
    entity: WeakEntity<Sonora>,
    window: &mut Window,
) {
    let (start, end) = (scene.start, scene.end);
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
            let target = geo.handle_at(x, start, end);
            entity
                .update(cx, |this, cx| match target {
                    Some(handle) => this.begin_handle_drag(handle, cx),
                    None => this.begin_range_drag(geo.time_at(x), x, cx),
                })
                .ok();
            cx.stop_propagation();
        }
    });
    window.on_mouse_event({
        let entity = entity.clone();
        move |event: &MouseMoveEvent, phase, window, cx| {
            if phase != DispatchPhase::Bubble {
                return;
            }
            let x = f32::from(event.position.x);
            let hover = hitbox
                .is_hovered(window)
                .then(|| geo.handle_at(x, start, end))
                .flatten();
            entity
                .update(cx, |this, cx| {
                    if event.dragging() {
                        this.drag_wave(geo.time_at(x), x, cx);
                    } else {
                        // A release outside the window never reached us.
                        this.end_drag(cx);
                    }
                    if this.drag.is_none() {
                        this.set_hover(hover, cx);
                    }
                })
                .ok();
        }
    });
    window.on_mouse_event(move |event: &MouseUpEvent, phase, _, cx| {
        if phase == DispatchPhase::Bubble && event.button == MouseButton::Left {
            entity.update(cx, |this, cx| this.end_drag(cx)).ok();
        }
    });
}

fn paint_waveform(
    scene: &WaveScene,
    geo: Geometry,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let xs = geo.x_at(scene.start);
    let xe = geo.x_at(scene.end);
    let height = geo.bottom - geo.top;
    let mid = geo.top + height / 2.0;
    let half = height / 2.0 * 0.94;
    let full_left = f32::from(bounds.origin.x);
    let full_right = full_left + f32::from(bounds.size.width);

    // Trimmed-away regions recede; the kept region gets a faint wash.
    window.paint_quad(fill(
        rect(full_left, geo.top - 6.0, xs - full_left, height + 6.0),
        theme::shade(0.16),
    ));
    window.paint_quad(fill(
        rect(xe, geo.top - 6.0, full_right - xe, height + 6.0),
        theme::shade(0.16),
    ));
    window.paint_quad(fill(
        rect(xs, geo.top - 6.0, xe - xs, height + 6.0),
        theme::accent(0.045),
    ));
    window.paint_quad(fill(
        rect(geo.left, mid - 0.5, geo.width, 1.0),
        theme::ivory(0.07),
    ));

    if let Some(peaks) = scene.peaks.as_deref().filter(|p| !p.is_empty()) {
        let gain = 10f32.powf(scene.gain_db / 20.0);
        let bins = peaks.len();
        let kept = if scene.exact {
            theme::accent(0.95)
        } else {
            theme::accent(0.62)
        };
        let trimmed = theme::ivory(0.26);
        let mut x = geo.left;
        while x + BAR <= geo.left + geo.width + 0.01 {
            let f0 = (x - geo.left) / geo.width;
            let f1 = (x + STEP - geo.left) / geo.width;
            let b0 = ((f0 * bins as f32) as usize).min(bins - 1);
            let b1 = ((f1 * bins as f32) as usize).clamp(b0 + 1, bins);
            let amp = peaks[b0..b1]
                .iter()
                .fold(0f32, |acc, p| acc.max(p.min.abs()).max(p.max.abs()));
            let level = amp * gain;
            let bar_half = (level.min(1.0) * half).max(0.75);
            let center = x + BAR / 2.0;
            let inside = center >= xs && center <= xe;
            let color = if inside { kept } else { trimmed };
            window.paint_quad(quad(
                rect(x, mid - bar_half, BAR, bar_half * 2.0),
                px(1.0),
                color,
                px(0.),
                gpui::transparent_black(),
                Default::default(),
            ));
            if inside && level > 1.0 {
                let cap = 3.0f32.min(bar_half);
                window.paint_quad(fill(rect(x, mid - bar_half, BAR, cap), theme::amber(0.95)));
                window.paint_quad(fill(
                    rect(x, mid + bar_half - cap, BAR, cap),
                    theme::amber(0.95),
                ));
            }
            x += STEP;
        }
    }

    if let Some((message, color)) = &scene.message {
        let line = shape(window, message.clone(), 11.5, *color, FontWeight::NORMAL);
        let origin = point(
            px(geo.left + (geo.width - f32::from(line.width)) / 2.0),
            px(mid - 22.0),
        );
        line.paint(origin, px(15.), window, cx).ok();
    }

    paint_ruler(geo, window, cx);

    for (handle, x) in [(Handle::Start, xs), (Handle::End, xe)] {
        let active = scene.dragging == Some(handle)
            || (scene.dragging.is_none() && scene.hover == Some(handle));
        let color = if active {
            theme::ivory(0.98)
        } else {
            theme::accent(1.0)
        };
        window.paint_quad(fill(
            rect(x - 0.75, geo.top - 6.0, 1.5, height + 6.0),
            color.opacity(0.85),
        ));
        let (gw, gh) = if active { (9.0, 34.0) } else { (8.0, 28.0) };
        window.paint_quad(quad(
            rect(x - gw / 2.0, mid - gh / 2.0, gw, gh),
            px(gw / 2.0),
            color,
            px(0.),
            gpui::transparent_black(),
            Default::default(),
        ));
        window.paint_quad(fill(
            rect(x - 0.5, mid - 6.0, 1.0, 12.0),
            theme::shade(0.45),
        ));
    }

    // The playhead.
    let xp = geo.x_at(scene.playhead);
    window.paint_quad(fill(
        rect(xp - 0.75, geo.top - 4.0, 1.5, height + 4.0),
        theme::ivory(0.92),
    ));
    window.paint_quad(quad(
        rect(xp - 4.0, geo.top - 11.0, 8.0, 8.0),
        px(4.0),
        theme::ivory(0.95),
        px(0.),
        gpui::transparent_black(),
        Default::default(),
    ));

    if let Some(handle) = scene.dragging {
        let (x, time) = match handle {
            Handle::Start => (xs, scene.start),
            Handle::End => (xe, scene.end),
        };
        paint_flag(theme::timecode(time), x, geo.top + 4.0, bounds, window, cx);
    }
}

fn paint_ruler(geo: Geometry, window: &mut Window, cx: &mut App) {
    const STEPS: [f64; 17] = [
        0.1, 0.2, 0.5, 1., 2., 5., 10., 15., 30., 60., 120., 300., 600., 900., 1800., 3600., 7200.,
    ];
    let px_per_second = geo.width as f64 / geo.duration;
    let step = STEPS
        .iter()
        .copied()
        .find(|s| s * px_per_second >= 72.0)
        .unwrap_or(7200.);
    let y = geo.bottom + 3.0;
    let mut i = 0u64;
    loop {
        let t = i as f64 * step;
        if t > geo.duration + 1e-9 {
            break;
        }
        let x = geo.x_at(t);
        window.paint_quad(fill(rect(x - 0.5, y, 1.0, 4.0), theme::ivory(0.26)));
        let line = shape(
            window,
            theme::tick_label(t, step).into(),
            10.0,
            theme::text_faint(),
            FontWeight::NORMAL,
        );
        if x + 4.0 + f32::from(line.width) <= geo.left + geo.width + PAD_X {
            line.paint(point(px(x + 4.0), px(y + 1.0)), px(13.), window, cx)
                .ok();
        }
        i += 1;
    }
}

fn paint_flag(
    text: String,
    x: f32,
    y: f32,
    bounds: Bounds<Pixels>,
    window: &mut Window,
    cx: &mut App,
) {
    let line = shape(window, text.into(), 10.5, theme::text(), FontWeight::MEDIUM);
    let w = f32::from(line.width) + 12.0;
    let min = f32::from(bounds.origin.x) + 4.0;
    let max = f32::from(bounds.origin.x) + f32::from(bounds.size.width) - w - 4.0;
    let left = (x - w / 2.0).clamp(min, max.max(min));
    window.paint_quad(quad(
        rect(left, y, w, 18.0),
        px(5.),
        theme::shade(0.86),
        px(1.),
        theme::ivory(0.10),
        Default::default(),
    ));
    line.paint(point(px(left + 6.0), px(y + 2.5)), px(13.), window, cx)
        .ok();
}

// ---- Gain slider -----------------------------------------------------------

pub(crate) struct GainScene {
    pub gain_db: f32,
    pub enabled: bool,
    pub clipping: bool,
    pub dragging: bool,
}

pub(crate) fn gain_slider(scene: GainScene, entity: WeakEntity<Sonora>) -> impl IntoElement {
    canvas(
        |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
        move |bounds, hitbox, window, _cx| {
            let left = f32::from(bounds.origin.x) + 7.0;
            let width = (f32::from(bounds.size.width) - 14.0).max(1.0);
            let mid = f32::from(bounds.origin.y) + f32::from(bounds.size.height) / 2.0;
            let x_at = move |db: f32| {
                left + (db - super::GAIN_MIN) / (super::GAIN_MAX - super::GAIN_MIN) * width
            };
            let db_at = move |x: f32| {
                super::GAIN_MIN
                    + ((x - left) / width).clamp(0.0, 1.0) * (super::GAIN_MAX - super::GAIN_MIN)
            };

            let zero = x_at(0.0);
            let thumb = x_at(scene.gain_db);
            let dim = if scene.enabled { 1.0 } else { 0.4 };
            window.paint_quad(quad(
                rect(left, mid - 2.0, width, 4.0),
                px(2.),
                theme::ivory(0.10 * dim),
                px(0.),
                gpui::transparent_black(),
                Default::default(),
            ));
            let tint = if scene.clipping {
                theme::amber(0.95)
            } else {
                theme::accent(0.95)
            };
            let (a, b) = if thumb < zero {
                (thumb, zero)
            } else {
                (zero, thumb)
            };
            window.paint_quad(fill(
                rect(a, mid - 2.0, b - a, 4.0),
                tint.opacity(0.95 * dim),
            ));
            window.paint_quad(fill(
                rect(zero - 0.5, mid - 6.0, 1.0, 12.0),
                theme::ivory(0.30 * dim),
            ));

            let r = if scene.dragging { 7.5 } else { 6.5 };
            window.paint_quad(quad(
                rect(thumb - r, mid - r, r * 2.0, r * 2.0),
                px(r),
                theme::ivory(if scene.enabled { 0.96 } else { 0.35 }),
                px(1.),
                theme::shade(0.25),
                Default::default(),
            ));

            if !scene.enabled {
                return;
            }
            window.set_cursor_style(CursorStyle::PointingHand, &hitbox);
            window.on_mouse_event({
                let entity = entity.clone();
                move |event: &MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble
                        || event.button != MouseButton::Left
                        || !hitbox.is_hovered(window)
                    {
                        return;
                    }
                    let db = if event.click_count >= 2 {
                        0.0
                    } else {
                        db_at(f32::from(event.position.x))
                    };
                    entity
                        .update(cx, |this, cx| this.begin_gain_drag(db, cx))
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
    .w(px(132.))
    .h(px(22.))
    .flex_none()
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
