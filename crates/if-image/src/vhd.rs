//! Read-only support for fixed and dynamic VHD 1.0 containers.

use crate::source::SectorSource;
use crate::{bad, Result, SECTOR};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

const VHD_SECTOR: u64 = 512;
const FOOTER_SIZE: u64 = 512;
const MAX_VIRTUAL_SIZE: u64 = 2 * 1024 * 1024 * 1024 * 1024;
const MAX_BAT_ENTRIES: u32 = 1_048_576;
const MAX_BLOCK_SIZE: u64 = 256 * 1024 * 1024;
const FOOTER_COOKIE: &[u8; 8] = b"conectix";
const HEADER_COOKIE: &[u8; 8] = b"cxsparse";

enum Layout {
    Fixed,
    Dynamic { block_size: u64, bitmap_size: u64, blocks: Vec<u32> },
}

pub struct VhdFile {
    file: File,
    bytes: u64,
    sectors: u64,
    layout: Layout,
}

impl VhdFile {
    pub fn open(path: &Path) -> Result<Self> {
        let mut file = File::open(path)?;
        let file_len = file.metadata()?.len();
        if file_len < FOOTER_SIZE || file_len % VHD_SECTOR != 0 {
            return bad("VHD file is truncated or not sector-aligned");
        }

        let footer_offset = file_len - FOOTER_SIZE;
        let mut footer = [0; FOOTER_SIZE as usize];
        read_at(&mut file, footer_offset, &mut footer)?;
        if &footer[..8] != FOOTER_COOKIE {
            return bad("VHD footer cookie is invalid");
        }
        verify_checksum(&mut footer, 64, "VHD footer")?;
        if be32(&footer, 12) != 0x0001_0000 {
            return bad("VHD footer version is unsupported");
        }

        let bytes = be64(&footer, 48);
        if bytes == 0 || bytes > MAX_VIRTUAL_SIZE || bytes % VHD_SECTOR != 0 {
            return bad("VHD virtual size is invalid or unsupported");
        }
        let layout = match be32(&footer, 60) {
            2 => {
                if be64(&footer, 16) != u64::MAX || bytes.checked_add(FOOTER_SIZE) != Some(file_len) {
                    return bad("fixed VHD size or data offset is invalid");
                }
                Layout::Fixed
            }
            3 => {
                let header_offset = be64(&footer, 16);
                let (block_size, bitmap_size, blocks) = parse_dynamic(&mut file, header_offset, footer_offset, bytes)?;
                Layout::Dynamic { block_size, bitmap_size, blocks }
            }
            _ => return bad("only fixed and dynamic VHD containers are supported"),
        };

        Ok(Self { file, bytes, sectors: bytes.div_ceil(SECTOR as u64), layout })
    }

    fn read_bytes(&mut self, offset: u64, out: &mut [u8]) -> Result<()> {
        let end =
            offset.checked_add(out.len() as u64).ok_or_else(|| crate::Error::Format("VHD read overflow".into()))?;
        if end > self.bytes {
            return bad("read past end of VHD virtual disk");
        }
        let file = &mut self.file;
        match &self.layout {
            Layout::Fixed => read_at(file, offset, out),
            Layout::Dynamic { block_size, bitmap_size, blocks } => {
                read_dynamic(file, offset, out, *block_size, *bitmap_size, blocks)
            }
        }
    }
}

impl SectorSource for VhdFile {
    fn sectors(&self) -> u64 {
        self.sectors
    }

    fn byte_len(&self) -> u64 {
        self.bytes
    }

    fn read_sectors(&mut self, lba: u64, buf: &mut [u8]) -> Result<()> {
        if buf.len() % SECTOR != 0 {
            return bad("sector read buffer is not 2048-byte aligned");
        }
        let start =
            lba.checked_mul(SECTOR as u64).ok_or_else(|| crate::Error::Format("VHD read offset overflow".into()))?;
        let requested_end = start
            .checked_add(buf.len() as u64)
            .ok_or_else(|| crate::Error::Format("VHD read length overflow".into()))?;
        if requested_end > self.sectors.saturating_mul(SECTOR as u64) {
            return bad("read past end of VHD virtual disk");
        }

        buf.fill(0);
        if start < self.bytes {
            let len = (self.bytes - start).min(buf.len() as u64) as usize;
            self.read_bytes(start, &mut buf[..len])?;
        }
        Ok(())
    }
}

fn read_at(file: &mut File, offset: u64, out: &mut [u8]) -> Result<()> {
    file.seek(SeekFrom::Start(offset))?;
    file.read_exact(out)?;
    Ok(())
}

