//! The two file-copy jobs: a generic bootable stick from an ISO's file
//! tree, and a Windows installer stick. Both partition + format + copy
//! through the same machinery; they differ in scheme choices and checks.
//!
//! Secure Boot note (roadmap, windows2usb research): a Windows installer
//! stick boots under Secure Boot because every boot file on it is
//! Microsoft's own signed file, copied unchanged from the ISO — this
//! writer never patches or replaces them.

use crate::device::BlockDevice;
use crate::fat32;
use crate::part::{self, GptPart};
use crate::{err, Result};
use if_core::job::{CancelToken, Progress};
use if_image::fs::Volume;
use std::io::Read;
use std::path::Path;

/// Partition scheme + filesystem for a file-copy job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheme {
    /// MBR + FAT32 — the windows2usb default: boots BIOS and UEFI.
    MbrFat32,
    /// GPT + FAT32 — UEFI only.
    GptFat32,
    /// GPT + NTFS — for install images over the FAT32 file-size limit.
    /// Formatting NTFS is delegated to the platform (the helper's
    /// Windows formatter); this crate only partitions for it and
    /// reports the step as external.
    GptNtfs,
}

impl Scheme {
    pub fn parse(s: &str) -> Result<Scheme> {
        match s.to_ascii_lowercase().as_str() {
            "mbr" | "mbr-fat32" => Ok(Scheme::MbrFat32),
            "gpt" | "gpt-fat32" => Ok(Scheme::GptFat32),
            "gpt-ntfs" | "ntfs" => Ok(Scheme::GptNtfs),
            other => err(format!("unknown scheme '{other}' (use mbr, gpt or gpt-ntfs)")),
        }
    }

    pub fn is_gpt(&self) -> bool {
        !matches!(self, Scheme::MbrFat32)
    }

    pub fn is_ntfs(&self) -> bool {
        matches!(self, Scheme::GptNtfs)
    }
}

/// What a finished copy job did.
#[derive(Debug, Default)]
pub struct CopyReport {
    pub files: usize,
    pub bytes: u64,
    /// Non-fatal observations (e.g. no UEFI boot files in the image).
    pub notes: Vec<String>,
}

const FAT32_MAX_FILE: u64 = 0xFFFF_FFFF;

/// Writes the partition table for `scheme` onto `dev` and returns the
/// partition spec. Formatting NTFS is the caller's platform step; the
/// FAT32 schemes are formatted by [`copy_iso_tree`].
pub fn partition<D: BlockDevice>(dev: &mut D, scheme: Scheme) -> Result<part::PartSpec> {
    let disk_sectors = dev.size() / part::SECTOR;
    let spec = part::single_partition(disk_sectors, scheme.is_gpt())?;
    if scheme.is_gpt() {
        let dg = part::new_disk_guid();
        let type_guid =
            if scheme.is_ntfs() { part::guid_bytes(part::GPT_BASIC) } else { part::guid_bytes(part::GPT_ESP) };
        let parts = vec![GptPart {
            type_guid,
            unique_guid: part::part_guid(&dg, 0),
            spec,
            name: if scheme.is_ntfs() { "INSTALL".into() } else { "BOOT".into() },
        }];
        for (lba, bytes) in part::gpt_writes(disk_sectors, &parts, dg) {
            dev.write_at(lba * part::SECTOR, &bytes)?;
        }
    } else {
        let sig = part::crc32(&disk_sectors.to_le_bytes()) ^ 0x55AA_0000;
        let mbr = part::mbr_sector(disk_sectors, &spec, 0x0C, sig);
        dev.write_at(0, &mbr)?;
    }
    dev.flush()?;
    Ok(spec)
}

fn open_iso(image: &Path) -> Result<Volume> {
    let (_, src) = if_image::container::open(image)?;
    Ok(Volume::open(src, None)?)
}

