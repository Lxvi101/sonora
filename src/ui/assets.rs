//! Icons compiled into the binary so the app needs no resource lookup.

use std::borrow::Cow;

use gpui::{AssetSource, Result, SharedString};

pub struct Assets;

const FILES: &[(&str, &[u8])] = &[
    (
        "icons/play.svg",
        include_bytes!("../../assets/icons/play.svg"),
    ),
    (
        "icons/pause.svg",
        include_bytes!("../../assets/icons/pause.svg"),
    ),
    (
        "icons/open.svg",
        include_bytes!("../../assets/icons/open.svg"),
    ),
    (
        "icons/export.svg",
        include_bytes!("../../assets/icons/export.svg"),
    ),
    (
        "icons/volume.svg",
        include_bytes!("../../assets/icons/volume.svg"),
    ),
    (
        "icons/check.svg",
        include_bytes!("../../assets/icons/check.svg"),
    ),
    (
        "icons/warning.svg",
        include_bytes!("../../assets/icons/warning.svg"),
    ),
    (
        "icons/close.svg",
        include_bytes!("../../assets/icons/close.svg"),
    ),
    (
        "icons/reset.svg",
        include_bytes!("../../assets/icons/reset.svg"),
    ),
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
