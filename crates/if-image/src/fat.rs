//! Read-only FAT12/16/32 support for raw floppy and MBR-partitioned images.

use crate::fs::{read_data, ByteExtent, Data, Entry, FileSystem, FsKind};
use crate::source::{read_bytes, SectorSource};
use crate::{bad, Result, SECTOR};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

const MAX_DIRECTORY_BYTES: u64 = 64 << 20;
const MAX_DIRECTORY_ENTRIES: usize = 100_000;
const FAT_CACHE_PAGES: usize = 64;

#[derive(Clone, Copy)]
pub(crate) enum FatType {
    Fat12,
    Fat16,
    Fat32,
}

#[derive(Clone, Copy)]
pub(crate) struct FatLayout {
    pub kind: FatType,
    pub base: u64,
    pub bytes_per_sector: u64,
    pub sectors_per_cluster: u64,
    pub reserved_sectors: u64,
    pub fat_count: u64,
    pub root_entries: u64,
    pub total_sectors: u64,
    pub cluster_count: u32,
    pub fat_offset: u64,
    pub fat_size: u64,
    pub data_offset: u64,
    pub backup_boot_sector: u16,
    pub fs_info_sector: u16,
    pub media_descriptor: u8,
}

pub struct Fat {
    kind: FatType,
    base: u64,
    bytes_per_sector: u64,
    sectors_per_cluster: u64,
    reserved_sectors: u64,
    fat_count: u64,
    root_entries: u64,
    total_sectors: u64,
    cluster_count: u32,
    fat_offset: u64,
    fat_size: u64,
    data_offset: u64,
    volume_end: u64,
    root_cluster: u32,
    backup_boot_sector: u16,
    fs_info_sector: u16,
    media_descriptor: u8,
    root: Entry,
    label: String,
    fat_cache: RefCell<HashMap<u64, Vec<u8>>>,
}

impl Fat {
    /// Finds a FAT superfloppy or the first supported FAT partition in an MBR.
    pub fn probe(src: &mut dyn SectorSource) -> Result<Option<Self>> {
        let image_bytes = src.byte_len();
        if image_bytes < 512 {
            return Ok(None);
        }
        let mbr = read_bytes(src, 0, 0, 512)?;
        let has_partition_table = mbr[510..512] == [0x55, 0xaa]
            && (0..4).any(|index| {
                let entry = &mbr[446 + index * 16..462 + index * 16];
                matches!(entry[0], 0x00 | 0x80)
                    && entry[4] != 0
                    && entry[8..12].iter().any(|byte| *byte != 0)
                    && entry[12..16].iter().any(|byte| *byte != 0)
            });
        if !has_partition_table {
            if let Some(fat) = Self::at(src, 0, image_bytes)? {
                return Ok(Some(fat));
            }
        }
        if mbr[510..512] != [0x55, 0xaa] {
            return Ok(None);
        }
        for index in 0..4 {
            let entry = &mbr[446 + index * 16..462 + index * 16];
            if !is_fat_partition(entry[4]) {
                continue;
            }
            let start = u64::from(u32::from_le_bytes(entry[8..12].try_into().unwrap()))
                .checked_mul(512)
                .ok_or_else(|| crate::Error::Format("partition offset overflow".into()))?;
            let length = u64::from(u32::from_le_bytes(entry[12..16].try_into().unwrap()))
                .checked_mul(512)
                .ok_or_else(|| crate::Error::Format("partition size overflow".into()))?;
            if length == 0 || start.checked_add(length).is_none_or(|end| end > image_bytes) {
                return bad("FAT partition extends beyond the image");
            }
            return Self::at(src, start, length)
                .and_then(|fat| fat.ok_or_else(|| crate::Error::Format("invalid FAT partition boot sector".into())))
                .map(Some);
        }
        Ok(None)
    }

