//! Opens an image file as a sector source, whatever its container format.

use crate::source::{IsoFile, SectorSource};
use crate::{bad, Result};
use std::path::Path;

/// Container formats IsoFoundry can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Iso,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::Iso => "ISO (2048-byte sectors)",
        }
    }
}

/// Guesses the container from the extension and contents.
pub fn detect(path: &Path) -> Result<Format> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "iso" | "udf" | "img" | "" => Ok(Format::Iso),
        other => bad(format!(".{other} images are not supported yet")),
    }
}

pub fn open(path: &Path) -> Result<(Format, Box<dyn SectorSource>)> {
    let f = detect(path)?;
    let src: Box<dyn SectorSource> = match f {
        Format::Iso => Box::new(IsoFile::open(path)?),
    };
    Ok((f, src))
}
