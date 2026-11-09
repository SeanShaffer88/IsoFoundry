# IsoFoundry 1.x Roadmap Work

## Task 1: Establishing the preserved baseline and boot-asset constraints — Done

**Description:** Verify the current workspace state and baseline checks without disturbing the existing USB-to-image work. Inspect XISO/FAT/VHD format references and identify lawful, reproducible multiboot boot assets and their Secure Boot properties.

**Acceptance criteria:**
- [x] Existing tracked and untracked changes are recorded and preserved.
- [x] Bootloader candidates, asset provenance/license obligations, and Secure Boot requirements are recorded in the feature docs or task implementation notes.
- [x] No unsupported format or boot compatibility claim is made.

**Verification:**
- [x] Run `cargo fmt --check`, `cargo test --workspace`, workspace Clippy with warnings denied, and `cargo build -p isofoundry`.
- [x] Record candidate references and terms before adding binary assets; final multiboot asset selection remains gated by Task 10.

**Dependencies:** None
**Files likely touched:** `docs/roadmap.md`, `docs/references.md` (only after evidence is established)
**Estimated scope:** Small

## Task 2: Reading and validating XDVDFS volumes — Done

**Description:** Add a pure-Rust XDVDFS reader behind the image engine's filesystem/source abstractions, including bounded directory traversal and file extent reads.

**Acceptance criteria:**
- [x] Valid XISO fixtures expose filesystem identity, directory entries, and file contents.
- [x] Invalid extents, truncated images, and unreasonable directory sizes are rejected before unbounded reads or allocation; recursive walking is bounded by the shared filesystem depth limit.
- [x] Filesystem name/label are available through the shared filesystem API, and editing XDVDFS is explicitly rejected as read-only.

**Verification:**
- [x] Add integration tests for valid, nested, empty, and malformed XDVDFS fixtures.
- [x] Run focused and complete `if-image` test suites, formatting, and Clippy.

**Dependencies:** Task 1
**Files likely touched:** `crates/if-image/src/xiso.rs`, `crates/if-image/src/fs.rs`, `crates/if-image/src/lib.rs`, `crates/if-image/tests/read.rs`
**Estimated scope:** Medium

## Task 3: Exposing XISO browse and extraction in CLI and GUI — Done

**Description:** Route XISO images through image-info, browse, and extract flows while keeping XISO-specific metadata visible and preserving existing formats.

**Acceptance criteria:**
- [x] `image-info` recognizes XISO and reports its filesystem.
- [x] CLI extraction and the existing browse/extract task window use the shared XDVDFS reader.
- [x] Unsupported image-edit operations fail with an explicit read-only message.

**Verification:**
- [x] Add CLI and integration tests that browse and extract nested XISO fixtures.
- [x] Build the app and verify that the existing GUI browse/extract task routes through the shared image API.

**Dependencies:** Task 2
**Files likely touched:** `crates/isofoundry/src/cli_image.rs`, `crates/isofoundry/src/win/tasks/browse.rs`, `crates/if-image/tests/read.rs`
**Estimated scope:** Medium

## Task 4: Creating XISO images and end-to-end coverage — Done

**Description:** Add an XDVDFS writer and an explicit XISO create path that produces compatible images without touching game executables or protected content.

**Acceptance criteria:**
- [x] A source tree can be written as XISO and reopened by the new reader.
- [x] File/directory extents, alignment, names, and size limits are validated.
- [x] Existing ISO creation and editing output remain unchanged.

**Verification:**
- [x] Round-trip tests compare every source file byte-for-byte and exercise invalid names/limits.
- [x] Document CLI and GUI availability, filesystem scope, and exclusions.

**Dependencies:** Task 2
**Files likely touched:** `crates/if-image/src/write/xiso.rs`, `crates/if-image/src/write/mod.rs`, `crates/isofoundry/src/cli_image.rs`, focused tests/docs
**Estimated scope:** Medium

## Task 5: Reading FAT volumes and locating supported partitions — Done

