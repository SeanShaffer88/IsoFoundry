//! Alcohol 120% `.mds`/`.mdf` images (descriptor format reverse-engineered
//! by others; read only, first session).

use crate::disc::{Disc, Layout, Track, TrackKind};
use crate::{bad, le16, le32, le64, Result};
use std::path::Path;

pub fn read(mds: &Path) -> Result<Disc> {
    let meta = std::fs::metadata(mds)?;
    if meta.len() > 16 << 20 {
        return bad("MDS descriptor larger than 16 MB");
    }
    let b = std::fs::read(mds)?;
    if b.len() < 0x58 || &b[..16] != b"MEDIA DESCRIPTOR" {
        return bad("not an Alcohol MDS descriptor");
    }
    if b[16] != 1 {
        return bad(format!("MDS version {}.{} is not supported", b[16], b[17]));
    }
    let mdf = mds.with_extension("mdf");
    if !mdf.is_file() {
        return bad(format!("{} is missing", mdf.display()));
    }
    let medium = le16(&b, 0x12);
    if medium >= 0x10 {
        // DVD: one track of 2048-byte sectors.
        let mut d = crate::disc::from_iso(&mdf)?;
        d.notes.push("MDS support is reverse-engineered and unverified".into());
        return Ok(d);
    }
    let sess = le32(&b, 0x50) as usize;
    if sess + 0x18 > b.len() {
        return bad("MDS session block outside the file");
    }
    let n_blocks = b[sess + 0x0A] as usize;
    let tblocks = le32(&b, sess + 0x14) as usize;
    let mut tracks = Vec::new();
    for i in 0..n_blocks {
        let t = tblocks + i * 0x50;
        let tb = b.get(t..t + 0x50).ok_or(crate::Error::Format("MDS track block outside the file".into()))?;
        let point = tb[4];
        if !(1..=99).contains(&point) {
            continue;
        }
        let kind = match tb[0] & 0x0F {
            0x09 => TrackKind::Audio,
            0x0A => TrackKind::Mode1,
            0x0C => TrackKind::Mode2Form1,
            _ => TrackKind::Mode2,
        };
        let size = le16(tb, 0x10) as u64;
        let layout = match size {
            2048 => Layout::Cooked,
            2336 => Layout::Mode2Headerless,
            2352 => Layout::Raw,
            2448 => Layout::RawSub,
            s => return crate::bad(format!("unsupported MDS sector size {s}")),
        };
        let extra = le32(tb, 0x0C) as usize;
        let (pregap, length) = match b.get(extra..extra + 8) {
            Some(e) if extra > 0 => (le32(e, 0) as u64, le32(e, 4) as u64),
            _ => return bad("MDS track without length information"),
        };
        tracks.push(Track {
            number: point,
            kind,
            start: le32(tb, 0x24) as i64,
            length,
            pregap,
            pregap_stored: false,
            file: mdf.clone(),
            offset: le64(tb, 0x28),
            layout,
            sub: None,
        });
    }
    if tracks.is_empty() {
        return bad("MDS descriptor lists no tracks");
    }
    Ok(Disc { tracks, notes: vec!["MDS support is reverse-engineered and unverified".into()] })
}
