# Architecture

Status: plan, pre-code. Every capability below is **Unverified** until a milestone marks it otherwise.

## 1. Process model

```
isofoundry.exe  (unelevated, GUI + CLI)
 ├─ ui/            egui task grid, task dialogs, progress, log view
 ├─ cli/           subcommands → same job structs as the GUI
 ├─ jobs/          Job queue, cancel tokens, operation journal
 └─ ipc/  ──named pipe (per-session nonce, ACL to current user SID)──┐
                                                                     ▼
isofoundry-helper.exe  (elevated on demand via ShellExecute "runas")
 └─ raw disk/USB writes, SPTI on drives that require admin, volume lock/dismount
```

The helper exits when its job finishes. It accepts only typed job messages (serde, versioned), never paths to executables or shell strings.

## 2. Crates (workspace)

| Crate | Role | Key deps |
|---|---|---|
| `isofoundry` | binary: GUI, CLI, job runner | eframe/egui, clap, fluent |
| `isofoundry-helper` | elevated binary | windows |
| `if-core` | job model, journal, settings, errors (stable codes) | serde, thiserror |
| `if-drive` | `OpticalDrive` trait; IMAPI2 backend; MMC/SPTI backend; fake backend | windows |
| `if-image` | read/write ISO9660/Joliet/UDF, BIN/CUE, CCD, raw IMG, CHD/zstd; readers for MDS, NRG, CDI | — |
| `if-audio` | decode (Symphonia, MF), encode (FLAC, LAME dyn, Opus, Vorbis, MF AAC), CD-DA ripping, CUE, CD-Text | symphonia |
| `if-backup` | ZIP64 spanning, manifest, AES-256 | zip |
| `usbwriter` | shared with WinISOPilot: enumerate, lock, partition (MBR/GPT), write image, boot files | windows |
| `if-hash` | CRC32, MD5, SHA-1, SHA-256 streaming | — |

2.0 adds shared Windows-image crates (D-034…D-038): `wim-inspect` (ISO/WIM/ESD metadata), `wim-download` (verified Microsoft ISO transfer + Fido link provider), `wim-service` (typed DISM API operation allowlist, mount journal), `wim-recipe` (versioned recipes, answer files). Servicing runs in `isofoundry-helper.exe`.

Rule: small dependency set, each one recorded in `CREDITS.md` with license.

## 3. Drive layer

```rust
pub trait OpticalDrive {
    fn info(&self) -> Result<DriveInfo>;           // vendor, model, fw, features (GET CONFIGURATION)
    fn media(&self) -> Result<MediaInfo>;          // type, ATIP / media ID, capacity, sessions
    fn read_sectors(&self, lba: u32, n: u32, mode: ReadMode, buf: &mut [u8]) -> Result<ReadStatus>;
    fn write(&self, plan: &WritePlan, sink: &dyn Progress, cancel: &CancelToken) -> Result<WriteReport>;
    fn erase(&self, kind: EraseKind, ...) -> Result<()>;
    fn quality_scan(&self, kind: ScanKind, ...) -> Result<ScanReport>; // vendor-specific; Unsupported if unknown
}
```

Backends:

- **ImapiDrive**: `IDiscFormat2Data`, `IDiscFormat2Erase`, `IDiscFormat2RawCD`, `IDiscFormat2TrackAtOnce`, `IFileSystemImage` (ISO9660/Joliet/UDF). Used by default for data, erase and audio TAO.
- **MmcDrive**: SPTI. Commands: INQUIRY, GET CONFIGURATION, READ DISC INFORMATION, READ TOC/PMA/ATIP, READ CD (raw 2352 + subchannel), READ(10/12), SET CD SPEED / SET STREAMING, SEND CUE SHEET, WRITE(10), SYNCHRONIZE CACHE, CLOSE TRACK/SESSION, BLANK, FORMAT UNIT. Used for raw copy, DAO-96, secure ripping, scans.
- **FakeDrive**: replays `.iftrace` files (CDB → status, sense, data). Records traces from real drives in later community testing.

Selection: the job asks for capabilities; the router picks IMAPI2 when it suffices, MMC otherwise, and reports which one ran in the log.

## 4. Jobs, safety and recovery

Every job follows: **plan → preview → confirm (proportional) → journal → execute → verify → report**.

