//! Image commands (M1): `image-info`, `extract`, `checksum`, `compare`,
//! `verify`. Shared by the console twin and, through the same functions,
//! the task windows.

use crate::cli::{Global, Out};
use if_core::error::Exit;
use if_hash::dat::{Dat, Game, Rom};
use if_hash::{Algorithms, Digests};
use if_image::fs::{FsKind, Volume};
use serde_json::json;
use std::io::Read;
use std::path::{Path, PathBuf};

pub const USAGE: &str = "  image-info IMAGE      File systems, label, size and boot entries of an image\n\
  extract IMAGE --out DIR [--path P] [--fs udf|joliet|iso9660]\n\
                       Copy files out of an image (all, or the file/folder P)\n\
  checksum FILE [--algo crc32,md5,sha1,sha256] [--dat OUT.dat]\n\
                       Hash a file (default: all four); optionally write a .dat\n\
  compare A B          Compare two files byte for byte\n\
  verify FILE --dat LIST.dat\n\
                       Look a file's hashes up in a Logiqx/redump .dat\n";

/// Parsed `--key value` options plus positionals.
struct Args {
    pos: Vec<String>,
    opts: Vec<(String, String)>,
}

fn parse(tail: &[String], keys: &[&str]) -> Result<Args, String> {
    let mut a = Args { pos: Vec::new(), opts: Vec::new() };
    let mut it = tail.iter();
    while let Some(x) = it.next() {
        if let Some(k) = x.strip_prefix("--") {
            if !keys.contains(&k) {
                return Err(format!("unknown option --{k}"));
            }
            let v = it.next().ok_or_else(|| format!("--{k} needs a value"))?;
            a.opts.push((k.to_string(), v.clone()));
        } else {
            a.pos.push(x.clone());
        }
    }
    Ok(a)
}

impl Args {
    fn get(&self, k: &str) -> Option<&str> {
        self.opts.iter().rev().find(|(x, _)| x == k).map(|(_, v)| v.as_str())
    }
}

fn image_err(e: if_image::Error) -> (Exit, String) {
    match e {
        if_image::Error::Cancelled => (Exit::Cancelled, "cancelled".into()),
        if_image::Error::Io(e) => (Exit::Io, e.to_string()),
        if_image::Error::Format(m) => (Exit::MediaIncompatible, m),
    }
}

pub fn open_volume(path: &Path, prefer: Option<FsKind>) -> Result<Volume, (Exit, String)> {
    if !path.is_file() {
        return Err((Exit::Io, format!("{} not found", path.display())));
    }
    let (_, src) = if_image::container::open(path).map_err(image_err)?;
    Volume::open(src, prefer).map_err(image_err)
}

fn fs_kind(s: &str) -> Result<FsKind, String> {
    match s.to_ascii_lowercase().as_str() {
        "udf" => Ok(FsKind::Udf),
        "joliet" => Ok(FsKind::Joliet),
        "iso9660" | "iso" => Ok(FsKind::Iso9660),
        o => Err(format!("unknown file system {o} (udf, joliet, iso9660)")),
    }
}

pub(crate) fn run(cmd: &str, tail: &[String], g: &Global, o: &mut Out<'_>, log: &if_core::log::Logger) -> i32 {
    let r = match cmd {
        "image-info" => info(tail, o),
        "extract" => extract(tail, g, o),
        "checksum" => checksum(tail, g, o),
        "compare" => compare(tail, o),
        "verify" => verify(tail, o),
        _ => Err((Exit::Usage, format!("unknown command {cmd}"))),
    };
    match r {
        Ok(()) => 0,
        Err((exit, m)) => {
            log.error(format!("{cmd}: {m}"));
            o.fail(exit, &m)
        }
    }
}

type R = Result<(), (Exit, String)>;

fn usage_err(m: impl Into<String>) -> (Exit, String) {
    (Exit::Usage, m.into())
}

fn one_path(a: &Args, what: &str) -> Result<PathBuf, (Exit, String)> {
    match a.pos.as_slice() {
        [p] => Ok(PathBuf::from(p)),
        [] => Err(usage_err(format!("missing {what}"))),
        [_, x, ..] => Err(usage_err(format!("unexpected argument: {x}"))),
    }
}

