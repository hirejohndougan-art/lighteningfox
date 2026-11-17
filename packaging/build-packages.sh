#!/bin/sh
# Build installable packages of Ironswift for Linux into dist/:
#   dist/ironswift_<version>_amd64.deb     Debian, Ubuntu, Mint, Pop!_OS…
#   dist/Ironswift-<version>-x86_64.AppImage   any distribution: download, chmod +x, run
#   dist/ironswift-<version>-linux-x86_64.tar.gz  the program, icons and desktop entry
#
#   sh packaging/build-packages.sh            build all three (runs cargo build --release)
#   sh packaging/build-packages.sh deb        just the .deb (also: appimage, tar)
#
# The packages run on distributions with the same or a newer glibc than the build machine,
# so release builds are made on the oldest supported Ubuntu (see .github/workflows/release.yml).
set -e
cd "$(dirname "$0")/.."
WHAT="${1:-all}"
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
ARCH=$(uname -m)
DEB_ARCH=$(dpkg --print-architecture 2>/dev/null || echo amd64)
SIZES="16 24 32 48 64 128 256 512"
DIST="$PWD/dist"
mkdir -p "$DIST"

[ -n "$SKIP_BUILD" ] || cargo build --release
BIN=target/release/ironswift
[ -x "$BIN" ] || { echo "No $BIN: build failed?"; exit 1; }

# Lay out /usr/bin, the desktop entry and icons under $1. (sh functions share variables,
# so this one only uses _d.)
stage() {
  _d="$1"
  install -D -m 755 "$BIN" "$_d/usr/bin/ironswift"
  install -D -m 644 packaging/linux/ironswift.desktop "$_d/usr/share/applications/ironswift.desktop"
  for n in $SIZES; do
    install -D -m 644 "assets/icon/png/ironswift-$n.png" "$_d/usr/share/icons/hicolor/${n}x${n}/apps/ironswift.png"
  done
  install -D -m 644 LICENSE "$_d/usr/share/doc/ironswift/copyright"
  install -D -m 644 packaging/linux/ironswift.metainfo.xml "$_d/usr/share/metainfo/ironswift.metainfo.xml"
}

build_deb() {
  root=$(mktemp -d)
  stage "$root"
  strip "$root/usr/bin/ironswift" 2>/dev/null || true
  size=$(du -sk "$root/usr" | cut -f1)
  glibc=$(objdump -T "$BIN" 2>/dev/null | grep -o 'GLIBC_[0-9.]*' | sed 's/GLIBC_//' | sort -V | tail -1)
  mkdir -p "$root/DEBIAN"
  cat > "$root/DEBIAN/control" <<CONTROL
Package: ironswift
Version: $VERSION
Section: web
Priority: optional
Architecture: $DEB_ARCH
Installed-Size: $size
Depends: libc6 (>= ${glibc:-2.35}), libgcc-s1, libx11-6, libxcursor1, libxkbcommon0, fonts-dejavu-core
Recommends: libvulkan1 | libegl1, fonts-noto-core
Suggests: fonts-noto-cjk, fonts-freefont-ttf, cups-client
Provides: www-browser
Maintainer: Ironswift developers <noreply@github.com>
Homepage: https://github.com/hirejohndougan-art/lighteningfox
Description: light web browser built from scratch in Rust
 Ironswift is a web browser with its own HTML, CSS and layout engine written
 from scratch in Rust, the Boa JavaScript engine, built-in ad and tracker
 blocking, and no telemetry.
CONTROL
  # Register as a browser so it can be picked with update-alternatives.
  cat > "$root/DEBIAN/postinst" <<'POST'
#!/bin/sh
set -e
if [ "$1" = configure ]; then
  update-alternatives --install /usr/bin/x-www-browser x-www-browser /usr/bin/ironswift 40 || true
  update-alternatives --install /usr/bin/www-browser www-browser /usr/bin/ironswift 40 || true
fi
POST
  cat > "$root/DEBIAN/prerm" <<'PRE'
#!/bin/sh
set -e
if [ "$1" = remove ]; then
  update-alternatives --remove x-www-browser /usr/bin/ironswift || true
  update-alternatives --remove www-browser /usr/bin/ironswift || true
fi
PRE
  chmod 755 "$root/DEBIAN/postinst" "$root/DEBIAN/prerm"
  out="$DIST/ironswift_${VERSION}_${DEB_ARCH}.deb"
  dpkg-deb --root-owner-group -Zxz --build "$root" "$out" >/dev/null
  rm -rf "$root"
  echo "Built $out"
}

