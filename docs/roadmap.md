# Roadmap

**Status: in development since 2026-10-31 (WinISOPilot 1.0.0 shipped).** Progress: M0 done (0.1.0). M1 done on `main` (reading, extraction, hashing, ISO creation, CD formats, CHD, conversion, editing, mounting and the GUI task windows); ships as 0.2.0. M2 USB built on `main` (usbwriter crate, both task windows, CLI, helper protocol v2); ships as 0.3.0 — physical-disk runs and QEMU boot checks still owed. M3 audio conversion built on `main` (if-audio crate: Symphonia decode, WAV/FLAC encode, MP3 via runtime LAME, AAC via Media Foundation, tags, ReplayGain, batch window + CLI); ships as 0.4.0 — the Media Foundation AAC path and LAME-on-Windows path are still to be exercised on a real machine. M4 burning built on `main` (if-drive crate: drive trait, fake drive with digest verify, IMAPI2 backend with hand-declared bindings; burn-image / burn-files with multisession append / burn-mp3 / burn-video / erase / drive-info windows + CLI, all runnable against `--fake`); shipped as 0.5.0 — no burn has run on real hardware, per this milestone's fake-drive test gate. M5 MMC ripping/copying built on `main` (SPTI raw reader with READ TOC / READ CD, rip/copy/scan engines in if-drive, fake drive read side with a damage map; copy-to-image / copy-disc / rip / burn-audio / surface-test / recover windows + CLI); shipped as 0.5.0 alongside M4 — no rip has run on real hardware either, per the same fake-drive gate. M6 integration built on `main` (spanning backup/restore with a catalog on every span, auto-action on disc insert, CLI completion scripts, Explorer verbs, opt-in update check, MusicBrainz disc lookup, and completion actions — sound/notification/shutdown); ships grouped with the RC pass as 0.9.0 (0.6.0 was never released separately). RC pass done on `main` (full audit, failure drills, accessibility pass, measured budgets, multi-drive burn from the command line); ships as 0.9.0. **1.0 released 2026-11-08** as the stable promotion of the 0.9.0 feature set (no feature changes; the release workflow marks it a full release, not a pre-release).

All milestones ship as public **0.x pre-releases**. **1.0** = every 1.0 row done, tested, documented.
Evidence label per feature: **Verified** (tested on real target), **Partially verified** (fake drive / VHDX / VM only), **Unverified**.

## 0.x → 1.0: disc and USB tool

Task grid modeled on AnyBurn Pro, with features drawn from Nero, Ashampoo Burning Studio, BurnAware, cdrtfe, ExplorerFab, Alcohol 120%, CDBurnerXP and CloneCD/CloneDVD (without copy-protection handling). Decisions: [DECISIONS.md](DECISIONS.md). Design: [ARCHITECTURE.md](ARCHITECTURE.md).

| Milestone | Release | Scope | Testable without hardware |
|---|---|---|---|
| M0 Shell | 0.1 | Workspace, CI (x64 + ARM64 MSVC, static CRT), local GNU build check, Win32 shell themed from tokens, main task grid, menus, settings, log view, Fluent strings, elevated helper + IPC skeleton, drive report export stub, installer/portable/PAF packaging | Yes |
| M1 Image tools | 0.2 | Create image from files/folders (ISO/Joliet/UDF/hybrid, El Torito), browse/extract, edit image (ISO9660/Joliet/UDF), convert image format, checksums + `.dat` hash files, verify against known-good hash lists, compare, mount (built-in + temp ISO), CHD writer | Yes |
| M2 USB | 0.3 | `usbwriter` crate (new; shared with WinISOPilot later), bootable USB (ISO → USB, GPT/MBR, UEFI/BIOS), Windows installer layouts, Install Windows to USB, USB → image, image → USB | VHDX + QEMU → Partially verified |
| M3 Audio convert | 0.4 | Decode/encode pipeline, tags, batch convert, ReplayGain | Yes |
| M4 IMAPI2 burning | 0.5 | Burn image, burn/append files (multisession), erase, Burn MP3 CD/DVD, Burn video folder, verify after burn, simulate, multi-drive burn (command line, RC), drive info (IMAPI level) | Fake drive only |
| M5 MMC | 0.5 | Disc → image (raw, subchannel) with dump reports, disc → disc (+ on-the-fly), rip audio (secure, AccurateRip opt-in, CUE), burn audio CD (DAO, CD-Text, gaps), surface test, quality scan (supported drives), full drive/disc info, recover files from damaged disc, preservation dumping UI | Fake drive only |
| M6 Integration | 0.9 | Spanning backup/restore, auto-action on disc insert, CLI completion, Explorer verbs, opt-in update check, MusicBrainz opt-in, completion actions (sound/toast/shutdown) | Yes |
| RC | 0.9 | Full audit pass, failure drills, accessibility pass, perf budgets measured, docs + screenshots | — |
| 1.0 | 1.0 | Release | — |
| 1.x | — | Spanish UI, native Win11 context menu (sparse package), multiboot USB, FAT/VHD image editing, Xbox XISO, more quality-scan vendors, MDS/NRG write if demand | — |

### Disc imaging (ideas from [DiscImageCreator](https://github.com/saramibreak/DiscImageCreator))

Ideas only; no DiscImageCreator code is used.

- [x] **Disc → image** from an optical drive, including retail Windows DVDs. Also accept a drive letter or a folder of files as the source. (M5)
- [x] **Hash file for every image created**: CRC32, MD5, SHA-1 and SHA-256, written next to the image as a ClrMamePro-style `.dat`. (M1)
- [x] **Verify images against known-good hash lists**, for example by importing redump.org `.dat` files, and report match / no match per track or file. (M1)
- [x] **Dump reports**: drive model and firmware, read errors and retries, and the disc's volume descriptors, saved next to the image. (M5)

