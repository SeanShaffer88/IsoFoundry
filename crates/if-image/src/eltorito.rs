//! El Torito boot catalog (BIOS and UEFI boot entries).

use crate::source::SectorSource;
use crate::{bad, le16, le32, Result, SECTOR};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Bios,
    Efi,
    Other(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Emulation {
    NoEmulation,
    Floppy12,
    Floppy144,
    Floppy288,
    HardDisk,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootEntry {
    pub platform: Platform,
    pub bootable: bool,
    pub emulation: Emulation,
    pub load_segment: u16,
    /// Virtual 512-byte sectors loaded (no-emulation BIOS) or image size hint.
    pub sector_count: u16,
    pub lba: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootCatalog {
    pub lba: u64,
    pub entries: Vec<BootEntry>,
}

impl BootCatalog {
    pub fn has(&self, p: Platform) -> bool {
        self.entries.iter().any(|e| e.platform == p && e.bootable)
    }
}

fn emulation(b: u8) -> Emulation {
    match b & 0x0F {
        1 => Emulation::Floppy12,
        2 => Emulation::Floppy144,
        3 => Emulation::Floppy288,
        4 => Emulation::HardDisk,
        _ => Emulation::NoEmulation,
    }
}

fn platform(id: u8) -> Platform {
    match id {
        0 => Platform::Bios,
        0xEF => Platform::Efi,
        x => Platform::Other(x),
    }
}

pub fn read(src: &mut dyn SectorSource, lba: u64) -> Result<BootCatalog> {
    let mut buf = vec![0u8; SECTOR];
    src.read_sectors(lba, &mut buf)?;
    // Validation entry: header 1, key bytes 0x55 0xAA, words sum to zero.
    if buf[0] != 1 || buf[30] != 0x55 || buf[31] != 0xAA {
        return bad("boot catalog validation entry is invalid");
    }
    let sum = (0..16).fold(0u16, |s, i| s.wrapping_add(le16(&buf, i * 2)));
    if sum != 0 {
        return bad("boot catalog checksum is wrong");
    }
    let mut entries = Vec::new();
    let entry = |e: &[u8], p: Platform| BootEntry {
        platform: p,
        bootable: e[0] == 0x88,
        emulation: emulation(e[1]),
        load_segment: le16(e, 2),
        sector_count: le16(e, 6),
        lba: le32(e, 8),
    };
    entries.push(entry(&buf[32..64], platform(buf[1])));
    let mut off = 64;
    while off + 32 <= SECTOR {
        let h = &buf[off..off + 32];
        if h[0] != 0x90 && h[0] != 0x91 {
            break;
        }
        let p = platform(h[1]);
        let count = le16(h, 2) as usize;
        off += 32;
        for _ in 0..count {
            if off + 32 > SECTOR {
                break;
            }
            let e = &buf[off..off + 32];
            if e[0] == 0x88 || e[0] == 0x00 {
                entries.push(entry(e, p));
            }
            off += 32;
        }
        if h[0] == 0x91 {
            break;
        }
    }
    Ok(BootCatalog { lba, entries })
}
