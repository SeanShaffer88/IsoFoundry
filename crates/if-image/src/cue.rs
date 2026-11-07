//! CUE sheets (with BIN files): reading and writing.

use crate::disc::{Disc, Layout, Track, TrackKind};
use crate::{bad, Result};
use std::path::{Path, PathBuf};

fn parse_msf(s: &str) -> Result<i64> {
    let p: Vec<&str> = s.split(':').collect();
    let n = |x: &str| x.trim().parse::<i64>().map_err(|_| crate::Error::Format(format!("bad time {s}")));
    match p.as_slice() {
        [m, sec, f] => Ok((n(m)? * 60 + n(sec)?) * 75 + n(f)?),
        _ => bad(format!("bad time {s}")),
    }
}

fn unquote(s: &str) -> (String, &str) {
    let s = s.trim_start();
    if let Some(r) = s.strip_prefix('"') {
        if let Some(e) = r.find('"') {
            return (r[..e].to_string(), &r[e + 1..]);
        }
    }
    let e = s.find(char::is_whitespace).unwrap_or(s.len());
    (s[..e].to_string(), &s[e..])
}

struct Pending {
    number: u8,
    kind: TrackKind,
    layout: Layout,
    file: usize,
    index0: Option<i64>,
    index1: Option<i64>,
    pregap: i64,
}

/// Parses a CUE sheet; BIN paths are resolved next to the sheet.
pub fn read(cue: &Path) -> Result<Disc> {
    let meta = std::fs::metadata(cue)?;
    if meta.len() > 1 << 20 {
        return bad("CUE sheet larger than 1 MB");
    }
    let text = std::fs::read_to_string(cue).map_err(|e| crate::Error::Format(format!("{}: {e}", cue.display())))?;
    let dir = cue.parent().unwrap_or(Path::new("."));
    let mut files: Vec<PathBuf> = Vec::new();
    let mut pend: Vec<Pending> = Vec::new();
    let mut notes = Vec::new();
    for line in text.lines() {
        let line = line.trim().trim_start_matches('\u{feff}');
        let (cmd, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        match cmd.to_ascii_uppercase().as_str() {
            "FILE" => {
                let (name, kind) = unquote(rest);
                let kind = kind.trim().to_ascii_uppercase();
                if kind != "BINARY" {
                    return bad(format!("FILE type {kind} is not supported (only BINARY)"));
                }
                let p = Path::new(&name);
                let p = if p.is_absolute() { p.to_path_buf() } else { dir.join(p) };
                // Some sheets name the file without its real case or folder.
                let p = if p.exists() { p } else { dir.join(p.file_name().unwrap_or_default()) };
                files.push(p);
            }
            "TRACK" => {
                let mut it = rest.split_whitespace();
                let num: u8 =
                    it.next().and_then(|n| n.parse().ok()).ok_or(crate::Error::Format(format!("bad line: {line}")))?;
                let ty = it.next().unwrap_or("").to_ascii_uppercase();
                let (kind, layout) = match ty.as_str() {
                    "AUDIO" => (TrackKind::Audio, Layout::Raw),
                    "MODE1/2048" => (TrackKind::Mode1, Layout::Cooked),
                    "MODE1/2352" => (TrackKind::Mode1, Layout::Raw),
                    "MODE2/2352" => (TrackKind::Mode2, Layout::Raw),
                    "MODE2/2336" => (TrackKind::Mode2, Layout::Mode2Headerless),
                    "MODE2/2048" => (TrackKind::Mode2Form1, Layout::Cooked),
                    other => return bad(format!("track type {other} is not supported")),
                };
                if files.is_empty() {
                    return bad("TRACK before FILE");
                }
                pend.push(Pending {
                    number: num,
                    kind,
                    layout,
                    file: files.len() - 1,
                    index0: None,
                    index1: None,
                    pregap: 0,
                });
            }
            "INDEX" => {
                let mut it = rest.split_whitespace();
                let n: u32 = it.next().and_then(|n| n.parse().ok()).unwrap_or(99);
                let t = parse_msf(it.next().unwrap_or(""))?;
                let Some(p) = pend.last_mut() else { return bad("INDEX before TRACK") };
                match n {
                    0 => p.index0 = Some(t),
                    1 => p.index1 = Some(t),
                    _ => {}
                }
            }
            "PREGAP" => {
                if let Some(p) = pend.last_mut() {
                    p.pregap = parse_msf(rest.trim())?;
                }
            }
            "POSTGAP" => notes.push("POSTGAP is ignored".to_string()),
            _ => {}
        }
    }
    if pend.is_empty() {
        return bad("no tracks in CUE sheet");
    }
    let mut tracks = Vec::new();
    let mut lba = 0i64;
    for i in 0..pend.len() {
        let p = &pend[i];
        let file = files[p.file].clone();
        let size = p.layout.size() as i64;
        let flen = std::fs::metadata(&file).map_err(|e| crate::Error::Format(format!("{}: {e}", file.display())))?.len()
            as i64;
        let i1 = p.index1.ok_or(crate::Error::Format(format!("track {} has no INDEX 01", p.number)))?;
        let stored_gap = p.index0.map(|i0| i1 - i0).unwrap_or(0);
        // End: next track in the same file starts (its INDEX 00 or 01), else file end.
        let end = pend
            .get(i + 1)
            .filter(|n| n.file == p.file)
            .map(|n| n.index0.or(n.index1).unwrap_or(0))
            .unwrap_or(flen / size);
        let length = end - i1;
        if length <= 0 {
            return bad(format!("track {} has no sectors", p.number));
        }
        let pregap = (stored_gap + p.pregap).max(0) as u64;
        lba += pregap as i64;
        let offset = (i1 - stored_gap) as u64 * size as u64;
        tracks.push(Track {
            number: p.number,
            kind: p.kind,
            start: lba,
            length: length as u64,
            pregap,
            pregap_stored: stored_gap > 0 && p.pregap == 0,
            file,
            offset,
            layout: p.layout,
            sub: None,
        });
        if stored_gap > 0 && p.pregap > 0 {
            notes.push(format!("track {} mixes stored and generated pregap; stored part kept", p.number));
        }
        lba += length;
    }
    // The first track's pregap is the standard 2 seconds before LBA 0.
    if let Some(t0) = tracks.first() {
        let shift = t0.start;
        if !t0.pregap_stored {
            for t in &mut tracks {
                t.start -= shift;
            }
        }
    }
    Ok(Disc { tracks, notes })
}

fn kind_name(t: &Track) -> &'static str {
    match t.kind {
        TrackKind::Audio => "AUDIO",
        TrackKind::Mode1 => "MODE1/2352",
        TrackKind::Mode2 | TrackKind::Mode2Form1 => "MODE2/2352",
    }
}

