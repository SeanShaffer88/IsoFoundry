//! Spanning backup and restore (M6).
//!
//! A backup packs a folder onto as many discs as it needs. The plan
//! is computed up front — files in name order, greedy packing, any
//! single file larger than a span is split into `.ifpartNNN` chunks —
//! so the complete catalog is known before the first disc is burned
//! and an identical copy rides on every span as
//! `ISOFBACKUP/CATALOG.JSON`, next to a `SPAN.TXT` saying which span
//! that disc is. Restore reads the catalogs, checks the set is
//! complete, then rebuilds every file and verifies its SHA-256.
//!
//! Staging copies the source once into a work folder (one subfolder
//! per span); from there a span can be burned as a files disc or
//! written to an ISO. Nothing here talks to a drive directly — the
//! CLI and the window drive that part through `if-drive`.

use if_core::job::{CancelToken, Progress};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

pub const CATALOG_DIR: &str = "ISOFBACKUP";
pub const CATALOG_FILE: &str = "CATALOG.JSON";
pub const SPAN_FILE: &str = "SPAN.TXT";

/// Nominal span capacities in bytes (data payload).
pub const CAP_CD: u64 = 700 * 1024 * 1024;
pub const CAP_DVD: u64 = 4_700_372_992;
pub const CAP_DVD_DL: u64 = 8_543_666_176;
pub const CAP_BD: u64 = 25_025_314_816;