Out of scope: anything for copy-protected discs (SecuROM, SafeDisc, LibCrypt and similar).

### Automation and image tools

- [x] **Auto-action on disc insert** (idea from [Automatic Ripping Machine](https://github.com/automatic-ripping-machine/automatic-ripping-machine), MIT): optional mode that detects a newly inserted disc and identifies it (audio / data). As built (M6, D-051) it opens the chosen job's window rather than starting the job itself — the output location is a per-job choice; encrypted video discs are not ripped.
- [ ] **Edit image files** (reference: [TotalImage](https://github.com/TotalImage/TotalImage), MIT): open an image, browse it, extract, add or remove files and save. ISO9660/Joliet/UDF first (M1), then FAT floppy/HDD images and VHD (1.x).
- [x] **Preservation dumping UI** (reference: [Redumper GUI](https://github.com/Deterous/Redumper-GUI), GPL-3.0, Rust): pick drive and system, show progress and the log, produce redump-style output. GPL code is not copied. (M5)
- [ ] Optional: open/extract/create Xbox XISO (XDVDFS) images (reference: [extract-xiso](https://github.com/XboxDev/extract-xiso)). Filesystem only; no patching of game executables. (1.x)

Not used, because they bypass console copy protection: PS3-Quick-Disc-Decryptor (decrypts PS3 disc images with keys), ftpdump (dumps games from modified PS4 consoles).

### USB

- [x] Bootable USB writer, `usbwriter` crate, later shared with WinISOPilot. (M2 — built; physical-disk verification owed)
- [x] Windows installer layouts (from [windows2usb](https://github.com/ValdikSS/windows2usb), ideas only): MBR+FAT32 (default; BIOS, UEFI, Secure Boot), GPT+FAT32, GPT+NTFS; Microsoft's own boot files so Secure Boot stays intact. As built, FAT32 layouts refuse an `install.wim`/`.esd` over the 4 GB FAT32 limit with a clear message (splitting not implemented yet) — GPT+NTFS is the route for those images. (M2 — built; physical-disk verification owed)
- [ ] **Multiboot USB** (ideas from [MultiOS-USB](https://github.com/Mexit/MultiOS-USB), GPL-3.0, [GLIM](https://github.com/thias/glim), public-domain configs, and [Ventoy](https://www.ventoy.net/en/index.html), GPL-3.0; also [YUMI](https://pendrivelinux.com/yumi-multiboot-usb-creator/) and [XBOOT](https://pendrivelinux.com/xboot-multiboot-iso-usb-creator/) as UX references): install a boot menu once, then copy any number of ISO/WIM files onto the stick and pick one at boot. Linux ISOs boot by GRUB loopback; WinPE/Windows setup from WIM/ISO; BIOS and UEFI; Secure Boot handled without asking users to disable it where possible; per-ISO boot options survive updates. Out of scope until the single-ISO writer is verified. (1.x)
- [ ] UI reference for a modern, simple writer: [Lufus](https://github.com/Hogjects/Lufus) (MIT, Linux).
- [ ] Write-safety and verify-after-write UX reference: [balenaEtcher](https://github.com/balena-io/etcher) (Apache-2.0). Single-ISO writer references: [Universal USB Installer](https://pendrivelinux.com/universal-usb-installer-easy-as-1-2-3/), [LiveUSB Install](https://pendrivelinux.com/liveusb-install-live-usb-creator/).

## 2.0: Windows ISO customization (WinISOPilot integration)

Every WinISOPilot feature, through shared Rust crates; the two apps stay separate (D-034…D-038). Entry point: grid button **Customize Windows ISO…** (▾ Download Windows ISO · Inspect image · Recipes) → stepper window. Backend: the DISM API that ships with Windows, plus wimlib.
Source scope: WinISOPilot `docs/roadmap.md` (main @ 8d15e62, 2026-10-29).

| Milestone | Scope (WinISOPilot IDs) | Admin | Test gate |
|---|---|---|---|
| W0 Kit | Shared repo, crate split, CI for both apps | — | Both apps build from the same tag |
| W1 Source | Windows 11 ISO download (Fido link + verified native transfer), ISO/WIM/ESD inspection, editions/indexes, SWM sets, WinRE discovery (P1-01…04) | No | Fixture images |
| W2 Mounting | Owned mount journal, recovery, lock detection, scratch preflight, DISM version gate (P1-05…08) | Yes | Elevated runs on throwaway copies |
| W3 Apps & components | Appx inventory/removal with preservation, features/FOD/capabilities, component-store cleanup, language pruning (P2-01…02, P3-01…09) | Yes | QEMU install + first logon |
| W4 Settings | Offline hives, privacy/update/Explorer defaults, services, tasks, hardware/OOBE options, device encryption (P2-04…06, P4-01…08) | Yes | QEMU install + policy readback |
| W5 Drivers & updates | CAB/MSU slipstream, INF drivers, cross-index sync (P5-01…03) | Yes | QEMU install + `dism /get-packages` readback |
| W6 Unattended & recipes | Answer-file builder, disk layout, identity, post-setup queue, OEM assets, root CA, versioned recipes, compare, undo (P5-04…10, P6-06) | No | Schema validation + QEMU unattended install |
| W7 Export | WIM/ESD recompression, selective index export, FAT32 split → USB, edition preference, footprint report, CLI (P6-01…09) | Mixed | Readback + QEMU boot (UEFI/BIOS) |
| 2.0 | Release | — | All W gates passed on a documented image profile |

Already provided by 1.0: ISO mastering (`if-image`), USB writing (`usbwriter`), checksums, mount, elevated helper.
