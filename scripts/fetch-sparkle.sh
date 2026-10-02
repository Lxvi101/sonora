#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
version=2.10.0
checksum=c2bf58aa8387266ac179357b1415d6f2635f044da8be41042af32425dae6da0c
root="$PWD/target/sparkle/$version"
archive="$PWD/target/sparkle/Sparkle-$version.tar.xz"
mkdir -p "$(dirname "$archive")"
if [ ! -f "$archive" ]; then
  curl --fail --location --proto '=https' --tlsv1.2 --retry 3 \
    "https://github.com/sparkle-project/Sparkle/releases/download/$version/Sparkle-$version.tar.xz" -o "$archive.part"
  mv "$archive.part" "$archive"
fi
printf '%s  %s\n' "$checksum" "$archive" | shasum -a 256 --check >&2
if [ ! -d "$root/Sparkle.framework" ]; then
  mkdir -p "$root"
  tar -xJf "$archive" -C "$root"
fi
printf '%s\n' "$root"
