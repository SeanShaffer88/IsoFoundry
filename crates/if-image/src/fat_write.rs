use crate::fat::{FatLayout, FatType};
use crate::write::{self, Kind, Node, Source, Summary};
use crate::{bad, Error, Result};
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const COPY_BUFFER: usize = 1 << 20;
const MAX_EDIT_CLUSTERS: u64 = 4_000_000;
const MAX_FAT_CLUSTERS: u32 = 16_000_000;
const MAX_FAT_TABLE_BYTES: u64 = 64 << 20;

struct Progress<F> {
    callback: F,
    done: u64,
    total: u64,
}

impl<F: FnMut(u64, u64) -> bool> Progress<F> {
    fn tick(&mut self, amount: u64) -> Result<()> {
        self.done = self.done.saturating_add(amount);
        if !(self.callback)(self.done, self.total) {
            return Err(Error::Cancelled);
        }
        Ok(())
    }
}

struct Planned {
    name: String,
    short: [u8; 11],
    file: Option<(Source, u64)>,
    children: Vec<Planned>,
    clusters: Vec<u32>,
}

impl Planned {
    fn is_dir(&self) -> bool {
        self.file.is_none()
    }
}

fn validate_name(name: &str) -> Result<Vec<u16>> {
    let units: Vec<u16> = name.encode_utf16().collect();
    if units.is_empty()
        || units.len() > 255
        || name.ends_with(['.', ' '])
        || name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
    {
        return bad(format!("FAT cannot store this filename: {name:?}"));
    }
    Ok(units)
}

fn alias(name: &str, used: &mut HashSet<[u8; 11]>) -> Result<[u8; 11]> {
    const ALLOWED: &str = "$%'-_@~`!(){}^#&";
    let (stem, extension) = name.rsplit_once('.').unwrap_or((name, ""));
    let clean = |part: &str| {
        part.chars()
            .filter_map(|c| {
                let c = c.to_ascii_uppercase();
                (c.is_ascii_alphanumeric() || ALLOWED.contains(c)).then_some(c as u8)
            })
            .collect::<Vec<_>>()
    };
    let stem = clean(stem);
    let extension = clean(extension);
    let stem = if stem.is_empty() { b"FILE".to_vec() } else { stem };
    for suffix in 1..=999_999u32 {
        let tail = format!("~{suffix}");
        let mut candidate = [b' '; 11];
        let stem_len = (8 - tail.len()).min(stem.len());
        candidate[..stem_len].copy_from_slice(&stem[..stem_len]);
        candidate[stem_len..stem_len + tail.len()].copy_from_slice(tail.as_bytes());
        candidate[8..8 + extension.len().min(3)].copy_from_slice(&extension[..extension.len().min(3)]);
        if used.insert(candidate) {
            return Ok(candidate);
        }
    }
    bad(format!("too many FAT filename collisions in {name:?}"))
}

fn plan(node: &Node, root: bool, depth: u32) -> Result<Planned> {
    if depth > 64 {
        return bad("folders nested deeper than 64 levels cannot be written to FAT");
    }
    let mut used = HashSet::new();
    let mut planned =
        Planned { name: node.name.clone(), short: [b' '; 11], file: None, children: Vec::new(), clusters: Vec::new() };
    match &node.kind {
        Kind::File { source, size } => {
            validate_name(&node.name)?;
            if *size > u64::from(u32::MAX) {
                return bad(format!("FAT cannot store files larger than 4 GiB: {}", node.name));
            }
            planned.file = Some((source.clone(), *size));
        }
        Kind::Dir(children) => {
            if children.len() > 100_000 {
                return bad(format!("{} contains too many entries for FAT editing", node.name));
            }
            for child in children {
                validate_name(&child.name)?;
                let short = alias(&child.name, &mut used)?;
                let mut child_plan = plan(child, false, depth + 1)?;
                child_plan.short = short;
                planned.children.push(child_plan);
            }
            if root {
                planned.name.clear();
            }
        }
    }
    Ok(planned)
}

fn count_files(node: &Planned) -> (u64, u64, u64) {
    if let Some((_, size)) = &node.file {
        (1, 0, *size)
    } else {
        node.children.iter().fold((0, 1, 0), |(files, dirs, bytes), child| {
            let (f, d, b) = count_files(child);
            (files + f, dirs + d, bytes + b)
        })
    }
}