/// A `Read` over one file inside an ISO volume, feeding progress.
struct VolumeReader<'a> {
    vol: &'a mut Volume,
    entry: if_image::fs::Entry,
    off: u64,
    progress: &'a Progress,
    cancel: &'a CancelToken,
}

impl Read for VolumeReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.is_cancelled() {
            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "cancelled"));
        }
        let n = self.vol.read(&self.entry, self.off, buf).map_err(|e| std::io::Error::other(e.to_string()))?;
        self.off += n as u64;
        self.progress.add(n as u64);
        Ok(n)
    }
}

/// Partitions (FAT32 schemes), formats and copies an ISO's whole file
/// tree onto the device. `windows_installer` turns on the install.wim
/// size check and the installer-volume label.
pub fn copy_iso_tree<D: BlockDevice>(
    mut dev: D,
    image: &Path,
    scheme: Scheme,
    windows_installer: bool,
    progress: &Progress,
    cancel: &CancelToken,
) -> Result<CopyReport> {
    if scheme.is_ntfs() {
        return err("NTFS sticks are partitioned here but formatted and filled by the platform step (helper) — use the GptNtfs flow");
    }
    let mut vol = open_iso(image)?;
    let root = vol.root();
    let items = vol.walk(&root)?;
    let total: u64 = items.iter().filter(|(_, e)| !e.dir).map(|(_, e)| e.size).sum();
    progress.set_total(total);

    if windows_installer {
        for (rel, e) in &items {
            let lower = rel.to_ascii_lowercase();
            if (lower.ends_with("sources/install.wim") || lower.ends_with("sources/install.esd"))
                && e.size > FAT32_MAX_FILE
            {
                return err(format!(
                    "{} is {:.1} GB — over the 4 GB FAT32 file limit. Use the gpt-ntfs scheme for this image (or a split-WIM image).",
                    rel,
                    e.size as f64 / 1_073_741_824.0
                ));
            }
        }
    }

    let spec = partition(&mut dev, scheme)?;
    let label = if windows_installer { "WINSETUP" } else { "BOOT" };
    let base = spec.first_lba * part::SECTOR;
    let mut fv = fat32::format(dev, base, spec.sectors, spec.first_lba as u32, label)?;

    let mut report = CopyReport::default();
    let mut has_uefi_boot = false;
    for (rel, entry) in &items {
        if cancel.is_cancelled() {
            return err("cancelled");
        }
        if entry.dir {
            fv.ensure_dir(rel)?;
            continue;
        }
        let lower = rel.to_ascii_lowercase();
        if lower == "efi/boot/bootx64.efi" || lower == "efi/boot/bootaa64.efi" {
            has_uefi_boot = true;
        }
        let mut reader = VolumeReader { vol: &mut vol, entry: entry.clone(), off: 0, progress, cancel };
        fv.write_file(rel, &mut reader, entry.size)?;
        report.files += 1;
        report.bytes += entry.size;
    }
    fv.sync()?;
    fv.flush()?;

    if !windows_installer && !has_uefi_boot {
        report.notes.push(
            "the image has no EFI boot files (efi/boot/bootx64.efi) — in files mode the stick may not boot; \
             raw mode copies the image byte for byte and keeps its own boot code"
                .into(),
        );
    }
    Ok(report)
}

