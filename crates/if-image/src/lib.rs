//! Disc image engine. Pure Rust and platform independent, so everything
//! here is tested on any OS; Windows-only work (mounting, drives) lives in
//! other crates.
//!
//! Layers:
//! - [`source`]: a [`source::SectorSource`] gives 2048-byte user-data
//!   sectors, whatever the container (ISO, BIN/CUE, CHD, …).
//! - [`iso9660`], [`udf`]: file systems on top of a sector source.
//! - [`fs`]: one tree API over them, plus extraction.
//! - [`eltorito`]: boot catalog.

pub mod ccd;
pub mod cd;
pub mod chd;
pub mod container;
pub mod convert;
pub mod cue;
pub mod disc;
pub mod edit;
pub mod eltorito;
pub mod fat;
mod fat_write;
pub mod fs;
pub mod iso9660;
pub mod mds;
pub mod nrg;
pub mod source;
pub mod udf;
pub mod vhd;
pub mod vhd_write;
pub mod write;
pub mod xiso;

pub const SECTOR: usize = 2048;

/// Errors carry a human message; I/O errors keep their kind.
#[derive(Debug)]
pub enum Error {
    Io(std::io::Error),
    Format(String),
    Cancelled,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Io(e) => write!(f, "{e}"),
            Error::Format(m) => f.write_str(m),
            Error::Cancelled => f.write_str("cancelled"),
        }
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        if e.kind() == std::io::ErrorKind::Interrupted {
            Error::Cancelled
        } else {
            Error::Io(e)
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn bad<T>(m: impl Into<String>) -> Result<T> {
    Err(Error::Format(m.into()))
}

pub(crate) fn le16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}

pub(crate) fn le32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

pub(crate) fn le64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}
