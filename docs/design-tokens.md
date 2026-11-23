# Design tokens

IsoFoundry follows [FluentGruv](https://github.com/SeanShaffer88/FluentGruv), version 1.3.0. Its `tokens.json` is authoritative; `crates/isofoundry/src/theme.rs` mirrors the runtime palettes and tests their contrast.

## Theme

- Dark is the default: Gruvbox charcoal surfaces, warm text and red active/focus accents.
- Light mode uses the complete FluentGruv light palette. Read `AppsUseLightTheme` from Windows; never mix light and dark tokens.
- Windows high-contrast mode overrides both palettes with system colors.
- When Windows changes theme, redraw the main window, dialogs and child controls.

## Dark palette

| Token | Value |
|---|---|
| `bg.canvas` | `#1D2021` |
| `bg.surface` | `#282828` |
| `bg.elevated` | `#32302F` |
| `bg.sunken` | `#171819` |
| `border.subtle` | `#3C3836` |
| `border.strong` | `#504945` |
| `fg.primary` | `#EBDBB2` |
| `fg.secondary` | `#D5C4A1` |
| `fg.muted` | `#A89984` |
| `accent.primary` | `#FB4934` |
| `accent.secondary` | `#CC241D` |
| `accent.subtle` | `#3B1D1B` |
| `status.success` | `#B8BB26` |
| `status.warning` | `#FABD2F` |
| `status.danger` | `#FB4934` |

## Light palette

| Token | Value |
|---|---|
| `bg.canvas` | `#F9F5D7` |
| `bg.surface` | `#FBF1C7` |
| `bg.elevated` | `#EBDBB2` |
| `bg.sunken` | `#F2E5BC` |
| `border.subtle` | `#D5C4A1` |
| `border.strong` | `#BDAE93` |
| `fg.primary` | `#282828` |
| `fg.secondary` | `#3C3836` |
| `fg.muted` | `#665C54` |
| `accent.primary` | `#CC241D` |
| `accent.secondary` | `#9D0006` |
| `accent.subtle` | `#F4D2AE` |
| `status.success` | `#79740E` |
| `status.warning` | `#AF3A03` |
| `status.danger` | `#CC241D` |

## Geometry and type

- Use 4px/8px spacing, 4px standard corners, 2px compact corners and 0px full-bleed edges.
- Use Segoe UI for interface text and Consolas/Cascadia Mono for paths, logs and numbers.
- Scale all dimensions and fonts for per-monitor DPI. Use a 2px primary-accent focus ring.
- Keep body/small text at 4.5:1 or better; active/focus/status marks need 3:1.

See [FluentGruv SPEC.md](https://github.com/SeanShaffer88/FluentGruv/blob/main/SPEC.md) for layout, drawers, radial progress, keyboard and viewport rules.