fn info(tail: &[String], o: &mut Out<'_>) -> R {
    let a = parse(tail, &[]).map_err(usage_err)?;
    let path = one_path(&a, "IMAGE")?;
    let v = open_volume(&path, None)?;
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let boot: Vec<String> = v
        .boot
        .as_ref()
        .map(|c| {
            c.entries
                .iter()
                .filter(|e| e.bootable)
                .map(|e| match e.platform {
                    if_image::eltorito::Platform::Bios => "BIOS".to_string(),
                    if_image::eltorito::Platform::Efi => "UEFI".to_string(),
                    if_image::eltorito::Platform::Other(x) => format!("platform 0x{x:02x}"),
                })
                .collect()
        })
        .unwrap_or_default();
    let fss: Vec<&str> = v.found.iter().map(|k| k.name()).collect();
    if o.json {
        o.ok_text(
            &json!({ "ok": true, "path": path.display().to_string(), "bytes": size, "label": v.fs.label(),
                     "file_systems": fss, "boot": boot })
            .to_string(),
        );
    } else {
        o.ok_text(&format!("Image:        {}", path.display()));
        o.ok_text(&format!("Size:         {size} bytes ({} sectors)", v.src.sectors()));
        o.ok_text(&format!("Label:        {}", v.fs.label()));
        o.ok_text(&format!("File systems: {}", fss.join(", ")));
        o.ok_text(&format!("Bootable:     {}", if boot.is_empty() { "no".to_string() } else { boot.join(", ") }));
    }
    Ok(())
}

fn extract(tail: &[String], g: &Global, o: &mut Out<'_>) -> R {
    let a = parse(tail, &["out", "path", "fs"]).map_err(usage_err)?;
    let path = one_path(&a, "IMAGE")?;
    let dest = PathBuf::from(a.get("out").ok_or_else(|| usage_err("missing --out DIR"))?);
    let prefer = a.get("fs").map(fs_kind).transpose().map_err(usage_err)?;
    let mut v = open_volume(&path, prefer)?;
    let entry = match a.get("path") {
        Some(p) => v.lookup(p).map_err(image_err)?,
        None => v.root(),
    };
    if g.dry_run {
        let items =
            if entry.dir { v.walk(&entry).map_err(image_err)? } else { vec![(entry.name.clone(), entry.clone())] };
        let bytes: u64 = items.iter().filter(|(_, e)| !e.dir).map(|(_, e)| e.size).sum();
        o.ok_text(&format!("would extract {} items ({bytes} bytes) to {}", items.len(), dest.display()));
        return Ok(());
    }
    let rep = v.extract(&entry, &dest, |_, _| true).map_err(image_err)?;
    if o.json {
        o.ok_text(
            &json!({ "ok": true, "files": rep.files, "folders": rep.folders, "bytes": rep.bytes,
                     "renamed": rep.renamed.iter().map(|(a, b)| json!({"from": a, "to": b})).collect::<Vec<_>>() })
            .to_string(),
        );
    } else {
        o.ok_text(&format!(
            "Extracted {} files and {} folders ({} bytes) to {}",
            rep.files,
            rep.folders,
            rep.bytes,
            dest.display()
        ));
        for (from, to) in &rep.renamed {
            o.ok_text(&format!("  renamed for Windows: {from} -> {to}"));
        }
    }
    Ok(())
}

pub fn hash_file(path: &Path, algos: Algorithms, progress: impl FnMut(u64) -> bool) -> Result<Digests, (Exit, String)> {
    let mut f = std::fs::File::open(path).map_err(|e| (Exit::Io, format!("{}: {e}", path.display())))?;
    if_hash::hash_reader(&mut f, algos, progress).map_err(|e| {
        if e.kind() == std::io::ErrorKind::Interrupted {
            (Exit::Cancelled, "cancelled".into())
        } else {
            (Exit::Io, format!("{}: {e}", path.display()))
        }
    })
}

fn digests_json(d: &Digests) -> serde_json::Value {
    json!({ "bytes": d.size, "crc32": d.crc32, "md5": d.md5, "sha1": d.sha1, "sha256": d.sha256 })
}

