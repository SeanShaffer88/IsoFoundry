//! Partition-table bytes: MBR and GPT, built purely so they can be
//! unit-tested byte for byte. Sector size is 512 throughout (every target
//! we write — USB sticks, VHDX — reports 512-byte logical sectors in the
//! paths we support).
//!
//! Layouts start the first partition at LBA 2048 (1 MiB alignment) and
//! end partitions on 2048-sector boundaries.

use crate::{err, Result};

pub const SECTOR: u64 = 512;
pub const ALIGN: u64 = 2048; // sectors (1 MiB)

/// CRC-32 (IEEE) — small table-free implementation; the tables it runs
/// over are at most a few tens of KB.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// One partition to place.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PartSpec {
    pub first_lba: u64,
    pub sectors: u64,
    pub bootable: bool,
}

impl PartSpec {
    pub fn last_lba(&self) -> u64 {
        self.first_lba + self.sectors - 1
    }
}

/// The single-partition layout both USB jobs use: one partition filling
/// the disk from LBA 2048, sized down to the alignment boundary (GPT
/// keeps 33 sectors at the end for the backup tables).
pub fn single_partition(disk_sectors: u64, gpt: bool) -> Result<PartSpec> {
    let reserved_end = if gpt { 34 } else { 0 };
    if disk_sectors < ALIGN + reserved_end + ALIGN {
        return err("the target is too small to partition");
    }
    let usable = disk_sectors - reserved_end;
    let sectors = (usable - ALIGN) / ALIGN * ALIGN;
    Ok(PartSpec { first_lba: ALIGN, sectors, bootable: true })
}

/// Builds the 512-byte MBR with one partition of the given type byte
/// (0x0C = FAT32 LBA, 0x07 = NTFS/exFAT).
pub fn mbr_sector(disk_sectors: u64, part: &PartSpec, part_type: u8, disk_signature: u32) -> [u8; 512] {
    let mut m = [0u8; 512];
    // No boot code: BIOS-bootable sticks from ISO file trees rely on the
    // files the image carries; raw writes keep the image's own MBR.
    m[440..444].copy_from_slice(&disk_signature.to_le_bytes());
    let e = &mut m[446..462];
    e[0] = if part.bootable { 0x80 } else { 0x00 };
    // CHS first (maxed, LBA mode), type, CHS last (maxed), LBA, count.
    e[1..4].copy_from_slice(&[0xFE, 0xFF, 0xFF]);
    e[4] = part_type;
    e[5..8].copy_from_slice(&[0xFE, 0xFF, 0xFF]);
    e[8..12].copy_from_slice(&(part.first_lba as u32).to_le_bytes());
    e[12..16].copy_from_slice(&(part.sectors as u32).to_le_bytes());
    let _ = disk_sectors;
    m[510] = 0x55;
    m[511] = 0xAA;
    m
}

/// GUID in GPT on-disk byte order (first three fields little-endian)
/// from the canonical text form.
pub fn guid_bytes(text: &str) -> [u8; 16] {
    let hex: String = text.chars().filter(|c| *c != '-').collect();
    let raw: Vec<u8> = (0..16).map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap_or(0)).collect();
    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&raw[0..4].iter().rev().copied().collect::<Vec<_>>());
    out[4..6].copy_from_slice(&raw[4..6].iter().rev().copied().collect::<Vec<_>>());
    out[6..8].copy_from_slice(&raw[6..8].iter().rev().copied().collect::<Vec<_>>());
    out[8..16].copy_from_slice(&raw[8..16]);
    out
}

/// EFI System Partition type GUID.
pub const GPT_ESP: &str = "C12A7328-F81F-11D2-BA4B-00A0C93EC93B";
/// Microsoft Basic Data type GUID.
pub const GPT_BASIC: &str = "EBD0A0A2-B9E5-4433-87C0-68B6B72699C7";

/// One GPT partition entry.
#[derive(Debug, Clone)]
pub struct GptPart {
    pub type_guid: [u8; 16],
    pub unique_guid: [u8; 16],
    pub spec: PartSpec,
    pub name: String,
}

