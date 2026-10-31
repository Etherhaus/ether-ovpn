<p align="center">
  <img src="assets/vpndeck.svg" alt="VPNdeck icon" width="112">
</p>

<h1 align="center">VPNdeck</h1>

<p align="center">
  A small desktop client for OpenVPN on Linux, written in Rust with <a href="https://slint.dev">Slint</a>.
  It works with the <code>.ovpn</code> profiles exported by UniFi gateways (UDM Pro / UDM Pro SE) and
  any other OpenVPN 2.x server.
</p>

![VPNdeck connected to a VPN](docs/screenshot.png)

## Features

- Import `.ovpn` profiles (stored privately under `~/.config/vpndeck`)
- Connect / disconnect with live status, VPN address, server and traffic counters
- Username/password and private-key password prompts. The username is remembered, and the
  password can be saved in your desktop keyring (GNOME Keyring / KWallet) for automatic sign-in
- Live OpenVPN log
- System tray icon showing connection status, with quick connect/disconnect.
  Closing the window keeps VPNdeck running in the tray.
- Checks GitHub for new releases and updates itself (tarball in place, .deb through apt, Flatpak through flatpak)
- Applies DNS pushed by the server when `update-systemd-resolved` or `update-resolv-conf` is installed

## Requirements

```
sudo apt install openvpn
```

The app runs as your user. `openvpn` is started through `pkexec`, so you'll get the normal
administrator prompt when connecting. VPNdeck talks to it over OpenVPN's management interface
on a random localhost port protected by a one-time password.

## Install

OpenVPN must be installed on the system whichever package you use (see Requirements).
Download a package from the [latest release](https://github.com/thuutien/vpndeck/releases/latest).
The prebuilt packages are for x86_64.

**Debian, Ubuntu, Linux Mint** (`.deb`, needs Ubuntu 24.04+, Mint 22+ or Debian 13; pulls in `openvpn`):

```bash
sudo apt install ./vpndeck_*_amd64.deb
```

**Any distro with Flatpak** (`.flatpak`; the Freedesktop runtime is fetched from Flathub if needed):

```bash
flatpak install --user ./vpndeck-*-x86_64.flatpak
```

The Flatpak starts OpenVPN on the host through `flatpak-spawn --host`, so it is not meaningfully
sandboxed: it needs the same root access to run a VPN as the other packages.

**Tarball** (`.tar.gz`, needs glibc 2.39+). It installs to `~/.local/bin`, plus the app icon and an app-menu entry:

```bash
tar xzf vpndeck-*-x86_64-linux.tar.gz
cd vpndeck-*-x86_64-linux
packaging/install.sh
```

**From source:**

```bash
cargo build --release
packaging/install.sh
```

`SHA256SUMS` in each release lists the checksums of the files.

## Releasing

```
./packaging/build-packages.sh
```

This builds `dist/` (tarball, .deb, Flatpak and `SHA256SUMS`; needs `dpkg-deb`, `flatpak` and the
`org.freedesktop.Platform//25.08` runtime). Attach all four to a GitHub release
tagged `v<version>` at https://github.com/thuutien/vpndeck so the in-app updater can find it.

## Getting the profile from a UDM Pro SE

In UniFi Network: Settings → VPN → VPN Server → your OpenVPN server → Download configuration.
Then click **Import…** in VPNdeck and pick the downloaded file.
