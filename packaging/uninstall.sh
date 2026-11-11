#!/bin/sh
# Removes Ether installed by packaging/install.sh.
#
#   ./packaging/uninstall.sh           keeps your profiles and settings
#   ./packaging/uninstall.sh --purge   deletes them too
#
# A .deb install is removed with: sudo apt remove ether-ovpn
set -eu

bin="${XDG_BIN_HOME:-$HOME/.local/bin}"
data="${XDG_DATA_HOME:-$HOME/.local/share}"
config="${XDG_CONFIG_HOME:-$HOME/.config}"
cache="${XDG_CACHE_HOME:-$HOME/.cache}"

rm -f "$bin/ether-ovpn"
rm -f "$data/applications/ether-ovpn.desktop"
rm -f "$data/icons/hicolor/scalable/apps/ether-ovpn.svg"
rm -f "$data/icons/hicolor/256x256/apps/ether-ovpn.png"

gtk-update-icon-cache -q "$data/icons/hicolor" 2>/dev/null || true
update-desktop-database -q "$data/applications" 2>/dev/null || true

if [ "${1:-}" = "--purge" ]; then
    rm -rf "$config/ether" "$cache/ether"
    echo "Removed Ether, its profiles and its settings."
else
    echo "Removed Ether. Profiles and settings are kept in $config/ether."
    echo "Run again with --purge to delete them."
fi
