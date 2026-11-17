#!/bin/sh
# Install Swiftiron for the current user: the program, its icon and an applications-menu entry.
#   sh packaging/linux/install.sh            install (run `cargo build --release` first)
#   sh packaging/linux/install.sh --remove   uninstall (your profile, bookmarks and history are kept)
set -e
cd "$(dirname "$0")/../.."
BIN="$HOME/.local/bin"
APPS="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
ICONS="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor"
SIZES="16 24 32 48 64 128 256 512"

if [ "$1" = "--remove" ]; then
  rm -f "$BIN/swiftiron" "$APPS/swiftiron.desktop"
  for n in $SIZES; do rm -f "$ICONS/${n}x${n}/apps/swiftiron.png"; done
  echo "Swiftiron removed. Your profile is still in ~/.local/share/swiftiron."
else
  [ -x target/release/swiftiron ] || { echo "Build it first: cargo build --release"; exit 1; }
  mkdir -p "$BIN" "$APPS"
  install -m 755 target/release/swiftiron "$BIN/swiftiron"
  for n in $SIZES; do
    mkdir -p "$ICONS/${n}x${n}/apps"
    install -m 644 "assets/icon/png/swiftiron-$n.png" "$ICONS/${n}x${n}/apps/swiftiron.png"
  done
  install -m 644 packaging/linux/swiftiron.desktop "$APPS/swiftiron.desktop"
  echo "Swiftiron installed: it's in your applications menu, or run 'swiftiron'."
  case ":$PATH:" in *":$BIN:"*) ;; *) echo "Note: add $BIN to your PATH to run 'swiftiron' from a terminal." ;; esac
fi
# Refresh the menus and icon caches where the tools exist.
command -v update-desktop-database >/dev/null && update-desktop-database "$APPS" 2>/dev/null || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$ICONS" 2>/dev/null || true
