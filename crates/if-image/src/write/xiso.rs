use super::{reader, Error, Kind, Node, Out, Source, Summary};
use crate::{bad, Result, SECTOR};
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const HEADER_SECTOR: u64 = 32;
const ROOT_SECTOR: u64 = 0x108;
const DIRECTORY_LIMIT: u64 = 64 << 20;
const MAX_NODES: usize = 100_000;
const MAX_DEPTH: usize = 64;
const SECTOR_SIZE: u64 = SECTOR as u64;
const IMAGE_ALIGNMENT_SECTORS: u64 = 32;

struct Plan {
    root: Directory,
    sectors: u64,
    files: u64,
    directories: u64,
}

struct Directory {
    entries: Vec<Entry>,
    order: Vec<usize>,
    table: Vec<u8>,
    allocated_size: u64,
    sector: u32,
}

struct Entry {
    name: Vec<u8>,
    size: u32,
    source: Option<Source>,
    directory: Option<Box<Directory>>,
    sector: u32,
    offset: u64,
    left: Option<usize>,
    right: Option<usize>,
}

fn validate_name(name: &str) -> Result<Vec<u8>> {
    let bytes = name.as_bytes();
    if bytes.is_empty()
        || bytes.len() > u8::MAX as usize
        || name == "."
        || name == ".."
        || name.contains(['/', '\\'])
        || name.chars().any(char::is_control)
    {
        return bad(format!("invalid XDVDFS filename: {name:?}"));
    }
    Ok(bytes.to_vec())
}

fn tree_order(indices: &[usize], entries: &mut [Entry], order: &mut Vec<usize>) -> Option<usize> {
    if indices.is_empty() {
        return None;
    }
    let middle = indices.len() / 2;
    let index = indices[middle];
    order.push(index);
    let left = tree_order(&indices[..middle], entries, order);
    let right = tree_order(&indices[middle + 1..], entries, order);
    entries[index].left = left;
    entries[index].right = right;
    Some(index)
}

fn plan_directory(
    nodes: &[Node],
    depth: usize,
    count: &mut usize,
    files: &mut u64,
    directories: &mut u64,
) -> Result<Directory> {
    if depth > MAX_DEPTH {
        return bad("folders nested deeper than 64 levels");
    }
    let mut names = HashSet::new();
    let mut entries = Vec::with_capacity(nodes.len());
    for node in nodes {
        *count += 1;
        if *count > MAX_NODES {
            return bad("XDVDFS contains too many files and folders");
        }
        let name = validate_name(&node.name)?;
        if !names.insert(node.name.to_ascii_lowercase()) {
            return bad(format!("duplicate XDVDFS filename: {}", node.name));
        }
        let entry = match &node.kind {
            Kind::Dir(children) => {
                *directories += 1;
                let directory = plan_directory(children, depth + 1, count, files, directories)?;
                let stored_size = directory.allocated_size;
                Entry {
                    name,
                    size: u32::try_from(stored_size)
                        .map_err(|_| Error::Format("XDVDFS directory is too large".into()))?,
                    source: None,
                    directory: Some(Box::new(directory)),
                    sector: 0,
                    offset: 0,
                    left: None,
                    right: None,
                }
            }
            Kind::File { source, size } => {
                *files += 1;
                Entry {
                    name,
                    size: u32::try_from(*size).map_err(|_| Error::Format("XDVDFS files cannot exceed 4 GiB".into()))?,
                    source: Some(source.clone()),
                    directory: None,
                    sector: 0,
                    offset: 0,
                    left: None,
                    right: None,
                }
            }
        };
        entries.push(entry);
    }

    let mut indices: Vec<usize> = (0..entries.len()).collect();
    indices.sort_by(|&a, &b| entries[a].name.to_ascii_uppercase().cmp(&entries[b].name.to_ascii_uppercase()));
    let mut order = Vec::with_capacity(entries.len());
    tree_order(&indices, &mut entries, &mut order);

    let mut end = 0u64;
    for &index in &order {
        let record_size = (14 + entries[index].name.len() as u64 + 3) & !3;
        if end % SECTOR_SIZE + record_size > SECTOR_SIZE {
            end = end.div_ceil(SECTOR_SIZE) * SECTOR_SIZE;
        }
        if end / 4 > u16::MAX as u64 {
            return bad("XDVDFS directory exceeds its 16-bit entry-offset limit");
        }
        entries[index].offset = end;
        end = end.checked_add(record_size).ok_or_else(|| Error::Format("XDVDFS directory size overflow".into()))?;
    }
    if end > DIRECTORY_LIMIT {
        return bad("XDVDFS directory is larger than 64 MB");
    }
    for entry in &entries {
        for child in [entry.left, entry.right].into_iter().flatten() {
            if entries[child].offset / 4 > u16::MAX as u64 {
                return bad("XDVDFS directory exceeds its 16-bit entry-offset limit");
            }
        }
    }

    let allocated_size = if end == 0 { SECTOR_SIZE } else { end.div_ceil(SECTOR_SIZE) * SECTOR_SIZE };
    Ok(Directory { entries, order, table: vec![0xff; end as usize], allocated_size, sector: 0 })
}

