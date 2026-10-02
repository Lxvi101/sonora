#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
export MACOSX_DEPLOYMENT_TARGET=13.0
cargo build --release --locked --offline
app="$PWD/dist/Sonora.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/LAME" "$app/Contents/Frameworks"
cp target/release/sonora "$app/Contents/MacOS/Sonora"
cp assets/Info.plist "$app/Contents/Info.plist"
cp LICENSE "$app/Contents/Resources/License.txt"
cp assets/ThirdPartyNotices.txt "$app/Contents/Resources/ThirdPartyNotices.txt"
cp target/release/libmp3lame.0.dylib "$app/Contents/Frameworks/"
cp vendor/lame-4.0.tar.gz vendor/LAME-COPYING.txt vendor/LAME-LICENSE.txt vendor/README.md scripts/build-lame.sh "$app/Contents/Resources/LAME/"
if [ -f assets/Sonora.icns ]; then cp assets/Sonora.icns "$app/Contents/Resources/"; fi
printf 'APPL????' > "$app/Contents/PkgInfo"
codesign --force --deep --sign - "$app"
printf '\nBuilt %s\n' "$app"
