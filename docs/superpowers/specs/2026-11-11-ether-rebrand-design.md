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

## Goals

- One brand across forks: `Ether (OpenVPN)`, `Ether (WireGuard)`, … with the same
  icon, palette and layout.
- A visual language that is recognisably "ether": depth, light, translucency —
  not a flat dark theme with a different hue.
- Every colour, radius, shadow and duration still comes from the single `Theme`
  global, so the next fork is a token swap.
- No change to behaviour: connection logic, i18n, tray, updater stay as they are.

## Non-goals

- No new features, no layout re-architecture beyond what the visual language needs.
- No new dependencies. Slint 1.18 only.
- No light-theme redesign beyond keeping it legible and consistent.

## Constraints

- Slint has no backdrop blur, so "glass" must be built from layered gradients,
  hairline borders and a top highlight — not from a real blur.
- Contrast: body text ≥ 4.5:1, white-on-accent ≥ 4.5:1 in both themes.
- The app id and config directory are hard to change after release, so they are
  decided once, here.

## Approaches considered

1. **Recolour only** — swap the palette, keep every shape. Cheapest, but the
   result still reads as the upstream app with a violet accent. Rejected: the
   request is a redesign, not a repaint.
2. **Full re-layout** — new information architecture, sidebar, dashboard. Rejected:
   the window is 440×740 and the current single-column flow is right for it; a
   re-layout would cost behaviour and buy nothing.
3. **New visual language on the existing structure** — keep the flow, rebuild the
   surface: depth, glass cards, glow, typographic hierarchy, new icon. Chosen.

## The design

### Palette (dark, primary)

| Token | Value | Role |
|---|---|---|
| `bg` | `#06070C` | page, near-black with a blue undertone |
| `surface` | `#0E1018` | cards |
| `surface-2` | `#161A24` | fields, hovered rows |
| `line` | `#1F2432` | hairlines |
| `text` | `#E9EDF6` | body |
| `text-muted` | `#7B859B` | meta |
| `accent` | `#7C5CFF` | violet, the brand |
| `accent-hover` | `#8F73FF` | |
| `success` | `#3DDC97` | connected |
| `error` | `#FF6B6B` | |
| `warning` | `#FFB84D` | |

Light theme mirrors it: `bg #F6F7FB`, `surface #FFFFFF`, `line #E5E8F0`,
`text #0B0E15`, `accent #6A45F5`.

### Depth

Three devices, used consistently:

- **Glass card** — a vertical gradient from `#FFFFFF0F` to `#FFFFFF03` over the
  page, a 1px `#FFFFFF14` border, and a 1px top highlight that fades out at both
  ends. This is what sells translucency without a blur.
- **Glow** — a radial gradient behind the status card, tinted by connection state.
  Only one glow on screen at a time.
- **Shadow** — soft, large, low-alpha, only in the light theme; in the dark theme
  the border does the separating.

### Type

Inter. Four steps: 28/700 display (status headline), 15/600 title, 13/500 body,
11/600 uppercase with 0.08em tracking for micro-labels. Numbers in the stat grid
are tabular where Slint allows.

### Shape and motion

Radii 10 / 14 / 20 / 26. Motion 220ms `ease-out` for colour and position, 600ms
for the connecting pulse. Nothing bounces.

### Components

- **Button** — 46px, radius 14, accent fill with a subtle top-light gradient;
  secondary is a hairline outline on glass.
- **Field** — 46px, `surface-2`, hairline; focus ring is a 2px accent border plus
  a 3px accent halo at 18% alpha.
- **Toggle / Check / Segmented** — same geometry as today, new tokens.
- **Avatar** — 36px, radius 12, gradient fill when selected.
- **Status dot** — 8px core inside a 20px halo at 14% alpha; the halo breathes
  while connecting.
- **Sparkline** — rounded bars with a vertical accent gradient.
- **Modal** — glass card, radius 26, 24px padding, dimmed backdrop at 72%.

### Icon

A tunnel mouth: one white arch with a rounded opening, on a violet-to-indigo
gradient tile with a hairline inner highlight. It reads at 16px in the tray and
says "tunnel" without borrowing the protocol's logo.

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

- The owner's GitHub handle for the app id (placeholder `dmitry` until confirmed).
- Whether existing `~/.config/vpndeck` profiles are migrated or re-imported.
- Light-theme fine-tuning after the dark theme is signed off.
