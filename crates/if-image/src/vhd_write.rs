//! Safe writers for fixed and dynamic VHD 1.0 containers.

use crate::source::SectorSource;
use crate::vhd::{MAX_BAT_ENTRIES, MAX_VIRTUAL_SIZE};
use crate::{bad, Error, Result, SECTOR};
use std::fs::{self, File, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const VHD_SECTOR: u64 = 512;
const FOOTER_SIZE: u64 = 512;
const HEADER_OFFSET: u64 = 512;
const BAT_OFFSET: u64 = 1536;
const BLOCK_SIZE: u64 = 2 * 1024 * 1024;
const COPY_BUFFER: usize = 1 << 20;
static NEXT_UUID: AtomicU64 = AtomicU64::new(0);

fn validate_source(source: &dyn SectorSource) -> Result<u64> {
    let bytes = source.byte_len();
    if bytes == 0 || bytes > MAX_VIRTUAL_SIZE || bytes % VHD_SECTOR != 0 {
        return bad("VHD source size must be a nonzero, 512-byte-aligned virtual disk no larger than 2 TiB");
    }
    if source.sectors() < bytes.div_ceil(SECTOR as u64) {
        return bad("image source does not expose all of its virtual bytes");
    }
    Ok(bytes)
}

fn read_source(source: &mut dyn SectorSource, offset: u64, out: &mut [u8]) -> Result<()> {
    let sector_count = out.len().div_ceil(SECTOR);
    let mut sectors = vec![0; sector_count * SECTOR];
    source.read_sectors(offset / SECTOR as u64, &mut sectors)?;
    out.copy_from_slice(&sectors[..out.len()]);
    Ok(())
}

fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.as_os_str().to_owned();
    name.push(".part");
    PathBuf::from(name)
}

fn cleanup_after_publish_failure(part: &Path, dest: &Path, error: std::io::Error) -> Result<()> {
    if let Err(cleanup) = fs::remove_file(part) {
        if cleanup.kind() != std::io::ErrorKind::NotFound {
            return bad(format!(
                "could not publish VHD to {} ({error}) or remove temporary file {} ({cleanup})",
                dest.display(),
                part.display()
            ));
        }
    }
    Err(error.into())
}

fn publish_new(part: &Path, dest: &Path) -> Result<()> {
    if let Err(error) = fs::hard_link(part, dest) {
        return cleanup_after_publish_failure(part, dest, error);
    }
    if let Err(error) = fs::remove_file(part) {
        return bad(format!(
            "VHD was written to {} but temporary file {} could not be removed ({error})",
            dest.display(),
            part.display()
        ));
    }
    Ok(())
}

fn write_atomic(dest: &Path, write: impl FnOnce(&mut File) -> Result<()>) -> Result<()> {
    if dest.exists() {
        return bad(format!("destination already exists: {}", dest.display()));
    }
    let part = part_path(dest);
    let mut file = OpenOptions::new().write(true).read(true).create_new(true).open(&part)?;
    let result = write(&mut file).and_then(|()| file.sync_all().map_err(Error::from));
    drop(file);
    if let Err(error) = result {
        if let Err(cleanup) = fs::remove_file(&part) {
            if cleanup.kind() != std::io::ErrorKind::NotFound {
                return bad(format!(
                    "could not write VHD to {} ({error}) or remove temporary file {} ({cleanup})",
                    dest.display(),
                    part.display()
                ));
            }
        }
        return Err(error);
    }
    publish_new(&part, dest)
}

fn make_uuid() -> [u8; 16] {
    let mut id = [0; 16];
    let ticks = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos();
    id[..8].copy_from_slice(&(ticks as u64).to_be_bytes());
    id[8..12].copy_from_slice(&std::process::id().to_be_bytes());
    id[12..].copy_from_slice(&(NEXT_UUID.fetch_add(1, Ordering::Relaxed) as u32).to_be_bytes());
    id[6] = (id[6] & 0x0f) | 0x40;
    id[8] = (id[8] & 0x3f) | 0x80;
    id
}

fn footer(bytes: u64, disk_type: u32, data_offset: u64) -> [u8; FOOTER_SIZE as usize] {
    let mut footer = [0; FOOTER_SIZE as usize];
    footer[..8].copy_from_slice(b"conectix");
    footer[8..12].copy_from_slice(&2u32.to_be_bytes());
    footer[12..16].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    footer[16..24].copy_from_slice(&data_offset.to_be_bytes());
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .saturating_sub(946_684_800)
        .min(u64::from(u32::MAX)) as u32;
    footer[24..28].copy_from_slice(&timestamp.to_be_bytes());
    footer[28..32].copy_from_slice(b"IFND");
    footer[32..36].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    footer[36..40].copy_from_slice(b"Wi2k");
    footer[40..48].copy_from_slice(&bytes.to_be_bytes());
    footer[48..56].copy_from_slice(&bytes.to_be_bytes());
    let sectors = bytes / VHD_SECTOR;
    let cylinders = (sectors / (16 * 63)).clamp(1, u64::from(u16::MAX)) as u16;
    footer[56..58].copy_from_slice(&cylinders.to_be_bytes());
    footer[58] = 16;
    footer[59] = 63;
    footer[60..64].copy_from_slice(&disk_type.to_be_bytes());
    footer[68..84].copy_from_slice(&make_uuid());
    set_checksum(&mut footer, 64);
    footer
}

