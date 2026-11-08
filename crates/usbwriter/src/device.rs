//! Block devices and target safety.

use crate::{err, Result};
use std::fs::File;
use std::path::Path;

/// A byte-addressable block target. Offsets are bytes; implementations
/// round reads/writes to their sector size internally when needed (the
/// file device and the helper's disk device both accept any offset that
/// is a multiple of 512 in practice; every caller here uses 512-multiples).
pub trait BlockDevice {
    /// Total size in bytes.
    fn size(&self) -> u64;
    /// Reads exactly `buf.len()` bytes at `off` (zero-fills past the end
    /// of file-backed devices when the file is shorter).
    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<()>;
    /// Writes all of `buf` at `off`.
    fn write_at(&mut self, off: u64, buf: &[u8]) -> Result<()>;
    /// Flushes pending writes to the medium.
    fn flush(&mut self) -> Result<()>;
}

/// A device backed by a file — a VHDX attached by Windows, or a plain
/// disk-image file used in tests and `--dev-allow-vdisk` runs.
pub struct FileDevice {
    file: File,
    size: u64,
    writable: bool,
}

impl FileDevice {
    /// Opens an existing file/device path. `size` comes from the file's
    /// length; pass `override_size` for paths whose length does not
    /// reflect the device (rare).
    pub fn open(path: &Path, writable: bool) -> Result<Self> {
        let file =
            if writable { std::fs::OpenOptions::new().read(true).write(true).open(path)? } else { File::open(path)? };
        let size = file.metadata()?.len();
        Ok(FileDevice { file, size, writable })
    }

    /// Creates (or truncates) a backing file of exactly `size` bytes.
    pub fn create(path: &Path, size: u64) -> Result<Self> {
        let file = std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(true).open(path)?;
        file.set_len(size)?;
        Ok(FileDevice { file, size, writable: true })
    }

    pub fn is_writable(&self) -> bool {
        self.writable
    }
}

#[cfg(unix)]
fn read_exact_at(file: &File, buf: &mut [u8], off: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.read_exact_at(buf, off)
}

#[cfg(windows)]
fn read_exact_at(file: &File, buf: &mut [u8], off: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0usize;
    while done < buf.len() {
        let n = file.seek_read(&mut buf[done..], off + done as u64)?;
        if n == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "short read"));
        }
        done += n;
    }
    Ok(())
}

#[cfg(unix)]
fn write_all_at(file: &File, buf: &[u8], off: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    file.write_all_at(buf, off)
}

#[cfg(windows)]
fn write_all_at(file: &File, buf: &[u8], off: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut done = 0usize;
    while done < buf.len() {
        let n = file.seek_write(&buf[done..], off + done as u64)?;
        if n == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::WriteZero, "short write"));
        }
        done += n;
    }
    Ok(())
}

impl BlockDevice for FileDevice {
    fn size(&self) -> u64 {
        self.size
    }

    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<()> {
        if off >= self.size {
            buf.fill(0);
            return Ok(());
        }
        let want = buf.len().min((self.size - off) as usize);
        read_exact_at(&self.file, &mut buf[..want], off)?;
        buf[want..].fill(0);
        Ok(())
    }

    fn write_at(&mut self, off: u64, buf: &[u8]) -> Result<()> {
        if !self.writable {
            return err("device was opened read-only");
        }
        if off + buf.len() as u64 > self.size {
            return err(format!("write past the end of the target ({} + {} > {})", off, buf.len(), self.size));
        }
        write_all_at(&self.file, buf, off)?;
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        self.file.sync_all()?;
        Ok(())
    }
}

/// How a disk is attached (from the storage descriptor, where known).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bus {
    Usb,
    Nvme,
    Sata,
    Scsi,
    Virtual,
    Other,
}

/// What a caller needs to know about a candidate target disk.
#[derive(Debug, Clone)]
pub struct DiskInfo {
    /// Windows physical-drive number (0 for file targets).
    pub number: u32,
    /// Model / product string, "disk image file" for file targets.
    pub model: String,
    pub size_bytes: u64,
    pub bus: Bus,
    /// Removable-media flag from the storage descriptor.
    pub removable: bool,
    /// The disk Windows itself is installed on.
    pub is_system: bool,
    /// The disk holding the running Windows boot volume.
    pub holds_boot: bool,
    /// A disk holding a pagefile.
    pub holds_pagefile: bool,
    /// Labels of volumes currently mounted from this disk.
    pub volume_labels: Vec<String>,
    /// True for file targets (VHDX / disk images) rather than real disks.
    pub is_file_target: bool,
}