**Description:** Implement bounded FAT12/16/32 volume parsing and identify supported superfloppy or MBR-partitioned FAT volumes in raw disk images.

**Acceptance criteria:**
- [x] FAT12/16/32 fixtures expose directory trees, long names, file extents, and labels where available.
- [x] Partition/BPB geometry and all cluster chains are bounds-checked; unsupported/exFAT layouts are rejected clearly.
- [x] Malformed loops and inconsistent chains terminate safely.

**Verification:**
- [x] Add fixture tests across FAT types, nested directories, long names, fragmented files, and corrupt chains.
- [x] Run focused `if-image` tests and malformed-input drills.

**Dependencies:** Task 1
**Files likely touched:** `crates/if-image/src/fat.rs`, `crates/if-image/src/fs.rs`, `crates/if-image/src/lib.rs`, `crates/if-image/tests/read.rs`
**Estimated scope:** Medium

## Task 6: Browsing and extracting FAT images — Done

**Description:** Connect raw FAT floppy and partition images to the existing image-info, browse, and extraction surfaces.

**Acceptance criteria:**
- [x] CLI image-info and extraction can open supported raw FAT images.
- [x] Existing GUI browse/extract flow handles FAT paths with the same progress and cancellation behavior.
- [x] ISO/Joliet/UDF selection and behavior remain unchanged.

**Verification:**
- [x] Add CLI/integration coverage for FAT images and readback after extraction.
- [x] Build app and run all `if-image` integration tests.

**Dependencies:** Task 5
**Files likely touched:** `crates/if-image/src/container.rs`, `crates/if-image/src/fs.rs`, `crates/isofoundry/src/cli_image.rs`, `crates/isofoundry/src/win/tasks/browse.rs`
**Estimated scope:** Medium

## Task 7: Editing FAT images and saving safely

**Description:** Extend edit-image operations to supported FAT volumes and rebuild to a separate output image while preserving boot/partition metadata that the implementation can safely retain.

**Acceptance criteria:**
- [ ] Add, remove, rename, create-folder, and label operations work for FAT images.
- [ ] The source is never modified; existing-destination and source/destination identity rules are enforced.
- [ ] Unsupported boot sectors, partitions, or metadata are reported rather than silently discarded.

**Verification:**
- [ ] Add edit round-trip tests for all supported FAT types, fragmentation, collisions, cancellation, and source preservation.
- [ ] Exercise the existing CLI and GUI edit paths against raw FAT fixtures.

**Dependencies:** Task 5, Task 6
**Files likely touched:** `crates/if-image/src/fat_write.rs`, `crates/if-image/src/edit.rs`, `crates/isofoundry/src/cli_image.rs`, `crates/isofoundry/src/win/tasks/edit.rs`, focused tests
**Estimated scope:** Medium

## Task 8: Reading and writing fixed/dynamic VHD containers

**Description:** Add a container adapter for legacy VHD fixed and dynamic disks, validating footer/header checksums, BAT entries, block bitmaps, and virtual-size bounds.

**Acceptance criteria:**
- [ ] Fixed and dynamic VHD fixtures expose logical sector reads correctly.
- [ ] Invalid checksums, overlapping/out-of-range blocks, and malformed BATs fail cleanly.
- [ ] Writes produce a new VHD file and never mutate the input.

**Verification:**
- [ ] Add fixed/dynamic read-write round-trip tests plus malformed-container tests.
- [ ] Compare generated fixtures with an independent VHD parser/tool where available.

**Dependencies:** Task 1
**Files likely touched:** `crates/if-image/src/vhd.rs`, `crates/if-image/src/lib.rs`, tests
**Estimated scope:** Medium

## Task 9: Integrating VHD-contained FAT editing

**Description:** Open VHD containers through the FAT image path and save edited virtual disks while retaining container type and safe metadata.

**Acceptance criteria:**
- [ ] Fixed and dynamic VHD images containing supported FAT volumes browse, extract, and edit through app workflows.
- [ ] Successful edits reopen in IsoFoundry and an independent VHD reader.
- [ ] All failures preserve both source and any pre-existing destination.