fn set_checksum(block: &mut [u8], offset: usize) {
    block[offset..offset + 4].fill(0);
    let sum: u32 = block.iter().map(|byte| u32::from(*byte)).sum();
    block[offset..offset + 4].copy_from_slice(&(!sum).to_be_bytes());
}

/// Writes a fixed VHD to a new destination. `progress(done, total)` returns false to cancel.
pub fn write_fixed(
    source: &mut dyn SectorSource,
    dest: &Path,
    mut progress: impl FnMut(u64, u64) -> bool,
) -> Result<()> {
    let bytes = validate_source(source)?;
    write_atomic(dest, |file| {
        let mut offset = 0u64;
        let mut buffer = vec![0; COPY_BUFFER];
        while offset < bytes {
            let len = (bytes - offset).min(buffer.len() as u64) as usize;
            read_source(source, offset, &mut buffer[..len])?;
            file.write_all(&buffer[..len])?;
            offset += len as u64;
            if !progress(offset, bytes) {
                return Err(Error::Cancelled);
            }
        }
        file.write_all(&footer(bytes, 2, u64::MAX))?;
        Ok(())
    })
}

/// Writes a sparse dynamic VHD to a new destination. `progress(done, total)` returns false to cancel.
pub fn write_dynamic(
    source: &mut dyn SectorSource,
    dest: &Path,
    mut progress: impl FnMut(u64, u64) -> bool,
) -> Result<()> {
    let bytes = validate_source(source)?;
    let entries = bytes.div_ceil(BLOCK_SIZE);
    if entries == 0 || entries > u64::from(MAX_BAT_ENTRIES) {
        return bad("VHD source requires an unsupported number of dynamic blocks");
    }
    let table_bytes = entries.checked_mul(4).ok_or_else(|| Error::Format("VHD block map size overflow".into()))?;
    let table_padded = table_bytes
        .checked_add(VHD_SECTOR - 1)
        .map(|size| size / VHD_SECTOR * VHD_SECTOR)
        .ok_or_else(|| Error::Format("VHD block map size overflow".into()))?;
    let table_len = usize::try_from(table_bytes).map_err(|_| Error::Format("VHD block map is too large".into()))?;
    let pad_len = usize::try_from(table_padded - table_bytes)
        .map_err(|_| Error::Format("VHD block map padding is too large".into()))?;
    let sectors_per_block = BLOCK_SIZE / VHD_SECTOR;
    let bitmap_size = sectors_per_block.div_ceil(8).div_ceil(VHD_SECTOR) * VHD_SECTOR;
    let bitmap_len = usize::try_from(bitmap_size).map_err(|_| Error::Format("VHD block bitmap is too large".into()))?;

    write_atomic(dest, |file| {
        file.write_all(&[0; HEADER_OFFSET as usize])?;

        let mut header = [0; 1024];
        header[..8].copy_from_slice(b"cxsparse");
        header[8..16].copy_from_slice(&u64::MAX.to_be_bytes());
        header[16..24].copy_from_slice(&BAT_OFFSET.to_be_bytes());
        header[24..28].copy_from_slice(&0x0001_0000u32.to_be_bytes());
        header[28..32].copy_from_slice(&(entries as u32).to_be_bytes());
        header[32..36].copy_from_slice(&(BLOCK_SIZE as u32).to_be_bytes());
        set_checksum(&mut header, 36);
        file.seek(SeekFrom::Start(HEADER_OFFSET))?;
        file.write_all(&header)?;
        file.seek(SeekFrom::Start(BAT_OFFSET))?;
        file.write_all(&vec![0xff; table_len])?;
        if pad_len != 0 {
            file.write_all(&vec![0; pad_len])?;
        }

        let mut offset = 0u64;
        let mut block_index = 0u64;
        while offset < bytes {
            let len = (bytes - offset).min(BLOCK_SIZE) as usize;
            let mut block = vec![0; BLOCK_SIZE as usize];
            read_source(source, offset, &mut block[..len])?;

            let mut bitmap = vec![0; bitmap_len];
            let mut allocated = false;
            for sector in 0..sectors_per_block as usize {
                let start = sector * VHD_SECTOR as usize;
                if block[start..start + VHD_SECTOR as usize].iter().any(|byte| *byte != 0) {
                    bitmap[sector / 8] |= 0x80 >> (sector % 8);
                    allocated = true;
                }
            }

            if allocated {
                let block_offset = file.stream_position()?;
                let table_sector = u32::try_from(block_offset / VHD_SECTOR)
                    .map_err(|_| Error::Format("dynamic VHD exceeds the BAT sector-offset limit".into()))?;
                file.write_all(&bitmap)?;
                file.write_all(&block)?;
                let end = file.stream_position()?;
                file.seek(SeekFrom::Start(BAT_OFFSET + block_index * 4))?;
                file.write_all(&table_sector.to_be_bytes())?;
                file.seek(SeekFrom::Start(end))?;
            }

            offset += len as u64;
            block_index += 1;
            if !progress(offset, bytes) {
                return Err(Error::Cancelled);
            }
        }
        file.write_all(&footer(bytes, 3, HEADER_OFFSET))?;
        Ok(())
    })
}