fn cluster_bytes(layout: FatLayout) -> Result<u64> {
    layout
        .bytes_per_sector
        .checked_mul(layout.sectors_per_cluster)
        .ok_or_else(|| Error::Format("FAT cluster size overflow".into()))
}

fn read_fat_entry(data: &[u8], layout: FatLayout, cluster: u32) -> Result<u32> {
    let (offset, width) = match layout.kind {
        FatType::Fat12 => (u64::from(cluster) + u64::from(cluster) / 2, 2),
        FatType::Fat16 => (u64::from(cluster) * 2, 2),
        FatType::Fat32 => (u64::from(cluster) * 4, 4),
    };
    if offset + width > layout.fat_size {
        return bad("FAT table is too small for the volume");
    }
    let start = usize::try_from(offset).map_err(|_| Error::Format("FAT offset exceeds address space".into()))?;
    let end = start + width as usize;
    let bytes = data.get(start..end).ok_or_else(|| Error::Format("FAT table is too small for the volume".into()))?;
    let value = match layout.kind {
        FatType::Fat12 => {
            let word = u16::from_le_bytes([bytes[0], bytes[1]]);
            u32::from(if cluster & 1 == 0 { word & 0x0fff } else { word >> 4 })
        }
        FatType::Fat16 => u32::from(u16::from_le_bytes([bytes[0], bytes[1]])),
        FatType::Fat32 => u32::from_le_bytes(bytes.try_into().unwrap()) & 0x0fff_ffff,
    };
    Ok(value)
}

fn special_value(kind: FatType, value: u32) -> bool {
    match kind {
        FatType::Fat12 => (0x0ff0..=0x0ff7).contains(&value),
        FatType::Fat16 => (0xfff0..=0xfff7).contains(&value),
        FatType::Fat32 => (0x0fff_fff0..=0x0fff_fff7).contains(&value),
    }
}

fn set_fat_entry(data: &mut [u8], layout: FatLayout, cluster: u32, value: u32) -> Result<()> {
    let (offset, bytes) = match layout.kind {
        FatType::Fat12 => {
            let offset = u64::from(cluster) + u64::from(cluster) / 2;
            let start =
                usize::try_from(offset).map_err(|_| Error::Format("FAT offset exceeds address space".into()))?;
            let old = u16::from_le_bytes(
                data.get(start..start + 2)
                    .ok_or_else(|| Error::Format("FAT table is too small for the volume".into()))?
                    .try_into()
                    .unwrap(),
            );
            let new = if cluster & 1 == 0 {
                (old & 0xf000) | (value as u16 & 0x0fff)
            } else {
                (old & 0x000f) | ((value as u16 & 0x0fff) << 4)
            };
            (start, new.to_le_bytes().to_vec())
        }
        FatType::Fat16 => (cluster as usize * 2, (value as u16).to_le_bytes().to_vec()),
        FatType::Fat32 => (cluster as usize * 4, (value & 0x0fff_ffff).to_le_bytes().to_vec()),
    };
    let end = offset + bytes.len();
    let target =
        data.get_mut(offset..end).ok_or_else(|| Error::Format("FAT table is too small for the volume".into()))?;
    target.copy_from_slice(&bytes);
    Ok(())
}

struct Allocator {
    layout: FatLayout,
    next: u32,
    blocked: Vec<bool>,
    fat_data: Vec<u8>,
    allocated: u64,
}

impl Allocator {
    fn new(file: &mut File, layout: FatLayout) -> Result<Self> {
        if layout.fat_size > MAX_FAT_TABLE_BYTES {
            return bad("FAT tables larger than 64 MB cannot be edited safely yet");
        }
        let fat_len =
            usize::try_from(layout.fat_size).map_err(|_| Error::Format("FAT table exceeds address space".into()))?;
        let mut old_fat = vec![0; fat_len];
        file.seek(SeekFrom::Start(layout.fat_offset))?;
        file.read_exact(&mut old_fat)?;
        let blocked_len = usize::try_from(layout.cluster_count)
            .map_err(|_| Error::Format("FAT cluster count exceeds address space".into()))?
            .saturating_add(2);
        let mut blocked = vec![false; blocked_len];
        let mut fat_data = vec![0; fat_len];
        for cluster in 2..=layout.cluster_count + 1 {
            let value = read_fat_entry(&old_fat, layout, cluster)?;
            if special_value(layout.kind, value) {
                blocked[cluster as usize] = true;
                set_fat_entry(&mut fat_data, layout, cluster, value)?;
            }
        }
        let reserved = match layout.kind {
            FatType::Fat12 => [layout.media_descriptor as u32, 0x0fff],
            FatType::Fat16 => [0xff00 | u32::from(layout.media_descriptor), 0xffff],
            FatType::Fat32 => [0x0fff_ff00 | u32::from(layout.media_descriptor), 0x0fff_ffff],
        };
        set_fat_entry(&mut fat_data, layout, 0, reserved[0])?;
        set_fat_entry(&mut fat_data, layout, 1, reserved[1])?;
        Ok(Self { layout, next: 2, blocked, fat_data, allocated: 0 })
    }