const GPT_ENTRIES: u32 = 128;
const GPT_ENTRY_SIZE: u32 = 128;

/// Builds the GPT structures as (offset_in_sectors, bytes) writes:
/// protective MBR at 0, primary header at 1, entries at 2, backup
/// entries at `disk_sectors - 33`, backup header at `disk_sectors - 1`.
pub fn gpt_writes(disk_sectors: u64, parts: &[GptPart], disk_guid: [u8; 16]) -> Vec<(u64, Vec<u8>)> {
    // Entries array.
    let mut entries = vec![0u8; (GPT_ENTRIES * GPT_ENTRY_SIZE) as usize];
    for (i, p) in parts.iter().enumerate() {
        let e = &mut entries[i * 128..(i + 1) * 128];
        e[0..16].copy_from_slice(&p.type_guid);
        e[16..32].copy_from_slice(&p.unique_guid);
        e[32..40].copy_from_slice(&p.spec.first_lba.to_le_bytes());
        e[40..48].copy_from_slice(&p.spec.last_lba().to_le_bytes());
        e[48..56].copy_from_slice(&0u64.to_le_bytes()); // attributes
        for (j, u) in p.name.encode_utf16().take(36).enumerate() {
            e[56 + j * 2..58 + j * 2].copy_from_slice(&u.to_le_bytes());
        }
    }
    let entries_crc = crc32(&entries);
    let entries_sectors = (GPT_ENTRIES * GPT_ENTRY_SIZE) as u64 / SECTOR;

    let header = |my_lba: u64, alternate: u64, entries_lba: u64| -> Vec<u8> {
        let mut h = vec![0u8; SECTOR as usize];
        h[0..8].copy_from_slice(b"EFI PART");
        h[8..12].copy_from_slice(&0x0001_0000u32.to_le_bytes()); // revision 1.0
        h[12..16].copy_from_slice(&92u32.to_le_bytes()); // header size
        h[24..32].copy_from_slice(&my_lba.to_le_bytes());
        h[32..40].copy_from_slice(&alternate.to_le_bytes());
        h[40..48].copy_from_slice(&34u64.to_le_bytes()); // first usable
        h[48..56].copy_from_slice(&(disk_sectors - 34).to_le_bytes()); // last usable
        h[56..72].copy_from_slice(&disk_guid);
        h[72..80].copy_from_slice(&entries_lba.to_le_bytes());
        h[80..84].copy_from_slice(&GPT_ENTRIES.to_le_bytes());
        h[84..88].copy_from_slice(&GPT_ENTRY_SIZE.to_le_bytes());
        h[88..92].copy_from_slice(&entries_crc.to_le_bytes());
        let crc = crc32(&h[..92]);
        h[16..20].copy_from_slice(&crc.to_le_bytes());
        h
    };

    // Protective MBR: one 0xEE partition spanning the disk.
    let mut pmbr = [0u8; 512];
    let e = &mut pmbr[446..462];
    e[4] = 0xEE;
    e[8..12].copy_from_slice(&1u32.to_le_bytes());
    let span = (disk_sectors - 1).min(u32::MAX as u64) as u32;
    e[12..16].copy_from_slice(&span.to_le_bytes());
    pmbr[510] = 0x55;
    pmbr[511] = 0xAA;

    let backup_entries_lba = disk_sectors - 1 - entries_sectors;
    vec![
        (0, pmbr.to_vec()),
        (1, header(1, disk_sectors - 1, 2)),
        (2, entries.clone()),
        (backup_entries_lba, entries),
        (disk_sectors - 1, header(disk_sectors - 1, 1, backup_entries_lba)),
    ]
}

/// A unique partition GUID derived from the disk GUID and an index
/// (deterministic; uniqueness comes from the disk GUID's randomness).
pub fn part_guid(disk_guid: &[u8; 16], index: u8) -> [u8; 16] {
    let mut g = *disk_guid;
    g[15] = g[15].wrapping_add(index + 1);
    g[8] ^= 0xA5;
    g
}

