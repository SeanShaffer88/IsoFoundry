# Audit

Nothing is implemented yet, so nothing is verified. This file will list each feature as Verified, Partially verified or Unverified, with the evidence (tests, recorded runs, drive reports).

| Area | Status | Evidence |
|---|---|---|
| Main window, menus, keyboard, focus ring (M0) | Partially verified | wine-9.0 (Ubuntu 9.0~repack-4build3) + Xvfb screenshot, 2026-11-07; not yet on Windows |
| CLI (`--version`, `list`, `helper-check`, task stubs) | Partially verified | Wine run and GitHub windows-latest CI smoke test (console twin), 2026-11-07 |
| Helper pipe handshake | Partially verified | Handshake + ping + version + shutdown passed under Wine and on GitHub windows-latest (runner account, no UAC prompt), 2026-11-07; unit tests for framing and nonce. UAC prompt path untested |
| Settings, logs, portable paths | Partially verified | Unit tests (Linux) |
| Disc, image, audio and USB tasks | Not started | Ship from M1 (see [docs/roadmap.md](docs/roadmap.md)) |

## 2026-11-07 — M0 shell

| # | Finding | Severity | Status |
|---|---|---|---|
| A-9 | UAC shield did not render under Wine (`SHGetStockIconInfo`); the button text still says "(requires administrator)" to screen readers and the menu says "(administrator)". | Low | Check on Windows; text fallback already present. |
| A-10 | Native menu bar stays light in dark mode (no documented dark-menu API). | Low | Recorded in LIMITATIONS. |
| A-11 | Fonts are recreated on DPI change; an open log/roadmap viewer keeps its old font size until reopened. | Low | Open. |

## 2026-10-30 — Planning audit
**Scope:** plan, decisions, design tokens. No code exists.

| # | Finding | Severity | Status |
|---|---|---|---|
| A-1 | Accent `#E0313A` on button `#6B0700` = 2.82:1, below WCAG 1.4.11 (3:1). | Medium | Fixed in tokens: focus ring drawn outside a `bg` gap, on `surface` (3.98:1). |
| A-2 | Decorative `border` `#2E2E34` = 1.33:1; unusable for input outlines. | Low | Added `border-control` for inputs. |
| A-3 | No optical/USB hardware → disc and USB paths can't be verified by the developer. | High | Mitigated: fake drive traces, VHDX + QEMU, community drive reports, Unverified labels (D-024, D-029, D-031). |
| A-4 | Elevated helper is an attack surface. | High | Design: typed messages only, pipe ACL to current user SID, per-session nonce, helper exits after job. Review at M0. |
| A-5 | USB writer could target the wrong disk. | High | Design: refuse system/boot/pagefile/fixed disks, typed confirmation for > 64 GB, VHDX only behind dev flag. Tests in M2. |
| A-6 | LAME is LGPL; static linking would affect MIT distribution. | Medium | Runtime `LoadLibrary`, separate DLL, notice in CREDITS. |
| A-7 | Reverse-engineered formats (MDS, NRG) could misread. | Low | Read-only, Unverified label, fuzz tests in M1. |
| A-8 | GNU toolchain + `windows` crate COM untested. | Medium | Verify in M0; CI MSVC remains the release path. |

## Planned audits
- M0: IPC/elevation review, dependency licenses (`cargo deny`), `cargo audit`.
- Each milestone: fuzz image parsers (`cargo fuzz`), failure drills for new destructive ops.
- RC: full a11y pass (keyboard-only, Narrator, high contrast, 200% scale), perf budgets, packaging/signing review.