fn serialize_tables(dir: &mut Directory) {
    for entry in &mut dir.entries {
        if let Some(child) = entry.directory.as_mut() {
            serialize_tables(child);
        }
    }
    let mut table = vec![0xff; dir.table.len()];
    for &index in &dir.order {
        let entry = &dir.entries[index];
        let offset = entry.offset as usize;
        let left = entry.left.map(|i| (dir.entries[i].offset / 4) as u16).unwrap_or(0);
        let right = entry.right.map(|i| (dir.entries[i].offset / 4) as u16).unwrap_or(0);
        table[offset..offset + 2].copy_from_slice(&left.to_le_bytes());
        table[offset + 2..offset + 4].copy_from_slice(&right.to_le_bytes());
        table[offset + 4..offset + 8].copy_from_slice(&entry.sector.to_le_bytes());
        table[offset + 8..offset + 12].copy_from_slice(&entry.size.to_le_bytes());
        table[offset + 12] = if entry.directory.is_some() { 0x10 } else { 0x20 };
        table[offset + 13] = entry.name.len() as u8;
        table[offset + 14..offset + 14 + entry.name.len()].copy_from_slice(&entry.name);
    }
    dir.table = table;
}

fn assign_sectors(dir: &mut Directory, next: &mut u64) -> Result<()> {
    dir.sector =
        u32::try_from(*next).map_err(|_| Error::Format("XDVDFS image exceeds 32-bit sector offsets".into()))?;
    *next = next
        .checked_add(dir.allocated_size / SECTOR_SIZE)
        .ok_or_else(|| Error::Format("XDVDFS image size overflow".into()))?;

    for entry in &mut dir.entries {
        if entry.directory.is_none() {
            entry.sector =
                u32::try_from(*next).map_err(|_| Error::Format("XDVDFS image exceeds 32-bit sector offsets".into()))?;
            *next = next
                .checked_add(u64::from(entry.size).div_ceil(SECTOR_SIZE))
                .ok_or_else(|| Error::Format("XDVDFS image size overflow".into()))?;
        }
    }
    for entry in &mut dir.entries {
        if let Some(child) = entry.directory.as_mut() {
            assign_sectors(child, next)?;
            entry.sector = child.sector;
            entry.size = child.allocated_size as u32;
        }
    }
    Ok(())
}

fn put_xdvdfs_data(out: &mut Out<impl Write>, lba: u64, data: &[u8], allocated_size: u64) -> Result<()> {
    out.zero(lba)?;
    out.w.write_all(data)?;
    let padding = allocated_size - data.len() as u64;
    let pad = [0xff; SECTOR];
    let mut remaining = padding;
    while remaining > 0 {
        let size = remaining.min(SECTOR_SIZE) as usize;
        out.w.write_all(&pad[..size])?;
        remaining -= size as u64;
    }
    out.at += allocated_size / SECTOR_SIZE;
    out.tick()
}