fn print_digests(o: &mut Out<'_>, d: &Digests) {
    o.ok_text(&format!("Size:    {} bytes", d.size));
    for (k, v) in [("CRC32:", &d.crc32), ("MD5:", &d.md5), ("SHA-1:", &d.sha1), ("SHA-256:", &d.sha256)] {
        if let Some(v) = v {
            o.ok_text(&format!("{k:<8} {v}"));
        }
    }
}

fn checksum(tail: &[String], g: &Global, o: &mut Out<'_>) -> R {
    let a = parse(tail, &["algo", "dat"]).map_err(usage_err)?;
    let path = one_path(&a, "FILE")?;
    let algos = Algorithms::parse(a.get("algo").unwrap_or("all")).map_err(usage_err)?;
    let d = hash_file(&path, algos, |_| true)?;
    if let Some(dat_path) = a.get("dat") {
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let stem = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let dat = Dat {
            name: stem.clone(),
            description: format!("Hashes written by IsoFoundry {}", if_core::APP_VERSION),
            games: vec![Game { name: stem, roms: vec![Rom::from_digests(&name, &d)] }],
        };
        if !g.dry_run {
            if_core::fsutil::atomic_write(Path::new(dat_path), dat.to_xml().as_bytes())
                .map_err(|e| (Exit::Io, format!("{dat_path}: {e}")))?;
        }
    }
    if o.json {
        let mut v = digests_json(&d);
        v["ok"] = json!(true);
        v["path"] = json!(path.display().to_string());
        o.ok_text(&v.to_string());
    } else {
        o.ok_text(&format!("File:    {}", path.display()));
        print_digests(o, &d);
    }
    Ok(())
}

/// Byte-for-byte comparison. Returns the first differing offset.
pub fn compare_files(
    a: &Path,
    b: &Path,
    mut progress: impl FnMut(u64, u64) -> bool,
) -> Result<Option<u64>, (Exit, String)> {
    let open = |p: &Path| std::fs::File::open(p).map_err(|e| (Exit::Io, format!("{}: {e}", p.display())));
    let (mut fa, mut fb) = (open(a)?, open(b)?);
    let (la, lb) = (
        fa.metadata().map_err(|e| (Exit::Io, e.to_string()))?.len(),
        fb.metadata().map_err(|e| (Exit::Io, e.to_string()))?.len(),
    );
    let total = la.max(lb);
    let (mut ba, mut bb) = (vec![0u8; 1 << 20], vec![0u8; 1 << 20]);
    let mut off = 0u64;
    loop {
        let na = read_full(&mut fa, &mut ba).map_err(|e| (Exit::Io, e.to_string()))?;
        let nb = read_full(&mut fb, &mut bb).map_err(|e| (Exit::Io, e.to_string()))?;
        let n = na.min(nb);
        if let Some(i) = (0..n).find(|&i| ba[i] != bb[i]) {
            return Ok(Some(off + i as u64));
        }
        if na != nb {
            return Ok(Some(off + n as u64));
        }
        if na == 0 {
            return Ok(None);
        }
        off += n as u64;
        if !progress(off, total) {
            return Err((Exit::Cancelled, "cancelled".into()));
        }
    }
}