    fn at(src: &mut dyn SectorSource, base: u64, available: u64) -> Result<Option<Self>> {
        if available < 512 {
            return Ok(None);
        }
        let boot = read_bytes(src, 0, base, 512)?;
        if &boot[3..11] == b"EXFAT   " {
            return bad("exFAT volumes are not supported");
        }
        if !matches!(boot[0], 0xeb | 0xe9) || &boot[3..11] == b"NTFS    " {
            return Ok(None);
        }
        let bytes_per_sector = u64::from(u16::from_le_bytes([boot[11], boot[12]]));
        if !matches!(bytes_per_sector, 512 | 1024 | 2048 | 4096) {
            return bad("unsupported FAT bytes-per-sector value");
        }
        let full_boot = read_bytes(src, 0, base, bytes_per_sector as usize)?;
        if full_boot[bytes_per_sector as usize - 2..] != [0x55, 0xaa] {
            return Ok(None);
        }

        let sectors_per_cluster = u64::from(boot[13]);
        let reserved_sectors = u64::from(u16::from_le_bytes([boot[14], boot[15]]));
        let fat_count = u64::from(boot[16]);
        let root_entries = u64::from(u16::from_le_bytes([boot[17], boot[18]]));
        let total16 = u64::from(u16::from_le_bytes([boot[19], boot[20]]));
        let fat16 = u64::from(u16::from_le_bytes([boot[22], boot[23]]));
        let total32 = u64::from(u32::from_le_bytes(boot[32..36].try_into().unwrap()));
        let fat32 = u64::from(u32::from_le_bytes(boot[36..40].try_into().unwrap()));
        let total_sectors = if total16 != 0 { total16 } else { total32 };
        let fat_sectors = if fat16 != 0 { fat16 } else { fat32 };
        if !sectors_per_cluster.is_power_of_two()
            || sectors_per_cluster > 128
            || reserved_sectors == 0
            || !(1..=2).contains(&fat_count)
            || total_sectors == 0
            || fat_sectors == 0
        {
            return bad("invalid FAT volume geometry");
        }

        let root_dir_sectors = root_entries
            .checked_mul(32)
            .and_then(|n| n.checked_add(bytes_per_sector - 1))
            .ok_or_else(|| crate::Error::Format("FAT root directory size overflow".into()))?
            / bytes_per_sector;
        let first_data_sector = reserved_sectors
            .checked_add(
                fat_count
                    .checked_mul(fat_sectors)
                    .ok_or_else(|| crate::Error::Format("FAT region size overflow".into()))?,
            )
            .and_then(|n| n.checked_add(root_dir_sectors))
            .ok_or_else(|| crate::Error::Format("FAT data offset overflow".into()))?;
        if first_data_sector >= total_sectors {
            return bad("FAT data region is outside the volume");
        }
        let cluster_count = (total_sectors - first_data_sector) / sectors_per_cluster;
        let cluster_count = u32::try_from(cluster_count)
            .map_err(|_| crate::Error::Format("FAT volume has too many clusters".into()))?;
        let kind = if cluster_count < 4085 {
            FatType::Fat12
        } else if cluster_count < 65_525 {
            FatType::Fat16
        } else {
            FatType::Fat32
        };
        if cluster_count > 0x0fff_ffed {
            return bad("FAT volume exceeds the supported cluster-number range");
        }
        if matches!(kind, FatType::Fat32) && root_entries != 0 {
            return bad("invalid FAT32 root directory geometry");
        }
        if !matches!(kind, FatType::Fat32) && (root_entries == 0 || fat16 == 0) {
            return bad("invalid FAT12/16 root directory geometry");
        }

        let entries = u64::from(cluster_count)
            .checked_add(2)
            .ok_or_else(|| crate::Error::Format("FAT entry count overflow".into()))?;
        let required_fat_bytes = match kind {
            FatType::Fat12 => entries.checked_mul(3).and_then(|n| n.checked_add(1)).map(|n| n / 2),
            FatType::Fat16 => entries.checked_mul(2),
            FatType::Fat32 => entries.checked_mul(4),
        }
        .ok_or_else(|| crate::Error::Format("FAT size overflow".into()))?;
        let fat_size = fat_sectors
            .checked_mul(bytes_per_sector)
            .ok_or_else(|| crate::Error::Format("FAT size overflow".into()))?;
        if fat_size < required_fat_bytes {
            return bad("FAT table is too small for the volume");
        }

        let volume_size = total_sectors
            .checked_mul(bytes_per_sector)
            .ok_or_else(|| crate::Error::Format("FAT volume size overflow".into()))?;
        if volume_size > available || base.checked_add(volume_size).is_none_or(|end| end > src.byte_len()) {
            return bad("FAT volume extends beyond the image");
        }
        let fat_offset = base
            .checked_add(
                reserved_sectors
                    .checked_mul(bytes_per_sector)
                    .ok_or_else(|| crate::Error::Format("FAT offset overflow".into()))?,
            )
            .ok_or_else(|| crate::Error::Format("FAT offset overflow".into()))?;
        let data_offset = base
            .checked_add(
                first_data_sector
                    .checked_mul(bytes_per_sector)
                    .ok_or_else(|| crate::Error::Format("FAT data offset overflow".into()))?,
            )
            .ok_or_else(|| crate::Error::Format("FAT data offset overflow".into()))?;
        let volume_end = base + volume_size;

        let label_offset: usize = if matches!(kind, FatType::Fat32) { 71 } else { 43 };
        let label = if full_boot.get(label_offset.wrapping_sub(5)) == Some(&0x29) {
            String::from_utf8_lossy(&full_boot[label_offset..label_offset + 11]).trim().to_string()
        } else {
            String::new()
        };
        let root_cluster = if matches!(kind, FatType::Fat32) {
            u32::from_le_bytes(full_boot[44..48].try_into().unwrap()) & 0x0fff_ffff
        } else {
            0
        };
        let backup_boot_sector = if matches!(kind, FatType::Fat32) {
            u16::from_le_bytes(full_boot[50..52].try_into().unwrap())
        } else {
            u16::MAX
        };
        let fs_info_sector = if matches!(kind, FatType::Fat32) {
            u16::from_le_bytes(full_boot[48..50].try_into().unwrap())
        } else {
            u16::MAX
        };
        let mut fat = Self {
            kind,
            base,
            bytes_per_sector,
            sectors_per_cluster,
            reserved_sectors,
            fat_count,
            root_entries,
            total_sectors,
            cluster_count,
            fat_offset,
            fat_size,
            data_offset,
            volume_end,
            root_cluster,
            backup_boot_sector,
            fs_info_sector,
            media_descriptor: boot[21],
            root: Entry {
                name: String::new(),
                dir: true,
                size: 0,
                mtime: None,
                hidden: false,
                data: Data::ByteExtents(Vec::new()),
            },
            label,
            fat_cache: RefCell::new(HashMap::new()),
        };

        let (root_data, root_size) = if matches!(kind, FatType::Fat32) {
            let cluster = fat.root_cluster;
            if cluster < 2 || cluster > cluster_count + 1 {
                return bad("invalid FAT32 root directory cluster");
            }
            let extents = fat.cluster_extents(src, cluster, None)?;
            let size = extents.iter().map(|e| e.len).sum();
            if size > MAX_DIRECTORY_BYTES {
                return bad("FAT root directory is larger than 64 MB");
            }
            (extents, size)
        } else {
            let root_sector = reserved_sectors + fat_count * fat_sectors;
            let size = root_dir_sectors * bytes_per_sector;
            (vec![ByteExtent { offset: base + root_sector * bytes_per_sector, len: size }], size)
        };
        fat.root.size = root_size;
        fat.root.data = Data::ByteExtents(root_data);
        Ok(Some(fat))
    }

