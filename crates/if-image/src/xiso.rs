//! Read-only XDVDFS filesystem support for Xbox XISO images.

use crate::fs::{read_data, Data, Entry, Extent, FileSystem, FsKind};
use crate::source::SectorSource;
use crate::{bad, Result, SECTOR};
use std::collections::HashSet;

const DESCRIPTOR_SECTOR: u64 = 32;
const SIGNATURE: &[u8; 20] = b"MICROSOFT*XBOX*MEDIA";
const ROOT_SECTOR_OFFSET: usize = 20;
const ROOT_SIZE_OFFSET: usize = 24;
const SECOND_SIGNATURE_OFFSET: usize = 0x7ec;
const ATTRIBUTE_DIRECTORY: u8 = 0x10;
const MAX_DIRECTORY_BYTES: u64 = 64 << 20;
const MAX_DIRECTORY_ENTRIES: usize = 100_000;

pub struct Xdvdfs {
    root: Entry,
}

impl Xdvdfs {
    /// Returns `None` when the descriptor signature is absent; a recognized
    /// but malformed descriptor is an error rather than a silent fallback.
    pub fn probe(src: &mut dyn SectorSource) -> Result<Option<Self>> {
        if src.sectors() <= DESCRIPTOR_SECTOR {
            return Ok(None);
        }

        let mut header = [0; SECTOR];
        src.read_sectors(DESCRIPTOR_SECTOR, &mut header)?;
        if &header[..SIGNATURE.len()] != SIGNATURE {
            return Ok(None);
        }
        if &header[SECOND_SIGNATURE_OFFSET..SECOND_SIGNATURE_OFFSET + SIGNATURE.len()] != SIGNATURE {
            return bad("invalid XDVDFS volume descriptor signature");
        }

        let root_sector = u32::from_le_bytes(header[ROOT_SECTOR_OFFSET..ROOT_SECTOR_OFFSET + 4].try_into().unwrap());
        let root_size = u32::from_le_bytes(header[ROOT_SIZE_OFFSET..ROOT_SIZE_OFFSET + 4].try_into().unwrap());
        validate_extent(src, root_sector, root_size, true)?;

        Ok(Some(Self {
            root: Entry {
                name: String::new(),
                dir: true,
                size: u64::from(root_size),
                mtime: None,
                hidden: false,
                data: Data::Extents(vec![Extent {
                    lba: u64::from(root_sector),
                    len: u64::from(root_size),
                    sparse: false,
                }]),
            },
        }))
    }

    fn list_directory(&self, src: &mut dyn SectorSource, dir: &Entry) -> Result<Vec<Entry>> {
        if !dir.dir {
            return bad(format!("{} is not a folder", dir.name));
        }
        if dir.size > MAX_DIRECTORY_BYTES {
            return bad("XDVDFS directory is larger than 64 MB");
        }
        let bytes_len =
            usize::try_from(dir.size).map_err(|_| crate::Error::Format("XDVDFS directory is too large".into()))?;
        let mut bytes = vec![0; bytes_len];
        read_data(src, &dir.data, dir.size, 0, &mut bytes)?;

        let mut entries = Vec::new();
        let mut names = HashSet::new();
        let mut offset = 0usize;
        while offset + 2 <= bytes.len() {
            if bytes[offset] == 0xff && bytes[offset + 1] == 0xff {
                offset = (offset / SECTOR + 1) * SECTOR;
                continue;
            }
            if offset + 14 > bytes.len() {
                return bad("truncated XDVDFS directory entry");
            }

            let file_sector = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().unwrap());
            let file_size = u32::from_le_bytes(bytes[offset + 8..offset + 12].try_into().unwrap());
            let attributes = bytes[offset + 12];
            let name_len = usize::from(bytes[offset + 13]);
            let name_end = offset
                .checked_add(14 + name_len)
                .ok_or_else(|| crate::Error::Format("XDVDFS directory entry size overflow".into()))?;
            if name_len == 0 || name_end > bytes.len() {
                return bad("invalid XDVDFS filename length");
            }
            let name = String::from_utf8_lossy(&bytes[offset + 14..name_end]).into_owned();
            if name == "." || name == ".." || name.contains(['/', '\\']) || name.chars().any(char::is_control) {
                return bad(format!("invalid XDVDFS filename: {name:?}"));
            }
            if !names.insert(name.to_lowercase()) {
                return bad(format!("duplicate XDVDFS filename: {name}"));
            }
            if entries.len() == MAX_DIRECTORY_ENTRIES {
                return bad("XDVDFS directory contains too many entries");
            }

            let is_dir = attributes & ATTRIBUTE_DIRECTORY != 0;
            validate_extent(src, file_sector, file_size, is_dir)?;
            entries.push(Entry {
                name,
                dir: is_dir,
                size: u64::from(file_size),
                mtime: None,
                hidden: attributes & 0x02 != 0,
                data: Data::Extents(vec![Extent {
                    lba: u64::from(file_sector),
                    len: u64::from(file_size),
                    sparse: false,
                }]),
            });

            let record_len = (14 + name_len + 3) & !3;
            offset = offset
                .checked_add(record_len)
                .ok_or_else(|| crate::Error::Format("XDVDFS directory offset overflow".into()))?;
        }

        if offset < bytes.len() && bytes[offset..].iter().any(|b| *b != 0xff) {
            return bad("invalid trailing bytes in XDVDFS directory");
        }
        Ok(entries)
    }
}

fn validate_extent(src: &dyn SectorSource, sector: u32, size: u32, directory: bool) -> Result<()> {
    let start = u64::from(sector);
    let sectors = u64::from(size).div_ceil(SECTOR as u64);
    let end = start.checked_add(sectors).ok_or_else(|| crate::Error::Format("XDVDFS extent overflow".into()))?;
    if end > src.sectors() || (directory && u64::from(size) > MAX_DIRECTORY_BYTES) {
        return bad("XDVDFS extent is outside the image");
    }
    Ok(())
}

impl FileSystem for Xdvdfs {
    fn kind(&self) -> FsKind {
        FsKind::Xdvdfs
    }

    fn label(&self) -> String {
        String::new()
    }

    fn root(&self) -> Entry {
        self.root.clone()
    }

    fn list(&self, src: &mut dyn SectorSource, dir: &Entry) -> Result<Vec<Entry>> {
        self.list_directory(src, dir)
    }
}
