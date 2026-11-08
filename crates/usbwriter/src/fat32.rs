//! A FAT32 formatter and file writer over any [`BlockDevice`].
//!
//! Deliberately write-focused: it formats a partition, creates folders
//! and streams files in (from ISO images, via `Read`), which is all the
//! USB jobs need. Structures follow the Microsoft FAT specification:
//! 32 reserved sectors, two FATs, FSInfo + backup boot sector, long-file
//! name entries with the SFN checksum. The FAT is held in memory and
//! written back on [`Fat32Volume::sync`] (and when large files finish),
//! so a cancelled job can leave the volume inconsistent — the job as a
//! whole has failed at that point and the stick is rewritten next time.

use crate::device::BlockDevice;
use crate::part::SECTOR;
use crate::{err, Result};
use std::io::Read;

const EOC: u32 = 0x0FFF_FFFF;
const MEDIA: u32 = 0x0FFF_FFF8;

/// Geometry chosen at format time.
#[derive(Debug, Clone, Copy)]
pub struct Geometry {
    pub sectors_per_cluster: u8,
    pub reserved_sectors: u32,
    pub fat_sectors: u32,
    pub total_clusters: u32,
}

fn choose_spc(total_sectors: u64) -> u8 {
    // Microsoft's FAT32 defaults by volume size (512-byte sectors).
    match total_sectors {
        0..=532_479 => 1,
        532_480..=16_777_215 => 8,
        16_777_216..=33_554_431 => 8,
        33_554_432..=67_108_863 => 16,
        67_108_864..=134_217_727 => 32,
        _ => 64,
    }
}

