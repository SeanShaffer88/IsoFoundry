# IsoFoundry

Open-source disc burning, disc imaging and bootable USB tool for Windows 10 22H2 / 11 (x64 and ARM64), in the spirit of AnyBurn Pro: dark UI with a red accent and a grid of tasks.

**Status: planning.** No code yet. See [docs/roadmap.md](docs/roadmap.md).

## Planned stack

- Rust, [`windows`](https://crates.io/crates/windows) crate, egui
- IMAPI2 for burning, plus an own MMC/SPTI layer for low-level drive access
- USB writing shared with [WinISOPilot](https://github.com/SeanShaffer88/WinISOPilot) through a common `usbwriter` crate

## Principles

- Every feature done before 1.0; 0.x pre-releases per milestone.
- No copy-protection circumvention.
- Disc features ship as "Unverified" until drive reports from users confirm them; USB features are tested with VHDX files and QEMU.

## License

MIT. See [LICENSE](LICENSE).
