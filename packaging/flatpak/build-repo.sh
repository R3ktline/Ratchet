#!/usr/bin/env bash
# Builds the signed Flatpak repository that GitHub Pages serves (target/site)
# and Ratchet-<version>-x86_64.flatpak, a one-file installer that keeps
# updating from that repository.
#
# Needs flatpak, flatpak-builder (or FLATPAK_BUILDER="flatpak run
# org.flatpak.Builder") and the signing key in gpg's keyring.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

VERSION="$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)"
SITE_URL="https://r3ktline.github.io/Ratchet"
FLATHUB="https://dl.flathub.org/repo/flathub.flatpakrepo"
SITE="target/site"
KEY="$(gpg --list-secret-keys --with-colons | awk -F: '/^fpr/ { print $10; exit }')"
test -n "$KEY" || { echo "No signing key in gpg's keyring" >&2; exit 1; }

rm -rf "$SITE"
mkdir -p "$SITE"
${FLATPAK_BUILDER:-flatpak-builder} --repo="$SITE/repo" --default-branch=stable --gpg-sign="$KEY" \
    --install-deps-from=flathub --disable-rofiles-fuse --force-clean \
    --state-dir=target/flatpak-state target/flatpak packaging/flatpak/app.ratchet.notes.yml
flatpak build-update-repo --gpg-sign="$KEY" --generate-static-deltas --prune \
    --title=Ratchet "$SITE/repo"

# The public key goes into the files users install from, so flatpak can
# check every update.
gpg --export "$KEY" > target/ratchet.gpg
GPG_KEY="$(base64 -w0 target/ratchet.gpg)"
cat > "$SITE/app.ratchet.notes.flatpakref" <<EOF
[Flatpak Ref]
Name=app.ratchet.notes
Branch=stable
Title=Ratchet
Url=$SITE_URL/repo/
SuggestRemoteName=ratchet
Homepage=https://github.com/R3ktline/Ratchet
RuntimeRepo=$FLATHUB
IsRuntime=false
GPGKey=$GPG_KEY
EOF
cat > "$SITE/ratchet.flatpakrepo" <<EOF
[Flatpak Repo]
Title=Ratchet
Url=$SITE_URL/repo/
Homepage=https://github.com/R3ktline/Ratchet
GPGKey=$GPG_KEY
EOF

flatpak build-bundle --gpg-sign="$KEY" --gpg-keys=target/ratchet.gpg \
    --repo-url="$SITE_URL/repo/" --runtime-repo="$FLATHUB" \
    "$SITE/repo" "Ratchet-$VERSION-x86_64.flatpak" app.ratchet.notes stable

echo "Built $SITE and Ratchet-$VERSION-x86_64.flatpak"
