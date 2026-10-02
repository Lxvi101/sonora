//! Sonora's palette and type. Strictly achromatic: matte near-black surfaces,
//! pure white ink and a few neutral greys. Emphasis comes from luminance,
//! weight and the dot matrix — never from hue.

use std::sync::Arc;

use gpui::{Font, FontFeatures, FontStyle, FontWeight, Hsla, hsla};

pub fn white(alpha: f32) -> Hsla {
    hsla(0., 0., 1., alpha)
}

pub fn black(alpha: f32) -> Hsla {
    hsla(0., 0., 0., alpha)
}

/// The window body: a rich, almost opaque black over the system blur.
pub fn surface() -> Hsla {
    hsla(0., 0., 0.043, 0.965)
}

/// Translucent black chrome for the titlebar strip.
pub fn chrome() -> Hsla {
    black(0.38)
}

/// The waveform well, a step deeper than the body.
pub fn well() -> Hsla {
    hsla(0., 0., 0.012, 0.92)
}

/// Raised panels: tooltips, the help sheet.
pub fn panel() -> Hsla {
    hsla(0., 0., 0.075, 0.985)
}

pub fn text() -> Hsla {
    white(0.94)
}

pub fn text_muted() -> Hsla {
    white(0.62)
}

pub fn text_faint() -> Hsla {
    white(0.44)
}

pub fn text_ghost() -> Hsla {
    white(0.26)
}

pub fn hairline() -> Hsla {
    white(0.085)
}

pub const UI_FONT: &str = ".SystemUIFont";

/// System UI font with tabular figures so changing times don't jitter.
pub fn numeric(weight: FontWeight) -> Font {
    Font {
        family: UI_FONT.into(),
        features: FontFeatures(Arc::new(vec![("tnum".into(), 1)])),
        fallbacks: None,
        weight,
        style: FontStyle::Normal,
    }
}

/// Letter-spaced small caps for labels: hair spaces between letters, a
/// wider gap between words.
pub fn tracked(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 4);
    for (i, ch) in text.chars().enumerate() {
        if i > 0 {
            out.push(if ch == ' ' { '\u{2002}' } else { '\u{200A}' });
        }
        if ch != ' ' {
            out.extend(ch.to_uppercase());
        }
    }
    out
}

/// `m:ss.cc`, or `h:mm:ss.cc` past an hour.
pub fn timecode(seconds: f64) -> String {
    let centis = (seconds.max(0.0) * 100.0).round() as u64;
    let (cs, total_s) = (centis % 100, centis / 100);
    let (s, total_m) = (total_s % 60, total_s / 60);
    let (m, h) = (total_m % 60, total_m / 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}.{cs:02}")
    } else {
        format!("{m}:{s:02}.{cs:02}")
    }
}

/// A coarse label for ruler ticks; finer steps show more decimals.
pub fn tick_label(seconds: f64, step: f64) -> String {
    let total = seconds.max(0.0);
    if step < 0.1 {
        let whole = total.floor() as u64;
        let centis = ((total - whole as f64) * 100.0).round() as u64 % 100;
        format!("{}:{:02}.{:02}", whole / 60, whole % 60, centis)
    } else if step < 1.0 {
        let whole = total.floor() as u64;
        let tenths = ((total - whole as f64) * 10.0).round() as u64 % 10;
        format!("{}:{:02}.{}", whole / 60, whole % 60, tenths)
    } else {
        let whole = total.round() as u64;
        if whole >= 3600 {
            format!(
                "{}:{:02}:{:02}",
                whole / 3600,
                (whole / 60) % 60,
                whole % 60
            )
        } else {
            format!("{}:{:02}", whole / 60, whole % 60)
        }
    }
}

/// `+4.5 dB`, `0.0 dB`, `−6.0 dB` with a true minus sign.
pub fn decibels(db: f32) -> String {
    let rounded = (db * 10.0).round() / 10.0;
    if rounded > 0.0 {
        format!("+{rounded:.1} dB")
    } else if rounded < 0.0 {
        format!("\u{2212}{:.1} dB", -rounded)
    } else {
        "0.0 dB".into()
    }
}

pub fn sample_rate(hz: f64) -> String {
    let khz = hz / 1000.0;
    if (khz - khz.round()).abs() < 0.001 {
        format!("{khz:.0} kHz")
    } else {
        format!("{khz:.1} kHz")
    }
}

pub fn channels(count: u32) -> String {
    match count {
        1 => "Mono".into(),
        2 => "Stereo".into(),
        n => format!("{n} ch"),
    }
}
