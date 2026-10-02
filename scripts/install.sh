#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [ ! -d dist/Sonora.app ]; then ./scripts/bundle.sh; fi
destination="$HOME/Applications/Sonora.app"
if [ -e "$destination" ]; then
  printf 'An app already exists at %s. Move it aside before installing this build.\n' "$destination" >&2
  exit 1
fi
mkdir -p "$HOME/Applications"
ditto dist/Sonora.app "$destination"
/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister -f "$destination"
printf 'Installed %s. It is now available in Finder → Open With.\n' "$destination"
open "$destination"