/// Bytes held back on every span for the ISO filesystem itself and
/// the catalog, so a packed span still fits its disc.
fn reserve(capacity: u64, file_count: usize) -> u64 {
    (2 * 1024 * 1024 + (file_count as u64) * 512).min(capacity / 8)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogPart {
    /// Path of the part file on its span, relative to the span root.
    pub rel: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogFile {
    /// Path of the restored file, relative to the backup root.
    pub rel: String,
    pub size: u64,
    pub sha256: String,
    /// 1-based span holding the file (or its first part).
    pub span: usize,
    /// Present when the file was split across spans.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parts: Option<Vec<CatalogPart>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Catalog {
    pub format: u32,
    pub id: String,
    pub name: String,
    pub created_unix: u64,
    pub capacity: u64,
    pub spans: usize,
    pub files: Vec<CatalogFile>,
}

impl Catalog {
    pub fn total_bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }
    pub fn from_json(text: &str) -> Result<Catalog, String> {
        let c: Catalog = serde_json::from_str(text).map_err(|e| format!("bad backup catalog: {e}"))?;
        if c.format != 1 {
            return Err(format!("unsupported backup catalog format {}", c.format));
        }
        Ok(c)
    }
}

/// A staged backup: one folder per span under `work_root`, catalog
/// finalized and written into every span.
pub struct StagedBackup {
    pub catalog: Catalog,
    pub span_dirs: Vec<PathBuf>,
    pub work_root: PathBuf,
}

struct Scanned {
    rel: String,
    abs: PathBuf,
    size: u64,
}

fn scan(source: &Path) -> Result<Vec<Scanned>, String> {
    let mut out = Vec::new();
    fn walk(dir: &Path, base: &Path, out: &mut Vec<Scanned>) -> Result<(), String> {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(|e| format!("cannot list {}: {e}", dir.display()))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        entries.sort();
        for p in entries {
            if p.is_dir() {
                walk(&p, base, out)?;
            } else if p.is_file() {
                let rel = p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
                let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
                out.push(Scanned { rel, abs: p, size });
            }
        }
        Ok(())
    }
    walk(source, source, &mut out)?;
    out.sort_by(|a, b| a.rel.cmp(&b.rel));
    Ok(out)
}

fn sha256_of(path: &Path) -> Result<String, String> {
    let d = crate::cli_image::hash_file(path, if_hash::Algorithms::parse("sha256").unwrap(), |_| true)
        .map_err(|(_, m)| m)?;
    d.sha256.ok_or_else(|| "no sha256 produced".to_string())
}

fn check_cancel(cancel: &CancelToken) -> Result<(), String> {
    cancel.check().map_err(|e| e.to_string())
}

/// Plan, stage and catalog a backup of `source`. Returns the staged
/// spans; the caller burns or images them, then removes `work_root`.
pub fn create_backup(
    source: &Path,
    name: &str,
    capacity: u64,
    work_root: &Path,
    progress: &Progress,
    cancel: &CancelToken,
) -> Result<StagedBackup, String> {
    if !source.is_dir() {
        return Err(format!("backup source {} is not a folder", source.display()));
    }
    if source.join(CATALOG_DIR).exists() {
        return Err(format!("the source already contains a {CATALOG_DIR} folder; move it aside first"));
    }
    let files = scan(source)?;
    if files.is_empty() {
        return Err("the source folder is empty — nothing to back up".into());
    }
    let usable = capacity.saturating_sub(reserve(capacity, files.len()));
    if usable < 8192 {
        return Err("span capacity is too small".into());
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let id = format!("{now:x}-{}", std::process::id());

    // Pack: whole files greedily; an oversize file becomes part
    // entries of at most `usable` bytes each.
    enum Entry {
        Whole(usize),
        Part(usize, u64, u64, String), // file idx, offset, len, part rel
    }
    let mut spans: Vec<Vec<Entry>> = vec![Vec::new()];
    let mut used: u64 = 0;
    for (i, f) in files.iter().enumerate() {
        if f.size <= usable {
            if used + f.size > usable {
                spans.push(Vec::new());
                used = 0;
            }
            used += f.size;
            spans.last_mut().unwrap().push(Entry::Whole(i));
        } else {
            let mut offset = 0u64;
            let mut n = 0usize;
            while offset < f.size {
                let len = usable.min(f.size - offset);
                if used + len > usable {
                    spans.push(Vec::new());
                    used = 0;
                }
                n += 1;
                used += len;
                spans.last_mut().unwrap().push(Entry::Part(i, offset, len, format!("{}.ifpart{n:03}", f.rel)));
                offset += len;
            }
        }
    }

    progress.set_total(files.iter().map(|f| f.size).sum());
    let mut catalog_files: Vec<Option<CatalogFile>> = vec![None; files.len()];
    let mut span_dirs = Vec::new();
    for (si, entries) in spans.iter().enumerate() {
        check_cancel(cancel)?;
        let dir = work_root.join(format!("span-{}", si + 1));
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot stage span {}: {e}", si + 1))?;
        for e in entries {
            match e {
                Entry::Whole(i) => {
                    let f = &files[*i];
                    let dst = dir.join(&f.rel);
                    if let Some(parent) = dst.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| format!("cannot stage {}: {e}", f.rel))?;
                    }
                    std::fs::copy(&f.abs, &dst).map_err(|e| format!("cannot copy {}: {e}", f.rel))?;
                    let sha = sha256_of(&f.abs)?;
                    progress.add(f.size);
                    catalog_files[*i] =
                        Some(CatalogFile { rel: f.rel.clone(), size: f.size, sha256: sha, span: si + 1, parts: None });
                }
                Entry::Part(i, offset, len, part_rel) => {
                    let f = &files[*i];
                    let dst = dir.join(part_rel);
                    if let Some(parent) = dst.parent() {
                        std::fs::create_dir_all(parent).map_err(|e| format!("cannot stage {part_rel}: {e}"))?;
                    }
                    let mut src = std::fs::File::open(&f.abs).map_err(|e| format!("cannot read {}: {e}", f.rel))?;
                    src.seek(SeekFrom::Start(*offset)).map_err(|e| format!("cannot read {}: {e}", f.rel))?;
                    let mut out = std::fs::File::create(&dst).map_err(|e| format!("cannot write {part_rel}: {e}"))?;
                    let mut hasher = if_hash::Hasher::new(if_hash::Algorithms::parse("sha256").unwrap());
                    let mut left = *len;
                    let mut buf = vec![0u8; 1024 * 1024];
                    while left > 0 {
                        check_cancel(cancel)?;
                        let want = (buf.len() as u64).min(left) as usize;
                        let n = src.read(&mut buf[..want]).map_err(|e| format!("cannot read {}: {e}", f.rel))?;
                        if n == 0 {
                            return Err(format!("{} ended early while splitting", f.rel));
                        }
                        out.write_all(&buf[..n]).map_err(|e| format!("cannot write {part_rel}: {e}"))?;
                        hasher.update(&buf[..n]);
                        left -= n as u64;
                    }
                    let sha = hasher.finish().sha256.ok_or_else(|| "no sha256 produced".to_string())?;
                    progress.add(*len);
                    let entry = catalog_files[*i].get_or_insert_with(|| CatalogFile {
                        rel: f.rel.clone(),
                        size: f.size,
                        sha256: String::new(), // filled below, after all parts
                        span: si + 1,
                        parts: Some(Vec::new()),
                    });
                    entry.parts.as_mut().unwrap().push(CatalogPart { rel: part_rel.clone(), size: *len, sha256: sha });
                }
            }
        }
        span_dirs.push(dir);
    }
    // Whole-file hashes for split files (their parts are hashed
    // already; the restored file is verified against this one).
    let mut final_files = Vec::with_capacity(files.len());
    for (i, cf) in catalog_files.into_iter().enumerate() {
        let mut cf = cf.ok_or("internal: unplanned file")?;
        if cf.parts.is_some() {
            cf.sha256 = sha256_of(&files[i].abs)?;
        }
        final_files.push(cf);
    }
    let catalog = Catalog {
        format: 1,
        id,
        name: name.to_string(),
        created_unix: now,
        capacity,
        spans: span_dirs.len(),
        files: final_files,
    };
    let json = catalog.to_json();
    for (si, dir) in span_dirs.iter().enumerate() {
        let cdir = dir.join(CATALOG_DIR);
        std::fs::create_dir_all(&cdir).map_err(|e| format!("cannot write catalog: {e}"))?;
        std::fs::write(cdir.join(CATALOG_FILE), &json).map_err(|e| format!("cannot write catalog: {e}"))?;
        std::fs::write(cdir.join(SPAN_FILE), format!("span {} of {}\nbackup {}\n", si + 1, catalog.spans, catalog.id))
            .map_err(|e| format!("cannot write span marker: {e}"))?;
    }
    Ok(StagedBackup { catalog, span_dirs, work_root: work_root.to_path_buf() })
}