    fn allocate(&mut self, count: u64) -> Result<Vec<u32>> {
        if self.allocated.saturating_add(count) > MAX_EDIT_CLUSTERS {
            return bad("editing this volume would require too many clusters in memory");
        }
        let count = usize::try_from(count).map_err(|_| Error::Format("too many FAT clusters requested".into()))?;
        let mut clusters = Vec::with_capacity(count);
        while clusters.len() < count {
            if self.next > self.layout.cluster_count + 1 {
                return bad("the edited files do not fit in the FAT volume");
            }
            let cluster = self.next;
            self.next += 1;
            if self.blocked[cluster as usize] {
                continue;
            }
            if let Some(previous) = clusters.last().copied() {
                set_fat_entry(&mut self.fat_data, self.layout, previous, cluster)?;
            }
            clusters.push(cluster);
        }
        if let Some(last) = clusters.last().copied() {
            set_fat_entry(&mut self.fat_data, self.layout, last, end_marker(self.layout.kind))?;
        }
        self.allocated += count as u64;
        Ok(clusters)
    }
}

fn end_marker(kind: FatType) -> u32 {
    match kind {
        FatType::Fat12 => 0x0fff,
        FatType::Fat16 => 0xffff,
        FatType::Fat32 => 0x0fff_ffff,
    }
}

fn directory_records(node: &Planned, root: bool) -> Result<u64> {
    let mut records = u64::from(!root) * 2;
    for child in &node.children {
        let units = validate_name(&child.name)?;
        records += (units.len().div_ceil(13) + 1) as u64;
    }
    if root {
        records += 1;
    }
    Ok(records)
}

fn allocate_tree(node: &mut Planned, root: bool, layout: FatLayout, allocator: &mut Allocator) -> Result<()> {
    let cb = cluster_bytes(layout)?;
    if node.is_dir() && (!root || matches!(layout.kind, FatType::Fat32)) {
        let bytes = directory_records(node, root)?
            .checked_mul(32)
            .ok_or_else(|| Error::Format("FAT directory size overflow".into()))?;
        let count = bytes.div_ceil(cb).max(1);
        node.clusters = allocator.allocate(count)?;
    } else if let Some((_, size)) = &node.file {
        if *size != 0 {
            node.clusters = allocator.allocate(size.div_ceil(cb))?;
        }
    }
    for child in &mut node.children {
        allocate_tree(child, false, layout, allocator)?;
    }
    Ok(())
}

fn lfn_records(name: &str, short: &[u8; 11]) -> Result<Vec<[u8; 32]>> {
    let units = validate_name(name)?;
    let slots = units.len().div_ceil(13);
    let checksum = short
        .iter()
        .fold(0u8, |sum, byte| (if sum & 1 != 0 { 0x80u8 } else { 0u8 }).wrapping_add(sum >> 1).wrapping_add(*byte));
    let mut output = Vec::with_capacity(slots + 1);
    for sequence in (1..=slots).rev() {
        let mut record = [0xffu8; 32];
        record[0] = sequence as u8 | if sequence == slots { 0x40 } else { 0 };
        record[11] = 0x0f;
        record[12] = 0;
        record[13] = checksum;
        record[26..28].fill(0);
        for index in 0..13 {
            let position = (sequence - 1) * 13 + index;
            let unit = if position < units.len() {
                units[position]
            } else if position == units.len() {
                0
            } else {
                0xffff
            };
            let offset = match index {
                0..=4 => 1 + index * 2,
                5..=10 => 14 + (index - 5) * 2,
                _ => 28 + (index - 11) * 2,
            };
            record[offset..offset + 2].copy_from_slice(&unit.to_le_bytes());
        }
        output.push(record);
    }
    Ok(output)
}

