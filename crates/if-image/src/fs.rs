//! One file-tree API over ISO 9660, Joliet and UDF, plus extraction.

use crate::source::SectorSource;
use crate::{bad, Error, Result, SECTOR};
use std::io::Write;
use std::path::{Path, PathBuf};

/// A run of sectors. `sparse` extents read as zeros (UDF unrecorded).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extent {
    pub lba: u64,
    pub len: u64,
    pub sparse: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Data {
    Extents(Vec<Extent>),
    /// Small files stored inside the UDF file entry.
    Inline(Vec<u8>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    pub size: u64,
    /// Seconds since 1970-01-01 UTC.
    pub mtime: Option<i64>,
    pub hidden: bool,
    pub data: Data,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FsKind {
    Iso9660,
    Joliet,
    Udf,
}

impl FsKind {
    pub fn name(self) -> &'static str {
        match self {
            FsKind::Iso9660 => "ISO 9660",
            FsKind::Joliet => "Joliet",
            FsKind::Udf => "UDF",
        }
    }
}

pub trait FileSystem {
    fn kind(&self) -> FsKind;
    fn label(&self) -> String;
    fn root(&self) -> Entry;
    fn list(&self, src: &mut dyn SectorSource, dir: &Entry) -> Result<Vec<Entry>>;
}

/// Reads `buf.len()` bytes of an entry's data from byte `offset`.
pub fn read_data(src: &mut dyn SectorSource, data: &Data, size: u64, offset: u64, buf: &mut [u8]) -> Result<usize> {
    if offset >= size {
        return Ok(0);
    }
    let want = buf.len().min((size - offset) as usize);
    match data {
        Data::Inline(b) => {
            let s = offset as usize;
            let n = want.min(b.len().saturating_sub(s));
            buf[..n].copy_from_slice(&b[s..s + n]);
            buf[n..want].fill(0);
            Ok(want)
        }
        Data::Extents(exts) => {
            let mut done = 0usize;
            let mut base = 0u64;
            let mut sector = vec![0u8; SECTOR];
            for e in exts {
                if done == want {
                    break;
                }
                let pos = offset + done as u64;
                if pos >= base + e.len {
                    base += e.len;
                    continue;
                }
                let mut in_ext = pos - base;
                while in_ext < e.len && done < want {
                    let n = ((e.len - in_ext) as usize).min(want - done);
                    let out = &mut buf[done..done + n];
                    if e.sparse {
                        out.fill(0);
                    } else {
                        let lba = e.lba + in_ext / SECTOR as u64;
                        let skip = (in_ext % SECTOR as u64) as usize;
                        if skip == 0 && n >= SECTOR {
                            let whole = n / SECTOR * SECTOR;
                            src.read_sectors(lba, &mut out[..whole])?;
                            done += whole;
                            in_ext += whole as u64;
                            continue;
                        }
                        src.read_sectors(lba, &mut sector)?;
                        let m = n.min(SECTOR - skip);
                        out[..m].copy_from_slice(&sector[skip..skip + m]);
                        done += m;
                        in_ext += m as u64;
                        continue;
                    }
                    done += n;
                    in_ext += n as u64;
                }
                base += e.len;
            }
            if done < want {
                buf[done..want].fill(0);
            }
            Ok(want)
        }
    }
}

pub fn read_all(src: &mut dyn SectorSource, e: &Entry, limit: u64) -> Result<Vec<u8>> {
    if e.size > limit {
        return bad(format!("{} is too large to load ({} bytes)", e.name, e.size));
    }
    let mut v = vec![0u8; e.size as usize];
    read_data(src, &e.data, e.size, 0, &mut v)?;
    Ok(v)
}

/// An opened image: the sector source and the file system chosen on it.
pub struct Volume {
    pub src: Box<dyn SectorSource>,
    pub fs: Box<dyn FileSystem>,
    /// Every file system found, best first.
    pub found: Vec<FsKind>,
    pub boot: Option<crate::eltorito::BootCatalog>,
}

impl Volume {
    /// Opens the best file system on `src`: UDF (large files, Unicode),
    /// then Joliet, then plain ISO 9660. `prefer` picks one explicitly.
    pub fn open(mut src: Box<dyn SectorSource>, prefer: Option<FsKind>) -> Result<Volume> {
        let iso = crate::iso9660::scan(src.as_mut()).ok();
        let udf_result = crate::udf::Udf::open(src.as_mut());
        let udf_error = udf_result.as_ref().err().map(|e| e.to_string()).filter(|m| m != "no UDF anchor");
        let udf = udf_result.ok();
        let mut found = Vec::new();
        if udf.is_some() {
            found.push(FsKind::Udf);
        }
        if let Some(s) = &iso {
            if s.joliet.is_some() {
                found.push(FsKind::Joliet);
            }
            if s.primary.is_some() {
                found.push(FsKind::Iso9660);
            }
        }
        if found.is_empty() {
            return match udf_error {
                Some(m) => bad(format!("UDF file system found but not readable: {m}")),
                None => bad("no ISO 9660, Joliet or UDF file system found"),
            };
        }
        let want = prefer.unwrap_or(found[0]);
        if !found.contains(&want) {
            return bad(format!("this image has no {} file system", want.name()));
        }
        let boot =
            iso.as_ref().and_then(|s| s.boot_catalog).and_then(|lba| crate::eltorito::read(src.as_mut(), lba).ok());
        let fs: Box<dyn FileSystem> = match want {
            FsKind::Udf => Box::new(udf.expect("checked")),
            FsKind::Joliet => Box::new(iso.and_then(|s| s.joliet).expect("checked")),
            FsKind::Iso9660 => Box::new(iso.and_then(|s| s.primary).expect("checked")),
        };
        Ok(Volume { src, fs, found, boot })
    }

    pub fn root(&self) -> Entry {
        self.fs.root()
    }

    pub fn list(&mut self, dir: &Entry) -> Result<Vec<Entry>> {
        self.fs.list(self.src.as_mut(), dir)
    }

    /// Looks up a `/`- or `\`-separated path, case-insensitively when no
    /// exact match exists.
    pub fn lookup(&mut self, path: &str) -> Result<Entry> {
        let mut cur = self.root();
        for part in path.split(['/', '\\']).filter(|p| !p.is_empty() && *p != ".") {
            if !cur.dir {
                return bad(format!("{} is not a folder", cur.name));
            }
            let kids = self.list(&cur)?;
            cur = match kids.iter().find(|k| k.name == part) {
                Some(k) => k.clone(),
                None => match kids.iter().find(|k| k.name.eq_ignore_ascii_case(part)) {
                    Some(k) => k.clone(),
                    None => return bad(format!("not found in image: {path}")),
                },
            };
        }
        Ok(cur)
    }

    pub fn read(&mut self, e: &Entry, offset: u64, buf: &mut [u8]) -> Result<usize> {
        read_data(self.src.as_mut(), &e.data, e.size, offset, buf)
    }

    /// Every file and folder under `dir`, depth first, with paths relative
    /// to it (`/`-separated).
    pub fn walk(&mut self, dir: &Entry) -> Result<Vec<(String, Entry)>> {
        let mut out = Vec::new();
        let mut stack = vec![(String::new(), dir.clone(), 0u32)];
        while let Some((prefix, d, depth)) = stack.pop() {
            if depth > 64 {
                return bad("folders nested deeper than 64 levels (corrupt image?)");
            }
            let mut kids = self.list(&d)?;
            kids.sort_by(|a, b| a.name.cmp(&b.name));
            for k in kids.into_iter().rev() {
                let p = if prefix.is_empty() { k.name.clone() } else { format!("{prefix}/{}", k.name) };
                if k.dir {
                    stack.push((p.clone(), k.clone(), depth + 1));
                }
                out.push((p, k));
            }
        }
        out.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(out)
    }

    /// Copies `dir` (or a single file) out to `dest`. Names are made safe
    /// for Windows; nothing is written outside `dest`. `progress` gets
    /// (bytes done, bytes total) and returns false to cancel.
    pub fn extract(
        &mut self,
        entry: &Entry,
        dest: &Path,
        mut progress: impl FnMut(u64, u64) -> bool,
    ) -> Result<ExtractReport> {
        let items = if entry.dir { self.walk(entry)? } else { vec![(entry.name.clone(), entry.clone())] };
        let total: u64 = items.iter().filter(|(_, e)| !e.dir).map(|(_, e)| e.size).sum();
        std::fs::create_dir_all(dest)?;
        let mut report = ExtractReport::default();
        let mut done = 0u64;
        let mut buf = vec![0u8; 1 << 20];
        for (rel, e) in &items {
            let out = safe_join(dest, rel)?;
            if e.dir {
                std::fs::create_dir_all(&out)?;
                report.folders += 1;
                continue;
            }
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut tmp_name = out.file_name().unwrap_or_default().to_os_string();
            tmp_name.push(".isofoundry-part");
            let tmp = out.with_file_name(tmp_name);
            let mut f = std::fs::File::create(&tmp)?;
            let mut off = 0u64;
            while off < e.size {
                let n = self.read(e, off, &mut buf)?;
                f.write_all(&buf[..n])?;
                off += n as u64;
                done += n as u64;
                if !progress(done, total) {
                    drop(f);
                    let _ = std::fs::remove_file(&tmp);
                    return Err(Error::Cancelled);
                }
            }
            f.sync_all()?;
            drop(f);
            std::fs::rename(&tmp, &out)?;
            report.files += 1;
            report.bytes += e.size;
            if rel != &sanitize_rel(rel) {
                report.renamed.push((rel.clone(), sanitize_rel(rel)));
            }
        }
        Ok(report)
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ExtractReport {
    pub files: u64,
    pub folders: u64,
    pub bytes: u64,
    /// (name in image, name written) where a name had to change.
    pub renamed: Vec<(String, String)>,
}

/// Makes one path component valid on Windows.
pub fn sanitize_name(n: &str) -> String {
    let mut s: String = n
        .chars()
        .map(|c| if c < ' ' || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') { '_' } else { c })
        .collect();
    while s.ends_with('.') || s.ends_with(' ') {
        s.pop();
    }
    if s.is_empty() || s == "." || s == ".." {
        s = "_".into();
    }
    let stem = s.split('.').next().unwrap_or("").to_ascii_uppercase();
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1",
        "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if RESERVED.contains(&stem.as_str()) {
        s.insert(0, '_');
    }
    s
}

fn sanitize_rel(rel: &str) -> String {
    rel.split('/').map(sanitize_name).collect::<Vec<_>>().join("/")
}

fn safe_join(dest: &Path, rel: &str) -> Result<PathBuf> {
    let mut p = dest.to_path_buf();
    for part in rel.split('/') {
        p.push(sanitize_name(part));
    }
    if !p.starts_with(dest) {
        return bad(format!("refusing to write outside the destination: {rel}"));
    }
    Ok(p)
}

/// Days since 1970-01-01 for a civil date (proleptic Gregorian).
pub(crate) fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// (year, month, day, hour, minute, second) UTC for a Unix time.
pub fn civil_from_unix(t: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = t.div_euclid(86_400);
    let secs = t.rem_euclid(86_400);
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d, (secs / 3600) as u32, (secs % 3600 / 60) as u32, (secs % 60) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_made_safe() {
        assert_eq!(sanitize_name("a:b?.txt"), "a_b_.txt");
        assert_eq!(sanitize_name("CON"), "_CON");
        assert_eq!(sanitize_name("con.txt"), "_con.txt");
        assert_eq!(sanitize_name(".."), "_");
        assert_eq!(sanitize_name("trail. "), "trail");
        let d = Path::new("/tmp/x");
        assert!(safe_join(d, "../../etc/passwd").unwrap().starts_with(d));
    }

    #[test]
    fn civil_roundtrip() {
        for t in [0i64, 951_782_400, 1_700_000_000, -86_400] {
            let (y, m, d, hh, mm, ss) = civil_from_unix(t);
            assert_eq!(days_from_civil(y, m as i64, d as i64) * 86_400 + (hh * 3600 + mm * 60 + ss) as i64, t);
        }
        assert_eq!(civil_from_unix(951_782_400), (2000, 2, 29, 0, 0, 0));
    }

    #[test]
    fn sparse_and_split_reads() {
        let mut img = vec![0u8; SECTOR * 4];
        for (i, b) in img.iter_mut().enumerate() {
            *b = (i / SECTOR) as u8 + 1;
        }
        let mut src = crate::source::MemSource(img);
        let data = Data::Extents(vec![
            Extent { lba: 1, len: 100, sparse: false },
            Extent { lba: 0, len: 50, sparse: true },
            Extent { lba: 3, len: SECTOR as u64 + 10, sparse: false },
        ]);
        let size = 100 + 50 + SECTOR as u64;
        let mut buf = vec![9u8; size as usize];
        assert_eq!(read_data(&mut src, &data, size, 0, &mut buf).unwrap(), size as usize);
        assert!(buf[..100].iter().all(|&b| b == 2));
        assert!(buf[100..150].iter().all(|&b| b == 0));
        assert!(buf[150..].iter().all(|&b| b == 4));
        let mut part = vec![0u8; 20];
        read_data(&mut src, &data, size, 90, &mut part).unwrap();
        assert_eq!(&part[..10], &[2; 10]);
        assert_eq!(&part[10..], &[0; 10]);
    }
}
