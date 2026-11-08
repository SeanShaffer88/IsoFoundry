# Audit

Each feature is listed as Verified, Partially verified or Unverified, with the evidence (tests, recorded runs, drive reports). The table is rebuilt at every milestone; dated findings follow it.

| Area | Status | Evidence |
|---|---|---|
| Main window, menus, keyboard, focus ring (M0) | Partially verified | wine-9.0 (Ubuntu 9.0~repack-4build3) + Xvfb screenshot, 2026-11-07; not yet on Windows |
| CLI (`--version`, `list`, `helper-check`, task stubs) | Partially verified | Wine run and GitHub windows-latest CI smoke test (console twin), 2026-11-07 |
| Helper pipe handshake | Partially verified | Handshake + ping + version + shutdown passed under Wine and on GitHub windows-latest (runner account, no UAC prompt), 2026-11-07; unit tests for framing and nonce. UAC prompt path untested |
| Settings, logs, portable paths | Partially verified | Unit tests (Linux) |
| PortableApps.com launcher | Partially verified | Wine 2026-11-07: starts the app, data lands in `Data\settings` |
| Setup.exe and MSI install/run/uninstall | Partially verified | Silent per-user Setup.exe and per-machine MSI install, `--version`, uninstall pass on GitHub windows-latest, 2026-11-07; interactive wizard and UAC paths untested |
| Image reading (ISO 9660/RR/Joliet/UDF/El Torito), extract, hashing, .dat | Partially verified | Tests against genisoimage 1.1.11 images (tests/fixtures), 2026-11-07. UDF metadata partition (Blu-ray) untested: no fixture; VAT (BD-R/CD-R packet writing) not supported |
| ISO writer (ISO 9660/Joliet/UDF 1.02, El Torito) | Partially verified | Round-trip tests; images checked with isoinfo (cdrkit 1.1.11), udfinfo (udftools 2.3: integrity closed, both anchors), xorriso (El Torito BIOS+UEFI, media check good) and 7-Zip (UDF extraction identical to source), 2026-11-07. Not yet booted in a VM or mounted on Windows; >4 GB files tested only by code review |
| CD formats (BIN/CUE, CCD/IMG/SUB, raw BIN), sector ECC/EDC | Partially verified | Round-trip tests; chdman 0.264 regenerates our ECC-stripped sectors and accepts the raw SHA-1 (it rebuilds ECC with its own code), 2026-11-07 |
| CHD writer and reader | Partially verified | chdman 0.264 `verify` passes; overall SHA-1 identical to chdman's for data-only, mixed-mode (audio + stored pregap) and DVD images; `extractcd`/`extractdvd` output identical to source; chdman-made fixtures read back exactly. LZMA/FLAC/Huffman/Zstandard CHD hunks not readable |
| Image editing | Partially verified | Tests: add/remove/rename/mkdir/label on UDF and CHD sources, boot entries kept (BIOS+UEFI images byte-identical); xorriso 1.5 reads the edited boot image, 2026-11-07 |
| Mount / unmount | Partially verified | CI windows-latest: ISO and CHD (temp ISO) mount, file visible, unmount. Standard-user (non-admin) path untested |
| NRG, MDS/MDF reading | Unverified | Written from format descriptions; no real images to test (A-7) |
| USB tasks (M2) | Partially verified | Engine tested end-to-end against disk-image files (round trips, refusal rules); the elevated helper's physical-disk path and QEMU boot checks are Unverified |
| Audio conversion (M3) | Partially verified | Decode/encode tests and measured reference tones; MP3 via a real `libmp3lame.dll` and the Media Foundation AAC path are Unverified on Windows |
| Burning (M4) | Partially verified | Full flows against the simulated drive with digest verify; the IMAPI2 backend is Unverified on real hardware |
| Rip and copy (M5) | Partially verified | Engines and the simulated read side under test (damage maps, secure-mode agreement); the SPTI/MMC read path is Unverified on a real drive |
| Backup and shell integration (M6) | Partially verified | Spanning round trips incl. split files; tampered and incomplete backups fail under drill; WinHTTP, Explorer verbs, disc-insert actions and the announced shutdown are Unverified on Windows |
| Failure drills (RC) | Verified | `crates/isofoundry/tests/drills.rs`, 13 drills in the workspace suite, 2026-11-08 |
| Accessibility (RC) | Partially verified | Code-level pass (docs/ACCESSIBILITY.md); task-window keyboard navigation fixed; screen-reader and scaling checks owed from a real desktop |
| Performance budgets (RC) | Partially verified | docs/BENCHMARK.md: image build and checksum measured inside budget; first-window and working-set figures owed from Windows |

## 2026-11-08 — RC audit (0.9.0)

**Scope:** the whole tree against the roadmap's claims, the public docs, and the "Planned audits" list this file carried (accessibility pass, perf budgets, packaging/signing review, dependency audit, parser robustness).

| # | Finding | Severity | Status |
|---|---|---|---|
| B-1 | README and roadmap credited M4 with "multi-drive burn" and a "job queue" that were never built. | Medium | Multi-drive burn delivered in the command line (`--all-drives`); the queue claim corrected to the parallel-task-window model (D-052). |
| B-2 | The `report` command listed no drives and still said probing "arrives in M4". | Low | Fixed: the stale note is corrected; the report points at `isofoundry info` for the live drive list. |
| B-10 | Listing drives crashed the process on Windows (CI access violation, found by the RC test runs): the hand-rolled COM Release call in the IMAPI backend never passed the object pointer, so it released through a garbage `this`. | High | Fixed: the pointer is passed; a regression test enumerates drives on every CI run. |
| B-3 | `--help` printed an empty "not available yet" section once every task had shipped. | Low | Fixed: the section appears only when it has entries. |
| B-4 | The bootable-USB variant menu showed a "planned for M2 (0.3)" notice for shipped work. | Low | Fixed: image → USB opens its window; USB → image points at the working `usb read` command. |
| B-5 | Keyboard navigation (Tab, arrows, access keys) did not work inside task windows: dialog-message handling ran only for the main window. | Medium | Fixed: it runs for the window each message belongs to. |
| B-6 | docs/LIMITATIONS.md still said "No task runs yet" (M0 text). | Low | Fixed: section rewritten to the current shell/packaging facts. |
| B-7 | Packaging/signing review: per-user Inno Setup (`PrivilegesRequired=lowest`), uninstall cleanup present, SHA256SUMS.txt generated at packaging and re-verified by download on every release. No signing step exists — binaries are unsigned (no certificate), as stated in LIMITATIONS. | Low | No change; SmartScreen note already documented. |
| B-8 | Dependency audit (`cargo audit` over the lockfile): could not be run for this pass — the audit tool would not install on the development VM (network transfer failures, twice; a third attempt at the 1.0 pass stalled the same way). The lockfile's crates are the workspace's long-standing set; no new dependency was added by M6 or RC. | — | Owed: run `cargo audit` in the repo and record the result here. |
| B-9 | Image parsers take malformed input without panicking (RC drills: empty, truncated, garbage and random files through open/walk). A dedicated fuzz campaign (cargo-fuzz over the ISO/UDF/CHD/CCD parsers) has not been run. | Medium | Drills in place; fuzz campaign still owed. |

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
