#!/bin/bash
set -euo pipefail
app="${1:?Usage: sign-app.sh /path/Sonora.app}"
identity="${SONORA_SIGN_IDENTITY:--}"
args=(--force --sign "$identity")
if [ "$identity" != '-' ]; then args+=(--options runtime --timestamp); fi
framework="$app/Contents/Frameworks/Sparkle.framework"
if [ -d "$framework" ]; then
  codesign "${args[@]}" "$framework/Versions/B/XPCServices/Installer.xpc"
  codesign "${args[@]}" --preserve-metadata=entitlements "$framework/Versions/B/XPCServices/Downloader.xpc"
  codesign "${args[@]}" "$framework/Versions/B/Autoupdate"
  codesign "${args[@]}" "$framework/Versions/B/Updater.app"
  codesign "${args[@]}" "$framework"
fi
codesign "${args[@]}" "$app/Contents/Frameworks/libmp3lame.0.dylib"
codesign "${args[@]}" "$app"
codesign --verify --deep --strict "$app"