    pub(crate) fn layout(&self) -> FatLayout {
        FatLayout {
            kind: self.kind,
            base: self.base,
            bytes_per_sector: self.bytes_per_sector,
            sectors_per_cluster: self.sectors_per_cluster,
            reserved_sectors: self.reserved_sectors,
            fat_count: self.fat_count,
            root_entries: self.root_entries,
            total_sectors: self.total_sectors,
            cluster_count: self.cluster_count,
            fat_offset: self.fat_offset,
            fat_size: self.fat_size,
            data_offset: self.data_offset,
            backup_boot_sector: self.backup_boot_sector,
            fs_info_sector: self.fs_info_sector,
            media_descriptor: self.media_descriptor,
        }
    }

    fn fat_bytes(&self, src: &mut dyn SectorSource, offset: u64, len: usize) -> Result<Vec<u8>> {
        let mut out = vec![0; len];
        let mut done = 0usize;
        while done < len {
            let position = offset + done as u64;
            let absolute = self
                .fat_offset
                .checked_add(position)
                .ok_or_else(|| crate::Error::Format("FAT offset overflow".into()))?;
            let page = absolute / SECTOR as u64;
            let in_page = (absolute % SECTOR as u64) as usize;
            let count = (SECTOR - in_page).min(len - done);
            if !self.fat_cache.borrow().contains_key(&page) {
                let bytes = read_bytes(src, 0, page * SECTOR as u64, SECTOR)?;
                let mut cache = self.fat_cache.borrow_mut();
                if cache.len() >= FAT_CACHE_PAGES {
                    cache.clear();
                }
                cache.insert(page, bytes);
            }
            let cache = self.fat_cache.borrow();
            let bytes = cache.get(&page).expect("FAT cache page was inserted");
            out[done..done + count].copy_from_slice(&bytes[in_page..in_page + count]);
            done += count;
        }
        Ok(out)
    }