**Verification:**
- [ ] Add end-to-end VHD/FAT edit tests, including malformed image and cancel cases.
- [ ] Run workspace tests and app build.

**Dependencies:** Task 7, Task 8
**Files likely touched:** `crates/if-image/src/container.rs`, `crates/if-image/src/edit.rs`, CLI/GUI adapters, tests
**Estimated scope:** Medium

## Task 10: Proving the multiboot boot chain and QEMU verification path

**Description:** Select bootloader assets and define the minimal supported boot profiles, asset licensing/provenance, BIOS/UEFI layout, and reproducible QEMU software-test matrix before any destructive writer changes.

**Acceptance criteria:**
- [ ] Selected boot assets have documented origin, version, license, and required notices.
- [ ] Linux ISO loopback and Windows/WinPE ISO/WIM boot approaches are demonstrated in QEMU or bounded as unsupported.
- [ ] Secure Boot claims are backed by the actual signed chain; otherwise the limitation is explicit.

**Verification:**
- [ ] Build the boot layout in a disposable disk-image fixture and boot supported profiles in QEMU.
- [ ] Review dependency/license changes before integrating assets.

**Dependencies:** Task 1
**Files likely touched:** `docs/roadmap.md`, `docs/references.md`, test scripts/fixtures, possible `CREDITS.md`
**Estimated scope:** Medium

## Task 11: Installing a multiboot layout through the guarded USB engine

**Description:** Add an install path that uses existing target assessment, helper IPC, confirmation, and development image-target gates to partition/format a multiboot device and place boot assets.

**Acceptance criteria:**
- [ ] Target safety checks apply to every GUI, CLI, and helper write route.
- [ ] Disk-image test targets require the existing explicit development flag.
- [ ] Install cancellation/errors do not report success and leave the target state diagnosable.

**Verification:**
- [ ] Add file-backed device tests for partitioning, formatting, asset placement, cancellation, and safety denials.
- [ ] Run helper protocol and USB writer suites; no physical disk is used.

**Dependencies:** Task 10
**Files likely touched:** `crates/usbwriter/src/multiboot.rs`, `crates/usbwriter/src/lib.rs`, CLI/helper route files, tests
**Estimated scope:** Medium

## Task 12: Managing ISO/WIM payloads and persistent boot options

**Description:** Add validated add/remove/list operations for supported ISO/WIM payloads and a versioned per-entry configuration that survives content updates.

**Acceptance criteria:**
- [ ] Multiple payloads are added and removed without rewriting unrelated payload data.
- [ ] Names/paths are safe and collisions are handled explicitly.
- [ ] Per-entry boot options persist across refresh/install updates and invalid config fails visibly.

**Verification:**
- [ ] Add config round-trip, path-safety, collision, and payload manifest tests.
- [ ] Boot at least one Linux ISO and one supported Windows/WinPE path under QEMU.

**Dependencies:** Task 10, Task 11
**Files likely touched:** `crates/usbwriter/src/multiboot.rs`, `crates/if-core/src/settings.rs` or a dedicated config module, tests
**Estimated scope:** Medium

## Task 13: Integrating multiboot management in the app and CLI

**Description:** Expose multiboot install/manage operations through a dedicated task window and documented CLI commands using existing progress, cancellation, confirmation, and helper patterns.

**Acceptance criteria:**
- [ ] Users can inspect a target, install/update the boot menu, add/remove payloads, and review per-entry options.
- [ ] CLI supports scripted list/add/remove/update flows with stable exit behavior.
- [ ] Dangerous operations display the target identity and require the established confirmation flow.

**Verification:**
- [ ] Add CLI parser/behavior tests and GUI adapter tests where the repository supports them.
- [ ] Build the Windows app/helper and exercise complete workflows against a disposable disk image.

**Dependencies:** Task 11, Task 12
**Files likely touched:** `crates/isofoundry/src/cli_usb.rs`, `crates/isofoundry/src/win/tasks`, `crates/isofoundry/src/win/gui.rs`, help/docs
**Estimated scope:** Medium

