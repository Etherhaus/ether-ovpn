<p align="center">
  <img src="assets/vpndeck.svg" alt="VPNdeck icon" width="112">
</p>

<h1 align="center">VPNdeck</h1>

<p align="center">
  A small desktop client for OpenVPN on Linux, written in Rust with <a href="https://slint.dev">Slint</a>.
  Import the <code>.ovpn</code> profiles from your OpenVPN server and connect with one click.
  Works with OpenVPN 2.4 or newer, including servers that ask for a 2FA code.
</p>

![VPNdeck connected to a VPN](docs/screenshot.png)

| Dark | Light |
|---|---|
| ![VPNdeck in dark mode](docs/screenshot-dark.png) | ![VPNdeck in light mode](docs/screenshot-light.png) |

## Features

- Import one or many `.ovpn` profiles at once (stored privately under `~/.config/vpndeck`).
  Files that are already in the list are skipped, so re-importing a folder is safe
- Search the profile list once it grows past a handful of servers
- Connect / disconnect with live status, VPN address, server and traffic counters
- Username/password and private-key password prompts. The username is remembered, and the
  password can be saved in your desktop keyring (GNOME Keyring / KWallet) for automatic sign-in
- Two-factor authentication: static challenge, CRV1 and crtext (see below)
- English and Russian, dark and light themes, both switchable in Settings
- Live OpenVPN log
- System tray icon showing connection status, with quick connect/disconnect.
  Closing the window keeps VPNdeck running in the tray.
- Checks GitHub for new releases and updates itself (tarball in place, .deb through apt, Flatpak through flatpak)
- Applies DNS pushed by the server when `update-systemd-resolved` or `update-resolv-conf` is installed

## Two-factor authentication

VPNdeck answers every challenge/response variant OpenVPN servers use:

| The server asks for | VPNdeck sends |
|---|---|
| `--static-challenge` (SCRV1) | `SCRV1:<base64 password>:<base64 code>` |
| `--static-challenge` with FORMAT=1 | the code appended to the password |
| A CRV1 dynamic challenge | `CRV1::<state>::<code>` on the next attempt |
| crtext (OpenVPN 2.5+) | `cr-response <base64 code>` while authentication is pending |

It announces `IV_SSO=crtext` at startup, because servers that check it never send a crtext
challenge otherwise. When the password is already in the keyring, the dialog asks only for the
code. The wire format is OpenVPN's own
[management-notes.txt](https://github.com/OpenVPN/openvpn/blob/master/doc/management-notes.txt).

## Languages

The interface is available in English and Russian. The first start follows the desktop locale
(`LANG`), and Settings switches it at any time; the choice is stored in
`~/.config/vpndeck/settings.json`. Every string lives in `src/i18n.rs`, so adding a language is
one more column in that table.

## Requirements

OpenVPN 2.4 or newer installed on the computer (tested with 2.6):

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

Bump the version in `Cargo.toml` and add a `<release>` entry to
`packaging/io.github.thuutien.VPNdeck.metainfo.xml`, then push a `v<version>` tag. The Release
workflow builds the tarball, .deb, .rpm, Flatpak and `SHA256SUMS` on Ubuntu 24.04 (so the binary
runs on glibc 2.39 and newer), attaches them to the GitHub release for the tag and publishes it,
which is where the in-app updater looks. To write the release notes yourself, create a draft
release for the tag before pushing it.

`./packaging/build-packages.sh` builds the same files into `dist/` locally (needs `dpkg-deb`,
`rpmbuild`, `flatpak` and the `org.freedesktop.Platform//25.08` runtime).

## Getting started

Download the client configuration (`.ovpn` files) from your OpenVPN server, then click
**+ Import** in VPNdeck and pick one or more files. On a fresh install the same button sits in
the middle of the window.

## License

VPNdeck is released under the [MIT License](LICENSE). It is built with [Slint](https://slint.dev),
used under the Slint Royalty-free Desktop, Mobile, and Web Applications License.