    fn next_cluster(&self, src: &mut dyn SectorSource, cluster: u32) -> Result<Option<u32>> {
        let cluster = u64::from(cluster);
        let (offset, width, value, eoc, bad_value) = match self.kind {
            FatType::Fat12 => {
                let offset = cluster + cluster / 2;
                if offset + 2 > self.fat_size {
                    return bad("FAT cluster entry is outside the allocation table");
                }
                let bytes = self.fat_bytes(src, offset, 2)?;
                let pair = u16::from_le_bytes([bytes[0], bytes[1]]);
                let value = if cluster & 1 == 0 { pair & 0x0fff } else { pair >> 4 };
                (offset, 2, u32::from(value), 0x0ff8, 0x0ff7)
            }
            FatType::Fat16 => {
                let offset = cluster * 2;
                if offset + 2 > self.fat_size {
                    return bad("FAT cluster entry is outside the allocation table");
                }
                let bytes = self.fat_bytes(src, offset, 2)?;
                (offset, 2, u32::from(u16::from_le_bytes([bytes[0], bytes[1]])), 0xfff8, 0xfff7)
            }
            FatType::Fat32 => {
                let offset = cluster * 4;
                if offset + 4 > self.fat_size {
                    return bad("FAT cluster entry is outside the allocation table");
                }
                let bytes = self.fat_bytes(src, offset, 4)?;
                (offset, 4, u32::from_le_bytes(bytes[..4].try_into().unwrap()) & 0x0fff_ffff, 0x0fff_fff8, 0x0fff_fff7)
            }
        };
        if offset + width > self.fat_size {
            return bad("FAT cluster entry is outside the allocation table");
        }
        if value >= eoc {
            return Ok(None);
        }
        if value == 0 || value == 1 || value == bad_value || value > self.cluster_count + 1 {
            return bad("invalid cluster in FAT allocation chain");
        }
        Ok(Some(value))
    }

    fn cluster_extents(
        &self,
        src: &mut dyn SectorSource,
        first: u32,
        expected_size: Option<u64>,
    ) -> Result<Vec<ByteExtent>> {
        if first < 2 || first > self.cluster_count + 1 {
            return bad("invalid first cluster in FAT directory entry");
        }
        let cluster_bytes = self.bytes_per_sector * self.sectors_per_cluster;
        let mut extents = Vec::new();
        let mut current = first;
        let mut run_start = first;
        let mut run_count = 0u64;
        let mut total = 0u64;
        let mut slow = Some(first);
        let mut fast = Some(first);

        loop {
            if current < 2 || current > self.cluster_count + 1 {
                return bad("FAT chain references a cluster outside the volume");
            }
            if run_count == 0 {
                run_start = current;
            } else if current != run_start + run_count as u32 {
                self.push_cluster_extent(&mut extents, run_start, run_count, cluster_bytes)?;
                run_start = current;
                run_count = 0;
            }
            run_count += 1;
            total = total
                .checked_add(cluster_bytes)
                .ok_or_else(|| crate::Error::Format("FAT chain size overflow".into()))?;
            if let Some(size) = expected_size {
                if total >= size {
                    break;
                }
            } else if total > MAX_DIRECTORY_BYTES {
                return bad("FAT directory is larger than 64 MB");
            }

            let Some(next) = self.next_cluster(src, current)? else { break };
            current = next;
            slow = match slow {
                Some(cluster) => self.next_cluster(src, cluster)?,
                None => None,
            };
            fast = match fast {
                Some(cluster) => self.next_cluster(src, cluster)?,
                None => None,
            };
            fast = match fast {
                Some(cluster) => self.next_cluster(src, cluster)?,
                None => None,
            };
            if slow.is_some() && slow == fast {
                return bad("cycle in FAT allocation chain");
            }
        }
        if let Some(size) = expected_size {
            if total < size {
                return bad("FAT allocation chain is shorter than the file size");
            }
        }
        if run_count > 0 {
            self.push_cluster_extent(&mut extents, run_start, run_count, cluster_bytes)?;
        }
        if let Some(size) = expected_size {
            let mut remaining = size;
            extents.retain_mut(|extent| {
                extent.len = extent.len.min(remaining);
                remaining -= extent.len;
                extent.len > 0
            });
        }
        Ok(extents)
    }