/// Formats the partition starting at byte offset `base` (length
/// `sectors` x 512) on `dev` and returns the open volume. `hidden` is
/// the partition's first LBA on the whole disk (BPB bookkeeping).
pub fn format<D: BlockDevice>(mut dev: D, base: u64, sectors: u64, hidden: u32, label: &str) -> Result<Fat32Volume<D>> {
    if sectors < 65_536 {
        return err("partition is too small for FAT32 (needs at least 32 MB)");
    }
    if sectors > u32::MAX as u64 {
        return err("partition is too large for FAT32");
    }
    let spc = choose_spc(sectors) as u32;
    let reserved = 32u32;
    // Iterate the FAT size: clusters depend on it and it depends on the
    // cluster count.
    let mut fat_sectors = (((sectors / spc as u64) * 4).div_ceil(SECTOR)) as u32;
    for _ in 0..8 {
        let data_sectors = sectors as u32 - reserved - 2 * fat_sectors;
        let clusters = data_sectors / spc;
        let need = ((clusters + 2) * 4).div_ceil(SECTOR as u32);
        if need == fat_sectors {
            break;
        }
        fat_sectors = need;
    }
    let data_sectors = sectors as u32 - reserved - 2 * fat_sectors;
    let total_clusters = data_sectors / spc;
    if total_clusters < 65_525 {
        return err("partition geometry does not reach the FAT32 minimum cluster count");
    }

    let label11 = label_field(label);
    let volume_id = crate::part::crc32(label.as_bytes()) ^ (std::process::id() << 7);

    // Zero the reserved area, both FATs and the root cluster, writing
    // the structures on top.
    let zero = vec![0u8; 1 << 20];
    let zero_range = |dev: &mut D, from_sector: u64, sectors: u64| -> Result<()> {
        let mut left = sectors * SECTOR;
        let mut at = base + from_sector * SECTOR;
        while left > 0 {
            let n = (zero.len() as u64).min(left) as usize;
            dev.write_at(at, &zero[..n])?;
            at += n as u64;
            left -= n as u64;
        }
        Ok(())
    };
    zero_range(&mut dev, 0, reserved as u64)?;
    zero_range(&mut dev, reserved as u64, fat_sectors as u64 * 2)?;
    let data_start = reserved + 2 * fat_sectors;
    zero_range(&mut dev, data_start as u64, spc as u64)?;

    // Boot sector.
    let mut bs = [0u8; 512];
    bs[0..3].copy_from_slice(&[0xEB, 0x58, 0x90]);
    bs[3..11].copy_from_slice(b"ISOFDY32");
    bs[11..13].copy_from_slice(&(SECTOR as u16).to_le_bytes());
    bs[13] = spc as u8;
    bs[14..16].copy_from_slice(&(reserved as u16).to_le_bytes());
    bs[16] = 2;
    bs[21] = 0xF8;
    bs[24..26].copy_from_slice(&63u16.to_le_bytes());
    bs[26..28].copy_from_slice(&255u16.to_le_bytes());
    bs[28..32].copy_from_slice(&hidden.to_le_bytes());
    bs[32..36].copy_from_slice(&(sectors as u32).to_le_bytes());
    bs[36..40].copy_from_slice(&fat_sectors.to_le_bytes());
    bs[44..48].copy_from_slice(&2u32.to_le_bytes()); // root cluster
    bs[48..50].copy_from_slice(&1u16.to_le_bytes()); // FSInfo sector
    bs[50..52].copy_from_slice(&6u16.to_le_bytes()); // backup boot sector
    bs[64] = 0x80;
    bs[66] = 0x29;
    bs[67..71].copy_from_slice(&volume_id.to_le_bytes());
    bs[71..82].copy_from_slice(&label11);
    bs[82..90].copy_from_slice(b"FAT32   ");
    bs[510] = 0x55;
    bs[511] = 0xAA;
    dev.write_at(base, &bs)?;
    dev.write_at(base + 6 * SECTOR, &bs)?; // backup

    // FSInfo: free = all clusters except the root's, next free = 3.
    let mut fsinfo = [0u8; 512];
    fsinfo[0..4].copy_from_slice(&0x4161_5252u32.to_le_bytes());
    fsinfo[484..488].copy_from_slice(&0x6141_7272u32.to_le_bytes());
    fsinfo[488..492].copy_from_slice(&(total_clusters - 1).to_le_bytes());
    fsinfo[492..496].copy_from_slice(&3u32.to_le_bytes());
    fsinfo[508..512].copy_from_slice(&0xAA55_0000u32.to_le_bytes());
    dev.write_at(base + SECTOR, &fsinfo)?;
    dev.write_at(base + 7 * SECTOR, &fsinfo)?;

    let mut vol = Fat32Volume {
        dev,
        base,
        spc,
        reserved,
        fat_sectors,
        data_start,
        total_clusters,
        fat: vec![0; (total_clusters + 2) as usize],
        next_free: 3,
        label: label11,
    };
    vol.fat[0] = MEDIA;
    vol.fat[1] = EOC;
    vol.fat[2] = EOC; // root directory, one cluster
    vol.sync()?;
    Ok(vol)
}

fn label_field(label: &str) -> [u8; 11] {
    let mut out = [b' '; 11];
    for (i, c) in label.chars().take(11).enumerate() {
        out[i] = if c.is_ascii() { c.to_ascii_uppercase() as u8 } else { b'_' };
    }
    out
}

/// An open, writable FAT32 volume.
pub struct Fat32Volume<D: BlockDevice> {
    dev: D,
    base: u64,
    spc: u32,
    reserved: u32,
    fat_sectors: u32,
    data_start: u32,
    total_clusters: u32,
    fat: Vec<u32>,
    next_free: u32,
    label: [u8; 11],
}

