# Security policy

Security fixes target the latest release. Older versions are not maintained separately.

Report vulnerabilities through [GitHub private vulnerability reporting](https://github.com/Lxvi101/sonora/security/advisories/new), rather than a public issue. Include the affected version, a synthetic reproducer if possible, and the impact. Do not include private recordings or credentials. This is a small volunteer project; response times are not guaranteed.

## Updates

Official releases use Sparkle 2 with HTTPS, an Ed25519-signed appcast, and Ed25519-signed archives validated before extraction. Release apps are Developer ID signed and notarized. Private update keys and Apple credentials stay in the maintainer's Keychain and are never committed or supplied to pull-request CI.

Automatic checks run approximately daily and can be disabled in the Sonora menu. Sparkle asks before installing; silent installation is not enabled by default. Source/development builds have updates disabled, so a fork cannot silently replace itself with an upstream release unless its maintainer explicitly configures that behavior.

## Data handling

Audio and video are processed locally. Sonora has no analytics or account system. Update checks contact GitHub's raw-content and release-download hosts using normal HTTPS requests; Sparkle system profiling is disabled. Audio, video, waveform contents and file paths are not sent to the update server. Video imports may use private temporary audio files and waveform summaries are cached locally; details are in the README.
