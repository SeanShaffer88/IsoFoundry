# Spec: Image Foundation — VHD Support

**Status:** Implemented and documented in the current source.

## Objective

Complete the next `image-foundation` increment: support fixed and dynamic VHD containers for FAT12/16/32 image workflows, including inspect, browse, extract, and safe edit-to-new-image. This extends the existing ISO/XISO/FAT image engine and the already implemented FAT editor. It does not replace or discard existing USB-to-image work.

The primary user is someone managing legacy disk images who needs to inspect and edit supported FAT volumes inside VHD files without writing to the original. Success means the application can recognize valid supported VHDs, expose the contained FAT filesystem through existing workflows, and safely save edits as a separate VHD that reopens with byte-correct file contents.

## Assumptions

1. The accepted container scope is fixed and dynamic VHD (VHD 1.0), with FAT12/16/32 volumes; differencing VHD chains, VHDX image editing, and filesystems other than supported FAT remain unsupported.
2. The existing GUI and CLI image workflows should use the shared `if-image` implementation rather than gain separate parsers.
3. Source images must remain unchanged; edits write to a new destination and reject an existing destination using the established project behavior.
4. Physical hardware tests are not required. VHD fixture tests and software-backed integration tests are the verification gate; unsupported host-dependent behavior must be documented.
5. No new dependency is needed unless repository evidence shows the existing Rust implementation cannot safely meet the format requirements; adding one requires approval.

## Tech Stack

- Rust 2021, minimum supported Rust 1.82.
- Workspace crates: `if-image` for format parsing and editing; `isofoundry` for CLI/GUI integration.
- Current `if-image` dependencies include `miniz_oxide` and `sha1`. Preserve the small-dependency policy.
- Windows desktop application with Win32 UI and CLI; format logic remains platform-independent where practical.

## Commands

- Format: `cargo fmt --check`
- Image tests: `cargo test -p if-image`
- App/CLI tests: `cargo test -p isofoundry`
- Lint: `cargo clippy -p if-image -p isofoundry --all-targets -- -D warnings`
- Build: `cargo build -p isofoundry`

## Project Structure

- `crates/if-image/src/` — image container, sector source, filesystem, and edit implementations.
- `crates/if-image/tests/` — generated image fixtures and format/edit round-trip tests.
- `crates/isofoundry/src/cli_image.rs` — command-line image workflows.
- `crates/isofoundry/src/win/tasks/` — GUI browse, extract, and edit tasks.
- `README.md`, `docs/ARCHITECTURE.md`, and `docs/LIMITATIONS.md` — supported behavior and limits.
- `tasks/plan.md` and `tasks/todo.md` — staged project work; preserve completed items and unrelated working-tree changes.

## Code Style

Follow the existing checked, fallible parser style and return the shared `if_image::Result` rather than silently accepting malformed metadata. For example, the current FAT probe uses a trait-object sector source and an optional match result:

```rust
pub fn probe(src: &mut dyn SectorSource) -> Result<Option<Self>> {
    let image_bytes = src
        .sectors()
        .checked_mul(SECTOR as u64)
        .ok_or_else(|| crate::Error::Format("image size overflow".into()))?;
    // Validate format metadata and bounds before exposing a filesystem.
    Ok(None)
}
```

Use checked arithmetic for offsets and lengths, explicit format errors for unsupported layouts, and the existing source/extent abstractions. Avoid unchecked casts, broad error swallowing, and edits in place.

## Testing Strategy

- Generate small fixed and dynamic VHD fixtures in Rust tests rather than depend on physical media.
- Test valid footer/header checksums, metadata bounds, fixed payloads, dynamic block maps, sparse/unallocated blocks, and FAT12/16/32 payload layouts.
- Test malformed/truncated headers, invalid offsets and block maps, overflow, unsupported differencing images, and corrupt checksums.
- Bound supported images to 2 TiB virtual size, 1,048,576 dynamic BAT entries, and 256 MiB dynamic blocks.
- Verify browse/extract contents and edit round trips; verify the source bytes are unchanged and destination collisions/cancellation fail safely.
- Exercise the shared CLI path and ensure the GUI selects an appropriate VHD output extension while using the shared implementation.
- Run the commands above. Do not require hardware tests; document real-host limitations separately.

## Boundaries

- **Always:** Preserve source images; use bounded parsing and checked arithmetic; write edits atomically to a separate file; report unsupported formats explicitly; add focused tests and update user documentation.
- **Ask first:** VHDX editing, differencing chains, support for additional filesystems, new runtime dependencies, or any change to the user's existing USB-to-image work.
- **Never:** Modify a source image in place; claim universal VHD compatibility; implement copy-protection circumvention; silently ignore invalid metadata.

## Success Criteria

- Valid fixed and dynamic VHDs containing supported FAT volumes are recognized and can be browsed and extracted through existing image workflows.
- FAT editing inside supported VHDs produces a valid separate VHD that reopens with correct edited contents and retained required container/partition metadata.
- Invalid, oversized, unsupported, or malformed VHD metadata is rejected with a clear error before unsafe allocation or I/O.
- The source remains byte-for-byte unchanged, an existing destination is not overwritten, and cancellation leaves no success-shaped partial output.
- Focused tests, formatting, Clippy, and the application build pass.
- README and limitations document supported VHD variants and explicit exclusions.

## Open Questions

- None. The fixed/dynamic VHD, FAT-only, separate-output scope follows the approved capability map and existing 1.x roadmap.