    fn push_cluster_extent(
        &self,
        extents: &mut Vec<ByteExtent>,
        first: u32,
        clusters: u64,
        cluster_bytes: u64,
    ) -> Result<()> {
        let offset = self
            .data_offset
            .checked_add(
                u64::from(first - 2)
                    .checked_mul(cluster_bytes)
                    .ok_or_else(|| crate::Error::Format("FAT cluster offset overflow".into()))?,
            )
            .ok_or_else(|| crate::Error::Format("FAT cluster offset overflow".into()))?;
        let len = clusters
            .checked_mul(cluster_bytes)
            .ok_or_else(|| crate::Error::Format("FAT extent size overflow".into()))?;
        if offset.checked_add(len).is_none_or(|end| end > self.volume_end) {
            return bad("FAT cluster extent is outside the volume");
        }
        extents.push(ByteExtent { offset, len });
        Ok(())
    }

    fn list_directory(&self, src: &mut dyn SectorSource, dir: &Entry) -> Result<Vec<Entry>> {
        if !dir.dir {
            return bad(format!("{} is not a folder", dir.name));
        }
        if dir.size > MAX_DIRECTORY_BYTES {
            return bad("FAT directory is larger than 64 MB");
        }
        let size = usize::try_from(dir.size).map_err(|_| crate::Error::Format("FAT directory is too large".into()))?;
        let mut bytes = vec![0; size];
        read_data(src, &dir.data, dir.size, 0, &mut bytes)?;

        let mut entries = Vec::new();
        let mut names = HashSet::new();
        let mut long_name = LongName::default();
        for record in bytes.chunks_exact(32) {
            if record[0] == 0 {
                break;
            }
            if record[0] == 0xe5 {
                long_name.clear();
                continue;
            }
            if record[11] == 0x0f {
                long_name.push(record);
                continue;
            }
            let attributes = record[11];
            if attributes & 0x08 != 0 {
                long_name.clear();
                continue;
            }
            let short_name = short_name(record)?;
            let name = long_name.take(&record[..11]).unwrap_or(short_name);
            if name == "." || name == ".." {
                continue;
            }
            if name.is_empty()
                || name.contains(['/', '\\'])
                || name.chars().any(char::is_control)
                || !names.insert(name.to_lowercase())
            {
                return bad(format!("invalid or duplicate FAT filename: {name:?}"));
            }
            if entries.len() == MAX_DIRECTORY_ENTRIES {
                return bad("FAT directory contains too many entries");
            }
            let first_cluster = (u32::from(u16::from_le_bytes([record[20], record[21]])) << 16)
                | u32::from(u16::from_le_bytes([record[26], record[27]]));
            let first_cluster =
                if matches!(self.kind, FatType::Fat32) { first_cluster & 0x0fff_ffff } else { first_cluster };
            let file_size = u64::from(u32::from_le_bytes(record[28..32].try_into().unwrap()));
            let is_dir = attributes & 0x10 != 0;
            let data = if is_dir {
                let extents = self.cluster_extents(src, first_cluster, None)?;
                let size = extents.iter().map(|e| e.len).sum();
                (Data::ByteExtents(extents), size)
            } else if file_size == 0 {
                (Data::ByteExtents(Vec::new()), 0)
            } else {
                (Data::ByteExtents(self.cluster_extents(src, first_cluster, Some(file_size))?), file_size)
            };
            entries.push(Entry {
                name,
                dir: is_dir,
                size: data.1,
                mtime: None,
                hidden: attributes & 0x02 != 0,
                data: data.0,
            });
            long_name.clear();
        }
        Ok(entries)
    }
}

