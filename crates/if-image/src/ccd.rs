//! CloneCD images: `.ccd` (TOC as INI text), `.img` (raw 2352-byte
//! sectors from LBA 0), `.sub` (96 bytes of subchannel per sector).

use crate::disc::{Disc, Layout, Track, TrackKind};
use crate::{bad, Result};
use std::collections::BTreeMap;
use std::path::Path;

fn sections(text: &str) -> Vec<(String, BTreeMap<String, String>)> {
    let mut out: Vec<(String, BTreeMap<String, String>)> = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') && l.ends_with(']') {
            out.push((l[1..l.len() - 1].trim().to_ascii_uppercase(), BTreeMap::new()));
        } else if let Some((k, v)) = l.split_once('=') {
            if let Some(last) = out.last_mut() {
                last.1.insert(k.trim().to_ascii_uppercase(), v.trim().to_string());
            }
        }
    }
    out
}

fn num(v: Option<&String>) -> Option<i64> {
    let v = v?.trim();
    if let Some(h) = v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")) {
        i64::from_str_radix(h, 16).ok()
    } else {
        v.parse().ok()
    }
}

pub fn read(ccd: &Path) -> Result<Disc> {
    if std::fs::metadata(ccd)?.len() > 1 << 20 {
        return bad("CCD file larger than 1 MB");
    }
    let text = std::fs::read_to_string(ccd)?;
    let img = ccd.with_extension("img");
    let sub = ccd.with_extension("sub");
    if !img.is_file() {
        return bad(format!("{} is missing", img.display()));
    }
    let secs = sections(&text);
    let mut starts: BTreeMap<u8, (i64, u8)> = BTreeMap::new();
    let mut leadout = None;
    for (name, kv) in &secs {
        if !name.starts_with("ENTRY") {
            continue;
        }
        if num(kv.get("SESSION")).unwrap_or(1) != 1 {
            continue;
        }
        let point = num(kv.get("POINT")).unwrap_or(-1);
        let plba = num(kv.get("PLBA"));
        let control = num(kv.get("CONTROL")).unwrap_or(0) as u8;
        match point {
            1..=99 => {
                if let Some(l) = plba {
                    starts.insert(point as u8, (l, control));
                }
            }
            0xA2 => leadout = plba,
            _ => {}
        }
    }
    if secs.iter().any(|(n, kv)| n.starts_with("ENTRY") && num(kv.get("SESSION")).unwrap_or(1) > 1) {
        // Only the first session is represented.
    }
    let img_sectors = std::fs::metadata(&img)?.len() as i64 / 2352;
    let leadout = leadout.unwrap_or(img_sectors).min(img_sectors);
    let mut tracks = Vec::new();
    let nums: Vec<u8> = starts.keys().copied().collect();
    for (i, &n) in nums.iter().enumerate() {
        let (start, control) = starts[&n];
        let tsec = secs.iter().find(|(name, _)| name == &format!("TRACK {n}"));
        let mode = tsec.and_then(|(_, kv)| num(kv.get("MODE"))).unwrap_or(if control & 4 != 0 { 1 } else { 0 });
        let index0 = tsec.and_then(|(_, kv)| num(kv.get("INDEX 0")));
        let kind = match mode {
            0 => TrackKind::Audio,
            1 => TrackKind::Mode1,
            _ => TrackKind::Mode2,
        };
        let end = nums.get(i + 1).map(|m| {
            let next = secs.iter().find(|(name, _)| name == &format!("TRACK {m}"));
            next.and_then(|(_, kv)| num(kv.get("INDEX 0"))).unwrap_or(starts[m].0)
        });
        let end = end.unwrap_or(leadout);
        let pregap = index0.map(|i0| (start - i0).max(0)).unwrap_or(0);
        let length = end - start;
        if length <= 0 || start < 0 {
            return bad(format!("track {n} has an invalid position in the CCD"));
        }
        tracks.push(Track {
            number: n,
            kind,
            start,
            length: length as u64,
            pregap: pregap as u64,
            pregap_stored: pregap > 0,
            file: img.clone(),
            offset: (start - pregap) as u64 * 2352,
            layout: Layout::Raw,
            sub: sub.is_file().then(|| (sub.clone(), 0)),
        });
    }
    if tracks.is_empty() {
        return bad("no tracks in CCD");
    }
    let mut notes = Vec::new();
    if !sub.is_file() {
        notes.push("no .sub file; subchannel data will be generated".into());
    }
    Ok(Disc { tracks, notes })
}

