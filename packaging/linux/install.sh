#!/bin/sh
# Install Ironswift for the current user: the program, its icon and an applications-menu entry.
#   sh packaging/linux/install.sh            install (run `cargo build --release` first)
#   sh packaging/linux/install.sh --remove   uninstall (your profile, bookmarks and history are kept)
set -e
cd "$(dirname "$0")/../.."
BIN="$HOME/.local/bin"
APPS="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICONS="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor"
SIZES="16 24 32 48 64 128 256 512"

if [ "$1" = "--remove" ]; then
  rm -f "$BIN/ironswift" "$APPS/ironswift.desktop"
  for n in $SIZES; do rm -f "$ICONS/${n}x${n}/apps/ironswift.png"; done
  echo "Ironswift removed. Your profile is still in ~/.local/share/ironswift."
else
  [ -x target/release/ironswift ] || { echo "Build it first: cargo build --release"; exit 1; }
  mkdir -p "$BIN" "$APPS"
  install -m 755 target/release/ironswift "$BIN/ironswift"
  for n in $SIZES; do
    mkdir -p "$ICONS/${n}x${n}/apps"
    install -m 644 "assets/icon/png/ironswift-$n.png" "$ICONS/${n}x${n}/apps/ironswift.png"
  done
  install -m 644 packaging/linux/ironswift.desktop "$APPS/ironswift.desktop"
  echo "Ironswift installed: it's in your applications menu, or run 'ironswift'."
  case ":$PATH:" in *":$BIN:"*) ;; *) echo "Note: add $BIN to your PATH to run 'ironswift' from a terminal." ;; esac
fi
# Refresh the menus and icon caches where the tools exist.
command -v update-desktop-database >/dev/null && update-desktop-database "$APPS" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$ICONS" 2>/dev/null || true
