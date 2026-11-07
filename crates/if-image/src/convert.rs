//! Image conversion: any readable image to ISO, BIN/CUE, CloneCD or CHD.

use crate::cd::RAW;
use crate::chd;
use crate::container::{self, CookedDisc, Format, Image};
use crate::disc::{DataTrack, Disc, RawDisc};
use crate::source::SectorSource;
use crate::{bad, ccd, cue, Error, Result, SECTOR};
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    Iso,
    BinCue,
    CloneCd,
    /// CHD; CD layout for CD sources, DVD layout for ISO/DVD sources.
    Chd,
    /// CHD in CD layout even for an ISO source.
    ChdCd,
}

impl Output {
    pub fn parse(s: &str) -> Option<Output> {
        Some(match s.to_ascii_lowercase().as_str() {
            "iso" => Output::Iso,
            "cue" | "bin" | "bincue" | "bin-cue" => Output::BinCue,
            "ccd" | "clonecd" => Output::CloneCd,
            "chd" => Output::Chd,
            "chd-cd" => Output::ChdCd,
            _ => return None,
        })
    }

    /// From the destination's extension.
    pub fn for_path(p: &Path) -> Option<Output> {
        Output::parse(&p.extension()?.to_string_lossy())
    }
}

pub struct Summary {
    pub source: Format,
    /// Files written, main file first.
    pub files: Vec<PathBuf>,
    pub warnings: Vec<String>,
}

fn part(p: &Path) -> PathBuf {
    let mut s = p.as_os_str().to_owned();
    s.push(".part");
    PathBuf::from(s)
}

fn commit(parts: &[PathBuf], finals: &[PathBuf]) -> Result<()> {
    for (a, b) in parts.iter().zip(finals) {
        std::fs::rename(a, b)?;
    }
    Ok(())
}

fn cleanup(parts: &[PathBuf]) {
    for p in parts {
        let _ = std::fs::remove_file(p);
    }
}

type Progress<'a> = &'a mut dyn FnMut(u64, u64) -> bool;

/// Converts `src` to `dest`. `progress(done, total)` returns false to cancel;
/// nothing is left behind on error or cancel.
pub fn convert(src: &Path, dest: &Path, out: Output, progress: Progress) -> Result<Summary> {
    if std::fs::canonicalize(src).ok() == std::fs::canonicalize(dest).ok() && dest.exists() {
        return bad("the destination is the source image");
    }
    let (format, img) = container::open_image(src)?;
    let mut warnings = Vec::new();
    if format.unverified() {
        warnings.push(format!("{} reading is unverified against real images", format.name()));
    }
    if let Image::Cd(d) = &img {
        warnings.extend(d.disc().notes.iter().cloned());
    }
    let files = match (out, img) {
        (Output::Iso, Image::Sectors(mut s)) => to_iso(&mut *s, dest, progress)?,
        (Output::Iso, Image::Cd(d)) => {
            let disc = d.disc().clone();
            if disc.tracks.len() > 1 {
                warnings.push(format!(
                    "ISO keeps only the first data track; {} other track(s) dropped",
                    disc.tracks.len() - 1
                ));
            }
            let mut s = DataTrack::over(d)?;
            to_iso(&mut s, dest, progress)?
        }
        (Output::Chd, Image::Sectors(mut s)) => {
            chd_result(dest, chd::write_dvd(&mut *s, dest, progress))?;
            vec![dest.to_path_buf()]
        }
        (Output::Chd | Output::ChdCd, Image::Cd(mut d)) => {
            chd_result(dest, chd::write_cd(&mut *d, dest, progress))?;
            vec![dest.to_path_buf()]
        }
        (Output::ChdCd, Image::Sectors(s)) => {
            let mut d = CookedDisc::new(s);
            chd_result(dest, chd::write_cd(&mut d, dest, progress))?;
            vec![dest.to_path_buf()]
        }
        (o @ (Output::BinCue | Output::CloneCd), img) => {
            let mut d: Box<dyn RawDisc> = match img {
                Image::Cd(d) => d,
                Image::Sectors(s) => Box::new(CookedDisc::new(s)),
            };
            if o == Output::BinCue {
                to_bincue(&mut *d, dest, progress)?
            } else {
                to_clonecd(&mut *d, dest, progress)?
            }
        }
    };
    Ok(Summary { source: format, files, warnings })
}

fn chd_result(dest: &Path, r: Result<chd::ChdSummary>) -> Result<()> {
    match r {
        Ok(_) => Ok(()),
        Err(e) => {
            chd::discard(dest);
            Err(e)
        }
    }
}