impl<D: BlockDevice> Fat32Volume<D> {
    /// Opens an existing FAT32 volume (validates the BPB and loads the
    /// first FAT). Used to add files after formatting, and by tests.
    pub fn open(dev: D, base: u64) -> Result<Self> {
        let mut bs = [0u8; 512];
        dev.read_at(base, &mut bs)?;
        if bs[510] != 0x55 || bs[511] != 0xAA || &bs[82..90] != b"FAT32   " {
            return err("not a FAT32 volume (bad boot sector)");
        }
        let spc = bs[13] as u32;
        if spc == 0 || !spc.is_power_of_two() {
            return err("not a FAT32 volume (bad cluster size)");
        }
        let reserved = u16::from_le_bytes(bs[14..16].try_into().unwrap()) as u32;
        let fat_sectors = u32::from_le_bytes(bs[36..40].try_into().unwrap());
        let total = u32::from_le_bytes(bs[32..36].try_into().unwrap()) as u64;
        let data_start = reserved + 2 * fat_sectors;
        let total_clusters = ((total - data_start as u64) / spc as u64) as u32;
        if total_clusters + 2 > 64_000_000 {
            return err("FAT is implausibly large");
        }
        let mut fat_bytes = vec![0u8; (fat_sectors as u64 * SECTOR) as usize];
        dev.read_at(base + reserved as u64 * SECTOR, &mut fat_bytes)?;
        let mut fat = vec![0u32; (total_clusters + 2) as usize];
        for (i, slot) in fat.iter_mut().enumerate() {
            let o = i * 4;
            if o + 4 <= fat_bytes.len() {
                *slot = u32::from_le_bytes(fat_bytes[o..o + 4].try_into().unwrap()) & 0x0FFF_FFFF;
            }
        }
        let mut label = [b' '; 11];
        label.copy_from_slice(&bs[71..82]);
        Ok(Fat32Volume { dev, base, spc, reserved, fat_sectors, data_start, total_clusters, fat, next_free: 3, label })
    }

    pub fn total_clusters(&self) -> u32 {
        self.total_clusters
    }

    pub fn cluster_bytes(&self) -> u64 {
        self.spc as u64 * SECTOR
    }

    fn cluster_off(&self, cluster: u32) -> u64 {
        self.base + (self.data_start as u64 + (cluster as u64 - 2) * self.spc as u64) * SECTOR
    }

