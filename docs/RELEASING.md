# Releasing Sonora

Official binaries are Apple Silicon macOS apps. CI produces development artifacts only. Public releases are built locally so private signing keys never enter GitHub Actions or a pull request.

## One-time maintainer setup

You need Rust 1.98.1, Xcode Command Line Tools, a Developer ID Application identity, and a working `notarytool` Keychain profile. Use Apple's `notarytool store-credentials` interactively to save your credentials; never put credentials in source, command-line flags committed to scripts, or issue reports.

The official updater's public key and feed are in `assets/UpdateConfig.plist`. Its private Ed25519 key is stored in Keychain under the Sparkle account `io.sonora.editor`. `scripts/fetch-sparkle.sh` downloads Sparkle 2.10.0 from its official release and verifies a pinned SHA-256. To create keys for a **new fork**, use its `bin/generate_keys --account YOUR_ACCOUNT`, then configure your own public key/feed. Do not regenerate or replace an existing official key casually. Keep a secure offline backup using Sparkle's documented key-export procedure; never commit it.

## Prepare

1. Update `Cargo.toml`, `Cargo.lock`, `assets/Info.plist` (both display version and increasing numeric build), and `CHANGELOG.md`. Update dependency notices when dependencies change.
2. Run formatting, Clippy and tests as described in CONTRIBUTING.md. Test native file opening, an audio/video import, trim/gain and export.
3. Commit the code changes. Then prepare the release:

```sh
export SONORA_SIGN_IDENTITY='Developer ID Application: YOUR NAME (TEAMID)'
export SONORA_NOTARY_PROFILE='your-existing-profile'
./scripts/prepare-release.sh
```

The script enables Sparkle explicitly, signs each nested helper/library from inside out with the hardened runtime, notarizes with Apple, staples the ticket, creates the final ZIP, and generates and verifies a signed appcast. It checks the Keychain key against the public key before publishing anything. Archives are under `dist/releases/VERSION/`. Reusing an existing release archive is refused.

## Publish in this order

Never replace an existing release archive: its exact bytes are signed.

1. Push the reviewed code to `main`.
2. Create a **draft** GitHub release with tag `vVERSION`, targeting that code commit, and upload `Sonora-VERSION-arm64.zip` and `SHA256SUMS`. Describe supported architecture and notable changes.
3. Download the uploaded ZIP and compare its SHA-256 with the local archive. Publish the release only after verification.
4. Copy `dist/releases/VERSION/appcast.xml` to the repository root. Verify it with Sparkle's `sign_update --account io.sonora.editor --verify appcast.xml`. Commit and push the exact bytes **without editing or formatting the signed XML**.
5. Check the public raw feed and release-download URLs. In Sonora, choose **Check for Updates…** and verify the current version is recognized. Test a real update from an older updater-enabled app when available, including signature rejection for tampered artifacts in a private test setup.

The code tag may precede the appcast-only commit; it identifies the exact released source. GitHub provides the tagged source archives; LAME's corresponding source and licenses are also shipped inside the app.

## Development and forks

`./scripts/bundle.sh` is offline after `cargo fetch`, ad-hoc signs, and does not enable an upstream update feed. Official releases use `SONORA_ENABLE_UPDATER=1`. Forks must change their bundle identifier, public key, feed, repository links and release scripts before enabling updates. Never disable signature verification to fix an update failure.

This app is not sandboxed. Sparkle's helper services are preserved and separately signed; the build does not add broad exceptions to hardened-runtime library validation. To replace LAME or modify the app, rebuild and ad-hoc sign your local copy; source builds will not be overwritten by the official updater.
