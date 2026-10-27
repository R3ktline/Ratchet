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
# Its header comment names the MSYS2 folder; only the entries matter.
if grep -v '^#' "$DIST/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache" | grep -qi "msys64"; then
    echo "loaders.cache has absolute MSYS2 paths:" >&2
    cat "$DIST/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache" >&2
    exit 1
fi

# Every MSYS2 DLL the exe and the loaders load, found by following ldd.
copy_deps() {
    # awk, not grep: grep fails on DLLs with no MSYS2 deps, ending the script.
    ldd "$1" | awk -v p="$PREFIX/bin/" 'index(tolower($3), tolower(p)) == 1 { print $3 }' | while read -r dll; do
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

# Translations: ours (compiled by build.rs, which needs gettext's msgfmt),
# plus GTK's, libadwaita's and GLib's for their own dialogs and buttons.
for lang in $(cat po/LINGUAS); do
    if [ ! -f "target/share/locale/$lang/LC_MESSAGES/ratchet.mo" ]; then
        echo "No $lang translation in target/share/locale; is msgfmt installed?" >&2
        exit 1
    fi
    mkdir -p "$DIST/share/locale/$lang/LC_MESSAGES"
    cp "target/share/locale/$lang/LC_MESSAGES/ratchet.mo" "$DIST/share/locale/$lang/LC_MESSAGES/"
    for domain in gtk40 libadwaita glib20; do
        mo="$PREFIX/share/locale/$lang/LC_MESSAGES/$domain.mo"
        [ -f "$mo" ] && cp "$mo" "$DIST/share/locale/$lang/LC_MESSAGES/"
    done
done

cp LICENSE "$DIST/LICENSE.txt"

echo "Bundled into $DIST ($(du -sh "$DIST" | cut -f1))"
