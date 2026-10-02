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

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(FILES
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> Result<Vec<SharedString>> {
        Ok(FILES
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| SharedString::from(*name))
            .collect())
    }
}
