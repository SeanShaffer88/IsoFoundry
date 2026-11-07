//! ISO 9660 and Joliet reading (ECMA-119; Joliet = supplementary volume
//! descriptor with UCS-2 names).

use crate::fs::{days_from_civil, Data, Entry, Extent, FileSystem, FsKind};
use crate::source::SectorSource;
use crate::{bad, le32, Result, SECTOR};

/// What the volume descriptor set contains.
pub struct Scan {
    pub primary: Option<Iso9660>,
    pub joliet: Option<Iso9660>,
    /// LBA of the El Torito boot catalog, if a boot record is present.
    pub boot_catalog: Option<u64>,
    pub volume_sectors: u64,
}

#[derive(Debug, Clone)]
pub struct Iso9660 {
    joliet: bool,
    label: String,
    root: Entry,
}

/// Reads the volume descriptors starting at sector 16.
pub fn scan(src: &mut dyn SectorSource) -> Result<Scan> {
    let mut s = Scan { primary: None, joliet: None, boot_catalog: None, volume_sectors: 0 };
    let mut buf = vec![0u8; SECTOR];
    for lba in 16..16 + 64u64 {
        if lba >= src.sectors() {
            break;
        }
        src.read_sectors(lba, &mut buf)?;
        if &buf[1..6] != b"CD001" {
            break;
        }
        match buf[0] {
            0 if &buf[7..30] == b"EL TORITO SPECIFICATION" => s.boot_catalog = Some(le32(&buf, 71) as u64),
            1 if s.primary.is_none() => {
                s.volume_sectors = le32(&buf, 80) as u64 * (crate::le16(&buf, 128).max(1) as u64) / SECTOR as u64;
                s.primary = Some(parse_vd(&buf, false)?);
            }
            2 if is_joliet(&buf) && s.joliet.is_none() => s.joliet = Some(parse_vd(&buf, true)?),
            255 => break,
            _ => {}
        }
    }
    if s.primary.is_none() && s.joliet.is_none() {
        return bad("no ISO 9660 volume descriptor at sector 16");
    }
    Ok(s)
}

fn is_joliet(vd: &[u8]) -> bool {
    vd[88] == 0x25 && vd[89] == 0x2F && matches!(vd[90], 0x40 | 0x43 | 0x45)
}

fn parse_vd(vd: &[u8], joliet: bool) -> Result<Iso9660> {
    if crate::le16(vd, 128) as usize != SECTOR {
        return bad(format!("unsupported logical block size {}", crate::le16(vd, 128)));
    }
    let label = if joliet { ucs2(&vd[40..72]) } else { String::from_utf8_lossy(&vd[40..72]).into() };
    let mut root =
        parse_record(&vd[156..190], joliet).ok_or(crate::Error::Format("bad root directory record".into()))?;
    root.name = String::new();
    Ok(Iso9660 { joliet, label: label.trim_end().trim_end_matches('\0').to_string(), root })
}

fn ucs2(b: &[u8]) -> String {
    let u: Vec<u16> = b.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
    String::from_utf16_lossy(&u).trim_end_matches('\0').to_string()
}

/// One directory record (without multi-extent joining).
fn parse_record(r: &[u8], joliet: bool) -> Option<Entry> {
    let len = *r.first()? as usize;
    if len < 34 || r.len() < len {
        return None;
    }
    let name_len = r[32] as usize;
    if 33 + name_len > len {
        return None;
    }
    let raw = &r[33..33 + name_len];
    let flags = r[25];
    let dir = flags & 0x02 != 0;
    let name = if name_len == 1 && raw[0] <= 1 {
        if raw[0] == 0 {
            ".".into()
        } else {
            "..".into()
        }
    } else if joliet {
        let mut n = ucs2(raw);
        if !dir {
            if let Some(p) = n.rfind(';') {
                n.truncate(p);
            }
        }
        n
    } else if let Some(rr) = rock_ridge_name(r, name_len) {
        rr
    } else {
        let mut n = String::from_utf8_lossy(raw).into_owned();
        if !dir {
            if let Some(p) = n.rfind(';') {
                n.truncate(p);
            }
            if n.ends_with('.') {
                n.pop();
            }
        }
        n
    };
    let lba = le32(r, 2) as u64 + r[1] as u64; // skip extended attribute record
    let size = le32(r, 10) as u64;
    let d = &r[18..25];
    let mtime = (d[1] >= 1 && d[1] <= 12 && d[2] >= 1).then(|| {
        let days = days_from_civil(1900 + d[0] as i64, d[1] as i64, d[2] as i64);
        days * 86_400 + d[3] as i64 * 3600 + d[4] as i64 * 60 + d[5] as i64 - (d[6] as i8 as i64) * 15 * 60
    });
    Some(Entry {
        name,
        dir,
        size,
        mtime,
        hidden: flags & 0x01 != 0,
        data: Data::Extents(vec![Extent { lba, len: size, sparse: false }]),
    })
}

