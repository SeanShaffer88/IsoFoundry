# Implementation Plan: IsoFoundry Capability Expansion

## Overview

Deliver the user's requested disc, image, archive, and USB capabilities in tested vertical slices, guided by the approved [capability map](../CAPABILITY-MAP.md). Preserve completed XISO/FAT work and the in-progress USB-to-image changes. The next implementation slice is fixed/dynamic VHD support for FAT12/16/32. The expanded catalog is staged; no feature is called complete until its user-facing path, focused tests, documentation, and software-only verification are in place.

## Scope and assumptions

- The first `image-foundation` module spec is approved: fixed and dynamic VHD 1.0 containing FAT12/16/32, using the existing image engine and GUI/CLI paths. VHDX editing, differencing chains, and other filesystems are outside this module.
- The previously approved XISO, FAT/VHD, multiboot USB, and Spanish UI work remains in scope.
- Hardware tests are not required. Use fixtures, simulated or image-backed devices, and QEMU where applicable; clearly mark hardware-dependent behavior unverified.
- Do not implement copy-protection circumvention or claim support for encrypted commercial media.
- Formatting, disc erasure, and raw writes require explicit target identification, proportionate confirmation, cancellation/error handling, and safe output behavior.
- New dependencies, broad format support, and OS-dependent operations require evidence and a per-module spec before implementation.

## Architecture decisions

- Keep parsing, filesystem integration, and image editing in the platform-independent `if-image` crate; GUI and CLI use the shared implementation.
- Validate all VHD offsets, checksums, block maps, lengths, and arithmetic before exposing a filesystem or allocating based on untrusted metadata.
- Always write edits to a separate destination; never modify a source image in place or overwrite an existing destination.
- Keep USB writes behind the existing guarded USB/helper path. Preserve the existing USB-to-image worktree changes and `.serena/` data.
- The approved capability map defines module boundaries and dependency order. Each remaining module receives its own spec and task breakdown before implementation.

## Task List

### Phase 0: Preserved baseline
- [x] Task 1: Record the existing workspace state and establish repository/asset constraints.

### Phase 1: Xbox XISO
- [x] Task 2: Read and validate XDVDFS volumes.
- [x] Task 3: Expose XISO browse and extraction in CLI and GUI.
- [x] Task 4: Create XISO images and add end-to-end coverage.

### Phase 2: Image foundation — FAT and VHD
- [x] Task 5: Read FAT12/16/32 volumes and locate supported partitions.
- [x] Task 6: Browse and extract FAT images through existing image workflows.
- [x] Task 7: Edit FAT images and save safely as a new image.
- [x] Task 8: Parse fixed/dynamic VHD containers and expose bounded logical reads.
- [x] Task 9: Write fixed/dynamic VHD containers safely to new files.
- [x] Task 10: Browse and extract FAT volumes inside supported VHDs.
- [x] Task 11: Edit FAT volumes inside VHDs through CLI and GUI.
- [x] Task 12: Document VHD support and limits.

### Checkpoint: Image foundation
- [x] Fixed/dynamic VHD fixtures with supported FAT volumes browse, extract, edit, and reopen correctly.
- [x] Malformed/unsupported inputs fail cleanly; source images remain unchanged and destination collisions are preserved.
- [x] Focused `if-image` and app tests, Clippy, formatting, and app build pass.
- [x] README and limitations document VHD variants and exclusions.

### Phase 3: USB creation and multiboot
- [ ] Task 13: Prove the multiboot boot chain, asset terms, and QEMU test path.
- [ ] Task 14: Install multiboot layouts through the guarded USB engine.
- [ ] Task 15: Manage ISO/WIM payloads and persistent boot options.
- [ ] Task 16: Integrate multiboot management in the Windows app and CLI.
- [ ] Task 17: Verify and document supported multiboot scenarios.

### Checkpoint: USB creation
- [ ] File-backed target install/manage flows pass safety, cancellation, and recovery tests.
- [ ] Supported BIOS/UEFI scenarios boot in QEMU; hardware behavior remains explicitly unverified.
- [ ] Secure Boot claims reflect the verified boot chain and asset terms.

### Phase 4: Remaining image and media capabilities

After image foundation and USB creation, specify and implement the approved-map modules in dependency order: `image-operations`, `archive-formats`, `optical-media`, and `virtual-mounting`; then `media-conversion` (which depends on optical media). This scope includes practical image create/extract/edit/convert/verify operations, useful supported compressed/container formats, supported optical read/write workflows, virtual mounting, and unprotected-media extraction/conversion. Each module gets its own reviewed spec and focused task list before implementation; format breadth and OS-dependent behavior are evidence-gated.

### Phase 5: Workflow surfaces and localization
- [ ] Task 18: Add locale resources and language selection with English fallback.
- [ ] Task 19: Localize shared menus, task catalog, and core dialogs.
- [ ] Task 20: Localize task windows in bounded groups.
- [ ] Task 21: Enforce localization completeness and final acceptance.

CLI/scripting, batch queues, portable operation, diagnostics, and capability/limitation documentation are part of `workflow-surfaces`. Their scope and dependencies will be specified after the underlying feature modules exist, so the UI/catalog coverage is complete rather than partial.

### Checkpoint: Complete
- [ ] Every approved module is end-to-end implemented, focused-tested, documented, and accurately bounded.
- [ ] Workspace formatting, tests, Clippy, and application build pass.
- [ ] No physical-hardware verification is claimed.

## Risks and mitigations

| Risk | Impact | Mitigation |
|---|---|---|
| Dynamic VHD metadata or FAT layout edge cases can cause unsafe reads or corruption. | High | Use bounded parsing and checked arithmetic; test malformed metadata; write only to a separate atomic output. |
| Multiboot assets may have restrictive distribution terms or key-enrollment constraints. | High | Validate provenance/license and the actual trust chain before choosing assets; avoid unsupported Secure Boot claims. |
| Loopback boot support varies among Linux and Windows images. | High | Define a tested compatibility set and document unsupported payloads rather than promise universal booting. |
| Destructive disk or optical operations can erase user data. | High | Preserve existing target safety checks and require clear target identity and proportionate confirmation on every route. |
| The feature catalog includes proprietary formats, protected media, hardware-specific operations, and platform-specific APIs. | High | Exclude copy-protection circumvention; implement only formats and operations with reliable evidence; document host/hardware limits. |
| Existing USB-to-image changes and `.serena/` are uncommitted. | High | Preserve them; do not reset, overwrite, or stage them with unrelated feature commits. |

## Open questions

- No blocking questions for the approved VHD module. Per-module specifications remain approval-gated as described in `CAPABILITY-MAP.md`.