build_tar() {
  root=$(mktemp -d)
  dir="$root/ironswift-$VERSION"
  stage "$dir"
  strip "$dir/usr/bin/ironswift" 2>/dev/null || true
  # Install for one user without root: copies usr/ into ~/.local.
  cat > "$dir/install.sh" <<'INSTALL'
#!/bin/sh
# Install Ironswift for the current user (no root needed): sh install.sh   Remove: sh install.sh --remove
set -e
cd "$(dirname "$0")"
DEST="${XDG_DATA_HOME:-$HOME/.local/share}/.."
if [ "$1" = "--remove" ]; then
  find usr -type f | while read -r f; do rm -f "$DEST/${f#usr/}"; done
  echo "Ironswift removed. Your profile is still in ~/.local/share/ironswift."
  exit 0
fi
find usr -type f | while read -r f; do install -D -m "$( [ "${f#usr/bin/}" != "$f" ] && echo 755 || echo 644)" "$f" "$DEST/${f#usr/}"; done
command -v update-desktop-database >/dev/null && update-desktop-database "$DEST/share/applications" 2>/dev/null || true
echo "Ironswift installed: it's in your applications menu, or run ~/.local/bin/ironswift."
INSTALL
  chmod 755 "$dir/install.sh"
  out="$DIST/ironswift-$VERSION-linux-$ARCH.tar.gz"
  tar -C "$root" -czf "$out" "ironswift-$VERSION"
  rm -rf "$root"
  echo "Built $out"
}

build_appimage() {
  tool="${APPIMAGETOOL:-$(command -v appimagetool || true)}"
  if [ -z "$tool" ]; then
    tool="$PWD/target/appimagetool-$ARCH.AppImage"
    if [ ! -x "$tool" ]; then
      echo "Downloading appimagetool…"
      curl -fsSL -o "$tool" "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-$ARCH.AppImage" || {
        echo "Couldn't download appimagetool; set APPIMAGETOOL=/path/to/appimagetool"; return 1; }
      chmod +x "$tool"
    fi
  fi
  app=$(mktemp -d)/Ironswift.AppDir
  stage "$app"
  strip "$app/usr/bin/ironswift" 2>/dev/null || true
  # The default font goes inside, for systems without DejaVu.
  for f in DejaVuSans DejaVuSans-Bold DejaVuSans-Oblique DejaVuSans-BoldOblique DejaVuSansMono DejaVuSansMono-Bold; do
    src=$(find /usr/share/fonts -name "$f.ttf" 2>/dev/null | head -1)
    [ -n "$src" ] && install -D -m 644 "$src" "$app/usr/share/fonts/truetype/dejavu/$f.ttf"
  done
  install -m 755 packaging/appimage/AppRun "$app/AppRun"
  cp packaging/linux/ironswift.desktop "$app/ironswift.desktop"
  cp assets/icon/png/ironswift-256.png "$app/ironswift.png"
  ln -sf ironswift.png "$app/.DirIcon"
  out="$DIST/Ironswift-$VERSION-$ARCH.AppImage"
  # --appimage-extract-and-run: works where FUSE isn't available (containers, CI).
  ARCH=$ARCH "$tool" --appimage-extract-and-run --no-appstream "$app" "$out" >/dev/null 2>&1 || ARCH=$ARCH "$tool" --no-appstream "$app" "$out"
  rm -rf "$(dirname "$app")"
  echo "Built $out"
}

case "$WHAT" in
  deb) build_deb ;;
  tar) build_tar ;;
  appimage) build_appimage ;;
  all) build_deb; build_tar; build_appimage ;;
  *) echo "Unknown package type: $WHAT (deb, tar, appimage or all)"; exit 1 ;;
esac
