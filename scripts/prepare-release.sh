#!/bin/bash
# Build, notarize and sign a release locally; no secrets leave Keychain.
set -euo pipefail
cd "$(dirname "$0")/.."
: "${SONORA_SIGN_IDENTITY:?Set your Developer ID Application identity}"
: "${SONORA_NOTARY_PROFILE:?Set an existing notarytool Keychain profile}"
[ "$SONORA_SIGN_IDENTITY" != '-' ] || { echo 'Release signing requires Developer ID.' >&2; exit 1; }
python3 scripts/check-release.py
version="$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' assets/Info.plist)"
arch="$(uname -m)"
[ "$arch" = arm64 ] || { echo 'Official releases currently target Apple Silicon only.' >&2; exit 1; }
release="$PWD/dist/releases/$version"
archive="Sonora-$version-arm64.zip"
[ ! -e "$release/$archive" ] || { echo 'A release archive already exists; use a new version or preserve it elsewhere first.' >&2; exit 1; }
export SONORA_ENABLE_UPDATER=1
./scripts/bundle.sh
sparkle="$(./scripts/fetch-sparkle.sh)"
account="${SONORA_SPARKLE_ACCOUNT:-io.sonora.editor}"
expected="$(/usr/libexec/PlistBuddy -c 'Print SUPublicEDKey' assets/UpdateConfig.plist)"
actual="$("$sparkle/bin/generate_keys" --account "$account" -p)"
[ "$actual" = "$expected" ] || { echo 'Keychain signing key does not match the public update key.' >&2; exit 1; }
mkdir -p "$release"
notary_archive="$release/notarization.zip"
ditto -c -k --sequesterRsrc --keepParent dist/Sonora.app "$notary_archive"
xcrun notarytool submit "$notary_archive" --keychain-profile "$SONORA_NOTARY_PROFILE" --wait --output-format json > "$release/notarization.json"
python3 - "$release/notarization.json" <<'PY'
import json,sys
r=json.load(open(sys.argv[1]));assert r.get('status')=='Accepted',r
print('Notarization accepted:',r['id'])
PY
xcrun stapler staple dist/Sonora.app
xcrun stapler validate dist/Sonora.app
codesign --verify --deep --strict dist/Sonora.app
spctl --assess --type execute --verbose=2 dist/Sonora.app
ditto -c -k --sequesterRsrc --keepParent dist/Sonora.app "$release/$archive"
# Only the actual update archive belongs in generate_appcast's input directory.
rm "$notary_archive"
if [ -f appcast.xml ]; then cp appcast.xml "$release/appcast.xml"; fi
"$sparkle/bin/generate_appcast" --account "$account" --maximum-deltas 0 \
  --download-url-prefix "https://github.com/Lxvi101/sonora/releases/download/v$version/" \
  --link 'https://github.com/Lxvi101/sonora' "$release"
"$sparkle/bin/sign_update" --account "$account" --verify "$release/appcast.xml"
(cd "$release" && shasum -a 256 "$archive" > SHA256SUMS)
printf '\nRelease prepared in %s\nPublish the archive before updating the public appcast. See docs/RELEASING.md.\n' "$release"
