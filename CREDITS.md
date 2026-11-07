# Credits

**Author:** Sean Shaffer ([@SeanShaffer88](https://github.com/SeanShaffer88))

## Inspiration (no code used)

- [AnyBurn](https://www.anyburn.com/): task-grid layout and feature set.
- [DiscImageCreator](https://github.com/saramibreak/DiscImageCreator) (Apache-2.0): hash `.dat` files, dump reports, verification against known-good hash lists.
- [windows2usb](https://github.com/ValdikSS/windows2usb) (Apache-2.0): Windows USB layout modes, write-through progress.
- [MultiOS-USB](https://github.com/Mexit/MultiOS-USB) (GPL-3.0) and [GLIM](https://github.com/thias/glim) (public domain configs): multiboot USB design. GPL code can't be copied into this MIT project.
- [Lufus](https://github.com/Hogjects/Lufus) (MIT): UI reference.
- [Automatic Ripping Machine](https://github.com/automatic-ripping-machine/automatic-ripping-machine) (MIT): auto-action on disc insert, parallel drives, notifications.
- [TotalImage](https://github.com/TotalImage/TotalImage) (MIT): image editor reference.
- [Redumper GUI](https://github.com/Deterous/Redumper-GUI) (GPL-3.0): preservation dumping UI reference.
- [extract-xiso](https://github.com/XboxDev/extract-xiso): Xbox XISO filesystem reference.
- [MAME](https://github.com/mamedev/mame) (BSD-3-Clause for the CHD code): CHD v5 format; chdman 0.264 used to check our output. Format reimplemented, no code copied.
- Nero, Ashampoo Burning Studio, BurnAware, cdrtfe, ExplorerFab, Alcohol 120%, CDBurnerXP, CloneCD/CloneDVD: feature references.

## Related

- [WinISOPilot](https://github.com/SeanShaffer88/WinISOPilot): shares the USB writer and, from 2.0, the Windows ISO customization.

## Third-party crates in use
- [miniz_oxide](https://github.com/Frommi/miniz_oxide) (MIT / Zlib / Apache-2.0): deflate for CHD.
- [sha1](https://github.com/RustCrypto/hashes) (RustCrypto, MIT / Apache-2.0): CHD checksums.


## Planned third-party components (D-012)
Final list and versions are pinned in `Cargo.lock`; licenses checked with `cargo deny` before each release.

| Component | Use | License | Linking |
|---|---|---|---|
| windows-sys / windows-rs | Win32 UI + system APIs / COM (IMAPI2) | MIT / Apache-2.0 | static |
| crc32fast, md-5, sha1, sha2 (RustCrypto) | hashing | MIT / Apache-2.0 | static |
| serde, serde_json, toml | settings, IPC, reports | MIT / Apache-2.0 | static |
| getrandom | IPC session nonce | MIT / Apache-2.0 | static |
| Symphonia | audio decoding | MPL-2.0 | static (file-level copyleft; unmodified) |
| Fluent (fluent-rs) | localization | MIT / Apache-2.0 | static |
| clap | CLI | MIT / Apache-2.0 | static |
| zip | spanning backup | MIT | static |
| LAME | MP3 encoding | LGPL-2.0+ | **dynamic**, user-replaceable DLL |
| libopus / libvorbis | Opus / Vorbis encoding | BSD-3-Clause | static |
| Microsoft Media Foundation | AAC / WMA | part of Windows | system |
| IMAPI2 | disc burning | part of Windows | system |

## Build and packaging tools (not redistributed as code)
- [Inno Setup](https://jrsoftware.org/isinfo.php): builds `Setup.exe`; its setup/uninstall runtime is part of that file (Inno Setup License).
- [WiX Toolset](https://wixtoolset.org/) v5: builds the MSI (MS-RL).
- PortableApps.com Format: package layout ([portableapps.com](https://portableapps.com/development/portableapps.com_format)).

## Data services (opt-in)
- MusicBrainz metadata (CC0 core data)
- AccurateRip checksum database

## Icons
- Application icon and logo: original artwork for this project.
- UAC shield: system stock icon (`SIID_SHIELD`).

