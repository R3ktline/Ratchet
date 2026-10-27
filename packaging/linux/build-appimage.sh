#!/usr/bin/env bash
# Builds Ratchet-<version>-x86_64.AppImage in the repository root.
#
# Run on Fedora 43 (CI uses the fedora:43 container) with:
#   dnf install rust cargo cargo-c gcc meson ninja-build gtk4-devel \
#     libadwaita-devel libpng-devel libjpeg-turbo-devel libxml2-devel \
#     adwaita-icon-theme hicolor-icon-theme desktop-file-utils file wget \
#     findutils xz gettext
# The distro's GTK, libadwaita and their dependencies go into the AppImage;
# glibc, graphics drivers and fontconfig come from the user's system.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
APPDIR="$ROOT/target/AppDir"
TOOLS="$ROOT/target/appimage-tools"

# Fedora's gdk-pixbuf loads images through glycin, whose sandbox can't see
# files inside an AppImage, so GTK couldn't load its SVG icons. Bundle a
# gdk-pixbuf built without glycin plus librsvg's classic SVG loader.
GDK_PIXBUF_VERSION=2.44.4
LIBRSVG_VERSION=2.61.4
PIXBUF="$TOOLS/pixbuf"
if [ ! -f "$PIXBUF/.done-$GDK_PIXBUF_VERSION-$LIBRSVG_VERSION" ]; then
    rm -rf "$PIXBUF" "$TOOLS/src"
    mkdir -p "$TOOLS/src"
    wget -q -O - "https://download.gnome.org/sources/gdk-pixbuf/${GDK_PIXBUF_VERSION%.*}/gdk-pixbuf-$GDK_PIXBUF_VERSION.tar.xz" \
        | tar -C "$TOOLS/src" -xJ
    wget -q -O - "https://download.gnome.org/sources/librsvg/${LIBRSVG_VERSION%.*}/librsvg-$LIBRSVG_VERSION.tar.xz" \
        | tar -C "$TOOLS/src" -xJ

    meson setup "$TOOLS/src/gdk-pixbuf-build" "$TOOLS/src/gdk-pixbuf-$GDK_PIXBUF_VERSION" \
        --prefix="$PIXBUF" --libdir=lib --buildtype=release \
        -Dglycin=disabled -Dpng=enabled -Djpeg=enabled -Dtiff=disabled -Dgif=disabled \
        -Dothers=disabled -Dintrospection=disabled -Dgtk_doc=false -Ddocumentation=false \
        -Dman=false -Dtests=false -Dinstalled_tests=false -Dthumbnailer=disabled
    meson install -C "$TOOLS/src/gdk-pixbuf-build"

    PKG_CONFIG_PATH="$PIXBUF/lib/pkgconfig" meson setup "$TOOLS/src/librsvg-build" \
        "$TOOLS/src/librsvg-$LIBRSVG_VERSION" \
        --prefix="$PIXBUF" --libdir=lib --buildtype=release \
        -Dpixbuf=enabled -Dpixbuf-loader=enabled -Dintrospection=disabled -Dvala=disabled \
        -Ddocs=disabled -Dtests=false -Drsvg-convert=disabled -Davif=disabled
    meson install -C "$TOOLS/src/librsvg-build"

    touch "$PIXBUF/.done-$GDK_PIXBUF_VERSION-$LIBRSVG_VERSION"
fi

cargo build --release --locked

rm -rf "$APPDIR"
install -Dm755 target/release/ratchet "$APPDIR/usr/bin/ratchet"

# Translations, compiled by build.rs next to target/release; the app looks
# for them in ../share/locale from its executable.
for lang in $(cat po/LINGUAS); do
    install -Dm644 "target/share/locale/$lang/LC_MESSAGES/ratchet.mo" \
        "$APPDIR/usr/share/locale/$lang/LC_MESSAGES/ratchet.mo"
done

