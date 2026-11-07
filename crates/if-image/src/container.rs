//! Opens an image file whatever its container format: as a whole disc
//! (tracks and raw sectors) or as the 2048-byte sectors of its data track.

use crate::cd::{self, RAW, SUB};
use crate::chd::Chd;
use crate::disc::{DataTrack, Disc, DiscReader, Layout, RawDisc, Track, TrackKind};
use crate::source::{IsoFile, SectorSource};
use crate::{bad, ccd, cue, mds, nrg, Result, SECTOR};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Container formats IsoFoundry can read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Iso,
    BinCue,
    CloneCd,
    Nrg,
    Mds,
    Chd,
    /// A sheetless raw `.bin`/`.img` of 2352-byte sectors.
    RawBin,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::Iso => "ISO (2048-byte sectors)",
            Format::BinCue => "BIN/CUE",
            Format::CloneCd => "CloneCD (CCD/IMG/SUB)",
            Format::Nrg => "Nero NRG",
            Format::Mds => "Alcohol MDS/MDF",
            Format::Chd => "MAME CHD",
            Format::RawBin => "raw 2352-byte sectors",
        }
    }

    /// Formats whose readers were written from third-party descriptions and
    /// not yet checked against real images.
    pub fn unverified(self) -> bool {
        matches!(self, Format::Nrg | Format::Mds)
    }
}

/// An opened image.
pub enum Image {
    /// A CD image with tracks.
    Cd(Box<dyn RawDisc>),
    /// 2048-byte sectors only (ISO, DVD CHD).
    Sectors(Box<dyn SectorSource>),
}

fn ext(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

fn sibling(p: &Path, e: &str) -> Option<PathBuf> {
    [e.to_string(), e.to_ascii_uppercase()].into_iter().map(|x| p.with_extension(x)).find(|q| q.is_file())
}

fn looks_raw(p: &Path) -> Result<bool> {
    let len = std::fs::metadata(p)?.len();
    if len == 0 || len % RAW as u64 != 0 {
        return Ok(false);
    }
    let mut head = [0u8; 16];
    std::fs::File::open(p)?.read_exact(&mut head)?;
    Ok(head[..12] == cd::SYNC)
}

/// Guesses the container from the extension, sibling files and contents.
/// For a `.bin` or `.img` with a `.cue`/`.ccd` next to it, returns the
/// sheet's format and path.
pub fn detect(path: &Path) -> Result<(Format, PathBuf)> {
    let p = path.to_path_buf();
    Ok(match ext(path).as_str() {
        "cue" => (Format::BinCue, p),
        "ccd" => (Format::CloneCd, p),
        "nrg" => (Format::Nrg, p),
        "mds" => (Format::Mds, p),
        "mdf" => match sibling(path, "mds") {
            Some(m) => (Format::Mds, m),
            None => return bad("an .mdf needs its .mds descriptor next to it"),
        },
        "chd" => (Format::Chd, p),
        "bin" | "img" => {
            if let Some(c) = sibling(path, "cue") {
                (Format::BinCue, c)
            } else if let Some(c) = sibling(path, "ccd") {
                (Format::CloneCd, c)
            } else if looks_raw(path)? {
                (Format::RawBin, p)
            } else {
                (Format::Iso, p)
            }
        }
        "iso" | "udf" | "" => (Format::Iso, p),
        other => return bad(format!(".{other} images are not supported")),
    })
}

fn raw_bin(path: &Path) -> Result<Disc> {
    let len = std::fs::metadata(path)?.len();
    let mut head = [0u8; 16];
    std::fs::File::open(path)?.read_exact(&mut head)?;
    let kind = if head[15] == 2 { TrackKind::Mode2 } else { TrackKind::Mode1 };
    Ok(Disc {
        tracks: vec![Track {
            number: 1,
            kind,
            start: 0,
            length: len / RAW as u64,
            pregap: 0,
            pregap_stored: false,
            file: path.to_path_buf(),
            offset: 0,
            layout: Layout::Raw,
            sub: None,
        }],
        notes: vec!["no cue sheet: read as one raw data track".into()],
    })
}

/// Opens any supported image.
pub fn open_image(path: &Path) -> Result<(Format, Image)> {
    let (f, p) = detect(path)?;
    let disc = |d: Disc| Image::Cd(Box::new(DiscReader::new(d)));
    let img = match f {
        Format::Iso => Image::Sectors(Box::new(IsoFile::open(&p)?)),
        Format::BinCue => disc(cue::read(&p)?),
        Format::CloneCd => disc(ccd::read(&p)?),
        Format::Nrg => disc(nrg::read(&p)?),
        Format::Mds => disc(mds::read(&p)?),
        Format::RawBin => disc(raw_bin(&p)?),
        Format::Chd => {
            let c = Chd::open(&p)?;
            if c.is_cd() {
                Image::Cd(Box::new(c))
            } else {
                Image::Sectors(Box::new(c))
            }
        }
    };
    Ok((f, img))
}

/// The image's data track as 2048-byte sectors, for file system readers.
pub fn open(path: &Path) -> Result<(Format, Box<dyn SectorSource>)> {
    let (f, img) = open_image(path)?;
    let src: Box<dyn SectorSource> = match img {
        Image::Sectors(s) => s,
        Image::Cd(d) => Box::new(DataTrack::over(d)?),
    };
    Ok((f, src))
}

/// 2048-byte sectors presented as a one-track Mode 1 CD.
pub struct CookedDisc {
    disc: Disc,
    src: Box<dyn SectorSource>,
}

impl CookedDisc {
    pub fn new(src: Box<dyn SectorSource>) -> Self {
        let disc = Disc {
            tracks: vec![Track {
                number: 1,
                kind: TrackKind::Mode1,
                start: 0,
                length: src.sectors(),
                pregap: 0,
                pregap_stored: false,
                file: PathBuf::new(),
                offset: 0,
                layout: Layout::Cooked,
                sub: None,
            }],
            notes: Vec::new(),
        };
        CookedDisc { disc, src }
    }
}

impl RawDisc for CookedDisc {
    fn disc(&self) -> &Disc {
        &self.disc
    }
    fn read_raw(&mut self, _t: usize, rel: i64, out: &mut [u8; RAW]) -> Result<()> {
        let mut user = [0u8; SECTOR];
        if rel >= 0 {
            self.src.read_sectors(rel as u64, &mut user)?;
        }
        *out = cd::mode1(rel, &user);
        Ok(())
    }
    fn read_sub(&mut self, _t: usize, rel: i64) -> Result<[u8; SUB]> {
        Ok(cd::subchannel(4, 1, if rel < 0 { 0 } else { 1 }, rel, rel))
    }
    fn read_user(&mut self, _t: usize, rel: i64, out: &mut [u8]) -> Result<()> {
        self.src.read_sectors(rel as u64, &mut out[..SECTOR])
    }
}
