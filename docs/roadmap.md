# Roadmap

Nothing below is implemented yet.

## 0.x → 1.0: disc and USB tool

Task grid modeled on AnyBurn Pro, with features drawn from Nero, Ashampoo Burning Studio, BurnAware, cdrtfe, ExplorerFab, Alcohol 120%, CDBurnerXP and CloneCD/CloneDVD (without copy-protection handling).

### Disc imaging (ideas from [DiscImageCreator](https://github.com/saramibreak/DiscImageCreator))

Ideas only; no DiscImageCreator code is used.

- [ ] **Disc → image** from an optical drive, including retail Windows DVDs. Also accept a drive letter or a folder of files as the source.
- [ ] **Hash file for every image created**: CRC32, MD5, SHA-1 and SHA-256, written next to the image as a ClrMamePro-style `.dat`.
- [ ] **Verify images against known-good hash lists**, for example by importing redump.org `.dat` files, and report match / no match per track or file.
- [ ] **Dump reports**: drive model and firmware, read errors and retries, and the disc's volume descriptors, saved next to the image.

Out of scope: anything for copy-protected discs (SecuROM, SafeDisc, LibCrypt and similar).

### USB

- [ ] Bootable USB writer shared with WinISOPilot (`usbwriter` crate).
- [ ] Windows installer layouts (from [windows2usb](https://github.com/ValdikSS/windows2usb), ideas only): MBR+FAT32 with split `install.wim` (default; BIOS, UEFI, Secure Boot), GPT+FAT32, GPT+NTFS; Microsoft's own boot files so Secure Boot stays intact; write-through I/O so progress is real.
- [ ] **Multiboot USB** (ideas from [MultiOS-USB](https://github.com/Mexit/MultiOS-USB), GPL-3.0, and [GLIM](https://github.com/thias/glim), public-domain configs): install a boot menu once, then copy any number of ISO/WIM files onto the stick and pick one at boot. Linux ISOs boot by GRUB loopback; WinPE/Windows setup from WIM/ISO; BIOS and UEFI; Secure Boot handled without asking users to disable it where possible; per-ISO boot options survive updates. Out of scope until the single-ISO writer is verified.
- [ ] UI reference for a modern, simple writer: [Lufus](https://github.com/Hogjects/Lufus) (MIT, Linux).

## 2.0: Windows ISO customization

- [ ] All WinISOPilot features, through shared Rust crates; the two apps stay separate.
- [ ] One "Customize Windows ISO…" task in the grid that opens the wizard.
- [ ] Backend: the DISM API that ships with Windows, plus wimlib.