fn short_entry(node: &Planned) -> [u8; 32] {
    let mut record = [0u8; 32];
    record[..11].copy_from_slice(&node.short);
    record[11] = if node.is_dir() { 0x10 } else { 0x20 };
    let cluster = node.clusters.first().copied().unwrap_or(0);
    record[20..22].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
    record[26..28].copy_from_slice(&(cluster as u16).to_le_bytes());
    let size = node.file.as_ref().map(|(_, size)| *size).unwrap_or(0);
    record[28..32].copy_from_slice(&(size as u32).to_le_bytes());
    record
}

fn dot_record(name: &[u8; 11], cluster: u32) -> [u8; 32] {
    let mut record = [0u8; 32];
    record[..11].copy_from_slice(name);
    record[11] = 0x10;
    record[20..22].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
    record[26..28].copy_from_slice(&(cluster as u16).to_le_bytes());
    record
}

fn label_record(label: &str) -> Result<[u8; 32]> {
    let mut record = [0u8; 32];
    let mut label_bytes = [b' '; 11];
    let label = label.trim().to_ascii_uppercase();
    if label.len() > 11
        || !label.is_ascii()
        || label.chars().any(|c| {
            c.is_control()
                || matches!(
                    c,
                    '"' | '*' | '+' | ',' | '.' | '/' | ':' | ';' | '<' | '=' | '>' | '?' | '[' | '\\' | ']' | '|'
                )
        })
    {
        return bad("FAT volume labels must be at most 11 ASCII characters and cannot contain path characters");
    }
    label_bytes[..label.len()].copy_from_slice(label.as_bytes());
    record[..11].copy_from_slice(&label_bytes);
    record[11] = 0x08;
    Ok(record)
}

fn build_directory(node: &Planned, root: bool, parent_cluster: u32, label: &str) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    if !root {
        output.extend_from_slice(&dot_record(b".          ", node.clusters[0]));
        output.extend_from_slice(&dot_record(b"..         ", parent_cluster));
    } else {
        output.extend_from_slice(&label_record(label)?);
    }
    for child in &node.children {
        for record in lfn_records(&child.name, &child.short)? {
            output.extend_from_slice(&record);
        }
        if child.is_dir() {
            let mut record = [0u8; 32];
            record[..11].copy_from_slice(&child.short);
            record[11] = 0x10;
            let cluster = child.clusters[0];
            record[20..22].copy_from_slice(&((cluster >> 16) as u16).to_le_bytes());
            record[26..28].copy_from_slice(&(cluster as u16).to_le_bytes());
            output.extend_from_slice(&record);
        } else {
            output.extend_from_slice(&short_entry(child));
        }
    }
    Ok(output)
}

fn write_file_data(
    output: &mut File,
    layout: FatLayout,
    node: &Planned,
    progress: &mut Progress<impl FnMut(u64, u64) -> bool>,
) -> Result<()> {
    let Some((source, size)) = &node.file else { return Ok(()) };
    if *size == 0 {
        return Ok(());
    }
    let cb = usize::try_from(cluster_bytes(layout)?)
        .map_err(|_| Error::Format("FAT cluster size exceeds address space".into()))?;
    let mut reader = write::reader(source, *size)?;
    let mut remaining = *size;
    for cluster in &node.clusters {
        let count = remaining.min(cb as u64) as usize;
        let mut data = vec![0u8; cb];
        reader.read_exact(&mut data[..count])?;
        let offset = layout.data_offset + u64::from(*cluster - 2) * cb as u64;
        output.seek(SeekFrom::Start(offset))?;
        output.write_all(&data)?;
        remaining -= count as u64;
        progress.tick(count as u64)?;
    }
    Ok(())
}

fn write_tree_data(
    output: &mut File,
    layout: FatLayout,
    node: &Planned,
    root: bool,
    parent_cluster: u32,
    label: &str,
    progress: &mut Progress<impl FnMut(u64, u64) -> bool>,
) -> Result<()> {
    if node.file.is_some() {
        return write_file_data(output, layout, node, progress);
    }
    for child in &node.children {
        write_tree_data(output, layout, child, false, node.clusters.first().copied().unwrap_or(0), label, progress)?;
    }
    let bytes = build_directory(node, root, parent_cluster, label)?;
    let offset = if root && !matches!(layout.kind, FatType::Fat32) {
        layout.fat_offset + layout.fat_size * layout.fat_count
    } else {
        layout.data_offset + u64::from(node.clusters[0] - 2) * cluster_bytes(layout)?
    };
    let capacity = if root && !matches!(layout.kind, FatType::Fat32) {
        layout.root_entries * 32
    } else {
        node.clusters.len() as u64 * cluster_bytes(layout)?
    };
    if bytes.len() as u64 > capacity {
        return bad(format!("{} exceeds its FAT directory capacity", node.name));
    }
    output.seek(SeekFrom::Start(offset))?;
    output.write_all(&bytes)?;
    let remaining = capacity - bytes.len() as u64;
    write_zeroes(output, remaining)?;
    progress.tick(0)
}

