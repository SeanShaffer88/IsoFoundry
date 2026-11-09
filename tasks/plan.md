# Implementation Plan: IsoFoundry 1.x Roadmap Features

## Overview

Complete the four explicitly selected 1.x roadmap features: multiboot USB, FAT/VHD image editing, Xbox XISO, and Spanish UI. Keep existing product behavior and the in-progress USB-to-image GUI changes intact. Deliver each feature through the appropriate GUI and CLI surfaces, with fixtures, focused tests, documentation, and software-only boot verification. Do not claim physical-hardware verification.

## Scope and assumptions

- Multiboot USB supports multiple Linux ISO loopback entries and Windows/WinPE ISO or WIM boot entries on BIOS and UEFI. The app manages installation, adding/removing payloads, and per-entry options. Secure Boot support must use distributable signed components or document a clearly bounded limitation; users must not be told to disable Secure Boot as the default path.
- FAT image editing supports FAT12/16/32 floppy and partition images. VHD support means legacy fixed and dynamic VHD containers containing supported FAT volumes. VHDX support is not included unless existing formats or implementation evidence require it.
- Xbox XISO means XDVDFS filesystem support for opening, listing, extracting, and creating images. No executable patching, decryption, or copy-protection circumvention.
- Spanish UI covers all user-facing GUI strings, including dialogs, menus, errors and task windows. CLI output and technical documentation remain English unless needed to explain language selection.
- Physical hardware tests are excluded. Unit/integration fixtures and QEMU boot checks remain required where applicable.

## Architecture decisions

- Extend `if-image`'s filesystem/source abstractions for FAT and XDVDFS rather than introducing parallel app-only parsers. Keep format writers pure Rust and platform-independent where practical.
- VHD parsing/writing is a container layer over byte-addressable storage; validate bounds, checksums, block maps and metadata before exposing a filesystem.
- Multiboot USB writes continue through the existing guarded USB device/helper path. Disk-image/VHDX test targets stay behind the current explicit development gate.
- Preserve the app's offline-by-default behavior. Any bootloader assets must have documented provenance and redistribution terms; no runtime download is required for ordinary use.
- Use a Spanish resource catalog with English fallback, explicit language selection, and completeness tests. Avoid scattering translated literals through Win32 window procedures.

## Task List

### Phase 0: Baseline and format investigation
- [ ] Task 1: Establish preserved baseline and validate third-party boot assets.

### Checkpoint: Baseline
- [ ] Existing working-tree changes are preserved and baseline workspace checks pass.
- [ ] XISO, FAT/VHD, and multiboot format/asset decisions are supported by repository evidence and documented references.

### Phase 1: Xbox XISO
- [ ] Task 2: Read and validate XDVDFS volumes.
- [ ] Task 3: Expose XISO browse and extraction in CLI and GUI.
- [ ] Task 4: Create XISO images and add end-to-end coverage.

### Checkpoint: XISO
- [ ] Fixture images list, extract and recreate files with byte-for-byte verification.
- [ ] CLI and GUI workflows are documented and tested without physical hardware.

### Phase 2: FAT and VHD image editing
- [ ] Task 5: Read FAT12/16/32 volumes and locate supported partitions.
- [ ] Task 6: Browse and extract FAT images through existing image workflows.
- [ ] Task 7: Edit FAT images and save safely as a new image.
- [ ] Task 8: Read and write fixed/dynamic VHD containers with validated metadata.
- [ ] Task 9: Integrate VHD-contained FAT editing and verify round trips.

### Checkpoint: FAT/VHD
- [ ] FAT12/16/32 fixture images and fixed/dynamic VHD fixtures browse, extract, edit and reopen correctly.
- [ ] Original images remain unchanged; malformed and oversized inputs fail cleanly.

### Phase 3: Multiboot USB
- [ ] Task 10: Prove the bootloader/asset path and reproducible BIOS/UEFI QEMU test setup.
- [ ] Task 11: Install a multiboot layout through the guarded USB engine.
- [ ] Task 12: Manage ISO/WIM payloads and persistent per-entry boot options.
- [ ] Task 13: Integrate multiboot management in the Windows task window and CLI.
- [ ] Task 14: Verify boot scenarios and document supported payloads and Secure Boot status.

### Checkpoint: Multiboot
- [ ] VHDX-backed install/manage operations pass safety tests; supported BIOS and UEFI scenarios boot under QEMU.
- [ ] No destructive physical-device operation is used for testing; Secure Boot claims match the verified asset chain.

### Phase 4: Spanish UI
- [ ] Task 15: Add locale/resource loading and language selection with English fallback.
- [ ] Task 16: Localize shared menus, task catalog, and core dialogs.
- [ ] Task 17: Localize task windows and operational messages in bounded groups.
- [ ] Task 18: Enforce catalog completeness and verify the Spanish UI workflow.

### Checkpoint: Spanish UI
- [ ] Every user-facing GUI string has an English source and Spanish translation.
- [ ] English remains the default/fallback; both locales render through the same GUI paths.

### Checkpoint: Complete
- [ ] All four selected feature acceptance criteria are met.
- [ ] Workspace formatting, tests, Clippy, and application build pass.
- [ ] README, roadmap, limitations, and changelog accurately describe support and verification.

## Risks and mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| Secure Boot-compatible multiboot assets may have restrictive distribution terms or key-enrollment constraints. | High | Validate provenance/license and the actual trust chain before choosing the bootloader; do not make unsupported Secure Boot claims. |
| Dynamic VHD and FAT partition edge cases can cause corruption or out-of-bounds reads. | High | Keep parsers read-only until validation is complete; test malformed metadata, bounds, and atomic new-file writes. |
| Booting arbitrary Linux ISOs by loopback is not universal. | High | Define a tested compatibility set and expose/document per-entry options rather than claiming every ISO works. |
| XDVDFS and FAT name/metadata semantics differ from ISO editing. | Medium | Preserve raw names and metadata where feasible; add adversarial fixtures and explicit unsupported-feature errors. |
| UI strings are spread across many Win32 task windows. | High | Centralize string lookup and migrate windows in small groups with a coverage test preventing untranslated literals. |
| Existing USB-to-image changes are uncommitted in this worktree. | High | Treat them and `.serena/` as user/worktree state; do not reset, overwrite, or clean them up. |

## Open questions

- None blocking at planning time. The plan uses the supported-format and Secure Boot assumptions above; revise only if format research disproves them.