fn read_full(f: &mut std::fs::File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

fn compare(tail: &[String], o: &mut Out<'_>) -> R {
    let a = parse(tail, &[]).map_err(usage_err)?;
    let [x, y] = a.pos.as_slice() else { return Err(usage_err("usage: isofoundry compare A B")) };
    match compare_files(Path::new(x), Path::new(y), |_, _| true)? {
        None => {
            if o.json {
                o.ok_text(&json!({ "ok": true, "identical": true }).to_string());
            } else {
                o.ok_text("Identical.");
            }
            Ok(())
        }
        Some(at) => Err((Exit::VerifyMismatch, format!("files differ at byte {at}"))),
    }
}

fn verify(tail: &[String], o: &mut Out<'_>) -> R {
    let a = parse(tail, &["dat"]).map_err(usage_err)?;
    let path = one_path(&a, "FILE")?;
    let dat_path = a.get("dat").ok_or_else(|| usage_err("missing --dat LIST.dat"))?;
    let meta = std::fs::metadata(dat_path).map_err(|e| (Exit::Io, format!("{dat_path}: {e}")))?;
    if meta.len() > 512 << 20 {
        return Err((Exit::Usage, "the .dat file is larger than 512 MB".into()));
    }
    let text = std::fs::read_to_string(dat_path).map_err(|e| (Exit::Io, format!("{dat_path}: {e}")))?;
    let dat = Dat::parse(&text).map_err(|m| (Exit::Usage, format!("{dat_path}: {m}")))?;
    let d = hash_file(&path, Algorithms::default(), |_| true)?;
    let hits = dat.find(&d);
    if hits.is_empty() {
        return Err((Exit::VerifyMismatch, format!("no entry in {} matches this file", dat.name)));
    }
    if o.json {
        let m: Vec<_> = hits.iter().map(|(g, r)| json!({ "game": g.name, "rom": r.name })).collect();
        o.ok_text(&json!({ "ok": true, "matches": m, "hashes": digests_json(&d) }).to_string());
    } else {
        for (g, r) in hits {
            o.ok_text(&format!("Verified: matches \"{}\" ({}) in {}", g.name, r.name, dat.name));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(n: &str) -> String {
        format!("{}/../if-image/tests/fixtures/{n}", env!("CARGO_MANIFEST_DIR"))
    }

    fn call(args: &[&str]) -> (i32, String, String) {
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let (g, rest) = crate::cli::split_globals(&args);
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = {
            let mut o = Out { out: &mut out, err: &mut err, json: g.json };
            run(&rest[0], &rest[1..], &g, &mut o, &if_core::log::Logger::memory())
        };
        (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
    }

    fn tmp(n: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("isofoundry-cli-{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn info_json() {
        let (code, out, _) = call(&["image-info", &fixture("boot.iso"), "--json"]);
        assert_eq!(code, 0);
        let v: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(v["label"], "BOOTME");
        assert_eq!(v["boot"], json!(["BIOS", "UEFI"]));
    }

    #[test]
    fn extract_one_file_and_dry_run() {
        let d = tmp("x");
        let (code, out, _) = call(&["extract", &fixture("udf.iso"), "--out", d.to_str().unwrap(), "--dry-run"]);
        assert_eq!(code, 0, "{out}");
        assert!(out.contains("would extract") && !d.exists());
        let (code, _, err) =
            call(&["extract", &fixture("udf.iso"), "--out", d.to_str().unwrap(), "--path", "Docs/Deep"]);
        assert_eq!(code, 0, "{err}");
        assert_eq!(std::fs::read(d.join("Deeper/leaf.dat")).unwrap(), b"leaf");
        std::fs::remove_dir_all(d).unwrap();
        let (code, _, err) = call(&["extract", &fixture("udf.iso"), "--out", "x", "--path", "nope"]);
        assert_eq!(code, 11, "{err}");
    }

    #[test]
    fn checksum_dat_and_verify() {
        let d = tmp("dat");
        std::fs::create_dir_all(&d).unwrap();
        let dat = d.join("plain.dat");
        let (code, out, _) = call(&["checksum", &fixture("plain.iso"), "--dat", dat.to_str().unwrap(), "--json"]);
        assert_eq!(code, 0);
        let v: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(v["sha256"].as_str().unwrap().len(), 64);
        let (code, out, _) = call(&["verify", &fixture("plain.iso"), "--dat", dat.to_str().unwrap()]);
        assert_eq!(code, 0);
        assert!(out.contains("Verified"));
        let (code, _, _) = call(&["verify", &fixture("udf.iso"), "--dat", dat.to_str().unwrap()]);
        assert_eq!(code, 13);
        let (code, _, err) = call(&["checksum", &fixture("plain.iso"), "--algo", "md4"]);
        assert_eq!(code, 2, "{err}");
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn compare_detects_difference() {
        let (code, _, _) = call(&["compare", &fixture("udf.iso"), &fixture("udf.iso")]);
        assert_eq!(code, 0);
        let (code, _, err) = call(&["compare", &fixture("udf.iso"), &fixture("plain.iso")]);
        assert_eq!(code, 13);
        assert!(err.contains("differ at byte"));
    }
}