- Journal: `%LOCALAPPDATA%\IsoFoundry\journal\<job-id>.json` written atomically (write temp, `MoveFileExW` replace) at each phase. On next launch, unfinished journals produce a recovery prompt (e.g. "Disc in E: may be unfinalized: close session / leave as is").
- Destructive targets (erase disc, write USB) show the device model, size, volume labels and a typed-confirmation for disks > 64 GB or any disk with mounted volumes.
- USB writer refuses: system disk, disk holding the pagefile/boot volume, fixed disks (unless `--dev-allow-vdisk` + VHDX).
- Verify after write is on by default for data and image burns.
- Cancel: every loop checks a `CancelToken`; disc writes cancel at a safe point and finalize or leave the disc appendable as chosen.

## 5. Image engine

- Writers: ISO9660 (levels 1–3, Rock Ridge off), Joliet, UDF 1.02/1.50/2.01/2.50/2.60, hybrid ISO+UDF, El Torito (BIOS + UEFI).
- Readers: above + BIN/CUE (multi-track, audio), CCD/IMG/SUB, MDS/MDF (v1 only, reverse-engineered: Unverified), NRG v1/v2, CDI, DMG (UDIF raw/zlib, HFS+ read-only browse), WIM (browse via wimlib or own reader: decide M1).
- Edit image: load tree, add/remove/rename, rebuild to new file (never in place).
- Convert: any readable → ISO / BIN-CUE / CCD / IMG / CHD.

## 6. Audio

- Rip: MMC READ CD with C2 pointers when supported, multiple read passes, AccurateRip v1/v2 checksum (opt-in lookup), offsets from a local table, CUE + per-track or single file, CD-Text read.
- Burn: DAO with CD-Text and custom gaps via MMC; TAO fallback via IMAPI2. ReplayGain/normalize optional.
- Convert: batch decode/encode with tag copy.

## 7. UI

- egui with a custom `Visuals` built from `docs/design-tokens.md`.
- Main window: task grid (see mockup). Each task opens a modal-less task window; multiple tasks can queue.
- Accessibility: egui AccessKit integration on (UI Automation), full keyboard navigation, visible focus ring, no color-only states, text scaling via Windows text size, high-contrast mode switches to system colors.
- Per-monitor DPI v2 manifest.

## 8. CLI

```
isofoundry burn-image <file> --drive E: [--speed max|<n>x] [--simulate] [--verify] [--copies N] [--drives E:,F:]
isofoundry burn-files <path>... --drive E: [--fs iso|joliet|udf|hybrid] [--multisession] [--label X]
isofoundry erase --drive E: [--full]
isofoundry copy-to-image --drive E: --out x.iso [--raw] [--subchannel]
isofoundry copy-disc --from E: --to F: [--on-the-fly]
isofoundry rip --drive E: --format flac --out <dir> [--cue] [--accuraterip]
isofoundry burn-audio <files>... --drive E: [--cd-text]
isofoundry convert-audio <files>... --to opus --out <dir>
isofoundry convert-image <in> --to iso --out <out>
isofoundry create-image <path>... --out x.iso [--fs udf] [--boot <img>]
isofoundry extract <image> --out <dir>
isofoundry checksum <file|drive> [--algo sha256] | compare <a> <b>
isofoundry surface-test --drive E: [--quality]
isofoundry info [--drive E:] 
isofoundry usb write <image> --disk N [--scheme gpt|mbr]    (elevates)
isofoundry usb read --disk N --out x.img                    (elevates)
isofoundry backup <path>... --drive E:|--out <dir> [--encrypt]
isofoundry restore <manifest> --out <dir>
```

Global: `--json`, `--yes` (skip confirms, refused for destructive ops without `--disk`/`--drive` explicitly set), `--log <file>`.
Exit codes: 0 ok, 1 generic, 2 usage, 3 cancelled, 10 no drive/media, 11 media incompatible, 12 write failed, 13 verify mismatch, 20 elevation denied, 30 I/O error.

## 9. Data and settings

- Settings: `settings.toml` (versioned `schema = 1`), portable mode when `IsoFoundry.portable` exists next to the exe (Data dir per PortableApps format).
- Logs: rolling text logs, Help → Open log folder; Export diagnostics bundle (redacted).
- Drive offsets table, codec presets: bundled read-only data files.

## 10. Performance budgets (targets, to be measured)

| Metric | Budget |
|---|---|
| Cold start to interactive grid | ≤ 400 ms on SSD |
| Idle CPU | 0% (no polling; drive arrival via `WM_DEVICECHANGE`) |
| Idle RAM | ≤ 40 MB |
| Burn buffer underruns | 0 at max rated speed with verify on (buffer 64 MB default) |
| ISO build from 50k files | ≤ 1.5× raw copy time |
