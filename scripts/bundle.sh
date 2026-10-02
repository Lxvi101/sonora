#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
export MACOSX_DEPLOYMENT_TARGET=13.0
cargo build --release --locked --offline
app="$PWD/dist/Sonora.app"
# Rebuild generated bundle contents so disabled updater builds cannot inherit a feed.
if [ -d "$app" ]; then rm -rf "$app"; fi
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/LAME" "$app/Contents/Frameworks"
cp target/release/sonora "$app/Contents/MacOS/Sonora"
cp assets/Info.plist "$app/Contents/Info.plist"
cp LICENSE "$app/Contents/Resources/License.txt"
cp assets/ThirdPartyNotices.txt "$app/Contents/Resources/ThirdPartyNotices.txt"
python3 scripts/assemble-notices.py "$app/Contents/Resources/SupplementalLicenses.txt"
cp vendor/licenses/sources.json "$app/Contents/Resources/LicenseSources.json"
cp target/release/libmp3lame.0.dylib "$app/Contents/Frameworks/"
cp vendor/lame-4.0.tar.gz vendor/LAME-COPYING.txt vendor/LAME-LICENSE.txt vendor/README.md scripts/build-lame.sh "$app/Contents/Resources/LAME/"
if [ "${SONORA_ENABLE_UPDATER:-0}" = 1 ]; then
  sparkle="$(./scripts/fetch-sparkle.sh)"
  ditto "$sparkle/Sparkle.framework" "$app/Contents/Frameworks/Sparkle.framework"
  cp vendor/SPARKLE-LICENSE.txt "$app/Contents/Resources/Sparkle-License.txt"
  python3 - "$app/Contents/Info.plist" assets/UpdateConfig.plist <<'PY'
import plistlib, sys
with open(sys.argv[1], 'rb') as f: info = plistlib.load(f)
with open(sys.argv[2], 'rb') as f: info.update(plistlib.load(f))
with open(sys.argv[1], 'wb') as f: plistlib.dump(info, f)
PY
fi
if [ -f assets/Sonora.icns ]; then cp assets/Sonora.icns "$app/Contents/Resources/"; fi
printf 'APPL????' > "$app/Contents/PkgInfo"
./scripts/sign-app.sh "$app"
printf '\nBuilt %s\n' "$app"
