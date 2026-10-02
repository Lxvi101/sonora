# Bundled MP3 encoder

LAME 4.0 source: https://downloads.sourceforge.net/project/lame/lame/4.0/lame-4.0.tar.gz

SHA-256: `3df5124d5ad3a98312ffd7ba6a9b36230e4f8a3e66d3ce0f425e336c32d216eb`

The unmodified corresponding source archive and licenses are included here and in the app's Resources/LAME folder. `scripts/build-lame.sh OUTPUT_DIRECTORY` builds an encoder-only shared library with a macOS 13 deployment target. No mpg123, command-line LAME, or FFmpeg dependency is used. LAME is LGPL-2.0-or-later; see COPYING and LICENSE. Sonora dynamically links the replaceable `Contents/Frameworks/libmp3lame.0.dylib`. The unmodified source, this build script, and Sonora source are supplied for rebuilding and relinking, including modifications for personal use and debugging such modifications. Ad-hoc re-sign a modified app locally after replacing the library.