    /// Writes both FAT copies back to the device.
    pub fn sync(&mut self) -> Result<()> {
        let mut bytes = vec![0u8; self.fat.len() * 4];
        for (i, v) in self.fat.iter().enumerate() {
            bytes[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        let fat_bytes_len = self.fat_sectors as u64 * SECTOR;
        if (bytes.len() as u64) < fat_bytes_len {
            bytes.resize(fat_bytes_len as usize, 0);
        }
        let fat1 = self.base + self.reserved as u64 * SECTOR;
        self.dev.write_at(fat1, &bytes[..fat_bytes_len as usize])?;
        self.dev.write_at(fat1 + fat_bytes_len, &bytes[..fat_bytes_len as usize])?;
        Ok(())
    }

    pub fn flush(&mut self) -> Result<()> {
        self.dev.flush()
    }

    pub fn into_device(self) -> D {
        self.dev
    }

    pub fn device_mut(&mut self) -> &mut D {
        &mut self.dev
    }

    fn alloc_cluster(&mut self) -> Result<u32> {
        let n = self.fat.len() as u32;
        for step in 0..n {
            let c = 2 + (self.next_free - 2 + step) % (n - 2);
            if self.fat[c as usize] == 0 {
                self.fat[c as usize] = EOC;
                self.next_free = c + 1;
                if self.next_free >= n {
                    self.next_free = 3;
                }
                // Zero the cluster so stale data never leaks into files
                // or directory slack.
                let zeros = vec![0u8; self.cluster_bytes() as usize];
                let off = self.cluster_off(c);
                self.dev.write_at(off, &zeros)?;
                return Ok(c);
            }
        }
        err("the USB stick is full")
    }

    fn chain(&self, first: u32) -> Vec<u32> {
        let mut out = Vec::new();
        let mut c = first;
        while (2..0x0FFF_FFF8).contains(&c) && (c as usize) < self.fat.len() {
            out.push(c);
            c = self.fat[c as usize];
        }
        out
    }

    /// Reads a whole directory (raw 32-byte entries).
    fn read_dir(&mut self, first_cluster: u32) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        let cb = self.cluster_bytes() as usize;
        for c in self.chain(first_cluster) {
            let mut buf = vec![0u8; cb];
            let off = self.cluster_off(c);
            self.dev.read_at(off, &mut buf)?;
            bytes.extend_from_slice(&buf);
        }
        Ok(bytes)
    }

    fn existing_sfns(&mut self, dir_cluster: u32) -> Result<Vec<[u8; 11]>> {
        let bytes = self.read_dir(dir_cluster)?;
        let mut out = Vec::new();
        for e in bytes.chunks_exact(32) {
            if e[0] == 0x00 {
                break;
            }
            if e[0] != 0xE5 && e[11] != 0x0F {
                let mut sfn = [0u8; 11];
                sfn.copy_from_slice(&e[0..11]);
                out.push(sfn);
            }
        }
        Ok(out)
    }

    /// Ensures a directory path exists (e.g. "EFI/BOOT"), returning its
    /// first cluster. Components are created as needed.
    pub fn ensure_dir(&mut self, path: &str) -> Result<u32> {
        let mut cur = 2u32; // root
        for part in path.split('/').filter(|p| !p.is_empty()) {
            cur = self.ensure_child_dir(cur, part)?;
        }
        Ok(cur)
    }

    fn find_child_dir(&mut self, parent: u32, name: &str) -> Result<Option<u32>> {
        let bytes = self.read_dir(parent)?;
        let mut pending_lfn = String::new();
        for e in bytes.chunks_exact(32) {
            if e[0] == 0x00 {
                break;
            }
            if e[0] == 0xE5 {
                pending_lfn.clear();
                continue;
            }
            if e[11] == 0x0F {
                // LFN entry: collect characters (entries come in
                // reverse order; prepending chunks in encounter order
                // works because the last-seen chunk is the first part).
                let mut chunk = String::new();
                for off in (1..11).step_by(2).chain((14..26).step_by(2)).chain((28..32).step_by(2)) {
                    let u = u16::from_le_bytes([e[off], e[off + 1]]);
                    if u == 0 || u == 0xFFFF {
                        break;
                    }
                    chunk.push(char::from_u32(u as u32).unwrap_or('_'));
                }
                pending_lfn = chunk + &pending_lfn;
                continue;
            }
            let is_dir = e[11] & 0x10 != 0;
            let entry_name = if !pending_lfn.is_empty() { std::mem::take(&mut pending_lfn) } else { sfn_display(e) };
            if is_dir && entry_name.eq_ignore_ascii_case(name) {
                let lo = u16::from_le_bytes(e[26..28].try_into().unwrap()) as u32;
                let hi = u16::from_le_bytes(e[20..22].try_into().unwrap()) as u32;
                return Ok(Some((hi << 16) | lo));
            }
            pending_lfn.clear();
        }
        Ok(None)
    }

    fn ensure_child_dir(&mut self, parent: u32, name: &str) -> Result<u32> {
        if let Some(c) = self.find_child_dir(parent, name)? {
            return Ok(c);
        }
        let cluster = self.alloc_cluster()?;
        // "." and ".." entries.
        let cb = self.cluster_bytes() as usize;
        let mut block = vec![0u8; cb];
        let (time, date) = dos_now();
        let dot = make_entry(b".          ", 0x10, cluster, 0, time, date, 0);
        block[0..32].copy_from_slice(&dot);
        let dotdot = make_entry(b"..         ", 0x10, if parent == 2 { 0 } else { parent }, 0, time, date, 0);
        block[32..64].copy_from_slice(&dotdot);
        let off = self.cluster_off(cluster);
        self.dev.write_at(off, &block)?;
        self.append_entry(parent, name, true, cluster, 0)?;
        Ok(cluster)
    }

    /// Writes `entry_bytes` (LFN set + SFN entry) into the first free
    /// run of slots in the directory, growing it by a cluster if full.
    fn append_entry(
        &mut self,
        dir_cluster: u32,
        name: &str,
        is_dir: bool,
        first_cluster: u32,
        size: u32,
    ) -> Result<()> {
        let existing = self.existing_sfns(dir_cluster)?;
        let (sfn, need_lfn, nt_flags) = make_sfn(name, &existing);
        let (time, date) = dos_now();
        let sfn_entry = make_entry(&sfn, if is_dir { 0x10 } else { 0x20 }, first_cluster, size, time, date, nt_flags);
        let mut entries: Vec<[u8; 32]> = Vec::new();
        if need_lfn {
            entries.extend(lfn_entries(name, &sfn));
        }
        entries.push(sfn_entry);
        let need = entries.len();

        // Find a run of free slots; remember each cluster's device
        // range so we can write in place.
        let chain = self.chain(dir_cluster);
        let cb = self.cluster_bytes() as usize;
        let mut slots: Vec<(u64, usize)> = Vec::new(); // (file offset in dir bytes, cluster idx)
        let mut found_at: Option<usize> = None;
        'scan: {
            let mut run = 0usize;
            for (ci, c) in chain.iter().enumerate() {
                let mut buf = vec![0u8; cb];
                let off = self.cluster_off(*c);
                self.dev.read_at(off, &mut buf)?;
                for (ei, e) in buf.chunks_exact(32).enumerate() {
                    let idx = ci * (cb / 32) + ei;
                    slots.push((idx as u64 * 32, ci));
                    if e[0] == 0x00 || e[0] == 0xE5 {
                        run += 1;
                        if run >= need {
                            found_at = Some(idx + 1 - need);
                            break 'scan;
                        }
                    } else {
                        run = 0;
                    }
                }
            }
        }
        let start = match found_at {
            Some(s) => s,
            None => {
                // Grow the directory by one cluster and use its start.
                let new_c = self.alloc_cluster()?;
                let last = *chain.last().unwrap();
                self.fat[last as usize] = new_c;
                slots.clear();
                (chain.len() * (cb / 32)) as u64 as usize
            }
        };
        // Write the entries, cluster by cluster.
        let per_cluster = cb / 32;
        let mut written = 0usize;
        while written < need {
            let slot = start + written;
            let ci = slot / per_cluster;
            let within = slot % per_cluster;
            let cluster = if ci < chain.len() {
                chain[ci]
            } else {
                // The freshly allocated growth cluster.
                self.chain(dir_cluster)[ci]
            };
            let count = (per_cluster - within).min(need - written);
            let mut bytes = Vec::with_capacity(count * 32);
            for k in 0..count {
                bytes.extend_from_slice(&entries[written + k]);
            }
            let off = self.cluster_off(cluster) + (within * 32) as u64;
            self.dev.write_at(off, &bytes)?;
            written += count;
        }
        Ok(())
    }