fn bcd_msf(abs: i64) -> (u8, u8, u8) {
    crate::cd::msf(abs)
}

/// CCD text for `disc` stored contiguously from LBA 0 in the `.img`.
pub fn sheet(disc: &Disc) -> String {
    let last = disc.tracks.last().expect("tracks");
    let leadout = last.start + last.length as i64;
    let xa = disc.tracks.iter().any(|t| matches!(t.kind, TrackKind::Mode2 | TrackKind::Mode2Form1));
    let mut entries: Vec<(u8, u8, i64, u8)> = vec![
        (0xA0, disc.tracks[0].kind.control(), -1, disc.tracks[0].number),
        (0xA1, last.kind.control(), -1, last.number),
        (0xA2, last.kind.control(), leadout, 0),
    ];
    for t in &disc.tracks {
        entries.push((t.number, t.kind.control(), t.start, 0));
    }
    let mut s = format!(
        "[CloneCD]\r\nVersion=3\r\n[Disc]\r\nTocEntries={}\r\nSessions=1\r\nDataTracksScrambled=0\r\nCDTextLength=0\r\n[Session 1]\r\nPreGapMode={}\r\nPreGapSubC=0\r\n",
        entries.len(),
        if xa { 2 } else { 1 }
    );
    for (i, (point, control, lba, special)) in entries.iter().enumerate() {
        let (pm, ps, pf, plba) = match point {
            0xA0 => (
                *special,
                if xa { 0x20 } else { 0 },
                0,
                (*special as i64) * 4500 + if xa { 0x20 * 75 } else { 0 } - 150,
            ),
            0xA1 => (*special, 0, 0, (*special as i64) * 4500 - 150),
            _ => {
                let (m, s, f) = bcd_msf(lba + 150);
                (m, s, f, *lba)
            }
        };
        s += &format!(
            "[Entry {i}]\r\nSession=1\r\nPoint=0x{point:02x}\r\nADR=0x01\r\nControl=0x{control:02x}\r\nTrackNo=0\r\nAMin=0\r\nASec=0\r\nAFrame=0\r\nALBA=-150\r\nZero=0\r\nPMin={pm}\r\nPSec={ps}\r\nPFrame={pf}\r\nPLBA={plba}\r\n"
        );
    }
    for t in &disc.tracks {
        let mode = match t.kind {
            TrackKind::Audio => 0,
            TrackKind::Mode1 => 1,
            _ => 2,
        };
        s += &format!("[TRACK {}]\r\nMODE={mode}\r\n", t.number);
        if t.pregap > 0 && t.number > disc.tracks[0].number {
            s += &format!("INDEX 0={}\r\n", t.start - t.pregap as i64);
        }
        s += &format!("INDEX 1={}\r\n", t.start);
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sheet_parses_back() {
        let d = std::env::temp_dir().join(format!("if-ccd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let disc = Disc {
            tracks: vec![
                Track {
                    number: 1,
                    kind: TrackKind::Mode1,
                    start: 0,
                    length: 300,
                    pregap: 0,
                    pregap_stored: false,
                    file: d.join("x.img"),
                    offset: 0,
                    layout: Layout::Raw,
                    sub: None,
                },
                Track {
                    number: 2,
                    kind: TrackKind::Audio,
                    start: 450,
                    length: 100,
                    pregap: 150,
                    pregap_stored: true,
                    file: d.join("x.img"),
                    offset: 300 * 2352,
                    layout: Layout::Raw,
                    sub: None,
                },
            ],
            notes: vec![],
        };
        std::fs::write(d.join("x.ccd"), sheet(&disc)).unwrap();
        std::fs::write(d.join("x.img"), vec![0u8; 550 * 2352]).unwrap();
        let back = read(&d.join("x.ccd")).unwrap();
        assert_eq!(back.tracks.len(), 2);
        assert_eq!(back.tracks[0].length, 300);
        assert_eq!((back.tracks[1].start, back.tracks[1].length, back.tracks[1].pregap), (450, 100, 150));
        assert_eq!(back.tracks[1].kind, TrackKind::Audio);
        std::fs::remove_dir_all(d).unwrap();
    }
}