/// The pre-write safety gate (ARCHITECTURE.md §USB / D-029). Every write
/// path — CLI, GUI, helper — runs the target through this first.
///
/// Rules: never the system disk, never the disk holding the boot volume
/// or a pagefile; fixed disks (not removable, not USB-attached) are
/// refused unless the caller explicitly allows them (the
/// `--dev-allow-vdisk` route, which callers set only for VHDX targets);
/// file targets must be allowed by the caller too. An empty disk is
/// refused as a probable enumeration bug.
pub fn assess_target(d: &DiskInfo, allow_fixed: bool, allow_file_target: bool) -> Result<()> {
    if d.size_bytes == 0 {
        return err("the target reports a size of 0 bytes — refusing to write to it");
    }
    if d.is_system {
        return err("that is the disk Windows is installed on — refusing to erase it");
    }
    if d.holds_boot {
        return err("that disk holds the Windows boot volume — refusing to erase it");
    }
    if d.holds_pagefile {
        return err("that disk holds a pagefile — refusing to erase it");
    }
    if d.is_file_target {
        if !allow_file_target {
            return err("file targets (VHDX / disk images) need the dev flag to be used");
        }
        return Ok(());
    }
    let fixed = !d.removable && d.bus != Bus::Usb;
    if fixed && !allow_fixed {
        return err("that is a fixed internal disk, not a USB stick — refusing (fixed disks need the dev flag)");
    }
    Ok(())
}

/// A one-line human summary of a disk, used in confirmations:
/// `Disk 2 — SanDisk Ultra (29.8 GB) — volumes: SETUP`.
pub fn describe(d: &DiskInfo) -> String {
    let gb = d.size_bytes as f64 / 1_000_000_000.0;
    let mut s = format!("Disk {} — {} ({:.1} GB)", d.number, d.model.trim(), gb);
    if !d.volume_labels.is_empty() {
        s.push_str(&format!(" — volumes: {}", d.volume_labels.join(", ")));
    }
    s
}

/// Typed-confirmation threshold from ARCHITECTURE.md §7: disks larger
/// than 64 GB, or with any mounted volumes, need the stronger confirm.
pub fn needs_typed_confirmation(d: &DiskInfo) -> bool {
    d.size_bytes > 64_000_000_000 || !d.volume_labels.is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk() -> DiskInfo {
        DiskInfo {
            number: 2,
            model: "SanDisk Ultra".into(),
            size_bytes: 32_000_000_000,
            bus: Bus::Usb,
            removable: true,
            is_system: false,
            holds_boot: false,
            holds_pagefile: false,
            volume_labels: vec![],
            is_file_target: false,
        }
    }

    #[test]
    fn usb_stick_passes() {
        assert!(assess_target(&disk(), false, false).is_ok());
    }

    #[test]
    fn system_disk_refused() {
        let mut d = disk();
        d.is_system = true;
        assert!(assess_target(&d, true, true).is_err());
    }

    #[test]
    fn fixed_disk_needs_dev_flag() {
        let mut d = disk();
        d.bus = Bus::Sata;
        d.removable = false;
        assert!(assess_target(&d, false, false).is_err());
        assert!(assess_target(&d, true, false).is_ok());
    }

    #[test]
    fn file_target_needs_its_flag() {
        let mut d = disk();
        d.is_file_target = true;
        assert!(assess_target(&d, false, false).is_err());
        assert!(assess_target(&d, false, true).is_ok());
    }

    #[test]
    fn typed_confirmation_thresholds() {
        let mut d = disk();
        assert!(!needs_typed_confirmation(&d));
        d.volume_labels.push("SETUP".into());
        assert!(needs_typed_confirmation(&d));
        let mut big = disk();
        big.size_bytes = 128_000_000_000;
        assert!(needs_typed_confirmation(&big));
    }

    #[test]
    fn file_device_roundtrip() {
        let dir = std::env::temp_dir().join(format!("usbw-dev-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("t.img");
        let mut dev = FileDevice::create(&p, 1 << 20).unwrap();
        assert_eq!(dev.size(), 1 << 20);
        dev.write_at(512, b"hello").unwrap();
        let mut buf = [0u8; 5];
        dev.read_at(512, &mut buf).unwrap();
        assert_eq!(&buf, b"hello");
        assert!(dev.write_at((1 << 20) - 2, b"toolong").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