    /// Streams a file into the volume at `path` (e.g.
    /// "SOURCES/INSTALL.WIM"), creating folders as needed. `size` is
    /// the exact byte count; `src` must yield at least that many.
    pub fn write_file(&mut self, path: &str, src: &mut dyn Read, size: u64) -> Result<()> {
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        if parts.is_empty() {
            return err("empty file path");
        }
        let (dirs, name) = parts.split_at(parts.len() - 1);
        let mut dir_cluster = 2u32;
        for d in dirs {
            dir_cluster = self.ensure_child_dir(dir_cluster, d)?;
        }
        // Stream the data cluster by cluster.
        let cb = self.cluster_bytes();
        let mut first = 0u32;
        let mut prev = 0u32;
        let mut left = size;
        let mut buf = vec![0u8; cb as usize];
        while left > 0 {
            let take = cb.min(left) as usize;
            buf.fill(0);
            src.read_exact(&mut buf[..take])?;
            let c = self.alloc_cluster()?;
            let off = self.cluster_off(c);
            self.dev.write_at(off, &buf)?;
            if first == 0 {
                first = c;
            }
            if prev != 0 {
                self.fat[prev as usize] = c;
            }
            prev = c;
            left -= take as u64;
        }
        self.append_entry(dir_cluster, name[0], false, first, size as u32)?;
        // Checkpoint after big files so a late failure keeps them.
        if size >= 64 * 1024 * 1024 {
            self.sync()?;
        }
        Ok(())
    }

