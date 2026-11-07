//! Track-level disc model shared by the CD image formats (ISO, BIN/CUE,
//! CCD, NRG, MDS, CHD): which tracks exist and where each sector lives.

use crate::cd::{self, RAW};
use crate::source::SectorSource;
use crate::{bad, Result, SECTOR};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Audio,
    Mode1,
    Mode2Form1,
    /// Mode 2 with Form 1 and Form 2 sectors (Video CD, PlayStation).
    Mode2,
}

impl TrackKind {
    pub fn is_data(self) -> bool {
        self != TrackKind::Audio
    }

    /// Q/TOC control nibble: 4 = data track.
    pub fn control(self) -> u8 {
        if self.is_data() {
            4
        } else {
            0
        }
    }
}

/// How sectors of a track are stored in its file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// 2048-byte user data (ISO, MODE1/2048).
    Cooked,
    /// Full 2352-byte sectors.
    Raw,
    /// 2336 bytes: Mode 2 without sync and header.
    Mode2Headerless,
    /// 2448 bytes: raw sector followed by 96 bytes of subchannel.
    RawSub,
}

impl Layout {
    pub fn size(self) -> u64 {
        match self {
            Layout::Cooked => 2048,
            Layout::Raw => 2352,
            Layout::Mode2Headerless => 2336,
            Layout::RawSub => 2448,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub number: u8,
    pub kind: TrackKind,
    /// LBA of index 1.
    pub start: i64,
    /// Sectors from index 1 to the end of the track.
    pub length: u64,
    /// Pregap sectors (index 0) before `start`.
    pub pregap: u64,
    /// Whether the pregap is stored in the file (otherwise generated).
    pub pregap_stored: bool,
    pub file: PathBuf,
    /// Byte offset of the first stored sector (pregap if stored, else index 1).
    pub offset: u64,
    pub layout: Layout,
    /// Optional subchannel file and byte offset (96 bytes per sector).
    pub sub: Option<(PathBuf, u64)>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Disc {
    pub tracks: Vec<Track>,
    /// Things the reader noticed but could not represent.
    pub notes: Vec<String>,
}

impl Disc {
    pub fn first_data_track(&self) -> Option<usize> {
        self.tracks.iter().position(|t| t.kind.is_data())
    }

    pub fn total_sectors(&self) -> u64 {
        self.tracks.last().map(|t| (t.start + t.length as i64).max(0) as u64).unwrap_or(0)
    }

    pub fn has_audio(&self) -> bool {
        self.tracks.iter().any(|t| t.kind == TrackKind::Audio)
    }
}

/// Raw access to a disc, whatever stores it (plain files or a CHD).
pub trait RawDisc {
    fn disc(&self) -> &Disc;
    fn read_raw(&mut self, t: usize, rel: i64, out: &mut [u8; RAW]) -> Result<()>;
    fn read_sub(&mut self, t: usize, rel: i64) -> Result<[u8; cd::SUB]>;
    /// 2048 bytes of user data of data track `t`.
    fn read_user(&mut self, t: usize, rel: i64, out: &mut [u8]) -> Result<()> {
        let mut raw = [0u8; RAW];
        self.read_raw(t, rel, &mut raw)?;
        match cd::user_data(&raw) {
            Some(d) => out[..SECTOR].copy_from_slice(d),
            None if raw[15] == 2 => out[..SECTOR].copy_from_slice(&raw[24..2072]),
            None => return bad(format!("sector {rel} of track {} is not a data sector", self.disc().tracks[t].number)),
        }
        Ok(())
    }
}

impl RawDisc for DiscReader {
    fn disc(&self) -> &Disc {
        &self.disc
    }
    fn read_raw(&mut self, t: usize, rel: i64, out: &mut [u8; RAW]) -> Result<()> {
        DiscReader::read_raw(self, t, rel, out)
    }
    fn read_sub(&mut self, t: usize, rel: i64) -> Result<[u8; cd::SUB]> {
        DiscReader::read_sub(self, t, rel)
    }
    fn read_user(&mut self, t: usize, rel: i64, out: &mut [u8]) -> Result<()> {
        DiscReader::read_user(self, t, rel, out)
    }
}

/// Reads raw sectors of a disc, opening files as needed.
pub struct DiscReader {
    pub disc: Disc,
    open: Option<(PathBuf, File)>,
    sub_open: Option<(PathBuf, File)>,
}

impl DiscReader {
    pub fn new(disc: Disc) -> Self {
        DiscReader { disc, open: None, sub_open: None }
    }

    fn file<'a>(slot: &'a mut Option<(PathBuf, File)>, p: &PathBuf) -> Result<&'a mut File> {
        if slot.as_ref().map(|(q, _)| q != p).unwrap_or(true) {
            let f = File::open(p).map_err(|e| crate::Error::Format(format!("{}: {e}", p.display())))?;
            *slot = Some((p.clone(), f));
        }
        Ok(&mut slot.as_mut().expect("just opened").1)
    }

    /// Raw 2352-byte sector `rel` of track `t` (negative = pregap).
    pub fn read_raw(&mut self, t: usize, rel: i64, out: &mut [u8; RAW]) -> Result<()> {
        let tr = self.disc.tracks[t].clone();
        let lba = tr.start + rel;
        if rel < 0 && !tr.pregap_stored {
            *out = match tr.kind {
                TrackKind::Audio => [0u8; RAW],
                TrackKind::Mode1 => cd::mode1(lba, &[0u8; SECTOR]),
                _ => cd::mode2_form1(lba, [0, 0, 0x08, 0], &[0u8; SECTOR]),
            };
            return Ok(());
        }
        let index = if tr.pregap_stored { rel + tr.pregap as i64 } else { rel };
        if index < 0 || rel >= tr.length as i64 {
            return bad(format!("sector {rel} outside track {}", tr.number));
        }
        let size = tr.layout.size();
        let f = Self::file(&mut self.open, &tr.file)?;
        f.seek(SeekFrom::Start(tr.offset + index as u64 * size))?;
        let mut buf = vec![0u8; size as usize];
        read_padded(f, &mut buf)?;
        *out = match tr.layout {
            Layout::Raw => buf.try_into().expect("2352"),
            Layout::RawSub => buf[..RAW].try_into().expect("2352"),
            Layout::Cooked if tr.kind == TrackKind::Mode1 => cd::mode1(lba, &buf),
            Layout::Cooked => cd::mode2_form1(lba, [0, 0, 0x08, 0], &buf),
            Layout::Mode2Headerless => {
                let mut s = [0u8; RAW];
                s[..12].copy_from_slice(&cd::SYNC);
                let (m, sec, fr) = cd::msf(lba + cd::MSF_OFFSET);
                let b = |v: u8| ((v / 10) << 4) | (v % 10);
                s[12..16].copy_from_slice(&[b(m), b(sec), b(fr), 2]);
                s[16..].copy_from_slice(&buf);
                s
            }
        };
        Ok(())
    }

    /// 96 bytes of subchannel (stored if available, else generated).
    pub fn read_sub(&mut self, t: usize, rel: i64) -> Result<[u8; cd::SUB]> {
        let tr = self.disc.tracks[t].clone();
        if let Some((p, off)) = &tr.sub {
            let abs = (tr.start + rel) as u64;
            let f = Self::file(&mut self.sub_open, p)?;
            f.seek(SeekFrom::Start(off + abs * cd::SUB as u64))?;
            let mut s = [0u8; cd::SUB];
            read_padded(f, &mut s)?;
            return Ok(s);
        }
        let index = if rel < 0 { 0 } else { 1 };
        Ok(cd::subchannel(tr.kind.control(), tr.number, index, rel, tr.start + rel))
    }

    /// The 2048-byte user data sector `rel` of data track `t`.
    pub fn read_user(&mut self, t: usize, rel: i64, out: &mut [u8]) -> Result<()> {
        let tr = &self.disc.tracks[t];
        if tr.layout == Layout::Cooked && rel >= 0 {
            let (file, offset) = (tr.file.clone(), tr.offset + if tr.pregap_stored { tr.pregap * 2048 } else { 0 });
            let f = Self::file(&mut self.open, &file)?;
            f.seek(SeekFrom::Start(offset + rel as u64 * 2048))?;
            return read_padded(f, &mut out[..SECTOR]);
        }
        let mut raw = [0u8; RAW];
        self.read_raw(t, rel, &mut raw)?;
        match cd::user_data(&raw) {
            Some(d) => {
                out[..SECTOR].copy_from_slice(d);
                Ok(())
            }
            None if raw[15] == 2 => {
                // Formless Mode 2 sector: take the Form 1 data area anyway.
                out[..SECTOR].copy_from_slice(&raw[24..2072]);
                Ok(())
            }
            None => bad(format!("sector {} of track {} is not a data sector", rel, self.disc.tracks[t].number)),
        }
    }
}

fn read_padded(f: &mut File, buf: &mut [u8]) -> Result<()> {
    let mut n = 0;
    while n < buf.len() {
        let k = f.read(&mut buf[n..])?;
        if k == 0 {
            buf[n..].fill(0);
            break;
        }
        n += k;
    }
    Ok(())
}

/// The first data track as 2048-byte sectors, for the file system readers.
/// Sector numbers are relative to the track start, as file systems expect.
pub struct DataTrack {
    reader: Box<dyn RawDisc>,
    track: usize,
}

impl DataTrack {
    pub fn new(disc: Disc) -> Result<DataTrack> {
        Self::over(Box::new(DiscReader::new(disc)))
    }

    pub fn over(reader: Box<dyn RawDisc>) -> Result<DataTrack> {
        let Some(track) = reader.disc().first_data_track() else {
            return bad("the image has no data track (audio CD?)");
        };
        Ok(DataTrack { reader, track })
    }
}

impl SectorSource for DataTrack {
    fn sectors(&self) -> u64 {
        self.reader.disc().tracks[self.track].length
    }

    fn read_sectors(&mut self, lba: u64, buf: &mut [u8]) -> Result<()> {
        for (i, chunk) in buf.chunks_mut(SECTOR).enumerate() {
            self.reader.read_user(self.track, (lba + i as u64) as i64, chunk)?;
        }
        Ok(())
    }
}

/// A single-track disc for a plain ISO file.
pub fn from_iso(path: &std::path::Path) -> Result<Disc> {
    let len = std::fs::metadata(path)?.len();
    Ok(Disc {
        tracks: vec![Track {
            number: 1,
            kind: TrackKind::Mode1,
            start: 0,
            length: len.div_ceil(2048),
            pregap: 0,
            pregap_stored: false,
            file: path.to_path_buf(),
            offset: 0,
            layout: Layout::Cooked,
            sub: None,
        }],
        notes: Vec::new(),
    })
}
