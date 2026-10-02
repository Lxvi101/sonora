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

## Keychain troubleshooting

App signing, notarization and updater signing use three separate credentials: the Developer ID identity, an Apple `notarytool` profile, and Sparkle's Ed25519 key. Having the certificate does not guarantee that the other two are accessible.

A notarization profile can serve multiple apps on the same developer account/team. Keychain Access does not need to stay open; its **login** keychain needs to be unlocked and allow the signing tools access. If a previously working profile reports “No Keychain password item found” or Sparkle reports “Access denied”, lock and unlock the login keychain, then retry:

```sh
xcrun notarytool history --keychain-profile YOUR_PROFILE
target/sparkle/2.10.0/bin/generate_keys --account io.sonora.editor -p
```

The second command prints only the public key; compare it with `SUPublicEDKey`. Never regenerate the official key to solve an access problem. If the profile is truly missing, create it interactively with `xcrun notarytool store-credentials YOUR_PROFILE`. Enter credentials locally, never in chat, source, logs or command arguments. Sleep or a subsequent keychain lock can require unlocking again. This also affects Claude Code's ability to save a refreshed login.

Release command sequences must use `set -euo pipefail`: a signing failure must prevent subsequent commits, pushes and publication. Verify the feed before pushing its exact bytes. Use a full commit SHA for `gh release create --target`; GitHub rejected the abbreviated SHA during the first release.

## First release verification (0.3.0, 2026-10-02)

- Apple accepted the signed app; the ticket was stapled. Strict `codesign`, `stapler validate` and `spctl` checks passed.
- The ZIP downloaded from the draft matched the signed local archive byte for byte before publication. The public raw appcast matched the verified signed feed.
- The installed app correctly reported 0.3.0 as current. Daily checks were enabled and installation required user interaction.
- An isolated signed copy marked version 0.2.99 / build 2 downloaded build 3, installed it and relaunched. The resulting executable matched the release and its notarization ticket validated.
- The first test under Documents stalled in macOS `renamex_np`, after download verification. Repeating from `/private/tmp` succeeded. This identifies the stalled operation, not its underlying cause. Use an isolated local temporary directory for updater tests.
- All 41 tests, formatting, all-target Clippy and the GitHub macOS build passed.

For future updater tests, quit other copies and use an isolated signed older build with the same bundle ID, signing team and update key. Choose **Install Update**, then **Install and Relaunch**. Wait without using a UI tool that might reopen the old bundle while it is being replaced. Check the resulting build, executable, signature and ticket, then reopen the normal installed app. Never replace published assets with test copies.