fn write_file(out: &mut Out<impl Write>, entry: &Entry) -> Result<()> {
    let size = u64::from(entry.size);
    out.zero(u64::from(entry.sector))?;
    if size == 0 {
        return Ok(());
    }

    let source = entry.source.as_ref().expect("file source");
    let mut input = reader(source, size)?;
    let mut buffer = vec![0; 1 << 20];
    let mut remaining = size;
    while remaining > 0 {
        let want = remaining.min(buffer.len() as u64) as usize;
        let mut read = 0;
        while read < want {
            let n = input.read(&mut buffer[read..want])?;
            if n == 0 {
                return bad("file data became shorter while the XISO was being built");
            }
            read += n;
        }
        out.w.write_all(&buffer[..read])?;
        remaining -= read as u64;
        out.at += read as u64 / SECTOR_SIZE;
        out.tick()?;
    }
    let padding = (SECTOR_SIZE - size % SECTOR_SIZE) % SECTOR_SIZE;
    if padding > 0 {
        out.w.write_all(&[0xff; SECTOR][..padding as usize])?;
        out.at += 1;
        out.tick()?;
    }
    Ok(())
}

fn write_directory(out: &mut Out<impl Write>, dir: &Directory) -> Result<()> {
    put_xdvdfs_data(out, u64::from(dir.sector), &dir.table, dir.allocated_size)?;
    for entry in &dir.entries {
        if entry.directory.is_none() {
            write_file(out, entry)?;
        }
    }
    for entry in &dir.entries {
        if let Some(child) = &entry.directory {
            write_directory(out, child)?;
        }
    }
    Ok(())
}

fn plan(root: &Node) -> Result<Plan> {
    let Kind::Dir(children) = &root.kind else {
        return bad("an XDVDFS image must be built from a folder");
    };
    let mut files = 0;
    let mut directories = 1;
    let mut count = 0;
    let mut root = plan_directory(children, 0, &mut count, &mut files, &mut directories)?;
    let mut next = ROOT_SECTOR;
    assign_sectors(&mut root, &mut next)?;
    serialize_tables(&mut root);
    let sectors = next.div_ceil(IMAGE_ALIGNMENT_SECTORS) * IMAGE_ALIGNMENT_SECTORS;
    Ok(Plan { root, sectors, files, directories })
}

/// Builds a standalone XDVDFS/XISO image, without modifying or patching file contents.
pub fn build_xiso(root: &Node, dest: &Path, progress: impl FnMut(u64, u64) -> bool + 'static) -> Result<Summary> {
    let plan = plan(root)?;
    let mut part = dest.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let file = File::create(&part)?;
    let result = (|| -> Result<()> {
        let mut out = Out {
            w: BufWriter::with_capacity(1 << 20, &file),
            at: 0,
            total: plan.sectors,
            progress: Box::new(progress),
        };
        let mut header = vec![0; SECTOR];
        header[..20].copy_from_slice(b"MICROSOFT*XBOX*MEDIA");
        header[20..24].copy_from_slice(&plan.root.sector.to_le_bytes());
        header[24..28].copy_from_slice(&(plan.root.table.len() as u32).to_le_bytes());
        let elapsed = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
        let filetime = 116_444_736_000_000_000u64
            .saturating_add(elapsed.as_secs().saturating_mul(10_000_000))
            .saturating_add(u64::from(elapsed.subsec_nanos() / 100));
        header[28..36].copy_from_slice(&filetime.to_le_bytes());
        header[0x7ec..0x7ec + 20].copy_from_slice(b"MICROSOFT*XBOX*MEDIA");
        out.put(HEADER_SECTOR, &header)?;
        write_directory(&mut out, &plan.root)?;
        out.zero(plan.sectors)?;
        out.w.flush()?;
        drop(out);
        file.sync_all()?;
        Ok(())
    })();
    drop(file);
    if let Err(error) = result {
        let _ = std::fs::remove_file(&part);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&part, dest) {
        if let Err(cleanup) = std::fs::remove_file(&part) {
            if cleanup.kind() != std::io::ErrorKind::NotFound {
                return bad(format!(
                    "could not publish XISO to {} ({error}) or remove temporary file {} ({cleanup})",
                    dest.display(),
                    part.display()
                ));
            }
        }
        return Err(error.into());
    }
    Ok(Summary { sectors: plan.sectors, files: plan.files, dirs: plan.directories })
}
