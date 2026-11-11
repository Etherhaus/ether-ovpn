#!/bin/sh
# Installs Ether for the current user: the binary, the app icon and an
# app-menu entry. Run it from an unpacked release, or from the project folder
# after `cargo build --release`.
set -eu
cd "$(dirname "$0")/.."

bin="${XDG_BIN_HOME:-$HOME/.local/bin}"
data="${XDG_DATA_HOME:-$HOME/.local/share}"

# From a source checkout, or from an unpacked release archive.
if [ -f target/release/ether-ovpn ]; then src=target/release/ether-ovpn; else src=ether-ovpn; fi
install -Dm755 "$src" "$bin/ether-ovpn"
install -Dm644 assets/ether.svg "$data/icons/hicolor/scalable/apps/ether-ovpn.svg"
install -Dm644 assets/ether-256.png "$data/icons/hicolor/256x256/apps/ether-ovpn.png"
# Point the launcher at the installed binary so it works even when
# ~/.local/bin isn't on the PATH of the desktop session.
mkdir -p "$data/applications"
sed "s|^Exec=ether-ovpn|Exec=$bin/ether-ovpn|" packaging/ether-ovpn.desktop > "$data/applications/ether-ovpn.desktop"
chmod 644 "$data/applications/ether-ovpn.desktop"

gtk-update-icon-cache -q "$data/icons/hicolor" 2>/dev/null || true
update-desktop-database -q "$data/applications" 2>/dev/null || true

echo "Installed Ether to $bin/ether-ovpn"
command -v openvpn >/dev/null || echo "Note: OpenVPN isn't installed yet: sudo apt install openvpn"
case ":$PATH:" in
  *":$bin:"*) ;;
  *) echo "Note: $bin is not on your PATH; add it to run 'ether-ovpn' from a terminal." ;;
esac
