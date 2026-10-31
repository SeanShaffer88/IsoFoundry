# Decision log

Format: `D-NNN — decision — why — status`. Newest at the bottom. Changing a decision appends a new entry that supersedes the old one; entries are never edited away.

| ID | Decision | Why | Status |
|---|---|---|---|
| D-001 | Name: **IsoFoundry**. Tagline "Open Source Disc Burning, Imaging & Bootable USB Suite". | Distinct, non-infringing name. | Accepted 2026-10-30 |
| D-002 | Rust (stable) + `windows` crate (COM) + `egui`/`eframe`. | Shares the WinISOPilot stack and theme; fastest path to ~25 task dialogs. | Accepted |
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
| D-013 | Distribution: portable zip, PortableApps.com Format package, Inno Setup installer. | No Visual Studio needed; covers portable and installed users. | Accepted |
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
