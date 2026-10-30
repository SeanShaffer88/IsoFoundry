# Testing without hardware

## 1. Fake optical drive
- `if-drive::fake::FakeDrive` loads `.iftrace` files: ordered list of `{ cdb, data_in, data_out, status, sense }`.
- Scenario library under `tests/traces/`: blank CD-R, appendable DVD+R, full BD-R, buffer underrun, write error mid-track, power-calibration failure, read error on sector N, disc ejected mid-job, unsupported command.
- Each burn/copy/rip/scan path has at least one success and one failure test.
- When community drive reports arrive with trace capture enabled (opt-in, redacted), they become regression fixtures.

## 2. USB targets via VHDX (Windows Home compatible)
Uses the Virtual Disk API (`CreateVirtualDisk`, `AttachVirtualDisk`), which works on Home. No Hyper-V needed.

```powershell
# create + attach a 16 GB dynamic VHDX (elevated PowerShell, diskpart is on Home)
@"
create vdisk file="$env:USERPROFILE\iftest\usb16.vhdx" maximum=16384 type=expandable
select vdisk file="$env:USERPROFILE\iftest\usb16.vhdx"
attach vdisk
"@ | Set-Content $env:TEMP\mk.txt; diskpart /s $env:TEMP\mk.txt

# then: isofoundry usb write <iso> --disk <N> --dev-allow-vdisk
# detach before booting:
"select vdisk file=`"$env:USERPROFILE\iftest\usb16.vhdx`"`ndetach vdisk" | Set-Content $env:TEMP\dt.txt; diskpart /s $env:TEMP\dt.txt
```

## 3. Boot test in QEMU (Windows Home)
```powershell
# UEFI boot (OVMF firmware ships with the QEMU Windows build as edk2-x86_64-code.fd)
qemu-system-x86_64 -m 4096 -machine q35 `
  -drive if=pflash,format=raw,readonly=on,file="C:\Program Files\qemu\share\edk2-x86_64-code.fd" `
  -drive file="$env:USERPROFILE\iftest\usb16.vhdx",format=vhdx,if=none,id=usb0 `
  -device qemu-xhci -device usb-storage,drive=usb0,bootindex=0
# Legacy BIOS boot: drop the pflash line.
```
Pass criteria: firmware finds the boot entry; installer / live OS reaches its first screen.
File paths above must be checked against the installed QEMU version.

## 4. Matrix (filled per release)
| Area | Win10 22H2 x64 | Win11 x64 | Win11 ARM64 | 100% | 150% | 200% | High contrast | Keyboard-only | Narrator |
|---|---|---|---|---|---|---|---|---|---|

## 5. Failure drills (RC)
Crash mid-burn, power loss (VM), disk full during image build, locked volume, permission denied, corrupt image input, old settings schema, drive removed mid-job.
