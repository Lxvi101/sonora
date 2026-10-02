# Dependency and license notes

The root MIT license covers Sonora's original code. It does not relicense third-party dependencies. `Cargo.lock` pins Rust dependencies; `assets/ThirdPartyNotices.txt` carries their declared licenses and available license texts, supplemented by `vendor/licenses/` where upstream crates omitted license files. Supplement source URLs are recorded alongside those files. The app includes these notices.

- **GPUI 0.2.2:** Apache-2.0; source at https://github.com/zed-industries/zed and the exact release at https://crates.io/crates/gpui/0.2.2.
- **LAME 4.0:** LGPL-2.0-or-later; dynamically linked and replaceable. The unmodified source archive, COPYING, LICENSE and build instructions are included in `vendor/` and `Sonora.app/Contents/Resources/LAME`. Build and re-sign modified copies locally; no upstream update feed is enabled in development builds. See `vendor/README.md`.
- **Sparkle 2.10.0:** MIT and included component licenses; `vendor/SPARKLE-LICENSE.txt` preserves the complete upstream notices. The framework is fetched from its official GitHub release using a pinned SHA-256 and is only included when updates are explicitly enabled. Source: https://github.com/sparkle-project/Sparkle/tree/2.10.0.
- **MPL-2.0 crates:** `option-ext` 0.2.0 and build dependency `cbindgen` 0.28.0 are unmodified. Their source is available at https://crates.io/api/v1/crates/option-ext/0.2.0/download and https://crates.io/api/v1/crates/cbindgen/0.28.0/download, under their original MPL-2.0 terms. Their license notices are included in the bundle.
- **Apple frameworks:** AppKit, AVFoundation, Core Audio and Core Media are provided by macOS, not copied into the app.

To obtain the exact Rust dependency source for the lockfile, run `cargo vendor --locked` in a separate checkout. Preserve third-party notices and source obligations when redistributing modified builds. If adding or updating a dependency, update the notices and provenance as part of the same pull request. Fixture recordings are synthetic and have no third-party media content.
