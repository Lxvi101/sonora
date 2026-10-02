//! Small shared pieces: icons, keycaps, tooltips and the button treatments.

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

/// A quiet keycap for inline shortcut hints.
pub fn keycap(keys: impl Into<SharedString>) -> Div {
    div()
        .flex_none()
        .px(px(5.))
        .h(px(17.))
        .flex()
        .items_center()
        .rounded(px(4.))
        .bg(theme::ivory(0.07))
        .border_1()
        .border_color(theme::ivory(0.06))
        .text_size(px(10.5))
        .text_color(theme::text_muted())
        .child(keys.into())
}

/// A shortcut hint: keycap followed by what it does.
pub fn hint(keys: &'static str, label: &'static str) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(5.))
        .child(keycap(keys))
        .child(div().text_color(theme::text_faint()).child(label))
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
                .gap(px(8.))
                .px(px(9.))
                .py(px(5.))
                .rounded(px(7.))
                .bg(theme::shade(0.88))
                .border_1()
                .border_color(theme::ivory(0.10))
                .shadow(vec![BoxShadow {
                    color: theme::shade(0.35),
                    offset: point(px(0.), px(4.)),
                    blur_radius: px(14.),
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

/// The round seafoam transport button.
pub fn round_button(id: &'static str, enabled: bool) -> Stateful<Div> {
    let base = div()
        .id(id)
        .flex_none()
        .size(px(36.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center();
    if enabled {
        base.bg(theme::accent(0.92))
            .border_1()
            .border_color(theme::accent(0.55))
            .shadow(vec![BoxShadow {
                color: theme::accent(0.22),
                offset: point(px(0.), px(2.)),
                blur_radius: px(12.),
                spread_radius: px(0.),
            }])
            .cursor_pointer()
            .hover(|s| s.bg(theme::accent(1.0)))
            .active(|s| s.bg(theme::accent_strong()))
    } else {
        base.bg(theme::ivory(0.06))
            .border_1()
            .border_color(theme::ivory(0.05))
    }
}

/// A translucent pill; `primary` tints it with the accent.
pub fn pill_button(id: &'static str, primary: bool, enabled: bool) -> Stateful<Div> {
    let base = div()
        .id(id)
        .flex_none()
        .h(px(30.))
        .px(px(12.))
        .flex()
        .items_center()
        .gap(px(7.))
        .rounded(px(8.))
        .border_1()
        .text_size(px(12.5))
        .font_weight(FontWeight::MEDIUM)
        .whitespace_nowrap();
    match (primary, enabled) {
        (_, false) => base
            .bg(theme::ivory(0.03))
            .border_color(theme::ivory(0.05))
            .text_color(theme::text_faint()),
        (true, true) => base
            .bg(theme::accent(0.13))
            .border_color(theme::accent(0.30))
            .text_color(theme::accent(1.0))
            .cursor_pointer()
            .hover(|s| s.bg(theme::accent(0.20)).border_color(theme::accent(0.45)))
            .active(|s| s.bg(theme::accent(0.26))),
        (false, true) => base
            .bg(theme::ivory(0.06))
            .border_color(theme::ivory(0.08))
            .text_color(theme::text())
            .cursor_pointer()
            .hover(|s| s.bg(theme::ivory(0.10)))
            .active(|s| s.bg(theme::ivory(0.13))),
    }
}

/// An inline text action, used in the footer.
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
        .hover(|s| s.bg(theme::ivory(0.07)))
        .active(|s| s.bg(theme::ivory(0.11)))
}