/// A random disk GUID from the OS RNG (if-core's getrandom wrapper is
/// not exposed; use a time+pid mix hashed with our CRC — uniqueness for
/// partition tables only needs to be practically unique, not secure).
pub fn new_disk_guid() -> [u8; 16] {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let mut seed = Vec::with_capacity(32);
    seed.extend_from_slice(&t.to_le_bytes());
    seed.extend_from_slice(&std::process::id().to_le_bytes());
    seed.extend_from_slice(&t.rotate_left(7).to_le_bytes());
    let mut out = [0u8; 16];
    for i in 0..4 {
        let c = crc32(&seed[i * 8..]);
        // Vary the input per word so the four words differ.
        let c = c ^ crc32(&[(i as u8) * 31 + 7]);
        out[i * 4..i * 4 + 4].copy_from_slice(&c.to_le_bytes());
    }
    // Mark as version-4-style GUID for tidiness.
    out[7] = (out[7] & 0x0F) | 0x40;
    out[8] = (out[8] & 0x3F) | 0x80;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_known_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn guid_byte_order() {
        let g = guid_bytes(GPT_ESP);
        // C12A7328 little-endian first field: 28 73 2A C1.
        assert_eq!(&g[0..4], &[0x28, 0x73, 0x2A, 0xC1]);
        assert_eq!(&g[8..10], &[0xBA, 0x4B]);
    }

    #[test]
    fn mbr_shape() {
        let part = PartSpec { first_lba: 2048, sectors: 1_000_000, bootable: true };
        let m = mbr_sector(2_000_000, &part, 0x0C, 0x1234_5678);
        assert_eq!([m[510], m[511]], [0x55, 0xAA]);
        assert_eq!(m[446], 0x80);
        assert_eq!(m[446 + 4], 0x0C);
        assert_eq!(u32::from_le_bytes(m[446 + 8..446 + 12].try_into().unwrap()), 2048);
        assert_eq!(u32::from_le_bytes(m[446 + 12..446 + 16].try_into().unwrap()), 1_000_000);
        assert_eq!(u32::from_le_bytes(m[440..444].try_into().unwrap()), 0x1234_5678);
    }

    #[test]
    fn single_partition_alignment() {
        let total = 61_734_912u64; // ~29.5 GiB in sectors
        let p = single_partition(total, true).unwrap();
        assert_eq!(p.first_lba, 2048);
        assert_eq!(p.sectors % 2048, 0);
        assert!(p.last_lba() < total - 33);
        let m = single_partition(total, false).unwrap();
        assert!(m.last_lba() < total);
        assert!(single_partition(1000, true).is_err());
    }

    #[test]
    fn gpt_writes_verify() {
        let total = 1_000_000u64;
        let spec = single_partition(total, true).unwrap();
        let dg = new_disk_guid();
        let parts =
            vec![GptPart { type_guid: guid_bytes(GPT_ESP), unique_guid: part_guid(&dg, 0), spec, name: "BOOT".into() }];
        let writes = gpt_writes(total, &parts, dg);
        assert_eq!(writes.len(), 5);
        // Header CRC checks out when recomputed with the CRC field zeroed.
        let hdr = &writes[1].1;
        assert_eq!(&hdr[0..8], b"EFI PART");
        let stored = u32::from_le_bytes(hdr[16..20].try_into().unwrap());
        let mut h = hdr[..92].to_vec();
        h[16..20].fill(0);
        assert_eq!(crc32(&h), stored);
        // Entries CRC in the header matches the entries bytes.
        let entries_crc = u32::from_le_bytes(hdr[88..92].try_into().unwrap());
        assert_eq!(crc32(&writes[2].1), entries_crc);
        // Backup header sits at the last sector and points back.
        let (off, bh) = &writes[4];
        assert_eq!(*off, total - 1);
        assert_eq!(u64::from_le_bytes(bh[24..32].try_into().unwrap()), total - 1);
        assert_eq!(u64::from_le_bytes(bh[32..40].try_into().unwrap()), 1);
        // Protective MBR type.
        assert_eq!(writes[0].1[446 + 4], 0xEE);
    }
}
