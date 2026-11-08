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

## Images
- UDF virtual partitions (VAT, used by packet-written CD-R/BD-R) cannot be read.
- UDF metadata partitions (Blu-ray) are implemented but not yet tested against a real disc image.
- CHD: reads `zlib` and `cdzl` hunks only. CHDs compressed with chdman's defaults (LZMA, FLAC, Huffman) need `chdman copy -c cdzl` (CD) or `-c zlib` (DVD) first. Parent (delta) CHDs are not supported. IsoFoundry writes `cdzl`/`zlib`, so its CHDs are a few percent larger than chdman's defaults.
- NRG and MDS/MDF are read only and unverified; first session only. Multi-session images and CD-Text are not converted.
- Converting a multi-track CD to ISO keeps only the first data track.
- Editing rewrites the image as ISO 9660 + Joliet + UDF 1.02. Hidden flags, Rock Ridge permissions and UDF 2.x extras of the source are not kept. Images that boot with floppy or hard-disk emulation can't be edited yet. Boot images that are also visible files are stored twice.
- Mounting uses `AttachVirtualDisk`; whether it works without administrator rights depends on Windows policy and is untested outside CI (where the account is an administrator). A mount lasts until `unmount` or restart.

## M0 shell
- No task runs yet: every task button and CLI subcommand reports the milestone it ships in.
- Verified only under Wine and in CI so far, not on a real Windows desktop (D-047).
- The menu bar uses the light system style; Windows has no documented dark menu bar.
- Installers and executables are not code-signed; Windows SmartScreen may warn on first run.
- Releases ship x64 only, as one Setup.exe installer and one portable zip (the MSI, ARM64 and PortableApps.com assets were dropped after 0.1.0).

## USB (M2)
- Writing to a physical disk is **Unverified**: the engine is tested end-to-end against disk-image files (the VHDX test route), but the elevated helper's physical-disk path has not yet run on real hardware, and no written stick has been boot-tested yet.
- The GPT+NTFS layout is formatted by Windows' own formatter through the helper; that call is **Unverified**.
- Cancelling a job that runs in the helper takes effect when the helper session ends; the file-target route cancels straight away.
- FAT32 layouts cannot hold an `install.wim`/`install.esd` over the 4 GB FAT32 file limit — the write is refused with a clear message; use the GPT+NTFS layout. Splitting an install image is not supported yet.
- Files mode needs a readable ISO image; raw mode accepts any image file.

## Burning (M4)
- The IMAPI2 backend is **Unverified**: no burn, erase or media query has run against a real recorder (development machines here have no optical drive). This milestone's test gate is the simulated drive (`--fake` in the CLI, "test drive" in the windows), which implements the job semantics — capacity, blank/appendable state, digest verify — for real.
- Verify after burn does a real digest comparison on the simulated drive only. On the IMAPI path it is reported as unavailable: reading a disc back arrives with the M5 ripping work.
- Burn progress on the IMAPI path is indeterminate while the write runs (IDiscFormat2Data's Write call blocks; there is no progress callback in this build), and Cancel cannot interrupt a write already in progress.
- "Simulate" on the IMAPI path runs every check (recorder, media, capacity) and writes nothing — IMAPI2's data format has no test-write mode.
- An MP3 disc is a *data* disc with the audio files plus a generated `playlist.m3u` (what car and hardware players read). A true audio CD is the M5 `burn-audio` task.

## Audio (M3)
- Opus and Ogg Vorbis **encoding** is not offered: there is no permissive pure-Rust encoder for either format and the C libraries cannot be built by this project's toolchain. Decoding both works.
- MP3 encoding needs `libmp3lame.dll` next to the program or on the PATH (loaded at runtime, never bundled, D-012); without it, MP3 jobs fail with a clear message. The LAME path is **Unverified** on a real Windows machine.
- AAC/M4A encoding goes through Windows Media Foundation with hand-declared bindings (windows-sys ships none) and is **Unverified** on real hardware.
- WMA and APE files cannot be decoded yet (Symphonia has no decoder for them); Media Foundation decoding is planned alongside the AAC work.
- Sources with more than two channels are reduced to the front left/right pair.
- A file is decoded fully into memory before encoding; very long recordings need the RAM to match (about 25 MB per minute of stereo audio).

## Platform
- Windows 10 22H2 and 11 only. ARM64 builds are produced but **Unverified** until run on ARM64 hardware.
- LAME MP3 encoding requires `libmp3lame.dll` (not bundled-linked); ARM64 build availability unverified.
- APE decoding pending verification.
- Context-menu verbs appear under "Show more options" on Windows 11.
- Run commands as `isofoundry …` or `isofoundry.com …` (console twin, D-048). Calling `isofoundry.exe …` explicitly still works but the shell does not wait for it or capture its output.
- Local GNU-toolchain builds are a convenience path; official releases are MSVC builds from CI.
