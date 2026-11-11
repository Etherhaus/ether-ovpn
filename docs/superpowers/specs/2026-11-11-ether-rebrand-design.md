# Ether — rebrand and visual redesign

Date: 2026-11-11
Status: approved by the owner ("принимайся за ребрендинг и редизайн")

## Intent

VPNdeck / TW.VPN is a fork of an upstream OpenVPN client. The fork needs its own
identity: a name that survives being one of several protocol forks, and a visual
language that reads as one product rather than a recoloured upstream.

The name is **Ether**. The short mark is **E.OVPN**. The family rule is that the
protocol never enters the product name — it lives in a descriptor slot, because
`OpenVPN` and `WireGuard` are registered trademarks and because `EtherOVPN` /
`EtherWG` are part numbers, not names.

Ether is one piece of a larger set: the EtherOS site, the OBS scene overlays and
the stream titles. The app therefore does not invent a look of its own — it
speaks the **EtherOS design system**, so a screenshot of the client can sit next
to a screenshot of the site without a seam.

## Goals

- One brand across forks: `Ether (OpenVPN)`, `Ether (WireGuard)`, … with the same
  icon, palette and layout.
- The same visual language as the rest of EtherOS: coral accent, Adwaita Sans and
  JetBrains Mono, the niri window frame, flat surfaces with hairline borders.
- Every colour, radius and duration comes from the single `Theme` global, so the
  next fork is a token swap.
- No change to behaviour: connection logic, i18n, tray, updater stay as they are.

## Non-goals

- No new features, no layout re-architecture beyond what the visual language needs.
- No new dependencies. Slint 1.18 only.
- No gradients-as-decoration, no glass, no glow behind cards: EtherOS is flat.

## Constraints

- Slint has no backdrop blur, so depth comes from borders, a top highlight and a
  soft shadow — never from a real blur.
- Contrast: body text ≥ 4.5:1, white-on-accent ≥ 4.5:1 in both themes.
- The app id and config directory are hard to change after release, so they are
  decided once, here.

## Approaches considered

1. **Recolour only** — swap the palette, keep every shape. Cheapest, but the
   result still reads as the upstream app with a different hue. Rejected: the
   request is a redesign, not a repaint.
2. **Full re-layout** — new information architecture, sidebar, dashboard. Rejected:
   the window is 440×740 and the current single-column flow is right for it; a
   re-layout would cost behaviour and buy nothing.
3. **EtherOS on the existing structure** — keep the flow, rebuild the surface:
   the niri frame, flat cards, mono technical text, the coral accent, the new
   shield icon. Chosen.

## The design

### Palette (dark, primary)

| Token | Value | Role |
|---|---|---|
| `bg` | `#16161a` | page |
| `surface` | `#242428` | cards |
| `surface-2` | `#303036` | title bar, hovered rows |
| `surface-3` | `#1b1b1f` | wells, the log body |
| `field` | `#1b1b1f` | inputs, below the card they sit on |
| `line` | `#FFFFFF1A` | hairlines |
| `line-soft` | `#FFFFFF12` | the quietest separators |
| `text` | `#f6f5f4` | body |
| `text-2` | `#FFFFFFE0` | secondary body |
| `dim` | `#8b8b93` | meta, mono labels |
| `accent` | `#fb8762` | coral, the brand |
| `on-accent` | `#16161a` | text on a coral fill |
| `accent-soft` | `#FB87621F` | tinted chips and rows |
| `accent-soft-2` | `#FB876242` | pressed / selected |
| `accent-line` | `#FB876273` | borders on tinted surfaces |
| `accent-glow` | `#FB876247` | the frame's inner edge |
| `warn` | `#f8e45c` | connecting, log warnings |
| `bad` | `#f66151` | failures, log errors |
| `info` | `#62a0ea` | neutral notices |

Light theme mirrors it: `bg #f6f5f4`, `surface #ffffff`, `surface-2 #f0efed`,
`field #f0efed`, `line #0000001F`, `text #1b1b1f`, `dim #5e5e66`,
`accent #d9531e`, `accent-ink #b8431a`, `on-accent #ffffff`,
`warn #8a6d00`, `bad #b3261e`, `info #1c5fb0`.

