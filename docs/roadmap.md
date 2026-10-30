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

## 2.0: Windows ISO customization

- [ ] All WinISOPilot features, through shared Rust crates; the two apps stay separate.
- [ ] One "Customize Windows ISO…" task in the grid that opens the wizard.
- [ ] Backend: the DISM API that ships with Windows, plus wimlib.