fn read_dynamic(
    file: &mut File,
    mut offset: u64,
    out: &mut [u8],
    block_size: u64,
    bitmap_size: u64,
    blocks: &[u32],
) -> Result<()> {
    let mut done = 0usize;
    while done < out.len() {
        let block_index = (offset / block_size) as usize;
        let block_offset = offset % block_size;
        let len = (block_size - block_offset).min((out.len() - done) as u64) as usize;
        let entry =
            *blocks.get(block_index).ok_or_else(|| crate::Error::Format("VHD block map is incomplete".into()))?;
        if entry == u32::MAX {
            out[done..done + len].fill(0);
        } else {
            let block_start = u64::from(entry) * VHD_SECTOR;
            let mut bitmap = vec![0; bitmap_size as usize];
            read_at(file, block_start, &mut bitmap)?;

            let end_byte = block_offset + len as u64;
            let first_sector = block_offset / VHD_SECTOR;
            let end_sector = end_byte.div_ceil(VHD_SECTOR);
            let mut sector = first_sector;
            while sector < end_sector {
                let bit = bitmap[(sector / 8) as usize] & (0x80 >> (sector % 8)) != 0;
                let mut run_end = sector + 1;
                while run_end < end_sector && (bitmap[(run_end / 8) as usize] & (0x80 >> (run_end % 8)) != 0) == bit {
                    run_end += 1;
                }
                let run_start = sector * VHD_SECTOR;
                let run_finish = (run_end * VHD_SECTOR).min(end_byte);
                let out_start = done + (run_start - block_offset) as usize;
                let run_len = (run_finish - run_start) as usize;
                if bit {
                    let data_offset = block_start
                        .checked_add(bitmap_size)
                        .and_then(|base| base.checked_add(run_start))
                        .ok_or_else(|| crate::Error::Format("VHD block offset overflow".into()))?;
                    read_at(file, data_offset, &mut out[out_start..out_start + run_len])?;
                } else {
                    out[out_start..out_start + run_len].fill(0);
                }
                sector = run_end;
            }
        }
        offset =
            offset.checked_add(len as u64).ok_or_else(|| crate::Error::Format("VHD read offset overflow".into()))?;
        done += len;
    }
    Ok(())
}

fn parse_dynamic(file: &mut File, header_offset: u64, footer_offset: u64, bytes: u64) -> Result<(u64, u64, Vec<u32>)> {
    let header_end =
        header_offset.checked_add(1024).ok_or_else(|| crate::Error::Format("VHD header offset overflow".into()))?;
    if header_offset % VHD_SECTOR != 0 || header_offset < VHD_SECTOR || header_end > footer_offset {
        return bad("dynamic VHD header is outside the file");
    }

    let mut header = [0; 1024];
    read_at(file, header_offset, &mut header)?;
    if &header[..8] != HEADER_COOKIE {
        return bad("dynamic VHD header cookie is invalid");
    }
    verify_checksum(&mut header, 36, "dynamic VHD header")?;
    if be64(&header, 8) != u64::MAX || be32(&header, 24) != 0x0001_0000 {
        return bad("dynamic VHD header version or data offset is unsupported");
    }

    let table_offset = be64(&header, 16);
    let max_entries = be32(&header, 28);
    let block_size = u64::from(be32(&header, 32));
    if max_entries == 0
        || max_entries > MAX_BAT_ENTRIES
        || !(VHD_SECTOR..=MAX_BLOCK_SIZE).contains(&block_size)
        || !block_size.is_power_of_two()
        || block_size % VHD_SECTOR != 0
    {
        return bad("dynamic VHD block map or block size is unsupported");
    }
    let needed_entries = bytes.div_ceil(block_size);
    if needed_entries > u64::from(max_entries) {
        return bad("dynamic VHD block map does not cover the virtual disk");
    }

    let table_bytes = u64::from(max_entries)
        .checked_mul(4)
        .ok_or_else(|| crate::Error::Format("VHD block map size overflow".into()))?;
    let table_end = table_offset
        .checked_add(table_bytes)
        .ok_or_else(|| crate::Error::Format("VHD block map offset overflow".into()))?;
    if table_offset % VHD_SECTOR != 0 || table_offset < header_end || table_end > footer_offset {
        return bad("dynamic VHD block map is outside the file");
    }

    let needed_bytes = needed_entries
        .checked_mul(4)
        .and_then(|value| usize::try_from(value).ok())
        .ok_or_else(|| crate::Error::Format("VHD block map is too large".into()))?;
    let mut raw_entries = vec![0; needed_bytes];
    read_at(file, table_offset, &mut raw_entries)?;
    let blocks: Vec<u32> =
        raw_entries.chunks_exact(4).map(|entry| u32::from_be_bytes(entry.try_into().unwrap())).collect();

    let sectors_per_block = block_size / VHD_SECTOR;
    let bitmap_size = sectors_per_block.div_ceil(8).div_ceil(VHD_SECTOR) * VHD_SECTOR;
    let mut ranges = Vec::new();
    for &entry in &blocks {
        if entry == u32::MAX {
            continue;
        }
        let start = u64::from(entry) * VHD_SECTOR;
        let end = start
            .checked_add(bitmap_size)
            .and_then(|value| value.checked_add(block_size))
            .ok_or_else(|| crate::Error::Format("VHD block extent overflow".into()))?;
        if start < table_end || end > footer_offset {
            return bad("dynamic VHD block extent is outside the data area");
        }
        ranges.push((start, end));
    }
    ranges.sort_unstable();
    if ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return bad("dynamic VHD blocks overlap");
    }

    Ok((block_size, bitmap_size, blocks))
}

fn verify_checksum(block: &mut [u8], offset: usize, name: &str) -> Result<()> {
    let expected = be32(block, offset);
    block[offset..offset + 4].fill(0);
    let sum: u32 = block.iter().map(|byte| u32::from(*byte)).sum();
    if !sum != expected {
        return bad(format!("{name} checksum is invalid"));
    }
    Ok(())
}

fn be32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn be64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes(bytes[offset..offset + 8].try_into().unwrap())
}
