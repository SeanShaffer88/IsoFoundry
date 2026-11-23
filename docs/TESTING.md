# Testing without hardware

## 1. Fake optical drive
- `if-drive::fake::FakeDrive` loads `.iftrace` files: ordered list of `{ cdb, data_in, data_out, status, sense }`.
- Scenario library under `tests/traces/`: blank CD-R, appendable DVD+R, full BD-R, buffer underrun, write error mid-track, power-calibration failure, read error on sector N, disc ejected mid-job, unsupported command.
- Each burn/copy/rip/scan path has at least one success and one failure test.
- When community drive reports arrive with trace capture enabled (opt-in, redacted), they become regression fixtures.

## 2. USB image-file targets (no hardware)
Physical-disk writes are disabled until separate EFI/System Reserved boot-disk protection is verified. This test writes only to a plain disk-image file; it does not attach or select a physical disk.

```powershell
# create a 16 GB blank raw disk image
$path = "$env:USERPROFILE\iftest\usb16.img"
New-Item -ItemType Directory -Force (Split-Path $path) | Out-Null
$file = [System.IO.File]::Open($path, [System.IO.FileMode]::Create)
$file.SetLength(16GB)
$file.Dispose()

# use --file so the command cannot target a physical disk
isofoundry usb write <iso> --file $path --dev-allow-vdisk
```

## 3. Boot test in QEMU (Windows Home)
```powershell
# UEFI boot (OVMF firmware ships with the QEMU Windows build as edk2-x86_64-code.fd)
qemu-system-x86_64 -m 4096 -machine q35 `
  -drive if=pflash,format=raw,readonly=on,file="C:\Program Files\qemu\share\edk2-x86_64-code.fd" `
  -drive file="$env:USERPROFILE\iftest\usb16.img",format=raw,if=none,id=usb0 `
  -device qemu-xhci -device usb-storage,drive=usb0,bootindex=0
# Legacy BIOS boot: drop the pflash line.
```
Pass criteria: firmware finds the boot entry; installer / live OS reaches its first screen.
File paths above must be checked against the installed QEMU version.

## 4. Matrix (filled per release)
| Area | Win11 x64 | 100% | 150% | 200% | High contrast | Keyboard-only | Narrator |
|---|---|---|---|---|---|---|---|
| FluentGruv UI | pending | pending | pending | pending | pending | pending | pending |

## 5. Failure drills (RC)
Crash mid-burn, power loss (VM), disk full during image build, locked volume, permission denied, corrupt image input, old settings schema, drive removed mid-job.

## 5. Failure drills (RC)

`crates/isofoundry/tests/drills.rs` runs the hostile cases against
the simulated drive and the real engines: oversize and non-blank
burns, a verify that must not pass, damaged-disc rips and surface
scans, tampered and incomplete backups, a corrupt span catalog, a
corrupt settings file, malformed image files and cancelled jobs.
They run with the normal workspace test suite — first recorded run:
the RC pass, 2026-11-08 (full workspace suite green).

Still owed, because they need a real machine or a power cut:
crash mid-burn, power loss during a job, a full disk during an
image build, a locked volume, and the drive removed mid-job.
