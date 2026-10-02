//! Icons compiled into the binary so the app needs no resource lookup.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

pub struct Assets;

macro_rules! icons {
    ($($name:literal),* $(,)?) => {
        &[$((
            concat!("icons/", $name, ".svg"),
            include_bytes!(concat!("../../assets/icons/", $name, ".svg")),
        )),*]
    };
}

const FILES: &[(&str, &[u8])] = icons![
    "play",
    "pause",
    "to-start",
    "loop",
    "undo",
    "redo",
    "reset",
    "zoom-in",
    "zoom-out",
    "zoom-selection",
    "zoom-fit",
    "open",
    "export",
    "check",
    "warning",
    "close",
    "help",
];

/// The clip tools' icons, drawn in the same 24 px, 1.8-stroke line style as
/// the files above.
const TOOLS: &[(&str, &[u8])] = &[
    (
        "icons/select.svg",
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><path d="M6 3.5v15.5l4.2-4.1 2.9 6.1 2.6-1.2-2.9-6h5.7z"/></svg>"##,
    ),
    (
        "icons/razor.svg",
        br##"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="#000" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round"><g transform="rotate(-40 12 12)"><path d="M3 8.5h18v7H3z"/><path d="M3 12h2.5M18.5 12H21M9 12h6"/></g></svg>"##,
    ),
];

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(FILES
            .iter()
            .chain(TOOLS)
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(FILES
            .iter()
            .chain(TOOLS)
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| SharedString::from(*name))
            .collect())
    }
}
