//! Sonora's palette and type: smoky translucent charcoal, warm ivory ink and a
//! single restrained seafoam accent. Amber is reserved for clipping, coral for
//! errors.

use std::sync::Arc;

use gpui::{Font, FontFeatures, FontStyle, FontWeight, Hsla, hsla};

/// The window surface laid over the macOS blur.
pub fn surface() -> Hsla {
    hsla(30. / 360., 0.06, 0.085, 0.70)
}

/// A recessed well (the waveform stage) sunk into the surface.
pub fn well() -> Hsla {
    hsla(30. / 360., 0.08, 0.04, 0.30)
}

pub fn ivory(alpha: f32) -> Hsla {
    hsla(40. / 360., 0.42, 0.93, alpha)
}

pub fn text() -> Hsla {
    ivory(0.92)
}

pub fn text_muted() -> Hsla {
    ivory(0.70)
}

pub fn text_faint() -> Hsla {
    ivory(0.54)
}

pub fn hairline() -> Hsla {
    ivory(0.075)
}

pub fn accent(alpha: f32) -> Hsla {
    hsla(158. / 360., 0.50, 0.72, alpha)
}

pub fn accent_strong() -> Hsla {
    hsla(160. / 360., 0.46, 0.64, 1.0)
}

pub fn amber(alpha: f32) -> Hsla {
    hsla(36. / 360., 0.90, 0.64, alpha)
}

pub fn coral(alpha: f32) -> Hsla {
    hsla(8. / 360., 0.80, 0.71, alpha)
}

pub fn shade(alpha: f32) -> Hsla {
    hsla(30. / 360., 0.10, 0.02, alpha)
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

/// A coarse `m:ss` label for ruler ticks.
pub fn tick_label(seconds: f64, step: f64) -> String {
    let total = seconds.max(0.0);
    if step < 1.0 {
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
        n => format!("{n} channels"),
    }
}