/// Rock Ridge alternate name (SUSP "NM" entries in the system use area),
/// as found on most Linux images that have no Joliet tree.
fn rock_ridge_name(r: &[u8], name_len: usize) -> Option<String> {
    let len = r[0] as usize;
    let mut p = 33 + name_len + usize::from(name_len % 2 == 0);
    let mut name = Vec::new();
    let mut found = false;
    while p + 4 <= len {
        let (sig, l) = (&r[p..p + 2], r[p + 2] as usize);
        if l < 4 || p + l > len {
            break;
        }
        if sig == b"NM" && l >= 5 {
            let flags = r[p + 4];
            if flags & 0x06 == 0 {
                name.extend_from_slice(&r[p + 5..p + l]);
                found = true;
            }
        } else if sig == b"ST" {
            break;
        }
        p += l;
    }
    (found && !name.is_empty()).then(|| String::from_utf8_lossy(&name).into_owned())
}

impl FileSystem for Iso9660 {
    fn kind(&self) -> FsKind {
        if self.joliet {
            FsKind::Joliet
        } else {
            FsKind::Iso9660
        }
    }

    fn label(&self) -> String {
        self.label.clone()
    }

    fn root(&self) -> Entry {
        self.root.clone()
    }

    fn list(&self, src: &mut dyn SectorSource, dir: &Entry) -> Result<Vec<Entry>> {
        let Data::Extents(ext) = &dir.data else { return bad("ISO 9660 directory without extents") };
        let Some(e) = ext.first() else { return Ok(Vec::new()) };
        if dir.size > 64 << 20 {
            return bad("directory larger than 64 MB (corrupt image?)");
        }
        let n = (dir.size as usize).div_ceil(SECTOR);
        let mut buf = vec![0u8; n * SECTOR];
        src.read_sectors(e.lba, &mut buf)?;
        let mut out: Vec<Entry> = Vec::new();
        let mut prev_multi = false;
        let mut sector_start = 0;
        while sector_start < dir.size as usize {
            let sec = &buf[sector_start..sector_start + SECTOR];
            let mut p = 0;
            while p < SECTOR && sec[p] != 0 {
                let len = sec[p] as usize;
                if p + len > SECTOR {
                    return bad("directory record crosses a sector boundary");
                }
                let rec = &sec[p..p + len];
                if let Some(ent) = parse_record(rec, self.joliet) {
                    let multi = rec[25] & 0x80 != 0;
                    if ent.name != "." && ent.name != ".." {
                        // Multi-extent files: records with the same name in a
                        // row; all but the last carry the 0x80 flag.
                        if prev_multi {
                            if let Some(last) = out.last_mut() {
                                if last.name == ent.name {
                                    last.size += ent.size;
                                    if let (Data::Extents(a), Data::Extents(b)) = (&mut last.data, ent.data) {
                                        a.extend(b);
                                    }
                                    prev_multi = multi;
                                    p += len;
                                    continue;
                                }
                            }
                        }
                        out.push(ent);
                    }
                    prev_multi = multi;
                }
                p += len;
            }
            sector_start += SECTOR;
        }
        Ok(out)
    }
}
