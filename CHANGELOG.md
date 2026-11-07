# Changelog

All notable changes are listed here. Versions follow [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added
- Image reading (`if-image`): ISO 9660 with Rock Ridge names, Joliet, UDF 1.02–2.60 (physical, sparable and metadata partitions), multi-extent files, El Torito boot catalog (BIOS and UEFI entries). Extraction keeps nothing half-written on cancel and renames names Windows cannot store.
- Hashing (`if-hash`): CRC32, MD5, SHA-1 and SHA-256 in one pass; Logiqx/redump `.dat` writing and reading.
- ISO creation (`if-image::write`, `create-image`): ISO 9660 + Joliet + UDF 1.02 bridge sharing one copy of the data, files over 4 GB (multi-extent / split UDF extents), El Torito BIOS (optional boot information table) and UEFI boot entries; written to a `.part` file and renamed when complete.
- Commands: `image-info`, `extract`, `create-image`, `checksum` (with `--dat`), `compare`, `verify --dat`.

## [0.1.0] - 2026-11-07

M0 shell: the window, command line and elevated helper. No disc, image, audio or USB task runs yet; each one says which version brings it.

### Added
- M0 shell: Win32 main window with the 21-task grid (owner-drawn, split-button menus, shield marks on administrator tasks), tooltips on every control, keyboard navigation (Tab, arrow keys, Enter, Alt+Down), visible focus ring, high-contrast palette, Per-Monitor DPI v2, dark title bar, saved and validated window placement, single instance.
- Menus: Task (every task), Options (log detail, finish sound, open data folder), Help (roadmap, command line, log viewer, drive report export, about).
- Command line, also as the console twin `isofoundry.com` so shells wait for it and capture output: `--version`, `list`, `helper-check [--elevated]`, `report --out FILE`, `clean`; every task name answers `not_implemented` (exit 40) until its milestone ships; `--json`, `--dry-run`, `--verbose/--debug`.
- Elevated helper (`isofoundry-helper.exe`) with a named-pipe handshake: user/Administrators/SYSTEM-only pipe, client process id check, per-session nonce, typed messages only.
- Settings (versioned TOML, atomic writes, quarantine of unreadable files), local logs, portable mode (`IsoFoundry.portable` → `data\` beside the exe).
- Release assets: `IsoFoundry-v0.1.0-x64-Setup.exe` (Inno Setup, per-user or all users, optional PATH), `-x64-Setup.msi` (WiX), x64 and ARM64 portable zips, x64 and ARM64 PortableApps.com Format zips (`IsoFoundryPortable.exe` launcher), `SHA256SUMS.txt`.
- CI: Windows MSVC x64 (fmt, clippy, tests, CLI smoke test, static-CRT check) and ARM64 build; every release asset is built, and both installers are installed, run and uninstalled.

### Changed
- Compressed image format decided: CHD (D-039).
- Shared-crate repo name confirmed: `foundry-kit` (D-040).

## [0.0.2] - 2026-10-30

Planning release: no application code yet.

### Added
- docs/ARCHITECTURE.md: process model (unelevated app + elevated helper), crates, drive layer (IMAPI2 / MMC / fake drive), job safety and recovery, CLI, performance budgets.
- docs/DECISIONS.md: decision log D-001…D-038.
- docs/LIMITATIONS.md, docs/TESTING.md (fake drive, VHDX on Windows Home, QEMU boot), docs/COMPATIBILITY.md, docs/design-tokens.md (measured contrast).
- Roadmap: milestones M0–M6, RC, 1.0, 1.x; 2.0 WinISOPilot integration (W0–W7); existing idea items mapped to milestones.
- Icon and logo (`assets/`), drive report issue form.
- README feature list; planned third-party components in CREDITS; planning audit findings in AUDIT.

### Changed
- Project parked until Win10Pilot is complete.

## [0.0.1] - 2026-10-30

Planning release: no application code yet.

### Added
- docs/references.md: curated primary sources (Windows deployment, disc formats, boot, security, testing).
- Roadmap: auto-action on disc insert (ARM), image editing (TotalImage), preservation dumping UI (Redumper GUI), optional Xbox XISO support (extract-xiso); console decryption tools listed as excluded.
- Release workflow (publishes a GitHub release from a `release/v*` branch).
- README, roadmap (including disc-imaging ideas from DiscImageCreator; USB layouts from windows2usb; multiboot USB from MultiOS-USB/GLIM; Lufus as UI reference), description, audit and credits.
