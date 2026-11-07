//! Sector sources: anything that yields 2048-byte user-data sectors.

use crate::{Result, SECTOR};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub trait SectorSource {
    /// Number of 2048-byte sectors.
    fn sectors(&self) -> u64;
    /// Reads `buf.len() / 2048` sectors starting at `lba`. `buf.len()` is a
    /// multiple of 2048. Reading past the end is an error.
    fn read_sectors(&mut self, lba: u64, buf: &mut [u8]) -> Result<()>;
}

impl<T: SectorSource + ?Sized> SectorSource for Box<T> {
    fn sectors(&self) -> u64 {
        (**self).sectors()
    }
    fn read_sectors(&mut self, lba: u64, buf: &mut [u8]) -> Result<()> {
        (**self).read_sectors(lba, buf)
    }
}

/// A plain `.iso` (or any file of 2048-byte sectors).
pub struct IsoFile {
    f: File,
    sectors: u64,
}

impl IsoFile {
    pub fn open(path: &Path) -> Result<Self> {
        let f = File::open(path)?;
        let len = f.metadata()?.len();
        Ok(Self { f, sectors: len.div_ceil(SECTOR as u64) })
    }
}

impl SectorSource for IsoFile {
    fn sectors(&self) -> u64 {
        self.sectors
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
        (self.0.len() / SECTOR) as u64
    }

    fn read_sectors(&mut self, lba: u64, buf: &mut [u8]) -> Result<()> {
        let s = lba as usize * SECTOR;
        let e = s + buf.len();
        if e > self.0.len() {
            return crate::bad("read past end of image");
        }
        buf.copy_from_slice(&self.0[s..e]);
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
