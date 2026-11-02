#!/bin/sh
# Builds the release packages into dist/:
#   vpndeck-<version>-x86_64-linux.tar.gz   binary + install script
#   vpndeck_<version>_amd64.deb             Debian/Ubuntu/Mint package
#   vpndeck-<version>-1.x86_64.rpm          Fedora/openSUSE package
#   vpndeck-<version>-x86_64.flatpak        Flatpak bundle (Freedesktop runtime)
#   SHA256SUMS
# Needs dpkg-deb, rpmbuild, flatpak and the org.freedesktop.Platform//25.08 runtime:
#   flatpak install --user flathub org.freedesktop.Platform//25.08
# Attach them all to a GitHub release tagged v<version>; the in-app updater
# downloads from the latest release.
set -eu
cd "$(dirname "$0")/.."

app_id=io.github.thuutien.VPNdeck
runtime_version=25.08
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)
maintainer="Bryan Tran <17092189+thuutien@users.noreply.github.com>"

cargo build --release
rm -rf dist
mkdir -p dist
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# --- tarball -----------------------------------------------------------------
name="vpndeck-$version-x86_64-linux"
mkdir -p "$work/$name/packaging"
cp target/release/vpndeck README.md "$work/$name/"
cp -r assets "$work/$name/"
cp packaging/install.sh packaging/vpndeck.desktop "$work/$name/packaging/"
tar -C "$work" -czf "dist/$name.tar.gz" "$name"

# --- .deb --------------------------------------------------------------------
deb="$work/deb"
install -Dm755 target/release/vpndeck "$deb/usr/bin/vpndeck"
install -Dm644 packaging/vpndeck.desktop "$deb/usr/share/applications/vpndeck.desktop"
install -Dm644 assets/vpndeck.svg "$deb/usr/share/icons/hicolor/scalable/apps/vpndeck.svg"
install -Dm644 assets/vpndeck-256.png "$deb/usr/share/icons/hicolor/256x256/apps/vpndeck.png"
install -Dm644 README.md "$deb/usr/share/doc/vpndeck/README.md"
mkdir -p "$deb/DEBIAN"
cat > "$deb/DEBIAN/control" <<CONTROL
Package: vpndeck
Version: $version
Architecture: amd64
Maintainer: $maintainer
Installed-Size: $(du -sk "$deb/usr" | cut -f1)
Depends: openvpn (>= 2.4), pkexec | policykit-1, libc6 (>= 2.39), libgcc-s1, libegl1, libfontconfig1, libxkbcommon-x11-0, libwayland-client0
Recommends: xdg-desktop-portal, xdg-utils
Section: net
Priority: optional
Homepage: https://github.com/thuutien/vpndeck
Description: Desktop client for OpenVPN profiles
 VPNdeck imports .ovpn profiles from an OpenVPN server and connects to
 them with OpenVPN 2.4 or newer, showing status, traffic and logs, with
 a system tray icon for quick connect and disconnect.
CONTROL
dpkg-deb --root-owner-group --build "$deb" "dist/vpndeck_${version}_amd64.deb" >/dev/null

# --- .rpm --------------------------------------------------------------------
# One package for Fedora and openSUSE: dependencies are on sonames and files,
# which both name the same way. The libraries below are loaded at run time,
# so rpmbuild can't find them by itself.
rpm="$work/rpm"
mkdir -p "$rpm/SPECS"
cat > "$rpm/SPECS/vpndeck.spec" <<SPEC
Name: vpndeck
Version: $version
Release: 1
Summary: Desktop client for OpenVPN profiles
License: LicenseRef-Unspecified
URL: https://github.com/thuutien/vpndeck
Packager: $maintainer
Requires: openvpn >= 2.4
Requires: /usr/bin/pkexec
Requires: libEGL.so.1()(64bit)
Requires: libfontconfig.so.1()(64bit)
Requires: libwayland-client.so.0()(64bit)
Requires: libxkbcommon-x11.so.0()(64bit)
Recommends: xdg-desktop-portal
Recommends: xdg-utils

%description
VPNdeck imports .ovpn profiles from an OpenVPN server and connects to
them with OpenVPN 2.4 or newer, showing status, traffic and logs, with
a system tray icon for quick connect and disconnect.

%install
install -Dm755 $PWD/target/release/vpndeck %{buildroot}/usr/bin/vpndeck
install -Dm644 $PWD/packaging/vpndeck.desktop %{buildroot}/usr/share/applications/vpndeck.desktop
install -Dm644 $PWD/assets/vpndeck.svg %{buildroot}/usr/share/icons/hicolor/scalable/apps/vpndeck.svg
install -Dm644 $PWD/assets/vpndeck-256.png %{buildroot}/usr/share/icons/hicolor/256x256/apps/vpndeck.png
install -Dm644 $PWD/README.md %{buildroot}/usr/share/doc/vpndeck/README.md

%files
/usr/bin/vpndeck
/usr/share/applications/vpndeck.desktop
/usr/share/icons/hicolor/scalable/apps/vpndeck.svg
/usr/share/icons/hicolor/256x256/apps/vpndeck.png
%doc /usr/share/doc/vpndeck/README.md
SPEC
rpmbuild -bb --quiet --define "_topdir $rpm" --define "debug_package %{nil}" --define "__strip /bin/true" \
  "$rpm/SPECS/vpndeck.spec"
cp "$rpm/RPMS/x86_64/vpndeck-$version-1.x86_64.rpm" dist/

# --- Flatpak -----------------------------------------------------------------
# OpenVPN has to run as root on the host, so the app starts it through
# flatpak-spawn --host (org.freedesktop.Flatpak) and reads host paths under /run/host.
fp="$work/flatpak"
flatpak build-init "$fp" "$app_id" org.freedesktop.Platform org.freedesktop.Platform "$runtime_version"
install -Dm755 target/release/vpndeck "$fp/files/bin/vpndeck"
sed "s/^Icon=vpndeck/Icon=$app_id/" packaging/vpndeck.desktop > "$work/$app_id.desktop"
install -Dm644 "$work/$app_id.desktop" "$fp/files/share/applications/$app_id.desktop"
install -Dm644 assets/vpndeck.svg "$fp/files/share/icons/hicolor/scalable/apps/$app_id.svg"
install -Dm644 assets/vpndeck-256.png "$fp/files/share/icons/hicolor/256x256/apps/$app_id.png"
install -Dm644 "packaging/$app_id.metainfo.xml" "$fp/files/share/metainfo/$app_id.metainfo.xml"
flatpak build-finish "$fp" --command=vpndeck \
  --share=ipc --share=network \
  --socket=wayland --socket=fallback-x11 --device=dri \
  --filesystem=home:ro --filesystem=host-os:ro --filesystem=host-etc:ro \
  --talk-name=org.freedesktop.Flatpak \
  --talk-name=org.kde.StatusNotifierWatcher \
  --talk-name=org.freedesktop.secrets >/dev/null
flatpak build-export "$work/repo" "$fp" stable >/dev/null
flatpak build-bundle --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
  "$work/repo" "dist/vpndeck-$version-x86_64.flatpak" "$app_id" stable

(cd dist && sha256sum -- * > SHA256SUMS)
ls -l dist
