//! Automatic updates through Sparkle 2's standard updater (see
//! `native/updater.m`), plus the About panel.
//!
//! The updater only runs in a release bundle that carries a valid feed and
//! EdDSA public key. Development runs and source builds without those keys
//! report it as unavailable, and nothing else changes.

use std::ffi::{CString, c_char};

pub const REPOSITORY: &str = "https://github.com/Lxvi101/sonora";
pub const NEW_ISSUE: &str = "https://github.com/Lxvi101/sonora/issues/new";

unsafe extern "C" {
    fn sonora_app_is_bundled() -> i32;
    fn sonora_bundle_string(key: *const c_char, out: *mut c_char, cap: usize) -> i32;
    fn sonora_updater_start(err: *mut c_char, cap: usize) -> i32;
    fn sonora_updater_check();
    fn sonora_updater_automatic() -> i32;
    fn sonora_updater_set_automatic(on: i32);
    fn sonora_show_about(version: *const c_char, credits: *const c_char, link: *const c_char);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    /// Sparkle is running; checks and installs go through its standard UI.
    Ready,
    /// Not running from an app bundle (`cargo run`, tests).
    Development,
    /// A bundle without a usable `SUFeedURL` / `SUPublicEDKey`, such as a
    /// source build: no feed is contacted.
    NotConfigured,
    /// Configured, but Sparkle failed to load or start.
    Failed(String),
}

impl Status {
    pub fn is_ready(&self) -> bool {
        matches!(self, Status::Ready)
    }
}

/// Whether this build may run the updater at all, from its bundle facts.
pub fn configuration(bundled: bool, feed: Option<&str>, key: Option<&str>) -> Result<(), Status> {
    if !bundled {
        return Err(Status::Development);
    }
    match (feed, key) {
        (Some(feed), Some(key)) if is_https_url(feed) && is_ed25519_public_key(key) => Ok(()),
        _ => Err(Status::NotConfigured),
    }
}

/// Feeds must be HTTPS so the appcast itself can't be swapped in transit.
fn is_https_url(url: &str) -> bool {
    let url = url.trim();
    url.strip_prefix("https://")
        .is_some_and(|rest| rest.contains('.') && !rest.starts_with('/'))
        && !url.chars().any(char::is_whitespace)
}

/// An EdDSA (ed25519) public key as Sparkle stores it: 32 bytes, base64.
fn is_ed25519_public_key(key: &str) -> bool {
    let key = key.trim();
    let body = key.as_bytes();
    // 32 bytes encode to 43 base64 characters plus one '=' of padding.
    body.len() == 44
        && body[43] == b'='
        && body[..43]
            .iter()
            .all(|b| b.is_ascii_alphanumeric() || *b == b'+' || *b == b'/')
}

fn bundle_string(key: &str) -> Option<String> {
    let key = CString::new(key).ok()?;
    let mut out = vec![0 as c_char; 512];
    let found = unsafe { sonora_bundle_string(key.as_ptr(), out.as_mut_ptr(), out.len()) };
    (found != 0).then(|| text(&out))
}

fn text(buffer: &[c_char]) -> String {
    let bytes: Vec<u8> = buffer
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| *c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).trim().to_string()
}

/// Starts Sparkle once. Call on the main thread after the window exists.
pub fn start() -> Status {
    let bundled = unsafe { sonora_app_is_bundled() } != 0;
    let feed = bundle_string("SUFeedURL");
    let key = bundle_string("SUPublicEDKey");
    if let Err(status) = configuration(bundled, feed.as_deref(), key.as_deref()) {
        return status;
    }
    let mut err = vec![0 as c_char; 1024];
    if unsafe { sonora_updater_start(err.as_mut_ptr(), err.len()) } == 0 {
        Status::Ready
    } else {
        let reason = text(&err);
        eprintln!("Sonora: updates are unavailable — {reason}");
        Status::Failed(reason)
    }
}

/// Sparkle's standard "Check for Updates…": it shows its own window.
pub fn check_for_updates() {
    unsafe { sonora_updater_check() }
}

pub fn automatic_checks() -> bool {
    unsafe { sonora_updater_automatic() != 0 }
}

pub fn set_automatic_checks(on: bool) {
    unsafe { sonora_updater_set_automatic(on as i32) }
}

/// The standard About panel with the version and a source link.
pub fn show_about() {
    let version = CString::new(env!("CARGO_PKG_VERSION")).unwrap_or_default();
    let credits = CString::new("Open source under the MIT License.\ngithub.com/Lxvi101/sonora")
        .unwrap_or_default();
    let link = CString::new(REPOSITORY).unwrap_or_default();
    unsafe { sonora_show_about(version.as_ptr(), credits.as_ptr(), link.as_ptr()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = "https://raw.githubusercontent.com/Lxvi101/sonora/main/appcast.xml";
    const KEY: &str = "pfIShU4dEXqPd5ObYNfDBiQWcXozk7estwzTnF9BamQ=";

    #[test]
    fn development_runs_never_update() {
        assert_eq!(
            configuration(false, Some(FEED), Some(KEY)),
            Err(Status::Development)
        );
    }

    #[test]
    fn a_configured_bundle_may_update() {
        assert_eq!(configuration(true, Some(FEED), Some(KEY)), Ok(()));
    }

    #[test]
    fn source_builds_without_feed_or_key_stay_offline() {
        assert_eq!(configuration(true, None, None), Err(Status::NotConfigured));
        assert_eq!(
            configuration(true, Some(FEED), None),
            Err(Status::NotConfigured)
        );
        assert_eq!(
            configuration(true, None, Some(KEY)),
            Err(Status::NotConfigured)
        );
        assert_eq!(
            configuration(true, Some(FEED), Some("")),
            Err(Status::NotConfigured)
        );
    }

    #[test]
    fn feeds_must_be_https() {
        let http = "http://raw.githubusercontent.com/Lxvi101/sonora/main/appcast.xml";
        assert_eq!(
            configuration(true, Some(http), Some(KEY)),
            Err(Status::NotConfigured)
        );
        assert_eq!(
            configuration(true, Some("https://"), Some(KEY)),
            Err(Status::NotConfigured)
        );
        assert_eq!(
            configuration(true, Some("$(SU_FEED_URL)"), Some(KEY)),
            Err(Status::NotConfigured)
        );
    }

    #[test]
    fn keys_must_be_a_base64_ed25519_public_key() {
        for bad in [
            "YOUR_PUBLIC_KEY",
            "pfIShU4dEXqPd5ObYNfDBiQWcXozk7estwzTnF9BamQ",
            "pfIShU4dEXqPd5ObYNfDBiQWcXozk7estwzTnF9Bam==",
            "pfIShU4dEXqPd5ObYNfDBiQWcXozk7estwzTnF9Ba*Q=",
        ] {
            assert_eq!(
                configuration(true, Some(FEED), Some(bad)),
                Err(Status::NotConfigured),
                "{bad}"
            );
        }
    }
}