/// Write one staged span to an ISO image file. Returns its size.
pub fn write_span_iso(
    staged_span: &Path,
    out: &Path,
    label: &str,
    progress: &Progress,
    cancel: &CancelToken,
) -> Result<u64, String> {
    check_cancel(cancel)?;
    let tree = crate::cli_image::source_tree(&[staged_span.to_path_buf()]).map_err(|(_, m)| m)?;
    let s = if_image::write::build(
        &tree,
        if_image::write::Options { label: label.to_string(), ..Default::default() },
        out,
        |_, _| true,
    )
    .map_err(|e| format!("cannot write {}: {e}", out.display()))?;
    let bytes = s.sectors * 2048;
    progress.add(bytes);
    Ok(bytes)
}

/// "1.5 GB" style formatting for reports.
pub fn human_bytes(n: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

/// Read a catalog from a folder source (an extracted span or a
/// staged span directory).
pub fn catalog_from_dir(dir: &Path) -> Result<(Catalog, usize), String> {
    let cdir = dir.join(CATALOG_DIR);
    let text = std::fs::read_to_string(cdir.join(CATALOG_FILE))
        .map_err(|e| format!("{} has no backup catalog: {e}", dir.display()))?;
    let catalog = Catalog::from_json(&text)?;
    let marker = std::fs::read_to_string(cdir.join(SPAN_FILE)).unwrap_or_default();
    let span = marker
        .split_whitespace()
        .nth(1)
        .and_then(|n| n.parse::<usize>().ok())
        .ok_or_else(|| format!("{} has no readable span marker", dir.display()))?;
    Ok((catalog, span))
}

/// Extract an ISO image's whole tree into `dest` (used to treat an
/// ISO span like a folder span during restore).
pub fn extract_iso_to(iso: &Path, dest: &Path) -> Result<(), String> {
    let (_fmt, src) = if_image::container::open(iso).map_err(|e| format!("cannot open {}: {e}", iso.display()))?;
    let mut vol = if_image::fs::Volume::open(src, None).map_err(|e| format!("cannot read {}: {e}", iso.display()))?;
    let root = vol.root();
    std::fs::create_dir_all(dest).map_err(|e| format!("cannot extract: {e}"))?;
    vol.extract(&root, dest, |_, _| true).map_err(|e| format!("cannot extract {}: {e}", iso.display()))?;
    Ok(())
}

#[derive(Debug, Clone)]
pub struct RestoreReport {
    pub files: usize,
    pub bytes: u64,
    pub backup_name: String,
    pub spans_used: usize,
}

fn safe_rel(rel: &str) -> Result<PathBuf, String> {
    let mut p = PathBuf::new();
    for part in rel.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return Err(format!("catalog path escapes the restore folder: {rel}"));
        }
        p.push(part);
    }
    if p.as_os_str().is_empty() {
        return Err(format!("catalog path is empty: {rel}"));
    }
    Ok(p)
}