    /// The volume label as stored in the boot sector.
    pub fn label(&self) -> String {
        String::from_utf8_lossy(&self.label).trim_end().to_string()
    }
}

fn sfn_display(e: &[u8]) -> String {
    let stem = String::from_utf8_lossy(&e[0..8]).trim_end().to_string();
    let ext = String::from_utf8_lossy(&e[8..11]).trim_end().to_string();
    if ext.is_empty() {
        stem
    } else {
        format!("{stem}.{ext}")
    }
}

/// Builds the 8.3 name for `name`, unique within `existing`, plus
/// whether an LFN entry set is needed to preserve the real name and the
/// NT case flags (byte 12: bit 3 = lowercase stem, bit 4 = lowercase
/// extension) for names that are uniformly cased.
pub fn make_sfn(name: &str, existing: &[[u8; 11]]) -> ([u8; 11], bool, u8) {
    let (stem_raw, ext_raw) = match name.rfind('.') {
        Some(i) if i > 0 => (&name[..i], &name[i + 1..]),
        _ => (name, ""),
    };
    let clean = |s: &str| -> String {
        s.chars()
            .map(|c| {
                let u = c.to_ascii_uppercase();
                if u.is_ascii_alphanumeric() || "!#$%&'()-@^_`{}~".contains(u) {
                    u
                } else {
                    '_'
                }
            })
            .collect()
    };
    let stem = clean(stem_raw);
    let ext = clean(ext_raw);
    let fits = stem_raw.len() <= 8
        && ext_raw.len() <= 3
        && stem == stem_raw.to_ascii_uppercase()
        && ext == ext_raw.to_ascii_uppercase()
        && name.chars().all(|c| c.is_ascii() && c != ' ');
    let uniform_case = |t: &str| {
        t.is_empty()
            || t.chars().all(|c| !c.is_ascii_alphabetic() || c.is_ascii_lowercase())
            || t.chars().all(|c| !c.is_ascii_alphabetic() || c.is_ascii_uppercase())
    };
    let case_needs_lfn = !(uniform_case(stem_raw) && uniform_case(ext_raw));
    let mut nt_flags = 0u8;
    if !stem_raw.is_empty() && stem_raw.chars().all(|c| !c.is_ascii_alphabetic() || c.is_ascii_lowercase()) {
        nt_flags |= 0x08;
    }
    if !ext_raw.is_empty() && ext_raw.chars().all(|c| !c.is_ascii_alphabetic() || c.is_ascii_lowercase()) {
        nt_flags |= 0x10;
    }
    let base_stem: String = stem.chars().take(8).collect();
    let base_ext: String = ext.chars().take(3).collect();
    for attempt in 0..100u32 {
        let candidate_stem = if attempt == 0 {
            base_stem.clone()
        } else {
            let suffix = format!("~{attempt}");
            let keep = 8usize.saturating_sub(suffix.len());
            format!("{}{}", &base_stem[..keep.min(base_stem.len())], suffix)
        };
        let mut sfn = [b' '; 11];
        sfn[..candidate_stem.len()].copy_from_slice(candidate_stem.as_bytes());
        sfn[8..8 + base_ext.len()].copy_from_slice(base_ext.as_bytes());
        if !existing.contains(&sfn) {
            let lfn = !fits || case_needs_lfn || attempt > 0;
            return (sfn, lfn, if lfn { 0 } else { nt_flags });
        }
    }
    // Pathological: fall back to a hashed name.
    let h = crate::part::crc32(name.as_bytes());
    let mut sfn = [b' '; 11];
    let text = format!("F{h:07X}");
    sfn[..8].copy_from_slice(&text.as_bytes()[..8]);
    (sfn, true, 0)
}

fn sfn_checksum(sfn: &[u8; 11]) -> u8 {
    let mut sum = 0u8;
    for &b in sfn.iter() {
        sum = sum.rotate_right(1);
        sum = sum.wrapping_add(b);
    }
    sum
}

