//! Small shared pieces: icons, keycaps, tooltips, buttons and the dotted
//! brand glyph.

use gpui::{
    AnyView, App, BoxShadow, Context, Div, FontWeight, Hsla, IntoElement, ParentElement, Render,
    SharedString, Stateful, Styled, Svg, Window, div, point, prelude::*, px, svg,
};

use super::theme;

pub fn icon(name: &'static str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(SharedString::new_static(name))
        .size(px(size))
        .flex_none()
        .text_color(color)
}

/// A letter-spaced caps label.
pub fn caps(text: &str, size: f32, color: Hsla) -> Div {
    div()
        .flex_none()
        .whitespace_nowrap()
        .text_size(px(size))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(color)
        .child(theme::tracked(text))
}

/// A quiet keycap for inline shortcut hints.
pub fn keycap(keys: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .min_w(px(18.))
        .px(px(5.))
        .h(px(18.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(4.))
        .bg(theme::white(0.06))
        .border_1()
        .border_color(theme::white(0.10))
        .font(theme::numeric(FontWeight::MEDIUM))
        .text_size(px(10.5))
        .text_color(theme::text_muted())
        .child(keys.into())
}

pub struct Tooltip {
    label: SharedString,
    keys: Option<SharedString>,
}

impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // Offset from the pointer so the tip never sits under it.
        div().pl(px(8.)).pt(px(16.)).child(
            div()
                .flex()
                .items_center()
                .gap(px(9.))
                .px(px(9.))
                .py(px(6.))
                .rounded(px(6.))
                .bg(theme::panel())
                .border_1()
                .border_color(theme::white(0.14))
                .shadow(vec![BoxShadow {
                    color: theme::black(0.6),
                    offset: point(px(0.), px(6.)),
                    blur_radius: px(18.),
                    spread_radius: px(0.),
                }])
                .font_family(theme::UI_FONT)
                .text_size(px(11.5))
                .text_color(theme::text())
                .whitespace_nowrap()
                .child(self.label.clone())
                .children(self.keys.clone().map(keycap)),
        )
    }
}

pub fn tooltip(
    label: impl Into<SharedString>,
    keys: Option<&'static str>,
) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    let label = label.into();
    let keys = keys.map(SharedString::new_static);
    move |_, cx| {
        let (label, keys) = (label.clone(), keys.clone());
        cx.new(|_| Tooltip { label, keys }).into()
    }
}

/// The white circular play button — the one solid shape in the transport.
pub fn play_button(id: &'static str, enabled: bool) -> Stateful<Div> {
    let base = div()
        .id(id)
        .flex_none()
        .size(px(40.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center();
    if enabled {
        base.bg(theme::white(0.96))
            .shadow(vec![BoxShadow {
                color: theme::white(0.10),
                offset: point(px(0.), px(0.)),
                blur_radius: px(16.),
                spread_radius: px(0.),
            }])
            .cursor_pointer()
            .hover(|s| s.bg(theme::white(1.0)))
            .active(|s| s.bg(theme::white(0.80)))
    } else {
        base.bg(theme::white(0.06))
            .border_1()
            .border_color(theme::white(0.08))
    }
}

/// A square icon button. `on` inverts it to white for engaged toggles.
pub fn icon_button(id: &'static str, enabled: bool, on: bool) -> Stateful<Div> {
    let base = div()
        .id(id)
        .flex_none()
        .size(px(28.))
        .rounded(px(7.))
        .flex()
        .items_center()
        .justify_center();
    match (enabled, on) {
        (false, _) => base,
        (true, true) => base
            .bg(theme::white(0.94))
            .cursor_pointer()
            .hover(|s| s.bg(theme::white(1.0)))
            .active(|s| s.bg(theme::white(0.78))),
        (true, false) => base
            .cursor_pointer()
            .hover(|s| s.bg(theme::white(0.08)))
            .active(|s| s.bg(theme::white(0.13))),
    }
}

/// Icon color to pair with [`icon_button`].
pub fn icon_ink(enabled: bool, on: bool) -> Hsla {
    match (enabled, on) {
        (false, _) => theme::text_ghost(),
        (true, true) => theme::black(0.92),
        (true, false) => theme::text_muted(),
    }
}

/// The solid white call to action (Export, Open).
pub fn primary_button(id: &'static str, enabled: bool) -> Stateful<Div> {
    let base = div()
        .id(id)
        .flex_none()
        .h(px(30.))
        .px(px(14.))
        .flex()
        .items_center()
        .gap(px(7.))
        .rounded(px(15.))
        .text_size(px(12.5))
        .font_weight(FontWeight::SEMIBOLD)
        .whitespace_nowrap();
    if enabled {
        base.bg(theme::white(0.96))
            .text_color(theme::black(0.94))
            .cursor_pointer()
            .hover(|s| s.bg(theme::white(1.0)))
            .active(|s| s.bg(theme::white(0.80)))
    } else {
        base.bg(theme::white(0.08)).text_color(theme::text_faint())
    }
}

/// An inline text action, used in the status line.
pub fn link_button(id: &'static str, color: Hsla) -> Stateful<Div> {
    div()
        .id(id)
        .flex_none()
        .px(px(6.))
        .h(px(20.))
        .flex()
        .items_center()
        .gap(px(4.))
        .rounded(px(5.))
        .text_color(color)
        .font_weight(FontWeight::MEDIUM)
        .cursor_pointer()
        .hover(|s| s.bg(theme::white(0.08)))
        .active(|s| s.bg(theme::white(0.13)))
}

/// The Sonora mark: a waveform drawn as columns of dots on a dot matrix.
/// `heights` are lit dots per column (odd numbers keep it centered).
pub fn dot_glyph(heights: &[u8], dot: f32, gap: f32, lit: Hsla, unlit: Hsla) -> Div {
    let rows = heights.iter().copied().max().unwrap_or(1) as usize;
    div()
        .flex()
        .flex_none()
        .gap(px(gap))
        .children(heights.iter().map(|&h| {
            let pad = (rows - h as usize) / 2;
            div()
                .flex()
                .flex_col()
                .gap(px(gap))
                .children((0..rows).map(move |row| {
                    let on = row >= pad && row < pad + h as usize;
                    div()
                        .size(px(dot))
                        .rounded_full()
                        .bg(if on { lit } else { unlit })
                }))
        }))
}

/// The small mark in the titlebar.
pub const MARK_SMALL: [u8; 5] = [1, 3, 5, 3, 1];
/// The large mark on the empty stage.
pub const MARK_LARGE: [u8; 9] = [1, 3, 5, 9, 7, 9, 5, 3, 1];