# Image loaders. gdk-pixbuf needs absolute paths in its loader cache, so the
# cache is a template that AppRun fills in with the AppImage's mount point.
LOADERS="$APPDIR/usr/lib/gdk-pixbuf-2.0/2.10.0/loaders"
mkdir -p "$LOADERS"
cp "$PIXBUF"/lib/gdk-pixbuf-2.0/2.10.0/loaders/*.so "$LOADERS/"
LD_LIBRARY_PATH="$PIXBUF/lib" GDK_PIXBUF_MODULEDIR="$LOADERS" "$PIXBUF/bin/gdk-pixbuf-query-loaders" \
    | sed "s|$APPDIR|@APPDIR@|" > "$APPDIR/usr/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache.in"
grep -q "@APPDIR@" "$APPDIR/usr/lib/gdk-pixbuf-2.0/2.10.0/loaders.cache.in"

# GSettings schemas (GTK's file chooser and settings read them).
mkdir -p "$APPDIR/usr/share/glib-2.0/schemas"
cp /usr/share/glib-2.0/schemas/*.xml "$APPDIR/usr/share/glib-2.0/schemas/"
glib-compile-schemas "$APPDIR/usr/share/glib-2.0/schemas"

# Icon theme used by the UI, plus hicolor as the fallback theme.
mkdir -p "$APPDIR/usr/share/icons"
cp -r /usr/share/icons/Adwaita "$APPDIR/usr/share/icons/"
cp -r /usr/share/icons/hicolor "$APPDIR/usr/share/icons/"
rm -f "$APPDIR"/usr/share/icons/*/icon-theme.cache
gtk4-update-icon-cache -q -t -f "$APPDIR/usr/share/icons/Adwaita"

# linuxdeploy copies the libraries the binary and loaders need and packs
# the AppImage. Our gdk-pixbuf comes first on the library path so it is
# bundled instead of Fedora's.
mkdir -p "$TOOLS"
LINUXDEPLOY_VERSION=1-alpha-20251107-1
LINUXDEPLOY_SHA256=c20cd71e3a4e3b80c3483cef793cda3f4e990aca14014d23c544ca3ce1270b4d
LINUXDEPLOY="$TOOLS/linuxdeploy-$LINUXDEPLOY_VERSION-x86_64.AppImage"
if [ ! -x "$LINUXDEPLOY" ]; then
    wget -q -O "$LINUXDEPLOY.part" \
        "https://github.com/linuxdeploy/linuxdeploy/releases/download/$LINUXDEPLOY_VERSION/linuxdeploy-x86_64.AppImage"
    echo "$LINUXDEPLOY_SHA256  $LINUXDEPLOY.part" | sha256sum -c -
    chmod +x "$LINUXDEPLOY.part"
    mv "$LINUXDEPLOY.part" "$LINUXDEPLOY"
fi

export APPIMAGE_EXTRACT_AND_RUN=1
# linuxdeploy's bundled strip can't read Fedora 43's .relr.dyn sections;
# Fedora's libraries come stripped anyway.
export NO_STRIP=1
export OUTPUT="Ratchet-$VERSION-x86_64.AppImage"
LD_LIBRARY_PATH="$PIXBUF/lib" "$LINUXDEPLOY" \
    --appdir "$APPDIR" \
    --executable "$APPDIR/usr/bin/ratchet" \
    --deploy-deps-only "$LOADERS" \
    --desktop-file data/app.ratchet.notes.desktop \
    --icon-file data/icons/scalable/apps/app.ratchet.notes.svg \
    --custom-apprun packaging/linux/AppRun \
    --output appimage

# The bundled gdk-pixbuf must be ours, never glycin's.
if [ -e "$APPDIR/usr/lib/libglycin-2.so.0" ]; then
    echo "glycin got bundled; the AppImage would show broken icons" >&2
    exit 1
fi

echo "Built $ROOT/$OUTPUT"
