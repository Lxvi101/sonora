#!/bin/bash
# Rebuild the pinned MP3 encoder locally; no system installation or network.
set -euo pipefail
project="$(cd "$(dirname "$0")/.." && pwd)"
output="$1"
if [ -f "$output/install/lib/libmp3lame.0.dylib" ]; then exit 0; fi
mkdir -p "$output"
archive="$project/vendor/lame-4.0.tar.gz"
expected=3df5124d5ad3a98312ffd7ba6a9b36230e4f8a3e66d3ce0f425e336c32d216eb
actual="$(shasum -a 256 "$archive" | cut -d ' ' -f 1)"
[ "$actual" = "$expected" ] || { echo 'LAME source checksum mismatch' >&2; exit 1; }
tar -xzf "$archive" -C "$output"
cd "$output/lame-4.0"
export MACOSX_DEPLOYMENT_TARGET=13.0
export CFLAGS='-O3 -mmacosx-version-min=13.0 -Wno-implicit-function-declaration'
export LDFLAGS='-mmacosx-version-min=13.0'
export ac_cv_prog_cc_c23=no
# Configure insists on pkg-config even when all external-codec features are
# disabled. This adapter explicitly reports that no optional packages exist.
cat > "$output/no-optional-packages" <<'PROBE'
#!/bin/sh
case "$1" in --atleast-pkgconfig-version) exit 0 ;; *) exit 1 ;; esac
PROBE
chmod +x "$output/no-optional-packages"
export PKG_CONFIG="$output/no-optional-packages"
./configure --prefix="$output/install" --disable-decoder --disable-frontend --enable-shared --disable-static --disable-dependency-tracking --disable-debug
make -j4
make install
install_name_tool -id @rpath/libmp3lame.0.dylib "$output/install/lib/libmp3lame.0.dylib"
codesign --force --sign - "$output/install/lib/libmp3lame.0.dylib"
