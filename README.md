<p align="center"><img src="assets/logo.svg" width="360" alt="IsoFoundry"></p>

# IsoFoundry

**Open Source Disc Burning, Imaging & Bootable USB Suite** for Windows 10 22H2 / 11 (x64 and ARM64), in the spirit of AnyBurn Pro: dark UI with a red accent and a grid of tasks.

> Status: **planning complete, parked** until Win10Pilot is complete. No application release yet (0.0.1 = planning docs). See [ROADMAP](docs/roadmap.md).

## What it does (planned for 1.0)

**Discs**
- Burn image files to CD, DVD, Blu-ray (incl. BD-XL, M-DISC); simulate, verify, multi-drive burning
- Burn or append files/folders (ISO9660, Joliet, UDF, hybrid; multisession; bootable El Torito)
- Burn DVD-Video / Blu-ray folders (VIDEO_TS, BDMV)
- Erase rewritable discs (quick / full)
- Copy disc to image (incl. raw + subchannel), copy disc to disc (incl. on-the-fly)
- Spanning backup across several discs (ZIP64, optional AES-256), restorable with any unzip tool
- Disc surface test, quality scan on supported drives, recover files from damaged discs
- Drive and disc information

**Audio**
- Secure ripping to MP3, FLAC, Opus, Ogg Vorbis, AAC/M4A, WAV; CUE sheets; optional AccurateRip and MusicBrainz (off by default)
- Burn audio CDs with CD-Text and custom gaps; burn MP3 CD/DVD
- Convert audio formats

**Images**
- Create, browse, extract, edit, convert, mount
- Formats: ISO, BIN/CUE, CCD/IMG/SUB, IMG, CHD (write); + MDS/MDF, NRG, CDI, DMG (read)
- Checksums (CRC32, MD5, SHA-1, SHA-256), `.dat` hash files, verification against known-good hash lists (e.g. redump), compare image ↔ disc

**USB**
- Create bootable USB drives (UEFI / BIOS, GPT / MBR)
- Windows installer layouts (MBR+FAT32 with split `install.wim`, GPT+FAT32, GPT+NTFS)
- Install Windows to a USB drive
- USB → image, image → USB

**Automation**
- Optional auto-action on disc insert (image + hash, or rip), several drives in parallel

**Everywhere**
- Command line for every task (`isofoundry --help`), JSON output
- Explorer right-click verbs (optional)
- Portable, PortableApps.com and installer builds
- Works offline; no telemetry

## Later
- 1.x: multiboot USB, FAT/VHD image editing, Xbox XISO, Spanish UI
- 2.0: Windows ISO customization from [WinISOPilot](https://github.com/SeanShaffer88/WinISOPilot) (download, debloat, drivers/updates, unattended setup)

## Not included
IsoFoundry does not remove or bypass copy protection, and does not author or transcode video.

## Screenshots
Will be captured from real runs at 0.1. Capture list: main window (100% and 150% scale), Burn image dialog, Copy disc dialog, progress with verify, drive info, USB write confirmation, high-contrast main window.

## Hardware compatibility
Disc features are developed against a simulated drive. Real-drive results come from users: Help → **Export drive report**, then open a "Drive report" issue. See [COMPATIBILITY](docs/COMPATIBILITY.md).

## Build
```powershell
# official builds: GitHub Actions (MSVC, x64 + ARM64)
# local (no Visual Studio):
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
```

## Docs
[Architecture](docs/ARCHITECTURE.md) · [Decisions](docs/DECISIONS.md) · [Limitations](docs/LIMITATIONS.md) · [Testing](docs/TESTING.md) · [References](docs/references.md) · [Design tokens](docs/design-tokens.md) · [Changelog](CHANGELOG.md) · [Audit](AUDIT.md) · [Credits](CREDITS.md)

## License
MIT. See [LICENSE](LICENSE). Third-party notices in [CREDITS](CREDITS.md).
