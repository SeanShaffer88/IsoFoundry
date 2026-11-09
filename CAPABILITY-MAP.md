# Capability Map: IsoFoundry

This map expands the existing 1.x roadmap to cover the user's disc, image, archive, and USB feature catalog. It is an index and build-order proposal, not a promise that every proprietary format, device, or hardware workflow can be supported. Existing features and in-progress work remain in place.

| Module id | Responsibility | Depends on |
|---|---|---|
| `image-foundation` | Shared image I/O, filesystem/container support, safe editing, integrity primitives, and VHD support. Builds on ISO/UDF, XISO, and FAT work already in the repository. | — |
| `image-operations` | Create images from files or readable media; browse and extract; edit; convert; compare and verify; sector-level inspection/editing where safe and format-defined. | `image-foundation` |
| `archive-formats` | Add practical compressed, split, encrypted, and additional image/container formats with explicit capability and compatibility reporting. | `image-foundation` |
| `usb-creation` | Format and create bootable USB media, BIOS/UEFI layouts, multiboot menus, write verification, and protections against selecting system or otherwise unsafe disks. | `image-foundation` |
| `optical-media` | Read and write supported optical media, including data-disc authoring, image-to-disc, disc-to-image, duplication, rewritable-media erase, and applicable write/media diagnostics. | `image-foundation` |
| `media-conversion` | Extract audio from unprotected media and convert supported user-provided or readable audio/video content using legally distributable codecs. | `image-foundation`, `optical-media` |
| `virtual-mounting` | Mount, browse, and unmount supported images through operating-system virtual-drive facilities, including multiple mounts where the OS supports them. | `image-foundation` |
| `workflow-surfaces` | Consistent GUI and CLI workflows, scripting and batch queues, portable operation, localization (including Spanish), and user-facing capability/limitation documentation. | `image-operations`, `archive-formats`, `usb-creation`, `optical-media`, `media-conversion`, `virtual-mounting` |

Build order: `image-foundation` → `image-operations`, `archive-formats`, `usb-creation`, `optical-media`, and `virtual-mounting` → `media-conversion` → `workflow-surfaces`. The existing FAT editor and USB-to-image work remain part of the current progress; the previously approved VHD, multiboot USB, and Spanish UI roadmap work is retained.

## Boundaries and delivery rules

- Do not implement copy-protection circumvention or claim support for encrypted commercial media.
- Treat formatting, disc erasure, and raw writes as destructive operations: require clear target identification, proportionate confirmation, cancellation/error handling, and safe output behavior.
- Hardware testing is not a completion gate. Verify with fixtures, simulated/image-backed devices, and software integration tests; document hardware-dependent behavior as unverified where appropriate.
- Prefer formats and operations with reliable specifications, existing project architecture, and redistributable dependencies. Report unsupported formats or OS-dependent behavior rather than claiming universal compatibility.
- Deliver one end-to-end vertical slice at a time, with focused tests and user documentation. A module is not complete merely because one code path exists.