`#d9531e` on white is 4.03:1, so light-theme *text* uses `accent-ink` (`#b8431a`,
5.9:1) while fills keep `accent`.

**There is no green.** Connected is coral, connecting is yellow, a failure is red.
A green "OK" dot is the one thing that would break the family resemblance.

### Window chrome

Every window — main, log, profile — is drawn as a niri window:

- a 3px `accent` frame around the whole window, with the inner surface inset 3px
  and clipped to a 23px radius, so the corners are round and the frame follows;
- a 40px title bar in `surface-2`: the app mark and a mono `spec` label on the
  left, the window title centred at 13px/700, round 26px icon buttons on the
  right;
- a 1px `line` hairline under the title bar, with an `accent-line` gradient
  running through the middle of it.

The frame is a setting, on by default. Turned off, the surface fills the window
and the corners are square, for desktops that draw their own borders.

### Type

`Adwaita Sans` for the interface, `JetBrains Mono` for anything technical: the
`spec` label, stat values, traffic counters, the duration, the log, the version
in the footer. Four steps: 28/700 display (the profile name on the status card),
15/600 title, 13/500 body, 10–11/700 mono with 0.5px tracking for micro-labels.

### Shape and motion

Radii: 26 for the window, 16 for cards, 12 for fields and small controls.
Motion is 160ms `ease-out` on colour and position. Nothing bounces, nothing
pulses.

### Components

- **Card** — `surface` fill, 1px `line` border, a 1px `highlight` line inset past
  the corner radius, and a soft shadow.
- **Button** — 46px, radius 12, three kinds: `solid` (coral fill, `on-accent`
  text), `outline` (hairline, `text`), `ghost` (no fill, `text-2`). Hover lifts
  the fill one step, never the hue.
- **Field** — 46px, `field` fill, hairline; focus is a 1px `accent` border.
- **StatusChip** — a pill with an 8px dot, sized to its content: coral when
  connected, yellow while connecting, red on failure, `dim` when idle.
- **Toggle / Check / Segmented** — same geometry as before, new tokens; the
  selected segment is `accent-soft` with `accent-ink` text.
- **Avatar** — 36px, radius 12, mono initials; `accent-soft` when selected.
- **Sparkline** — rounded bars in `accent`, no gradient.
- **Modal** — a card at radius 26 with its own title bar, over a dimmed backdrop.
- **Link / TextButton** — mono, `accent-ink`, underline on hover.

### Icon

A coral shield with a keyhole, on a rounded-square tile: `#2b2b2b → #1a1a1a` for
the dark theme, `#fafaf9 → #efeeec` for the light one, with a soft coral glow
behind the shield. The shield is an homage to the protocol's own mark without
copying it; the keyhole is the "private" half of the idea.

`assets/ether.svg` is the dark tile (app icon, packaging, README),
`assets/ether-light.svg` the light one, and `assets/ether-256.png` is rendered
from the dark tile by `cargo run --example render_icon -- 256 assets/ether-256.png`.

The tray has no room for a tile, so it draws the bare shield, tinted by state:
`dim` when idle, `warn` while connecting, `accent` when connected, `bad` on
failure.

## Naming and packaging

| Thing | Value |
|---|---|
| Product name | `Ether` |
| Short mark | `E.OVPN` |
| Desktop entry | `Name=Ether`, `Comment=OpenVPN client for Linux` |
| Binary / crate | `ether-ovpn` |
| App id | `io.github.<owner>.EtherOvpn` |
| Config | `~/.config/ether` |
| Repos | `ether-ovpn`, `ether-wg`, `ether-core` |

The updater reads `CARGO_PKG_REPOSITORY`, so it follows `repository` in
`Cargo.toml` with no code change.

## Left to the builder

- The owner's GitHub handle for the app id: `etherhaus`.
- Whether existing `~/.config/vpndeck` profiles are migrated or re-imported.
