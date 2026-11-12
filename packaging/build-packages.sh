#!/bin/sh
# Builds the release packages into dist/:
#   ether-ovpn-<version>-x86_64-linux.tar.gz   binary + install script
#   ether-ovpn_<version>_amd64.deb             Debian/Ubuntu/Mint package
#   ether-ovpn-<version>-1.x86_64.rpm          Fedora/openSUSE package
#   ether-ovpn-<version>-x86_64.flatpak        Flatpak bundle (Freedesktop runtime)
#   SHA256SUMS
# Needs dpkg-deb, rpmbuild, flatpak and the org.freedesktop.Platform//25.08 runtime:
#   flatpak install --user flathub org.freedesktop.Platform//25.08
# Attach them all to a GitHub release tagged v<version>; the in-app updater
# downloads from the latest release.
set -eu
cd "$(dirname "$0")/.."

app_id=io.github.etherhaus.EtherOvpn
runtime_version=25.08
version=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -n1)
# A pre-release (0.3.2-beta.1) can't go into an RPM Version, which forbids a
# dash, and in Debian it would sort above the release it precedes. Each format
# gets the pre-release part where it belongs: RPM's Release, Debian's tilde.
rpm_version=${version%%-*}
rpm_release=1
deb_version=$version
deb_file_version=$version
case $version in
  *-*)
    pre=$(printf '%s' "${version#*-}" | tr '-' '.')
    rpm_release="0.$pre"
    deb_version="${version%%-*}~$pre"
    # The tilde belongs in the control file, where it makes dpkg sort the
    # pre-release below the release it precedes. GitHub rewrites a tilde in an
    # uploaded asset name to a dot, which would leave SHA256SUMS pointing at a
    # file that doesn't exist, so the name uses the dot from the start.
    deb_file_version="${version%%-*}.$pre"
    ;;
esac
maintainer="Dmitry Etherhaus <Lehzenblad@proton.me>"

cargo build --release
rm -rf dist
mkdir -p dist
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# --- tarball -----------------------------------------------------------------
name="ether-ovpn-$version-x86_64-linux"
mkdir -p "$work/$name/packaging"
cp target/release/ether-ovpn README.md LICENSE "$work/$name/"
cp -r assets "$work/$name/"
cp packaging/install.sh packaging/uninstall.sh packaging/ether-ovpn.desktop "$work/$name/packaging/"
tar -C "$work" -czf "dist/$name.tar.gz" "$name"

# --- .deb --------------------------------------------------------------------
deb="$work/deb"
install -Dm755 target/release/ether-ovpn "$deb/usr/bin/ether-ovpn"
install -Dm644 packaging/ether-ovpn.desktop "$deb/usr/share/applications/ether-ovpn.desktop"
install -Dm644 assets/ether.svg "$deb/usr/share/icons/hicolor/scalable/apps/ether-ovpn.svg"
install -Dm644 assets/ether-256.png "$deb/usr/share/icons/hicolor/256x256/apps/ether-ovpn.png"
install -Dm644 README.md "$deb/usr/share/doc/ether-ovpn/README.md"
install -Dm644 LICENSE "$deb/usr/share/doc/ether-ovpn/copyright"
mkdir -p "$deb/DEBIAN"
cat > "$deb/DEBIAN/control" <<CONTROL
Package: ether-ovpn
Version: $deb_version
Architecture: amd64
Maintainer: $maintainer
Installed-Size: $(du -sk "$deb/usr" | cut -f1)
Depends: openvpn (>= 2.4), pkexec | policykit-1, libc6 (>= 2.39), libgcc-s1, libegl1, libfontconfig1, libxkbcommon-x11-0, libwayland-client0
Recommends: xdg-desktop-portal, xdg-utils
Section: net
Priority: optional
Homepage: https://github.com/Etherhaus/ether-ovpn
Description: Desktop client for OpenVPN profiles
 Ether imports .ovpn profiles from an OpenVPN server and connects to
 them with OpenVPN 2.4 or newer, showing status, traffic and logs, with
 a system tray icon for quick connect and disconnect.
CONTROL
dpkg-deb --root-owner-group --build "$deb" "dist/ether-ovpn_${deb_file_version}_amd64.deb" >/dev/null