fn msf_text(sectors: i64) -> String {
    format!("{:02}:{:02}:{:02}", sectors / 4500, (sectors / 75) % 60, sectors % 75)
}

/// CUE text for a single BIN that stores every track raw, pregaps of
/// track 2 onward included (INDEX 00).
pub fn sheet(disc: &Disc, bin_name: &str) -> String {
    let mut s = format!("FILE \"{bin_name}\" BINARY\n");
    let mut pos = 0i64;
    for (i, t) in disc.tracks.iter().enumerate() {
        s += &format!("  TRACK {:02} {}\n", t.number, kind_name(t));
        if i == 0 {
            if t.pregap > 0 && t.pregap != 150 {
                s += &format!("    PREGAP {}\n", msf_text(t.pregap as i64 - 150));
            }
        } else if t.pregap > 0 {
            s += &format!("    INDEX 00 {}\n", msf_text(pos));
            pos += t.pregap as i64;
        }
        s += &format!("    INDEX 01 {}\n", msf_text(pos));
        pos += t.length as i64;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(n: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("if-cue-{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn mixed_mode_single_file() {
        let d = tmp("mixed");
        std::fs::write(d.join("game.bin"), vec![0u8; 2352 * (1000 + 150 + 500)]).unwrap();
        std::fs::write(
            d.join("game.cue"),
            "FILE \"game.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n  TRACK 02 AUDIO\n    INDEX 00 00:13:25\n    INDEX 01 00:15:25\n",
        )
        .unwrap();
        let disc = read(&d.join("game.cue")).unwrap();
        assert_eq!(disc.tracks.len(), 2);
        assert_eq!(disc.tracks[0].length, 1000);
        assert_eq!(disc.tracks[1].pregap, 150);
        assert!(disc.tracks[1].pregap_stored);
        assert_eq!(disc.tracks[1].start, 1150);
        assert_eq!(disc.tracks[1].length, 500);
        assert!(sheet(&disc, "x.bin").contains("INDEX 00 00:13:25"));
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn multi_file_redump_style() {
        let d = tmp("multi");
        std::fs::write(d.join("t1.bin"), vec![0u8; 2352 * 300]).unwrap();
        std::fs::write(d.join("t2.bin"), vec![0u8; 2352 * (150 + 75)]).unwrap();
        std::fs::write(
            d.join("x.cue"),
            "FILE \"t1.bin\" BINARY\n TRACK 01 MODE1/2352\n  INDEX 01 00:00:00\nFILE \"t2.bin\" BINARY\n TRACK 02 AUDIO\n  INDEX 00 00:00:00\n  INDEX 01 00:02:00\n",
        )
        .unwrap();
        let disc = read(&d.join("x.cue")).unwrap();
        assert_eq!(disc.tracks[1].start, 300 + 150);
        assert_eq!(disc.tracks[1].offset, 0);
        assert_eq!(disc.tracks[1].length, 75);
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn rejects_wave_and_garbage() {
        let d = tmp("bad");
        std::fs::write(d.join("a.cue"), "FILE \"a.wav\" WAVE\n").unwrap();
        assert!(read(&d.join("a.cue")).is_err());
        std::fs::write(d.join("b.cue"), "hello\n").unwrap();
        assert!(read(&d.join("b.cue")).is_err());
        std::fs::remove_dir_all(d).unwrap();
    }
}
