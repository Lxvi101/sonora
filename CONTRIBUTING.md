# Contributing

Sonora is a small macOS utility for quick trims and level changes. Bug fixes, accessibility work, format compatibility, and measured performance improvements are welcome. Discuss large changes in an issue before implementing them.

## Development

Use macOS 13 or newer, Xcode Command Line Tools, and Rust 1.98.1. The published app currently targets Apple Silicon. Linux and Windows are not supported.

```sh
git clone https://github.com/Lxvi101/sonora.git
cd sonora
cargo fetch --locked
cargo run --release -- /path/to/audio.wav
```

The LAME encoder builds from the checksum-pinned source included under `vendor/`. Plain `cargo run` needs no signing identity or Sparkle download. `scripts/bundle.sh` creates an ad-hoc-signed development app with updates disabled. See [release instructions](docs/RELEASING.md) for official builds; forks should use their own bundle identifier, feed and signing key.

Before submitting:

```sh
cargo fmt --check
cargo clippy --locked --offline --all-targets -- -D warnings
cargo test --locked --offline
python3 scripts/check-release.py
```

Keep native audio work off the UI thread. Preserve source files, cancellation, bounded memory, and temporary-file cleanup. Tests use synthetic recordings; do not commit personal audio, credentials, signing keys, build products or large recordings. Document meaningful runtime verification, especially for native dialogs and drag-and-drop that automated tests cannot fully cover.

## Pull requests

Explain the concrete problem and resulting behavior. Add regression coverage where it can catch real failures. Keep unrelated refactors separate. Changes are reviewed by the maintainer; there is no CLA. Contributions are accepted under the project's MIT license, with third-party code retaining its original license and attribution. If you used generated code, review it and take responsibility for its correctness and provenance.

## Community

Be considerate, discuss the code rather than the person, and avoid harassment. Do not disclose someone else's private information. Maintainers may remove abusive content or restrict participation. Security issues should use GitHub's [private reporting](https://github.com/Lxvi101/sonora/security/advisories/new).