## Task 14: Verifying and documenting multiboot scenarios

**Description:** Complete QEMU scenarios for supported BIOS/UEFI combinations and document compatibility boundaries, Secure Boot behavior, and recovery/update semantics.

**Acceptance criteria:**
- [ ] Each claimed boot profile has an automated or repeatable QEMU test.
- [ ] Unsupported ISO/WIM layouts and Secure Boot cases are explicitly described.
- [ ] Physical hardware remains labeled unverified and is not required for completion.

**Verification:**
- [ ] Run the documented QEMU matrix and USB file-target suite.
- [ ] Review README, LIMITATIONS, TESTING, and roadmap statements against results.

**Dependencies:** Task 13
**Files likely touched:** `docs/TESTING.md`, `docs/LIMITATIONS.md`, `docs/roadmap.md`, `README.md`, `CHANGELOG.md`
**Estimated scope:** Medium

## Task 15: Adding locale resources and language selection

**Description:** Add a resource-based string lookup with English and Spanish catalogs, English fallback, and an explicit language setting integrated with existing settings.

**Acceptance criteria:**
- [ ] GUI strings resolve through a stable typed/keyed lookup instead of per-window literals.
- [ ] Users can choose English or Spanish; invalid/missing keys fall back to English and are detectable in tests.
- [ ] Default behavior remains English unless existing product language-selection conventions establish a locale-based default.

**Verification:**
- [ ] Add locale selection, fallback, settings migration, and missing-key tests.
- [ ] Confirm existing task/window strings remain English before the translation sweep.

**Dependencies:** Tasks 3, 6, 9, 13
**Files likely touched:** `crates/if-core/src/strings.rs`, settings, resource files, `Cargo.toml`/`Cargo.lock` if needed
**Estimated scope:** Medium

## Task 16: Localizing shared menus, task catalog, and core dialogs

**Description:** Move the main window, task catalog, menu, common control, and shared confirmation/error strings to the resource catalogs and provide Spanish translations.

**Acceptance criteria:**
- [ ] Main window, task grid, menus, shared dialogs and common status text are localized.
- [ ] Accessibility names/tooltips are localized along with visible labels.
- [ ] English and Spanish catalogs have matching keys.

**Verification:**
- [ ] Add catalog parity tests and exercise menu/task selection in both locales.
- [ ] Build the Windows app and inspect key dialog text through UI automation or accessibility tree.

**Dependencies:** Task 15
**Files likely touched:** `crates/if-core/src/strings.rs`, main GUI, task catalog, resource files, tests
**Estimated scope:** Medium

## Task 17: Localizing task windows in bounded groups

**Description:** Migrate all task-window labels, prompts, progress/status messages, errors, and confirmation text to keyed resources in small batches.

**Acceptance criteria:**
- [ ] Every existing and newly added task window has Spanish-visible user-facing text.
- [ ] Technical identifiers, paths, and media names remain intact and safely formatted.
- [ ] No task window silently falls back due to a missing Spanish key.

**Verification:**
- [ ] Migrate and test no more than a few windows per change batch.
- [ ] Run string coverage tests after each batch; finish with both-locale workflow smoke tests.

**Dependencies:** Task 16
**Files likely touched:** task-window modules in groups, resource catalogs, focused tests
**Estimated scope:** Medium per batch

## Task 18: Enforcing localization completeness and final acceptance

**Description:** Add a repository test that enumerates all GUI string keys and ensures Spanish coverage, then align release docs and perform a final full validation.

**Acceptance criteria:**
- [ ] All catalog keys required by GUI code are present in both locales.
- [ ] README, roadmap, limitations, changelog, and compatibility statements reflect actual support and test evidence.
- [ ] No physical-hardware verification is claimed.

**Verification:**
- [ ] Run `cargo fmt --check`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo build -p isofoundry`.
- [ ] Run the QEMU matrix and all software-only format/USB integration tests.

**Dependencies:** Tasks 4, 9, 14, 17
**Files likely touched:** localization tests, README/docs, changelog, relevant feature tests
**Estimated scope:** Medium