/// Restore a backup from its span sources (folders and/or ISO
/// files, in any order). Every file's SHA-256 is verified.
pub fn restore(
    sources: &[PathBuf],
    dest: &Path,
    work_root: &Path,
    progress: &Progress,
    cancel: &CancelToken,
) -> Result<RestoreReport, String> {
    if sources.is_empty() {
        return Err("no span sources given".into());
    }
    // Normalize ISO sources to extracted folders.
    let mut dirs: Vec<PathBuf> = Vec::new();
    for (i, s) in sources.iter().enumerate() {
        if s.is_dir() {
            dirs.push(s.clone());
        } else {
            let x = work_root.join(format!("iso-{i}"));
            std::fs::create_dir_all(&x).map_err(|e| format!("cannot stage restore: {e}"))?;
            extract_iso_to(s, &x)?;
            dirs.push(x);
        }
    }
    let mut catalog: Option<Catalog> = None;
    let mut span_dir: std::collections::HashMap<usize, PathBuf> = std::collections::HashMap::new();
    for d in &dirs {
        let (c, span) = catalog_from_dir(d)?;
        match &catalog {
            None => catalog = Some(c),
            Some(prev) if prev.id != c.id => {
                return Err("the sources belong to different backups".into());
            }
            _ => {}
        }
        span_dir.insert(span, d.clone());
    }
    let catalog = catalog.unwrap();
    let missing: Vec<usize> = (1..=catalog.spans).filter(|s| !span_dir.contains_key(s)).collect();
    if !missing.is_empty() {
        return Err(format!(
            "span(s) {} of {} are missing",
            missing.iter().map(|s| s.to_string()).collect::<Vec<_>>().join(", "),
            catalog.spans
        ));
    }
    progress.set_total(catalog.total_bytes());
    std::fs::create_dir_all(dest).map_err(|e| format!("cannot create {}: {e}", dest.display()))?;
    let mut files = 0usize;
    let mut bytes = 0u64;
    for f in &catalog.files {
        check_cancel(cancel)?;
        let rel = safe_rel(&f.rel)?;
        let out_path = dest.join(&rel);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("cannot restore {}: {e}", f.rel))?;
        }
        let mut hasher = if_hash::Hasher::new(if_hash::Algorithms::parse("sha256").unwrap());
        {
            let mut out = std::fs::File::create(&out_path).map_err(|e| format!("cannot write {}: {e}", f.rel))?;
            match &f.parts {
                None => {
                    let src_dir = &span_dir[&f.span];
                    copy_hashed(&src_dir.join(&f.rel), &mut out, cancel, &mut |b| hasher.update(b))
                        .map_err(|e| format!("{}: {e}", f.rel))?;
                }
                Some(parts) => {
                    for part in parts {
                        let src = find_part(&span_dir, part)?;
                        let mut part_hasher = if_hash::Hasher::new(if_hash::Algorithms::parse("sha256").unwrap());
                        copy_hashed(&src, &mut out, cancel, &mut |b| {
                            part_hasher.update(b);
                            hasher.update(b);
                        })
                        .map_err(|e| format!("{} (part {}): {e}", f.rel, part.rel))?;
                        let got = part_hasher.finish().sha256.unwrap_or_default();
                        if got != part.sha256 {
                            return Err(format!("{} part {} failed its checksum", f.rel, part.rel));
                        }
                    }
                }
            }
        }
        let got = hasher.finish().sha256.unwrap_or_default();
        if got != f.sha256 {
            let _ = std::fs::remove_file(&out_path);
            return Err(format!("{} failed its checksum after restore", f.rel));
        }
        files += 1;
        bytes += f.size;
        progress.add(f.size);
    }
    Ok(RestoreReport { files, bytes, backup_name: catalog.name.clone(), spans_used: catalog.spans })
}

