//! Nero `.nrg` images (format reverse-engineered by others; read only).
//! Disc-at-once (DAOX/DAOI) and track-at-once (ETN2/ETNF) layouts.

use crate::disc::{Disc, Layout, Track, TrackKind};
use crate::{bad, Result};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

fn be32(b: &[u8], o: usize) -> u32 {
    u32::from_be_bytes(b[o..o + 4].try_into().unwrap())
}

fn be64(b: &[u8], o: usize) -> u64 {
    u64::from_be_bytes(b[o..o + 8].try_into().unwrap())
}

fn layout(size: u64) -> Result<Layout> {
    Ok(match size {
        2048 => Layout::Cooked,
        2336 => Layout::Mode2Headerless,
        2352 => Layout::Raw,
        2448 => Layout::RawSub,
        s => return bad(format!("unsupported NRG sector size {s}")),
    })
}

fn kind_for(mode: u32, size: u64) -> TrackKind {
    if mode == 7 || mode == 16 {
        return TrackKind::Audio;
    }
    if mode <= 1 {
        return TrackKind::Mode1;
    }
    if size == 2048 {
        TrackKind::Mode2Form1
    } else {
        TrackKind::Mode2
    }
}

pub fn read(path: &Path) -> Result<Disc> {
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    if len < 12 {
        return bad("file too small for an NRG image");
    }
    let mut tail = [0u8; 12];
    f.seek(SeekFrom::Start(len - 12))?;
    f.read_exact(&mut tail)?;
    let (start, v2) = if &tail[0..4] == b"NER5" {
        (be64(&tail, 4), true)
    } else if &tail[4..8] == b"NERO" {
        (be32(&tail, 8) as u64, false)
    } else {
        return bad("no NRG footer");
    };
    if start >= len {
        return bad("NRG chunk offset outside the file");
    }
    let chunk_len = (len - start).min(16 << 20) as usize;
    let mut buf = vec![0u8; chunk_len];
    f.seek(SeekFrom::Start(start))?;
    f.read_exact(&mut buf)?;
    let mut p = 0;
    let mut tracks = Vec::new();
    while p + 8 <= buf.len() {
        let id = &buf[p..p + 4];
        let size = be32(&buf, p + 4) as usize;
        let body = buf.get(p + 8..p + 8 + size).ok_or(crate::Error::Format("truncated NRG chunk".into()))?;
        match id {
            b"DAOX" | b"DAOI" => {
                let wide = id == b"DAOX";
                let per = if wide { 42 } else { 30 };
                let (first, last) = (body[20], body[21]);
                for (i, t) in body[22..].chunks_exact(per).enumerate() {
                    let ssize = u16::from_be_bytes([t[12], t[13]]) as u64;
                    let mode = t[14] as u32;
                    let (i0, i1, end) = if wide {
                        (be64(t, 18), be64(t, 26), be64(t, 34))
                    } else {
                        (be32(t, 18) as u64, be32(t, 22) as u64, be32(t, 26) as u64)
                    };
                    if ssize == 0 || i1 < i0 || end < i1 {
                        return bad("invalid NRG track entry");
                    }
                    tracks.push(Track {
                        number: first + i as u8,
                        kind: kind_for(mode, ssize),
                        start: 0,
                        length: (end - i1) / ssize,
                        pregap: (i1 - i0) / ssize,
                        pregap_stored: i1 > i0,
                        file: path.to_path_buf(),
                        offset: i0,
                        layout: layout(ssize)?,
                        sub: None,
                    });
                    if first + i as u8 >= last {
                        break;
                    }
                }
            }
            b"ETN2" | b"ETNF" => {
                let wide = id == b"ETN2";
                let per = if wide { 32 } else { 20 };
                for (i, t) in body.chunks_exact(per).enumerate() {
                    let (off, l, mode) = if wide {
                        (be64(t, 0), be64(t, 8), be32(t, 16))
                    } else {
                        (be32(t, 0) as u64, be32(t, 4) as u64, be32(t, 8))
                    };
                    let ssize = match mode {
                        0 => 2048,
                        2 | 3 => 2336,
                        7 => 2352,
                        _ => 2352,
                    };
                    tracks.push(Track {
                        number: i as u8 + 1,
                        kind: if mode == 7 { TrackKind::Audio } else { kind_for(mode, ssize) },
                        start: 0,
                        length: l / ssize,
                        pregap: 0,
                        pregap_stored: false,
                        file: path.to_path_buf(),
                        offset: off,
                        layout: layout(ssize)?,
                        sub: None,
                    });
                }
            }
            b"END!" => break,
            _ => {}
        }
        p += 8 + size;
        let _ = v2;
    }
    if tracks.is_empty() {
        return bad("NRG image has no tracks");
    }
    let mut lba = 0i64;
    for (i, t) in tracks.iter_mut().enumerate() {
        if i > 0 {
            lba += t.pregap as i64;
        }
        t.start = lba;
        lba += t.length as i64;
    }
    Ok(Disc { tracks, notes: vec!["NRG support is reverse-engineered and unverified".into()] })
}