fn write_zeroes(output: &mut File, mut count: u64) -> Result<()> {
    let zeroes = [0u8; 64 * 1024];
    while count != 0 {
        let n = count.min(zeroes.len() as u64) as usize;
        output.write_all(&zeroes[..n])?;
        count -= n as u64;
    }
    Ok(())
}

fn copy_original(
    src: &Path,
    part: &Path,
    created: &mut bool,
    progress: &mut Progress<impl FnMut(u64, u64) -> bool>,
) -> Result<()> {
    let mut input = File::open(src)?;
    let mut output = OpenOptions::new().write(true).create_new(true).open(part)?;
    *created = true;
    let mut buffer = vec![0u8; COPY_BUFFER];
    loop {
        let n = input.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        output.write_all(&buffer[..n])?;
        progress.tick(n as u64)?;
    }
    output.flush()?;
    Ok(())
}

fn check_fat_mirrors(file: &mut File, layout: FatLayout) -> Result<()> {
    if layout.fat_count < 2 {
        return Ok(());
    }
    let mut left = vec![0; COPY_BUFFER];
    let mut right = vec![0; COPY_BUFFER];
    let mut offset = 0;
    while offset < layout.fat_size {
        let n = (layout.fat_size - offset).min(COPY_BUFFER as u64) as usize;
        file.seek(SeekFrom::Start(layout.fat_offset + offset))?;
        file.read_exact(&mut left[..n])?;
        file.seek(SeekFrom::Start(layout.fat_offset + layout.fat_size + offset))?;
        file.read_exact(&mut right[..n])?;
        if left[..n] != right[..n] {
            return bad("FAT copies differ; editing this volume is unsafe");
        }
        offset += n as u64;
    }
    Ok(())
}

fn write_fat_copy(
    output: &mut File,
    layout: FatLayout,
    progress: &mut Progress<impl FnMut(u64, u64) -> bool>,
) -> Result<()> {
    if layout.fat_count == 1 {
        return Ok(());
    }
    let second = layout.fat_offset + layout.fat_size;
    let mut buffer = vec![0u8; COPY_BUFFER];
    let mut offset = 0;
    while offset < layout.fat_size {
        let n = (layout.fat_size - offset).min(COPY_BUFFER as u64) as usize;
        output.seek(SeekFrom::Start(layout.fat_offset + offset))?;
        output.read_exact(&mut buffer[..n])?;
        output.seek(SeekFrom::Start(second + offset))?;
        output.write_all(&buffer[..n])?;
        offset += n as u64;
    }
    progress.tick(layout.fat_size * layout.fat_count)
}

fn write_fat_table(output: &mut File, layout: FatLayout, data: &[u8]) -> Result<()> {
    output.seek(SeekFrom::Start(layout.fat_offset))?;
    output.write_all(data)?;
    Ok(())
}