fn is_fat_partition(kind: u8) -> bool {
    matches!(kind, 0x01 | 0x04 | 0x06 | 0x0b | 0x0c | 0x0e | 0x11 | 0x14 | 0x16 | 0x1b | 0x1c | 0x1e)
}

fn short_name(record: &[u8]) -> Result<String> {
    let mut base = record[..8].to_vec();
    let extension = &record[8..11];
    if base.first() == Some(&0x05) {
        base[0] = 0xe5;
    }
    let trim = |bytes: &[u8]| {
        let end = bytes.iter().rposition(|b| *b != b' ').map_or(0, |i| i + 1);
        String::from_utf8_lossy(&bytes[..end]).into_owned()
    };
    let mut name = trim(&base);
    let mut ext = trim(extension);
    if record[12] & 0x08 != 0 {
        name.make_ascii_lowercase();
    }
    if record[12] & 0x10 != 0 {
        ext.make_ascii_lowercase();
    }
    if !ext.is_empty() {
        name.push('.');
        name.push_str(&ext);
    }
    if name.is_empty() {
        return bad("empty FAT short filename");
    }
    Ok(name)
}

#[derive(Default)]
struct LongName {
    checksum: Option<u8>,
    parts: Vec<Option<Vec<u16>>>,
    next_sequence: usize,
    invalid: bool,
}

impl LongName {
    fn clear(&mut self) {
        self.checksum = None;
        self.parts.clear();
        self.next_sequence = 0;
        self.invalid = false;
    }

    fn push(&mut self, record: &[u8]) {
        let sequence = usize::from(record[0] & 0x1f);
        let is_last = record[0] & 0x40 != 0;
        if record[0] & 0xa0 != 0 || record[12] != 0 || record[26..28] != [0, 0] {
            self.invalid = true;
            return;
        }
        if is_last {
            self.clear();
            if sequence == 0 || sequence > 20 {
                self.invalid = true;
                return;
            }
            self.checksum = Some(record[13]);
            self.parts.resize(sequence, None);
            self.next_sequence = sequence;
        }
        if sequence == 0
            || sequence > self.parts.len()
            || self.checksum != Some(record[13])
            || sequence != self.next_sequence
            || self.parts[sequence - 1].is_some()
        {
            self.invalid = true;
            return;
        }
        let mut units = Vec::with_capacity(13);
        for range in [1..11, 14..26, 28..32] {
            for pair in record[range].chunks_exact(2) {
                units.push(u16::from_le_bytes([pair[0], pair[1]]));
            }
        }
        self.parts[sequence - 1] = Some(units);
        self.next_sequence -= 1;
    }

    fn take(&mut self, short: &[u8]) -> Option<String> {
        let expected = short_checksum(short);
        let mut units = Vec::new();
        let valid = !self.invalid
            && self.checksum == Some(expected)
            && self.next_sequence == 0
            && !self.parts.is_empty()
            && self.parts.iter().all(Option::is_some);
        if valid {
            for part in self.parts.iter().flatten() {
                for unit in part {
                    if *unit == 0 || *unit == 0xffff {
                        break;
                    }
                    units.push(*unit);
                }
            }
        }
        self.clear();
        String::from_utf16(&units).ok().filter(|name| !name.is_empty())
    }
}

fn short_checksum(short: &[u8]) -> u8 {
    short
        .iter()
        .fold(0u8, |sum, byte| (if sum & 1 != 0 { 0x80u8 } else { 0u8 }).wrapping_add(sum >> 1).wrapping_add(*byte))
}

impl FileSystem for Fat {
    fn kind(&self) -> FsKind {
        FsKind::Fat
    }

    fn label(&self) -> String {
        self.label.clone()
    }

    fn root(&self) -> Entry {
        self.root.clone()
    }

    fn list(&self, src: &mut dyn SectorSource, dir: &Entry) -> Result<Vec<Entry>> {
        self.list_directory(src, dir)
    }
}
