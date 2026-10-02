#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
export MACOSX_DEPLOYMENT_TARGET=13.0
cargo build --release --locked --offline
app="$PWD/dist/Sonora.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/sonora "$app/Contents/MacOS/Sonora"
cp assets/Info.plist "$app/Contents/Info.plist"
cp LICENSE "$app/Contents/Resources/License.txt"
cp assets/ThirdPartyNotices.txt "$app/Contents/Resources/ThirdPartyNotices.txt"
if [ -f assets/Sonora.icns ]; then cp assets/Sonora.icns "$app/Contents/Resources/"; fi
printf 'APPL????' > "$app/Contents/PkgInfo"
codesign --force --deep --sign - "$app"
printf '\nBuilt %s\n' "$app"