/// Checks an ISO for the files a Windows installer stick needs. Used by
/// the windows-to-usb job before it touches the target.
pub fn check_windows_installer(image: &Path) -> Result<()> {
    let mut vol = open_iso(image)?;
    let root = vol.root();
    let items = vol.walk(&root)?;
    let has = |suffix: &str| items.iter().any(|(rel, e)| !e.dir && rel.to_ascii_lowercase().ends_with(suffix));
    if !has("sources/install.wim") && !has("sources/install.esd") {
        return err(
            "this image has no sources/install.wim or install.esd — it does not look like a Windows installer image",
        );
    }
    if !has("efi/microsoft/boot/efisys.bin") && !has("efi/boot/bootx64.efi") {
        return err("this image has no UEFI boot files — it does not look like a Windows installer image");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::FileDevice;
    use if_image::write::{build, Kind, Node, Options};

    fn insert(children: &mut Vec<Node>, parts: &[&str], data: &[u8]) {
        if parts.len() == 1 {
            children.push(Node::bytes(parts[0], 0, data.to_vec()));
            return;
        }
        let pos = children.iter().position(|n| n.name == parts[0] && n.is_dir());
        let idx = match pos {
            Some(i) => i,
            None => {
                children.push(Node::dir(parts[0], 0, vec![]));
                children.len() - 1
            }
        };
        if let Kind::Dir(kids) = &mut children[idx].kind {
            insert(kids, &parts[1..], data);
        }
    }

    fn make_iso(dir: &Path, files: &[(&str, &[u8])]) -> std::path::PathBuf {
        let mut children = Vec::new();
        for (rel, data) in files {
            let parts: Vec<&str> = rel.split('/').collect();
            insert(&mut children, &parts, data);
        }
        let root = Node::dir("", 0, children);
        let out = dir.join("test.iso");
        build(&root, Options { label: "TEST".into(), ..Default::default() }, &out, |_, _| true).unwrap();
        out
    }

    #[test]
    fn windows_installer_to_mbr_fat32() {
        let dir = std::env::temp_dir().join(format!("usbw-inst-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wim = vec![42u8; 1_000_000];
        let iso = make_iso(
            &dir,
            &[
                ("sources/install.wim", &wim),
                ("efi/boot/bootx64.efi", b"fake-efi"),
                ("efi/microsoft/boot/efisys.bin", b"efisys"),
                ("bootmgr", b"bootmgr"),
            ],
        );
        check_windows_installer(&iso).unwrap();
        let dev_path = dir.join("stick.img");
        let dev = FileDevice::create(&dev_path, 256 << 20).unwrap();
        let progress = Progress::default();
        let report = copy_iso_tree(dev, &iso, Scheme::MbrFat32, true, &progress, &CancelToken::new()).unwrap();
        assert_eq!(report.files, 4);
        // Partition + volume are readable back.
        let dev = FileDevice::open(&dev_path, false).unwrap();
        let mut mbr = [0u8; 512];
        dev.read_at(0, &mut mbr).unwrap();
        assert_eq!(mbr[446 + 4], 0x0C);
        assert_eq!(mbr[446], 0x80);
        let first_lba = u32::from_le_bytes(mbr[446 + 8..446 + 12].try_into().unwrap()) as u64;
        let fv = fat32::Fat32Volume::open(dev, first_lba * 512).unwrap();
        assert_eq!(fv.label(), "WINSETUP");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn ntfs_scheme_refused_in_crate() {
        let dir = std::env::temp_dir().join(format!("usbw-ntfs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let iso = make_iso(&dir, &[("a.txt", b"hi")]);
        let dev = FileDevice::create(&dir.join("s.img"), 128 << 20).unwrap();
        let r = copy_iso_tree(dev, &iso, Scheme::GptNtfs, false, &Progress::default(), &CancelToken::new());
        assert!(r.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gpt_fat32_bootable() {
        let dir = std::env::temp_dir().join(format!("usbw-gpt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let iso = make_iso(&dir, &[("efi/boot/bootx64.efi", b"efi"), ("syslinux.cfg", b"cfg")]);
        let dev_path = dir.join("stick.img");
        let dev = FileDevice::create(&dev_path, 256 << 20).unwrap();
        let report =
            copy_iso_tree(dev, &iso, Scheme::GptFat32, false, &Progress::default(), &CancelToken::new()).unwrap();
        assert_eq!(report.files, 2);
        assert!(report.notes.is_empty());
        let dev = FileDevice::open(&dev_path, false).unwrap();
        let mut hdr = [0u8; 512];
        dev.read_at(512, &mut hdr).unwrap();
        assert_eq!(&hdr[0..8], b"EFI PART");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
