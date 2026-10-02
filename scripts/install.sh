#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [ ! -d dist/Sonora.app ]; then ./scripts/bundle.sh; fi
destination="$HOME/Applications/Sonora.app"
if [ -e "$destination" ]; then
  if [ "${1:-}" != '--replace' ]; then
    printf 'An app already exists at %s. Close it and use --replace to update with a backup.\n' "$destination" >&2
    exit 1
  fi
  identity="$(/usr/libexec/PlistBuddy -c 'Print CFBundleIdentifier' "$destination/Contents/Info.plist")"
  [ "$identity" = 'io.sonora.editor' ] || { echo 'Refusing to replace an unrelated app.' >&2; exit 1; }
fi
mkdir -p "$HOME/Applications"
staging="$(mktemp -d "$HOME/Applications/.sonora-install.XXXXXX")"
trap 'rmdir "$staging" 2>/dev/null || true' EXIT
ditto dist/Sonora.app "$staging/Sonora.app"
codesign --verify --deep --strict "$staging/Sonora.app"
if [ -e "$destination" ]; then
  mkdir -p target/install-backup
  backup="$PWD/target/install-backup/Sonora-$(date +%Y%m%d-%H%M%S).app"
  [ ! -e "$backup" ] || { echo 'Backup path already exists.' >&2; exit 1; }
  mv "$destination" "$backup"
  if ! mv "$staging/Sonora.app" "$destination"; then
    mv "$backup" "$destination"
    exit 1
  fi
else
  mv "$staging/Sonora.app" "$destination"
fi
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$destination"
printf 'Installed %s. It is now available in Finder → Open With.\n' "$destination"
open "$destination"