fn to_iso(s: &mut dyn SectorSource, dest: &Path, progress: Progress) -> Result<Vec<PathBuf>> {
    let p = part(dest);
    let r = (|| {
        let mut w = BufWriter::with_capacity(1 << 20, File::create(&p)?);
        let total = s.sectors();
        let mut buf = vec![0u8; 32 * SECTOR];
        let mut lba = 0;
        while lba < total {
            let n = (total - lba).min(32);
            let b = &mut buf[..n as usize * SECTOR];
            s.read_sectors(lba, b)?;
            w.write_all(b)?;
            lba += n;
            if !progress(lba, total) {
                return Err(Error::Cancelled);
            }
        }
        w.into_inner().map_err(|e| Error::Io(e.into_error()))?.sync_all()?;
        commit(std::slice::from_ref(&p), &[dest.to_path_buf()])
    })();
    if r.is_err() {
        cleanup(&[p]);
    }
    r.map(|_| vec![dest.to_path_buf()])
}

/// Every sector in disc order: pregaps of track 2 onward, then the track.
fn sectors_in_order(disc: &Disc) -> Vec<(usize, i64, i64)> {
    disc.tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let from = if i > 0 { -(t.pregap as i64) } else { 0 };
            (i, from, t.length as i64)
        })
        .collect()
}

fn write_raw(d: &mut dyn RawDisc, img: &Path, sub: Option<&Path>, progress: Progress) -> Result<()> {
    let disc = d.disc().clone();
    let mut w = BufWriter::with_capacity(1 << 20, File::create(img)?);
    let mut ws = match sub {
        Some(s) => Some(BufWriter::new(File::create(s)?)),
        None => None,
    };
    let order = sectors_in_order(&disc);
    let total: u64 = order.iter().map(|(_, a, b)| (b - a) as u64).sum();
    let mut done = 0;
    let mut s = [0u8; RAW];
    for (t, from, to) in order {
        for rel in from..to {
            d.read_raw(t, rel, &mut s)?;
            w.write_all(&s)?;
            if let Some(ws) = ws.as_mut() {
                ws.write_all(&d.read_sub(t, rel)?)?;
            }
            done += 1;
            if (done % 256 == 0 || done == total) && !progress(done, total) {
                return Err(Error::Cancelled);
            }
        }
    }
    w.into_inner().map_err(|e| Error::Io(e.into_error()))?.sync_all()?;
    if let Some(ws) = ws {
        ws.into_inner().map_err(|e| Error::Io(e.into_error()))?.sync_all()?;
    }
    Ok(())
}

/// Track starts as they land in a contiguous image.
fn contiguous(disc: &Disc) -> Disc {
    let mut d = disc.clone();
    let mut pos = 0i64;
    for (i, t) in d.tracks.iter_mut().enumerate() {
        if i > 0 {
            pos += t.pregap as i64;
        }
        t.start = pos;
        pos += t.length as i64;
    }
    d
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn to_bincue(d: &mut dyn RawDisc, dest: &Path, progress: Progress) -> Result<Vec<PathBuf>> {
    let (cue_path, bin_path) = if dest.extension().is_some_and(|e| e.eq_ignore_ascii_case("bin")) {
        (dest.with_extension("cue"), dest.to_path_buf())
    } else {
        (dest.with_extension("cue"), dest.with_extension("bin"))
    };
    let parts = [part(&bin_path), part(&cue_path)];
    let r = (|| {
        write_raw(d, &parts[0], None, progress)?;
        std::fs::write(&parts[1], cue::sheet(&contiguous(d.disc()), &file_name(&bin_path)))?;
        commit(&parts, &[bin_path.clone(), cue_path.clone()])
    })();
    if r.is_err() {
        cleanup(&parts);
    }
    r.map(|_| vec![cue_path, bin_path])
}

fn to_clonecd(d: &mut dyn RawDisc, dest: &Path, progress: Progress) -> Result<Vec<PathBuf>> {
    let ccd_path = dest.with_extension("ccd");
    let img_path = dest.with_extension("img");
    let sub_path = dest.with_extension("sub");
    let parts = [part(&img_path), part(&sub_path), part(&ccd_path)];
    let r = (|| {
        write_raw(d, &parts[0], Some(&parts[1]), progress)?;
        std::fs::write(&parts[2], ccd::sheet(&contiguous(d.disc())))?;
        commit(&parts, &[img_path.clone(), sub_path.clone(), ccd_path.clone()])
    })();
    if r.is_err() {
        cleanup(&parts);
    }
    r.map(|_| vec![ccd_path, img_path, sub_path])
}
