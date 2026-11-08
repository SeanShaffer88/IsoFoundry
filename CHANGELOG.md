# Changelog

All notable changes are listed here. Versions follow [Semantic Versioning](https://semver.org/).

## [0.9.0] - 2026-11-08

### Added — RC pass: audit, failure drills, accessibility, measured budgets

- **Failure-drill test suite**: hostile media (oversize images, non-blank discs, failed verifies), damaged discs (zero-filled bad sectors, surface damage maps), tampered and incomplete backups, malformed image files and cancelled jobs — all must fail cleanly, and now do so under test.
- **Multi-drive burn** from the command line: the burn commands accept `--all-drives` and write the same source to every drive in turn, reporting each drive's result.
- **Measured performance budgets** (docs/BENCHMARK.md), an **accessibility statement** (docs/ACCESSIBILITY.md), a **capability matrix** (docs/PARITY.md) and a **quality scorecard** (QUALITY.md).

### Added — M6 integration: spanning backup, shell integration, opt-in online lookups

- **Spanning backup and restore** (`backup`, `backup-restore`, plus a task window with a restore variant). A folder is packed onto as many discs as it needs: files in name order, greedy packing, and any file larger than a span is split into `.ifpartNNN` chunks. The full plan is computed before the first disc is written, so an identical catalog (`ISOFBACKUP/CATALOG.JSON`, with a SHA-256 per file and per part) rides on every span next to a span marker. Backup writes one ISO per span (`--out-dir`), burns each span to a drive (prompting for the next blank), or rehearses against the simulated drive (`--fake`). Restore takes span folders or span ISOs in any order, names any missing span, joins split files and verifies every checksum.
- **Completion actions that now actually fire**: the sound on finish (a setting since M0 that nothing consumed until now), an optional notification balloon, and an optional shutdown after a successful task (announced, 20-second grace, cancellable with `shutdown /a`). The backup command has `--shutdown-when-done` too.
- **Auto-action on disc insert** (Options menu, off by default): a newly inserted data disc can open Copy disc to image, an audio CD can open Rip audio CD, or both. The job itself still starts with one click — its output folder is a per-job choice.
- **Shell completion** (`completions powershell` / `completions bash`): scripts generated from the live command list, so they cannot drift.
- **Explorer verbs** (`explorer-verbs install|uninstall|status`): per-user right-click entries (Checksum, Image info) for image files, no administrator rights, removed cleanly by the uninstall action.
- **Opt-in update check** (`check-update`, or on start from the Options menu): asks GitHub for the newest release and says so; off by default and never done silently — IsoFoundry still makes no network calls the user did not ask for.
- **MusicBrainz disc lookup** (`rip --musicbrainz`, a checkbox in the Rip window, an Options-menu default): disc IDs (MusicBrainz and freedb/CDDB) are computed for every audio rip and written into the rip log along with the lookup page; with the opt-in lookup the matching release's track titles name the ripped files. Rips stay fully offline when it is off.
- The M5 tasks now show as available in the task grid and `list` (they were built in 0.5.0 but still labelled as planned there, so the grid showed a "ships later" note instead of opening their windows; the command line was unaffected).

### Fixed

- Keyboard navigation (Tab, arrow keys, access keys) did not work inside task windows — only the main window processed them. It now works in every window.
- The `report` command listed no drives and carried a stale note; it now reports the drives the drive layer finds.
- `--help` printed an empty "not available yet" section once every task had shipped; the section only appears when it has entries.
- The Create bootable USB variant menu pointed both variants at a "planned" notice: image → USB now opens the window that does it, and USB → image says plainly that it works from the command line today (`usb read`).

## [0.5.0] - 2026-11-08

### Added
- M5 ripping and copying: **Copy disc to image**, **Copy disc**, **Rip audio CD**, **Burn audio CD**, **Surface test** and **Recover files** — a raw MMC/SPTI read side in `if-drive` (READ TOC / READ CD with a READ(10) fallback, subchannel capture) behind the same drive trait, plus rip/copy/scan engines: disc → image (ISO or raw BIN+CUE) with a dump report and a `.dat` hash file, disc → disc, secure audio ripping (every sector read twice and compared) to WAV/FLAC/MP3 with AccurateRip checksums in the rip log, audio CD burning (disc-at-once with CD-Text on the simulated drive; track-at-once through IMAPI2), a surface/quality scan with a grade, full table-of-contents drive info, and tolerant recovery that zero-fills unreadable sectors and reports which files they damaged. The simulated drive grew a read side (loaded data/audio discs, a damage map) so every flow runs with `--fake`; the MMC/SPTI path is Unverified on real hardware.
- M4 burning: **Burn image**, **Burn files**, **Burn MP3 CD/DVD**, **Burn video folder**, **Erase disc** and **Drive info** — a new `if-drive` engine with an IMAPI2 backend on Windows (drive/media info, image and files burns with multisession append, erase) and a simulated drive with real job semantics (capacity and blank checks, SHA-256 verify after burn) that every flow can run against with `--fake`. Task windows and commands (`burn-image`, `burn-files`, `burn-mp3`, `burn-video`, `erase`, `info`) throughout. The IMAPI2 path is Unverified on real hardware (this milestone's test gate is the simulated drive); a disc burned through IMAPI can be read back and compared through the M5 rip side below.

## [0.4.0] - 2026-11-08

### Added
- M3 audio: **Convert audio** — a new `if-audio` engine (Symphonia decoding; WAV and FLAC encoding natively; MP3 through `libmp3lame.dll` loaded at runtime, never bundled; AAC/M4A through Windows Media Foundation), a batch task window and a `convert-audio` command. Tags are carried across conversions, and ReplayGain can be measured into the output's tags or baked into the samples. Opus/Ogg Vorbis encoding is not offered yet (no permissive pure-Rust encoder exists); decoding them works.
- M2 USB: **Create bootable USB** and **Install Windows to USB** — a new `usbwriter` engine (MBR/GPT partitioning, FAT32 formatting, ISO file copy, raw image write with optional SHA-256 verify, USB → image), task windows with disk picking and a typed ERASE confirmation for targets with mounted volumes or over 64 GB, `usb list` / `usb write` / `usb read` and `windows-to-usb` commands, and helper protocol v2 for the elevated disk work. Windows installer layouts: MBR+FAT32, GPT+FAT32 and GPT+NTFS (for install images over the FAT32 file limit).

### Fixed
- Task windows: checkboxes, radio buttons and the progress bar now follow the dark control theme — their labels rendered in a low-contrast gray on the dark background and the progress bar painted as a bright strip.

## [0.2.0] - 2026-11-07

### Changed
- Release shape: releases now carry only the x64 Setup.exe installer and the x64 Portable.zip, plus SHA256SUMS.txt — the WiX MSI, the ARM64 builds and the PortableApps.com zips are dropped (one installer and one portable zip per release). The release workflow builds a single x64 leg, and `packaging/package.ps1` no longer produces the removed assets. The published v0.1.0 assets were pruned to the same pair.

### Added
- Task windows (GUI): the five M1 tasks now open working windows from the task grid — Create image (files/folders, label, optional BIOS/UEFI boot images with boot info table, Joliet/UDF toggles), Browse image (image info, contents list, extract all or a selected file/folder), Edit image (stage add/remove/rename/new folder/label, then save as a new image; the original is never modified), Convert image (ISO, BIN/CUE, CloneCD, CHD, CHD-CD) and Checksum (CRC32/MD5/SHA-1/SHA-256, optional `.dat` write, or verify a file against a `.dat` list). Jobs run on a worker thread with a progress bar, byte counts and Cancel, using the same operations as the CLI; closing a window mid-job cancels it safely.
- Image reading (`if-image`): ISO 9660 with Rock Ridge names, Joliet, UDF 1.02–2.60 (physical, sparable and metadata partitions), multi-extent files, El Torito boot catalog (BIOS and UEFI entries). Extraction keeps nothing half-written on cancel and renames names Windows cannot store.
- Hashing (`if-hash`): CRC32, MD5, SHA-1 and SHA-256 in one pass; Logiqx/redump `.dat` writing and reading.
- ISO creation (`if-image::write`, `create-image`): ISO 9660 + Joliet + UDF 1.02 bridge sharing one copy of the data, files over 4 GB (multi-extent / split UDF extents), El Torito BIOS (optional boot information table) and UEFI boot entries; written to a `.part` file and renamed when complete.
- Commands: `image-info`, `extract`, `create-image`, `checksum` (with `--dat`), `compare`, `verify --dat`, `convert-image`.
- CD image formats: BIN/CUE (single and multi-file, MODE1/MODE2/AUDIO), CloneCD CCD/IMG/SUB, sheetless raw `.bin`/`.img`; Nero NRG and Alcohol MDS/MDF read only and marked unverified. Raw sector ECC/EDC generation and checks, Q-subchannel generation.
- MAME CHD v5 (D-039): CD (`cdzl`) and DVD (`zlib`) writing with duplicate-hunk sharing and the compressed hunk map; reading `zlib`/`cdzl` CHDs from chdman. Output matches chdman 0.264's SHA-1s byte for byte on the test images.
- Image editing (`edit-image`): add, remove, rename, new folders and label, saved as a new image; untouched files are copied straight from the source (ISO, BIN/CUE, CCD, CHD…), BIOS and UEFI boot entries kept.
- Mounting (`mount`, `unmount`) with Windows' built-in virtual disk support, read only; other formats go through a temporary ISO in the data folder, removed on unmount.
- Image conversion (`convert-image`): any readable image to ISO, BIN/CUE, CloneCD or CHD (CD or DVD layout). `image-info` now shows the container format and tracks.

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
