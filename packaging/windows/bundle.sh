#!/usr/bin/env bash
# Collects ratchet.exe with the GTK runtime it needs into target/dist, ready
# for Inno Setup (ratchet.iss). Run in the MSYS2 UCRT64 shell after
# `cargo build --release`.
#
# Layout mirrors the MSYS2 prefix, so GLib and GTK find their data relative
# to bin/: bin/ (exe + DLLs), lib/gdk-pixbuf-2.0, share/glib-2.0/schemas,
# share/icons, etc/fonts.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

PREFIX="${MINGW_PREFIX:-/ucrt64}"
DIST="$ROOT/target/dist"

rm -rf "$DIST"
mkdir -p "$DIST/bin" "$DIST/lib" "$DIST/share/glib-2.0/schemas" "$DIST/share/icons" "$DIST/etc"

cp target/release/ratchet.exe "$DIST/bin/"

# Image loaders (SVG icons go through librsvg's loader). MSYS2's cache lists
# them relative to the prefix, so it can be copied as is.
cp -r "$PREFIX/lib/gdk-pixbuf-2.0" "$DIST/lib/"
if grep -qi "msys64" "$DIST/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache"; then
    echo "loaders.cache has absolute MSYS2 paths:" >&2
    cat "$DIST/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache" >&2
    exit 1
fi

# Every MSYS2 DLL the exe and the loaders load, found by following ldd.
copy_deps() {
    ldd "$1" | awk '{print $3}' | grep -i "^$PREFIX/bin/" | while read -r dll; do
        name="$(basename "$dll")"
        if [ ! -f "$DIST/bin/$name" ]; then
            cp "$dll" "$DIST/bin/"
            copy_deps "$dll"
        fi
    done
}
copy_deps "$DIST/bin/ratchet.exe"
for loader in "$DIST"/lib/gdk-pixbuf-2.0/2.10.0/loaders/*.dll; do
    copy_deps "$loader"
done

# GSettings schemas (GTK's file chooser and settings read them).
cp "$PREFIX"/share/glib-2.0/schemas/*.xml "$DIST/share/glib-2.0/schemas/"
glib-compile-schemas "$DIST/share/glib-2.0/schemas"

# Icon theme used by the UI, plus hicolor as the fallback theme.
cp -r "$PREFIX/share/icons/Adwaita" "$PREFIX/share/icons/hicolor" "$DIST/share/icons/"
rm -f "$DIST"/share/icons/*/icon-theme.cache
gtk4-update-icon-cache -q -t -f "$DIST/share/icons/Adwaita"

# Fontconfig's configuration, used by Pango's font lookup.
cp -r "$PREFIX/etc/fonts" "$DIST/etc/"

cp LICENSE "$DIST/LICENSE.txt"

echo "Bundled into $DIST ($(du -sh "$DIST" | cut -f1))"