fn lfn_entries(name: &str, sfn: &[u8; 11]) -> Vec<[u8; 32]> {
    let units: Vec<u16> = name.encode_utf16().collect();
    let chunks: Vec<&[u16]> = units.chunks(13).collect();
    let checksum = sfn_checksum(sfn);
    let mut out = Vec::with_capacity(chunks.len());
    // Entries are written last-chunk first, with the 0x40 flag on the
    // first physical entry.
    for (rev, chunk) in chunks.iter().enumerate().rev() {
        let seq = (chunks.len() - rev) as u8;
        let mut e = [0xFFu8; 32];
        e[0] = if rev == 0 { seq | 0x40 } else { seq };
        e[11] = 0x0F;
        e[12] = 0;
        e[13] = checksum;
        e[26] = 0;
        e[27] = 0;
        let positions: Vec<usize> = (1..11).step_by(2).chain((14..26).step_by(2)).chain((28..32).step_by(2)).collect();
        for (i, pos) in positions.iter().enumerate() {
            let v = if i < chunk.len() {
                chunk[i]
            } else if i == chunk.len() {
                0x0000
            } else {
                0xFFFF
            };
            e[*pos..*pos + 2].copy_from_slice(&v.to_le_bytes());
        }
        out.push(e);
    }
    out
}

fn make_entry(sfn: &[u8; 11], attr: u8, cluster: u32, size: u32, time: u16, date: u16, nt_flags: u8) -> [u8; 32] {
    let mut e = [0u8; 32];
    e[0..11].copy_from_slice(sfn);
    e[11] = attr;
    e[12] = nt_flags;
    e[14..16].copy_from_slice(&time.to_le_bytes()); // create time
    e[16..18].copy_from_slice(&date.to_le_bytes()); // create date
    e[18..20].copy_from_slice(&date.to_le_bytes()); // access date
    e[20..22].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
    e[22..24].copy_from_slice(&time.to_le_bytes()); // write time
    e[24..26].copy_from_slice(&date.to_le_bytes()); // write date
    e[26..28].copy_from_slice(&(cluster as u16).to_le_bytes());
    e[28..32].copy_from_slice(&size.to_le_bytes());
    e
}