# --- .rpm --------------------------------------------------------------------
# One package for Fedora and openSUSE: dependencies are on sonames and files,
# which both name the same way. The libraries below are loaded at run time,
# so rpmbuild can't find them by itself.
rpm="$work/rpm"
mkdir -p "$rpm/SPECS"
cat > "$rpm/SPECS/ether-ovpn.spec" <<SPEC
Name: ether-ovpn
Version: $rpm_version
Release: $rpm_release
Summary: Desktop client for OpenVPN profiles
License: MIT
URL: https://github.com/Etherhaus/ether-ovpn
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
Ether imports .ovpn profiles from an OpenVPN server and connects to
them with OpenVPN 2.4 or newer, showing status, traffic and logs, with
a system tray icon for quick connect and disconnect.

%install
install -Dm755 $PWD/target/release/ether-ovpn %{buildroot}/usr/bin/ether-ovpn
install -Dm644 $PWD/packaging/ether-ovpn.desktop %{buildroot}/usr/share/applications/ether-ovpn.desktop
install -Dm644 $PWD/assets/ether.svg %{buildroot}/usr/share/icons/hicolor/scalable/apps/ether-ovpn.svg
install -Dm644 $PWD/assets/ether-256.png %{buildroot}/usr/share/icons/hicolor/256x256/apps/ether-ovpn.png
install -Dm644 $PWD/README.md %{buildroot}/usr/share/doc/ether-ovpn/README.md
install -Dm644 $PWD/LICENSE %{buildroot}/usr/share/licenses/ether-ovpn/LICENSE

%files
/usr/bin/ether-ovpn
/usr/share/applications/ether-ovpn.desktop
/usr/share/icons/hicolor/scalable/apps/ether-ovpn.svg
/usr/share/icons/hicolor/256x256/apps/ether-ovpn.png
%doc /usr/share/doc/ether-ovpn/README.md
%license /usr/share/licenses/ether-ovpn/LICENSE
SPEC
rpmbuild -bb --quiet --define "_topdir $rpm" --define "debug_package %{nil}" --define "__strip /bin/true" \
  "$rpm/SPECS/ether-ovpn.spec"
cp "$rpm/RPMS/x86_64/ether-ovpn-$rpm_version-$rpm_release.x86_64.rpm" dist/

# --- Flatpak -----------------------------------------------------------------
# OpenVPN has to run as root on the host, so the app starts it through
# flatpak-spawn --host (org.freedesktop.Flatpak) and reads host paths under /run/host.
fp="$work/flatpak"
flatpak build-init "$fp" "$app_id" org.freedesktop.Platform org.freedesktop.Platform "$runtime_version"
install -Dm755 target/release/ether-ovpn "$fp/files/bin/ether-ovpn"
sed "s/^Icon=ether-ovpn/Icon=$app_id/" packaging/ether-ovpn.desktop > "$work/$app_id.desktop"
install -Dm644 "$work/$app_id.desktop" "$fp/files/share/applications/$app_id.desktop"
install -Dm644 assets/ether.svg "$fp/files/share/icons/hicolor/scalable/apps/$app_id.svg"
install -Dm644 assets/ether-256.png "$fp/files/share/icons/hicolor/256x256/apps/$app_id.png"
install -Dm644 "packaging/$app_id.metainfo.xml" "$fp/files/share/metainfo/$app_id.metainfo.xml"
flatpak build-finish "$fp" --command=ether-ovpn \
  --share=ipc --share=network \
  --socket=wayland --socket=fallback-x11 --device=dri \
  --filesystem=home:ro --filesystem=host-os:ro --filesystem=host-etc:ro \
  --talk-name=org.freedesktop.Flatpak \
  --talk-name=org.kde.StatusNotifierWatcher \
  --talk-name=org.freedesktop.secrets >/dev/null
flatpak build-export "$work/repo" "$fp" stable >/dev/null
flatpak build-bundle --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo \
  "$work/repo" "dist/ether-ovpn-$version-x86_64.flatpak" "$app_id" stable

(cd dist && sha256sum -- * > SHA256SUMS)
ls -l dist
