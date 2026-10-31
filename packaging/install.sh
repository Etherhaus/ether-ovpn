#!/bin/sh
# Installs VPNdeck for the current user: the binary, the app icon and an
# app-menu entry. Run it from an unpacked release, or from the project folder
# after `cargo build --release`.
set -eu
cd "$(dirname "$0")/.."

bin="${XDG_BIN_HOME:-$HOME/.local/bin}"
data="${XDG_DATA_HOME:-$HOME/.local/share}"

# From a source checkout, or from an unpacked release archive.
if [ -f target/release/vpndeck ]; then src=target/release/vpndeck; else src=vpndeck; fi
install -Dm755 "$src" "$bin/vpndeck"
install -Dm644 assets/vpndeck.svg "$data/icons/hicolor/scalable/apps/vpndeck.svg"
install -Dm644 assets/vpndeck-256.png "$data/icons/hicolor/256x256/apps/vpndeck.png"
# Point the launcher at the installed binary so it works even when
# ~/.local/bin isn't on the PATH of the desktop session.
mkdir -p "$data/applications"
sed "s|^Exec=vpndeck|Exec=$bin/vpndeck|" packaging/vpndeck.desktop > "$data/applications/vpndeck.desktop"
chmod 644 "$data/applications/vpndeck.desktop"

gtk-update-icon-cache -q "$data/icons/hicolor" 2>/dev/null || true
update-desktop-database -q "$data/applications" 2>/dev/null || true

echo "Installed VPNdeck to $bin/vpndeck"
command -v openvpn >/dev/null || echo "Note: OpenVPN isn't installed yet: sudo apt install openvpn"
case ":$PATH:" in
  *":$bin:"*) ;;
  *) echo "Note: $bin is not on your PATH; add it to run 'vpndeck' from a terminal." ;;
esac
