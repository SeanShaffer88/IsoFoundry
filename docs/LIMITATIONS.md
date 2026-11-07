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

## M0 shell
- No task runs yet: every task button and CLI subcommand reports the milestone it ships in.
- Verified only under Wine and in CI so far, not on a real Windows desktop (D-047).
- The menu bar uses the light system style; Windows has no documented dark menu bar.
- The PortableApps.com package is a zip in PortableApps.com Format, not a `.paf.exe` installer: extract `IsoFoundryPortable` into your PortableApps folder.
- Installers and executables are not code-signed; Windows SmartScreen may warn on first run.
- Setup.exe and the MSI are x64 only; ARM64 ships as portable zips.

## Platform
- Windows 10 22H2 and 11 only. ARM64 builds are produced but **Unverified** until run on ARM64 hardware.
- LAME MP3 encoding requires `libmp3lame.dll` (not bundled-linked); ARM64 build availability unverified.
- APE decoding pending verification.
- Context-menu verbs appear under "Show more options" on Windows 11.
- Run commands as `isofoundry …` or `isofoundry.com …` (console twin, D-048). Calling `isofoundry.exe …` explicitly still works but the shell does not wait for it or capture its output.
- Local GNU-toolchain builds are a convenience path; official releases are MSVC builds from CI.
