# Decision log

Format: `D-NNN — decision — why — status`. Newest at the bottom. Changing a decision appends a new entry that supersedes the old one; entries are never edited away.

| ID | Decision | Why | Status |
|---|---|---|---|
| D-001 | Name: **IsoFoundry**. Tagline "Open Source Disc Burning, Imaging & Bootable USB Suite". | Distinct, non-infringing name. | Accepted 2026-10-30 |
| D-002 | Rust (stable) + `windows` crate (COM) + `egui`/`eframe`. | Shares the WinISOPilot stack and theme; fastest path to ~25 task dialogs. | Superseded by D-043 (UI) |
| D-003 | Hybrid burn engine: IMAPI2 for standard data/erase/image writes; own SCSI pass-through (MMC via `IOCTL_SCSI_PASS_THROUGH_DIRECT`) for raw read, subchannel, write-mode control, quality scans, secure audio ripping. | IMAPI2 is built into Windows and stable; it cannot do raw/subchannel/C1C2/PIPO. | Accepted |
| D-004 | No decryption or circumvention of copy protection (CSS, AACS, BD+, SafeDisc, SecuROM, etc.). Raw/subchannel copying of unprotected discs is in scope. | DMCA §1201 / EU InfoSoc Art. 6; protects the public repo and users. | Accepted, permanent |
| D-005 | Mounting uses Windows' built-in ISO mount (`AttachVirtualDisk`/Shell). Non-ISO images are converted to a temp ISO, then mounted. No kernel driver. | No driver signing, no BSOD risk, works on Home editions. | Accepted |
| D-006 | Main window keeps the classic two-column task grid. Extra modes live in split-button dropdowns; two new rows for genuinely new tasks. | Familiar layout, compact window. | Accepted |
| D-007 | Colors: buttons `#6B0700` + white text; brighter accent `#E0313A` for focus, hover outline, progress. | Inherits the product-family red; maroon alone is too dim for focus rings. | Accepted |
| D-008 | Everything ships before 1.0. Public 0.x pre-releases at each milestone; 1.0 = full feature set. | Sean's choice (7C). | Accepted |
| D-009 | Targets: Windows 10 22H2 and Windows 11; x64 and ARM64. | Wide reach for little Rust cost. Support claims stay "Unverified" until tested per arch. | Accepted |
| D-010 | Shared `usbwriter` crate extracted from WinISOPilot, used by both apps. | One audited engine for destructive disk writes. | Accepted |
| D-011 | Offline by default. MusicBrainz / AccurateRip / update check are opt-in and off out of the box. | Charter: offline-first, no network without approval. | Accepted |
| D-012 | License MIT. Codecs: Symphonia (decode), Rust FLAC encoder, LAME via runtime `LoadLibrary` (not bundled-linked), Media Foundation for AAC/WMA. | Permissive app license; LGPL LAME kept dynamic. | Accepted |
| D-013 | Distribution: portable zip, PortableApps.com Format package, Inno Setup installer. | No Visual Studio needed; covers portable and installed users. | Superseded by D-044 |
| D-014 | Main app runs unelevated; a separate elevated helper (`isofoundry-helper.exe`) runs only for tasks that need it, over a named pipe with a per-session nonce. Shield icons mark those tasks. | Least privilege; keeps Explorer drag-drop working. | Accepted |
| D-015 | Explorer integration: classic context-menu verbs via HKCU registry, registered/unregistered from Options. Native Win11 top-level menu deferred (needs MSIX/sparse package). | Works on Win10 and Win11 without packaging. | Accepted |
| D-016 | CLI: subcommands in the same exe (`isofoundry burn-image …`), `--json` output, stable exit codes. GUI subsystem exe attaches to the parent console when launched with arguments. | Scriptable; one binary. Console-attach quirks recorded in LIMITATIONS. | Accepted |
| D-017 | Image write formats: ISO, BIN/CUE, CCD/IMG/SUB, raw IMG, plus one compressed open format (CHD preferred; zstd-ISO fallback). Read: + MDS/MDF, NRG, CDI, DMG (HFS read-only), WIM (browse). | Open formats preserve exact copies. CHD write support decided after the M1 spike. | Accepted; CHD vs zstd pending spike |
| D-018 | Audio encode: MP3, FLAC, WAV, Opus, Ogg Vorbis, AAC/M4A (Media Foundation). Decode adds WMA, APE (APE decoder unverified). | Broad coverage without bundling patent-encumbered codecs. | Accepted |
| D-019 | Video: burn existing VIDEO_TS / BDMV folders with correct UDF (1.02 / 2.50) and file ordering. No transcoding or authoring. | Authoring is a separate product. | Accepted |
| D-020 | Spanning backup: ZIP64 split volumes + JSON manifest + per-file SHA-256 + optional AES-256 (WinZip AE-2). | Restorable with any unzip tool. | Accepted |
| D-021 | Update check: opt-in, GitHub Releases API, notify only, never auto-install. | Charter. | Accepted |
| D-022 | Strings externalized as Fluent `.ftl`; English at 1.0, Spanish in 1.x. | Cheap later localization. | Accepted |
| D-023 | Public repo `SeanShaffer88/IsoFoundry`. | Sean's choice. | Accepted |
| D-024 | No optical hardware available. Burn/copy/rip/scan ship **Unverified** until community drive reports confirm them. | Honest evidence labels. | Accepted |
| D-025 | Drive abstraction trait + fake MMC device replaying recorded command/response traces; CI runs burn/copy/scan logic against it. | Logic is testable without hardware. | Accepted |
| D-026 | Milestones ordered hardware-free first (see roadmap.md). | Most work stays verifiable. | Accepted |
| D-027 | Single design direction; mock main window + Burn image + Copy disc before UI code. | Layout and colors are already fixed. | Accepted |
| D-028 | Icon/logo: disc in a foundry crucible, molten `#E0313A` glow, white "IsoFoundry" wordmark. | Name fit. | Accepted |
| D-029 | No USB stick available: USB tasks are tested against an attached **VHDX** target (Virtual Disk API, works on Home) and boot-tested in **QEMU** (Sean runs Windows Home, so no Hyper-V). VHDX targets only appear behind a `--dev-allow-vdisk` flag. | Real write/partition/boot path without hardware; flag prevents fixed disks showing in normal use. | Accepted |
| D-030 | Official builds: GitHub Actions `windows-latest` (MSVC) for x64 + ARM64. Local dev: `x86_64-pc-windows-gnu`. | No local Visual Studio. GNU + `windows` crate COM behaviour must be verified in M0. | Accepted; GNU path unverified |
| D-031 | Help → "Export drive report": redacted JSON (model, firmware, capabilities, media ID, task log; serials and paths stripped) + GitHub issue form. Results feed `docs/COMPATIBILITY.md`. | Community verification. | Accepted |
| D-032 | Amends D-010: WinISOPilot (main @ 8d15e62, C#/.NET) has no USB writer yet, so `usbwriter` is written fresh in IsoFoundry M2 as a standalone crate that WinISOPilot's Rust rewrite adopts. | Nothing to extract today. | Accepted 2026-10-30 |
| D-033 | All WinISOPilot features will be added to IsoFoundry later (Sean, 2026-10-30). See D-034…D-038. | Sean's request. | Accepted |
| D-034 | Integration model: shared Rust crates; WinISOPilot and IsoFoundry stay separate apps over one servicing engine. | One engine to test, two front ends. | Accepted |
| D-035 | Timing: WinISOPilot features ship in IsoFoundry **2.0**, after 1.0. 1.0 scope unchanged. | Keeps 1.0 fixed; servicing needs its own VM gates. | Accepted |
| D-036 | UI: one new grid button "Customize Windows ISO…" (split ▾: Download Windows ISO, Inspect image, Recipes) opening a stepper window with the WinISOPilot workspaces. | Keeps the single task grid. | Accepted |
| D-037 | Servicing backend: inbox DISM API (`dismapi.dll`) + wimlib (LGPL, dynamic) for WIM/ESD export/split; IsoFoundry's own ISO writer replaces ADK Oscdimg. Gate: when host DISM is older than the image build, servicing is blocked unless a matching ADK DISM is found. | No ADK install for most users; Microsoft requires host DISM ≥ image version. | Accepted; version gate unverified |
| D-038 | Shared crates (`usbwriter`, `if-image`, `if-hash`, Windows-image crates) live in one repo (working name `SeanShaffer88/foundry-kit`); both apps pin by tag. Fido stays an unmodified, separately licensed GPL-3.0 script file beside the app (aggregation), as WinISOPilot does today. | Avoids circular git deps between the two app repos. | Accepted (see D-040) |
| D-039 | Compressed image format: **CHD** (supersedes the CHD-vs-zstd spike in D-017). Writer implemented in `if-image`; compatibility checked against `chdman` output. | Wide emulator/archival support; zstd-ISO dropped. | Accepted 2026-10-31 |
| D-040 | Shared-crate repo name confirmed: `SeanShaffer88/foundry-kit` (D-038 accepted). Created at W0 / when `usbwriter` is first shared, not before. | Sean confirmed. | Accepted 2026-10-31 |
| D-041 | Supersedes D-032: WinISOPilot 1.0.0 (Rust) now has a USB writer (`wip-core::usb`, `wip-engine::win::usb`). M2 extracts it into `foundry-kit`'s `usbwriter` instead of writing one from scratch. | Reuse the shipped, tested code. | Accepted 2026-10-31 |
| D-042 | Dependency alignment with WinISOPilot 1.0.0 for shared engine code: `windows-sys` 0.59, `serde` 1, `toml` 0.8. The `windows` crate (COM) is added in M4 for IMAPI2. IsoFoundry's manifest stays `asInvoker` (D-014 keeps elevation in the helper). | Shared crates compile against the same versions. | Accepted 2026-10-31 |
| D-043 | Supersedes D-002's UI choice: **pure Win32 + Common Controls v6** via `windows-sys` (owner-drawn task grid, native dialogs, Per-Monitor DPI V2 manifest), static CRT. No egui/eframe. Budgets per System Directive V2.0: idle working set < 15 MB, idle CPU ~0 %, first window render < 50 ms. | Sean's System Directive V2.0; a task grid + dialogs is Win32's strength; shared crates are engine-only so WinISOPilot's egui UI is irrelevant. | Accepted 2026-10-31 |
| D-044 | Supersedes D-013: release assets `IsoFoundry-v{VERSION}-x64-Setup.exe`, `-x64-Setup.msi` (WiX), `-x64-Portable.zip`, `-arm64-Portable.zip`, PortableApps.com package, `SHA256SUMS.txt`. Portable builds write only to `data\` beside the exe. | System Directive V2.0 asset set + PAF (Sean, 2026-10-31). | Accepted 2026-10-31 |
| D-045 | UI design gate passed: main window (split buttons + 2 new rows), Burn image and Copy disc mockups approved as-is. | Sean approved 2026-10-31. | Accepted |
| D-046 | Supersedes D-022 for now: English UI strings live in one table in `if-core::strings` (keys = task/variant ids). Fluent `.ftl` files come with the first second language (1.x). | No Fluent dependency or runtime cost before a translation exists. | Accepted 2026-11-07 |
| D-047 | M0 verification without Windows hardware: Linux unit tests, `x86_64-pc-windows-gnu` cross-build, Wine + Xvfb smoke runs (CLI, helper pipe handshake, window screenshot), then CI on `windows-latest` (MSVC, x64 tests, ARM64 build, static-CRT check). Evidence is labelled Partially verified until run on Sean's Windows machine. | Honest evidence labels; Wine is not Windows. | Accepted 2026-11-07 |
| D-048 | Amends D-016: the CLI also ships as a console-subsystem twin, `isofoundry.com` (built as `isofoundry-cli.exe`). Windows tries `.com` before `.exe`, so typing `isofoundry …` in cmd or PowerShell runs the console program: the shell waits, redirection and pipes work, the exit code is real. `isofoundry.exe` stays the window. | CI showed PowerShell neither waits for nor captures a GUI-subsystem exe, which breaks scripting (Directive V2.0 §10). | Accepted 2026-11-07 |
| D-049 | Packaging tools for D-044: `Setup.exe` built with Inno Setup 6 (per-user by default, all-users optional, optional PATH entry, asks before deleting settings on uninstall); MSI built with WiX v5.0.2 (per-machine, adds to system PATH); PortableApps.com Format layout with our own `IsoFoundryPortable.exe` launcher (the app already keeps data in `Data\settings`), shipped as a zip. One script, `packaging/package.ps1`, builds every asset for CI and releases. CI installs, runs and uninstalls both installers on every push. | Inno and WiX are free, run on GitHub's Windows runners, and need nothing installed on users' machines. WiX v5 avoids the v6+ maintenance-fee EULA step. The official PortableApps.com Installer (`.paf.exe`) needs its own NSIS toolchain; deferred. | Accepted 2026-11-07 |
| D-050 | M5 read side: raw MMC over SPTI (SCSI Pass Through Direct) with hand-declared pass-through structures and CDBs (READ TOC, READ CD, READ(10) fallback), behind the same `OpticalDrive` trait as IMAPI2; audio burning on real drives uses IMAPI2 track-at-once (fixed gaps, no CD-Text), while disc-at-once with CD-Text is implemented on the simulated drive only until an MMC DAO writer is justified. AccurateRip v1/v2 checksums are computed and logged locally; the online lookup stays unwired (offline-first). Milestone versioning: M5 ships grouped with M4 in 0.5.0 and M6 becomes 0.6.0 (grouped releases). | SPTI is the only user-mode route to raw sectors/subchannel on Windows; IMAPI2 cannot read. TAO is IMAPI2's only audio write mode. | Accepted 2026-11-08 |

## D-051 — M6 shape: catalog-first backup, opt-in online, honest auto-action (2026-11-08)

**Backup is plan-first.** The whole span layout (packing, splits, SHA-256 per file and part) is computed and staged before any disc is written, so the identical catalog can ride on every span and restore never has to guess. The cost — staging space equal to the backup — is documented in LIMITATIONS rather than engineered away in v1.

**GUI/CLI split for span burning.** The `backup` command burns spans directly (blank-disc prompts between spans). The backup window writes span ISOs and restores; burning those ISOs is the existing Burn image window's job. One burning path per surface, no second prompt machinery in the window.

**Online features are per-feature opt-ins, off by default.** The update check and MusicBrainz are two separate switches (plus explicit one-shot commands/flags); the zero-telemetry rule is read as "no call the user did not choose", and each feature can be chosen independently. Both are plain HTTPS GETs over WinHTTP; the disc-ID algorithms are implemented from spec and pinned by libdiscid's published test vector.

**Auto-action opens, never starts.** Detection is exact (drive media snapshot compared on WM_DEVICECHANGE), but starting a rip/copy needs an output location, which is a user choice — so the action opens the right window. Recorded so nobody "fixes" it into unattended writes later.

**Completion shutdown is the announced kind.** InitiateSystemShutdownEx with a message and 20 s grace (cancellable), never a forced immediate power-off.

## D-052 — RC audit outcomes (2026-11-08)

- M6 and the RC pass ship grouped as **0.9.0**; 0.6.0 is never
  released separately (owner's call, fewer releases).
- The roadmap's M4 "multi-drive burn" is delivered in the command
  line (`--all-drives`), not as a window mode: multi-drive jobs
  are scripted, sequential operations.
- The roadmap's "job queue" is the parallel-task-window model:
  each task window is an independent job with its own progress
  and cancellation. No central queue is built for 1.0.
- USB → image stays command-line only for 1.0; its grid variant
  says so instead of showing a "planned" notice.

## D-053 — 1.0 promotion (2026-11-08)

- **1.0.0 is a promotion of 0.9.0, not a feature release.** Every
  1.0 roadmap row (M0–M6 + RC) is built, gated and documented; the
  1.0 version carries no feature changes over 0.9.0. The release
  workflow already marks only `v0.*` as pre-release, so 1.0.0
  publishes as the first full release with no workflow change.
- The hardware paths stay labelled **Unverified** in the docs and
  release notes: promotion asserts the feature set is complete
  and gated (simulated drive + CI), not that real-hardware field
  runs have happened.
