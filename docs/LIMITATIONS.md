# Known limitations

Kept current every release. Items move out only with evidence.

## By design
- No decryption or bypass of copy protection (CSS, AACS, BD+, game-disc protections). Protected discs will fail or copy without working protection.
- No video authoring or transcoding; only existing VIDEO_TS / BDMV folders are burned.
- No virtual drive driver; non-ISO images are converted to a temporary ISO before mounting.
- Network features (metadata lookup, AccurateRip, update check) are off until enabled.

## Hardware verification
- The developer has no optical drive or USB stick. All disc burning, copying, ripping and scanning features are **Unverified** on real hardware until confirmed through drive reports (`docs/COMPATIBILITY.md`).
- USB features are tested against attached VHDX disks and QEMU boot only. USB bus enumeration, removable-media flags and real flash write speeds are **Unverified**.
- Quality scans (C1/C2, PI/PO) rely on vendor-specific commands and work only on drives known to support them.

## Planned (2.0)
- Windows image customization arrives in 2.0. Offline servicing with the inbox DISM API is blocked when the host DISM is older than the image build unless a matching Windows ADK DISM is installed.
- Windows ISO download relies on Microsoft's site and the Fido script; Microsoft can block link generation.

## Platform
- Windows 10 22H2 and 11 only. ARM64 builds are produced but **Unverified** until run on ARM64 hardware.
- LAME MP3 encoding requires `libmp3lame.dll` (not bundled-linked); ARM64 build availability unverified.
- APE decoding pending verification.
- Context-menu verbs appear under "Show more options" on Windows 11.
- CLI from the GUI exe: output attaches to the parent console; in some shells the prompt returns before output finishes. Use `start /wait` or `--log` if needed.
- Local GNU-toolchain builds are a convenience path; official releases are MSVC builds from CI.
