# Design tokens

Base: product-family black/red (InstallScope/Expunge). Deviations recorded at the bottom.

## Color
| Token | Value | Use | Contrast (measured) |
|---|---|---|---|
| `bg` | `#0E0E10` | window background | — |
| `surface` | `#17171A` | group box / panels | white text 17.89:1 |
| `surface-2` | `#1F1F23` | inputs, list rows | — |
| `border` | `#2E2E34` | decorative dividers only | 1.33:1 (non-essential) |
| `border-control` | `#7A7A84` | input / checkbox borders | 3.87:1 on surface-2, 4.21:1 on surface |
| `text` | `#FFFFFF` | primary text | — |
| `text-muted` | `#B8B8C0` | secondary text | 9.08:1 on surface |
| `btn` | `#6B0700` | task buttons, primary buttons | white 12.68:1 |
| `btn-hover` | `#861008` | hover | white 10.04:1 |
| `btn-pressed` | `#540500` | pressed | — |
| `accent` | `#E0313A` | focus ring, progress, selection, links (never body text) | 3.98:1 on surface (UI-component ≥ 3:1 ✔) |
| `ok` | `#3FB950` | success icon + text label | 7.04:1 |
| `warn` | `#D29922` | warning icon + text label | 7.09:1 |
| `error` | `#FF6B6B` | error icon + text label | 6.45:1 |
| `shield` | system UAC shield icon | elevation marker | — |

Focus ring: 2 px `accent` drawn **outside** a 2 px `bg` gap so it sits on `surface` (3.98:1), not on `btn` (2.82:1, fails).

## Type
- Segoe UI Variable (fallback Segoe UI). Sizes: 12 caption, 14 body, 14 semibold buttons, 18 dialog title, 28 wordmark.
- Follows Windows text-scaling setting.

## Spacing / shape
- 4 px grid. Button height 32, grid gap 12 × 10, group padding 16.
- Radius 4 (buttons, inputs), 6 (group box), 8 (dialogs).

## Motion
- 120 ms ease-out hover/press; progress bar animates only while active; all motion off when Windows "Animation effects" is off.

## High contrast
- When a Windows contrast theme is active, all tokens map to system colors (`ButtonFace`, `ButtonText`, `Highlight`, …).

## Deviations
- `accent` `#E0313A` added (Expunge uses only `#6B0700`): needed for a visible focus ring.
