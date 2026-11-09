//! Sector sources: anything that yields 2048-byte user-data sectors.

use crate::{Result, SECTOR};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub trait SectorSource {
    /// Number of 2048-byte sectors.
    fn sectors(&self) -> u64;
    /// Exact virtual byte length; defaults to the complete logical sectors.
    fn byte_len(&self) -> u64 {
        self.sectors().saturating_mul(SECTOR as u64)
    }
    /// Reads `buf.len() / 2048` sectors starting at `lba`. `buf.len()` is a
    /// multiple of 2048. Reading past the end is an error.
    fn read_sectors(&mut self, lba: u64, buf: &mut [u8]) -> Result<()>;
}

impl<T: SectorSource + ?Sized> SectorSource for Box<T> {
    fn sectors(&self) -> u64 {
        (**self).sectors()
    }
    fn byte_len(&self) -> u64 {
        (**self).byte_len()
    }
    fn read_sectors(&mut self, lba: u64, buf: &mut [u8]) -> Result<()> {
        (**self).read_sectors(lba, buf)
    }
}

/// A plain `.iso` (or any file of 2048-byte sectors).
pub struct IsoFile {
    f: File,
    bytes: u64,
    sectors: u64,
}

impl IsoFile {
    pub fn open(path: &Path) -> Result<Self> {
        let f = File::open(path)?;
        let len = f.metadata()?.len();
        Ok(Self { f, bytes: len, sectors: len.div_ceil(SECTOR as u64) })
    }
}

impl SectorSource for IsoFile {
    fn sectors(&self) -> u64 {
        self.sectors
    }

    fn byte_len(&self) -> u64 {
        self.bytes
    }

    fn read_sectors(&mut self, lba: u64, buf: &mut [u8]) -> Result<()> {
        let n = (buf.len() / SECTOR) as u64;
        if lba + n > self.sectors {
            return crate::bad(format!("read past end of image (sector {} of {})", lba + n, self.sectors));
        }
        self.f.seek(SeekFrom::Start(lba * SECTOR as u64))?;
        // The last sector of a file that is not a multiple of 2048 is padded.
        let mut done = 0;
        while done < buf.len() {
            let r = self.f.read(&mut buf[done..])?;
            if r == 0 {
                buf[done..].fill(0);
                break;
            }
            done += r;
        }
        Ok(())
    }
}

/// In-memory image, for tests and small builds.
pub struct MemSource(pub Vec<u8>);

impl SectorSource for MemSource {
    fn sectors(&self) -> u64 {
        self.0.len().div_ceil(SECTOR) as u64
    }

    fn byte_len(&self) -> u64 {
        self.0.len() as u64
    }

    fn read_sectors(&mut self, lba: u64, buf: &mut [u8]) -> Result<()> {
        if buf.len() % SECTOR != 0 {
            return crate::bad("sector read buffer is not 2048-byte aligned");
        }
        let count = (buf.len() / SECTOR) as u64;
        if lba.checked_add(count).is_none_or(|end| end > self.sectors()) {
            return crate::bad("read past end of image");
        }
        let start = lba
            .checked_mul(SECTOR as u64)
            .and_then(|offset| usize::try_from(offset).ok())
            .ok_or_else(|| crate::Error::Format("image read offset overflow".into()))?;
        start.checked_add(buf.len()).ok_or_else(|| crate::Error::Format("image read length overflow".into()))?;
        buf.fill(0);
        if start < self.0.len() {
            let len = (self.0.len() - start).min(buf.len());
            buf[..len].copy_from_slice(&self.0[start..start + len]);
        }
        Ok(())
    }
}

/// Reads `len` bytes starting at byte `offset` of sector `lba`.
pub fn read_bytes(src: &mut dyn SectorSource, lba: u64, offset: u64, len: usize) -> Result<Vec<u8>> {
    let first = lba + offset / SECTOR as u64;
    let skip = (offset % SECTOR as u64) as usize;
    let count = (skip + len).div_ceil(SECTOR);
    let mut buf = vec![0u8; count * SECTOR];
    src.read_sectors(first, &mut buf)?;
    buf.drain(..skip);
    buf.truncate(len);
    Ok(buf)
}