fn update_boot_metadata(output: &mut File, layout: FatLayout, label: &str, root_cluster: u32) -> Result<()> {
    let label = label.trim().to_ascii_uppercase();
    let label_offset = if matches!(layout.kind, FatType::Fat32) { 71 } else { 43 };
    let signature_offset = label_offset - 5;
    let write_boot = |file: &mut File, sector: u64| -> Result<()> {
        let base = layout.base + sector * layout.bytes_per_sector;
        file.seek(SeekFrom::Start(base + signature_offset))?;
        let mut signature = [0u8; 1];
        file.read_exact(&mut signature)?;
        if signature[0] == 0x29 {
            file.seek(SeekFrom::Start(base + label_offset))?;
            let mut field = [b' '; 11];
            field[..label.len()].copy_from_slice(label.as_bytes());
            file.write_all(&field)?;
        } else if !label.is_empty() {
            return bad("this FAT volume has no extended boot signature for a safe volume-label update");
        }
        Ok(())
    };
    write_boot(output, 0)?;
    if matches!(layout.kind, FatType::Fat32)
        && layout.backup_boot_sector != 0
        && layout.backup_boot_sector != u16::MAX
        && u64::from(layout.backup_boot_sector) < layout.reserved_sectors
    {
        write_boot(output, u64::from(layout.backup_boot_sector))?;
        output
            .seek(SeekFrom::Start(layout.base + 44 + u64::from(layout.backup_boot_sector) * layout.bytes_per_sector))?;
        output.write_all(&root_cluster.to_le_bytes())?;
    }
    if matches!(layout.kind, FatType::Fat32) {
        output.seek(SeekFrom::Start(layout.base + 44))?;
        output.write_all(&root_cluster.to_le_bytes())?;
        if layout.fs_info_sector != 0
            && layout.fs_info_sector != u16::MAX
            && u64::from(layout.fs_info_sector) < layout.reserved_sectors
        {
            update_fs_info(output, layout.base + u64::from(layout.fs_info_sector) * layout.bytes_per_sector)?;
            let backup_info = u64::from(layout.backup_boot_sector)
                .checked_add(u64::from(layout.fs_info_sector))
                .filter(|sector| *sector < layout.reserved_sectors);
            if let Some(sector) = backup_info {
                update_fs_info(output, layout.base + sector * layout.bytes_per_sector)?;
            }
        }
    }
    Ok(())
}

fn update_fs_info(output: &mut File, offset: u64) -> Result<()> {
    let mut signatures = [0u8; 512];
    output.seek(SeekFrom::Start(offset))?;
    output.read_exact(&mut signatures)?;
    if signatures[..4] == 0x4161_5252u32.to_le_bytes()
        && signatures[484..488] == 0x6141_7272u32.to_le_bytes()
        && signatures[508..512] == 0xaa55_0000u32.to_le_bytes()
    {
        output.seek(SeekFrom::Start(offset + 488))?;
        output.write_all(&u32::MAX.to_le_bytes())?;
        output.write_all(&u32::MAX.to_le_bytes())?;
    }
    Ok(())
}

fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

pub(crate) fn build<F: FnMut(u64, u64) -> bool>(
    root: &Node,
    label: &str,
    layout: FatLayout,
    source: &Path,
    dest: &Path,
    mut callback: F,
) -> Result<Summary> {
    if dest.exists() {
        return bad(format!("destination already exists: {}", dest.display()));
    }
    let image_bytes = std::fs::metadata(source)?.len();
    if layout.cluster_count > MAX_FAT_CLUSTERS {
        return bad("FAT volumes with more than 16 million clusters cannot be edited safely yet");
    }
    let volume_end = layout
        .total_sectors
        .checked_mul(layout.bytes_per_sector)
        .and_then(|size| layout.base.checked_add(size))
        .ok_or_else(|| Error::Format("FAT volume size overflow".into()))?;
    if volume_end > image_bytes {
        return bad("FAT volume extends beyond the source image");
    }
    let mut planned = plan(root, true, 0)?;
    let (files, dirs, file_bytes) = count_files(&planned);
    let total = image_bytes.saturating_mul(3).saturating_add(file_bytes).max(1);
    let mut progress = Progress { callback: &mut callback, done: 0, total };
    let part = part_path(dest);
    let mut created_part = false;
    let result = (|| -> Result<Summary> {
        copy_original(source, &part, &mut created_part, &mut progress)?;
        let mut output = OpenOptions::new().read(true).write(true).open(&part)?;
        check_fat_mirrors(&mut output, layout)?;
        let mut allocator = Allocator::new(&mut output, layout)?;
        allocate_tree(&mut planned, true, layout, &mut allocator)?;
        let root_cluster = planned.clusters.first().copied().unwrap_or(0);
        write_tree_data(&mut output, layout, &planned, true, 0, label, &mut progress)?;
        write_fat_table(&mut output, layout, &allocator.fat_data)?;
        write_fat_copy(&mut output, layout, &mut progress)?;
        update_boot_metadata(&mut output, layout, label, root_cluster)?;
        output.sync_all()?;
        drop(output);
        if dest.exists() {
            return bad(format!("destination already exists: {}", dest.display()));
        }
        std::fs::rename(&part, dest)?;
        Ok(Summary { sectors: image_bytes.div_ceil(crate::SECTOR as u64), files, dirs })
    })();
    if result.is_err() && created_part {
        let _ = std::fs::remove_file(&part);
    }
    result
}