/// Current time as DOS (time, date), via a civil-date conversion.
pub fn dos_now() -> (u16, u16) {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let tod = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    if y < 1980 {
        return (0, 0x0021); // 1980-01-01
    }
    let date = (((y - 1980) as u16) << 9) | ((m as u16) << 5) | d as u16;
    let time = (((tod / 3600) as u16) << 11) | ((((tod % 3600) / 60) as u16) << 5) | ((tod % 60) as u16 / 2);
    (time, date)
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    // Howard Hinnant's algorithm.
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::FileDevice;
    use std::io::Cursor;

    fn temp_dev(name: &str, mib: u64) -> (std::path::PathBuf, FileDevice) {
        let dir = std::env::temp_dir().join(format!("usbw-fat-{}-{}", std::process::id(), name));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("part.img");
        let dev = FileDevice::create(&p, mib << 20).unwrap();
        (p, dev)
    }

    #[test]
    fn sfn_rules() {
        let (sfn, lfn, _) = make_sfn("BOOTX64.EFI", &[]);
        assert_eq!(&sfn, b"BOOTX64 EFI");
        assert!(!lfn);
        let (sfn, lfn, nt) = make_sfn("bootmgfw.efi", &[]);
        assert_eq!(&sfn, b"BOOTMGFWEFI");
        assert!(!lfn); // uniformly lowercase -> NT case flags, no LFN
        assert_eq!(nt, 0x18);
        let (_, lfn, _) = make_sfn("BootMgfw.efi", &[]);
        assert!(lfn); // mixed case -> LFN preserves it
        let (sfn, lfn, _) = make_sfn("my long file name.txt", &[]);
        assert!(lfn);
        assert_eq!(&sfn[0..8], b"MY_LONG_");
        let existing = vec![{
            let (s, _, _) = make_sfn("README.TXT", &[]);
            s
        }];
        let (s2, _, _) = make_sfn("README.TXT", &existing);
        assert_ne!(s2, existing[0]);
    }

    #[test]
    fn format_and_write_files() {
        let (p, dev) = temp_dev("a", 128);
        let mut vol = format(dev, 0, 128 * 2048, 2048, "SETUP").unwrap();
        assert_eq!(vol.label(), "SETUP");
        vol.write_file("HELLO.TXT", &mut Cursor::new(b"hello world".to_vec()), 11).unwrap();
        let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        vol.write_file("EFI/BOOT/BOOTX64.EFI", &mut Cursor::new(big.clone()), big.len() as u64).unwrap();
        vol.write_file("EFI/BOOT/SECOND.BIN", &mut Cursor::new(vec![7u8; 100]), 100).unwrap();
        vol.sync().unwrap();
        let dev = vol.into_device();
        drop(dev);

        // Reopen with the independent reader (open() parses the BPB).
        let dev = FileDevice::open(&p, true).unwrap();
        let mut vol = Fat32Volume::open(dev, 0).unwrap();
        // Walk root: find HELLO.TXT via the LFN/SFN listing helper.
        let root = vol.read_dir(2).unwrap();
        let mut found = false;
        for e in root.chunks_exact(32) {
            if e[0] == 0 {
                break;
            }
            if &e[0..11] == b"HELLO   TXT" {
                found = true;
                let size = u32::from_le_bytes(e[28..32].try_into().unwrap());
                assert_eq!(size, 11);
                let lo = u16::from_le_bytes(e[26..28].try_into().unwrap()) as u32;
                let hi = u16::from_le_bytes(e[20..22].try_into().unwrap()) as u32;
                let first = (hi << 16) | lo;
                let chain = vol.chain(first);
                assert_eq!(chain.len(), 1);
                let mut buf = vec![0u8; vol.cluster_bytes() as usize];
                let off = vol.cluster_off(first);
                vol.dev.read_at(off, &mut buf).unwrap();
                assert_eq!(&buf[..11], b"hello world");
            }
        }
        assert!(found, "HELLO.TXT in root");
        // The nested EFI/BOOT dir was created and holds BOOTX64.EFI.
        let efi = vol.find_child_dir(2, "EFI").unwrap().expect("EFI dir");
        let boot = vol.find_child_dir(efi, "BOOT").unwrap().expect("BOOT dir");
        let entries = vol.existing_sfns(boot).unwrap();
        assert!(entries.iter().any(|s| s == b"BOOTX64 EFI"));
        assert!(entries.iter().any(|s| s == b"SECOND  BIN"));
        // BOOTX64.EFI content round-trips through its chain.
        let bytes = vol.read_dir(boot).unwrap();
        let mut data = Vec::new();
        for e in bytes.chunks_exact(32) {
            if &e[0..11] == b"BOOTX64 EFI" {
                let lo = u16::from_le_bytes(e[26..28].try_into().unwrap()) as u32;
                let hi = u16::from_le_bytes(e[20..22].try_into().unwrap()) as u32;
                let size = u32::from_le_bytes(e[28..32].try_into().unwrap()) as usize;
                for c in vol.chain((hi << 16) | lo) {
                    let mut buf = vec![0u8; vol.cluster_bytes() as usize];
                    let off = vol.cluster_off(c);
                    vol.dev.read_at(off, &mut buf).unwrap();
                    data.extend_from_slice(&buf);
                }
                data.truncate(size);
            }
        }
        assert_eq!(data, big);
        let _ = std::fs::remove_dir_all(p.parent().unwrap());
    }

    #[test]
    fn format_rejects_tiny() {
        let (_p, dev) = temp_dev("b", 16);
        assert!(format(dev, 0, 16 * 2048, 0, "X").is_err());
    }

    #[test]
    fn dos_date_sane() {
        let (_, date) = dos_now();
        let year = 1980 + (date >> 9);
        assert!((2024..=2100).contains(&year));
    }
}