fn find_part(span_dir: &std::collections::HashMap<usize, PathBuf>, part: &CatalogPart) -> Result<PathBuf, String> {
    for dir in span_dir.values() {
        let p = dir.join(&part.rel);
        if p.is_file() {
            return Ok(p);
        }
    }
    Err(format!("part {} was not found on any supplied span", part.rel))
}

fn copy_hashed(
    src: &Path,
    out: &mut std::fs::File,
    cancel: &CancelToken,
    sink: &mut dyn FnMut(&[u8]),
) -> Result<(), String> {
    let mut f = std::fs::File::open(src).map_err(|e| format!("cannot read {}: {e}", src.display()))?;
    let mut buf = vec![0u8; 1024 * 1024];
    loop {
        check_cancel(cancel)?;
        let n = f.read(&mut buf).map_err(|e| format!("cannot read {}: {e}", src.display()))?;
        if n == 0 {
            return Ok(());
        }
        out.write_all(&buf[..n]).map_err(|e| format!("cannot write: {e}"))?;
        sink(&buf[..n]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("isofoundry-backup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn spanning_round_trip_with_split_file() {
        let root = tmp("rt");
        let src = root.join("src");
        std::fs::create_dir_all(src.join("sub")).unwrap();
        std::fs::write(src.join("a.txt"), b"hello backup").unwrap();
        std::fs::write(src.join("sub/b.bin"), vec![7u8; 5_000]).unwrap();
        // One file far larger than a span -> must be split.
        let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(src.join("big.bin"), &big).unwrap();

        let work = root.join("work");
        let progress = Progress::default();
        let cancel = CancelToken::new();
        let staged = create_backup(&src, "test", 64 * 1024, &work, &progress, &cancel).unwrap();
        assert!(staged.catalog.spans >= 3, "spans: {}", staged.catalog.spans);
        let big_entry = staged.catalog.files.iter().find(|f| f.rel == "big.bin").unwrap();
        assert!(big_entry.parts.as_ref().unwrap().len() >= 3);

        // Image every span, then restore from the ISOs alone.
        let iso_dir = root.join("isos");
        std::fs::create_dir_all(&iso_dir).unwrap();
        let mut isos = Vec::new();
        for (i, dir) in staged.span_dirs.iter().enumerate() {
            let iso = iso_dir.join(format!("span{}.iso", i + 1));
            let p2 = Progress::default();
            write_span_iso(dir, &iso, "BACKUP", &p2, &cancel).unwrap();
            isos.push(iso);
        }
        let dest = root.join("restored");
        let p3 = Progress::default();
        let report = restore(&isos, &dest, &root.join("rwork"), &p3, &cancel).unwrap();
        assert_eq!(report.files, 3);
        assert_eq!(std::fs::read(dest.join("a.txt")).unwrap(), b"hello backup");
        assert_eq!(std::fs::read(dest.join("big.bin")).unwrap(), big);
        assert_eq!(std::fs::read(dest.join("sub/b.bin")).unwrap(), vec![7u8; 5_000]);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn missing_span_is_named() {
        let root = tmp("missing");
        let src = root.join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("x.bin"), vec![1u8; 150_000]).unwrap();
        let progress = Progress::default();
        let cancel = CancelToken::new();
        let staged = create_backup(&src, "t", 64 * 1024, &root.join("w"), &progress, &cancel).unwrap();
        assert!(staged.catalog.spans >= 2);
        // Restore with only the first span folder.
        let p2 = Progress::default();
        let err = restore(&staged.span_dirs[..1], &root.join("out"), &root.join("rw"), &p2, &cancel).unwrap_err();
        assert!(err.contains("missing"), "{err}");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
