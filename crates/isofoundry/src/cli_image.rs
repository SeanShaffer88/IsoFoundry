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

pub const USAGE: &str = "  create-image SRC... --out FILE.iso [--label L] [--fs iso9660,joliet,udf|xdvdfs]\n\
       [--boot-bios IMG [--boot-load-sectors N] [--boot-info-table]] [--boot-efi IMG]\n\
                       Build an ISO from files and folders (one folder = its contents)\n\
  image-info IMAGE      Format, tracks, file systems, label and boot entries\n\
  convert-image IMAGE --out FILE [--to iso|cue|ccd|chd|chd-cd]\n\
                       Convert ISO, BIN/CUE, CCD, NRG, MDS, CHD (format from --to or extension)\n\
  edit-image IMAGE --out NEW [--add DIR=PATH] [--remove P] [--rename P=NAME]\n\
       [--mkdir P] [--label L]   Change an image and save it as a new file (options repeat, run in order)\n\
  extract IMAGE --out DIR [--path P] [--fs udf|joliet|iso9660|xdvdfs]\n\
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
        "fat" => Ok(FsKind::Fat),
        "udf" => Ok(FsKind::Udf),
        "joliet" => Ok(FsKind::Joliet),
        "iso9660" | "iso" => Ok(FsKind::Iso9660),
        "xdvdfs" | "xiso" => Ok(FsKind::Xdvdfs),
        o => Err(format!("unknown file system {o} (udf, joliet, iso9660, xdvdfs)")),
    }
}

pub(crate) fn run(cmd: &str, tail: &[String], g: &Global, o: &mut Out<'_>, log: &if_core::log::Logger) -> i32 {
    let r = match cmd {
        "image-info" => info(tail, o),
        "create-image" => create(tail, g, o),
        "convert-image" => convert(tail, g, o),
        "edit-image" => edit_image(tail, g, o),
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

/// The root for `create-image`: one folder means its contents; several
/// sources (files or folders) are placed side by side.
pub fn source_tree(sources: &[PathBuf]) -> Result<if_image::write::Node, (Exit, String)> {
    use if_image::write::{Kind, Node};
    let node = |p: &PathBuf| Node::from_path(p).map_err(|e| (Exit::Io, format!("{}: {e}", p.display())));
    match sources {
        [] => Err(usage_err("no source files or folders")),
        [one] if one.is_dir() => {
            let mut n = node(one)?;
            n.name.clear();
            Ok(n)
        }
        many => {
            let mut kids = Vec::new();
            for p in many {
                kids.push(node(p)?);
            }
            let mut names: Vec<String> = kids.iter().map(|k| k.name.to_lowercase()).collect();
            names.sort();
            if names.windows(2).any(|w| w[0] == w[1]) {
                return Err(usage_err("two sources have the same name"));
            }
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            Ok(Node { name: String::new(), mtime: now, kind: Kind::Dir(kids) })
        }
    }
}

fn create(tail: &[String], g: &Global, o: &mut Out<'_>) -> R {
    use if_image::write::{build, build_xiso, BootImage, Options};
    let a = parse(tail, &["out", "label", "fs", "boot-bios", "boot-efi", "boot-load-sectors"]).map_err(usage_err)?;
    let mut info_table = false;
    let pos: Vec<PathBuf> = a
        .pos
        .iter()
        .filter(|p| {
            if *p == "--boot-info-table" {
                info_table = true;
                false
            } else {
                true
            }
        })
        .map(PathBuf::from)
        .collect();
    let dest = PathBuf::from(a.get("out").ok_or_else(|| usage_err("missing --out FILE.iso"))?);
    let mut opts = Options::default();
    let mut xdvdfs = false;
    if let Some(fs) = a.get("fs") {
        let set: Vec<String> = fs
            .split(',')
            .map(|x| x.trim().to_ascii_lowercase())
            .map(|x| if x == "xiso" { "xdvdfs".to_string() } else { x })
            .collect();
        for x in &set {
            if !["iso9660", "joliet", "udf", "xdvdfs"].contains(&x.as_str()) {
                return Err(usage_err(format!("unknown file system {x}")));
            }
        }
        xdvdfs = set.iter().any(|x| x == "xdvdfs");
        if xdvdfs && set.len() != 1 {
            return Err(usage_err("XDVDFS cannot be combined with ISO 9660, Joliet or UDF"));
        }
        opts.joliet = set.iter().any(|x| x == "joliet");
        opts.udf = set.iter().any(|x| x == "udf");
    }
    if xdvdfs
        && (a.get("label").is_some()
            || a.get("boot-bios").is_some()
            || a.get("boot-efi").is_some()
            || a.get("boot-load-sectors").is_some()
            || info_table)
    {
        return Err(usage_err("XDVDFS does not support volume labels or El Torito boot options"));
    }
    opts.label = a.get("label").map(str::to_string).unwrap_or_else(|| {
        match pos.as_slice() {
            [one] => one.file_name().map(|n| n.to_string_lossy().into_owned()),
            _ => None,
        }
        .unwrap_or_else(|| "CDROM".into())
    });
    let boot = |p: &str| BootImage::from_path(std::path::Path::new(p)).map_err(image_err);
    if let Some(b) = a.get("boot-bios") {
        let mut img = boot(b)?;
        img.info_table = info_table;
        img.load_sectors = a
            .get("boot-load-sectors")
            .map(|n| n.parse::<u16>().map_err(|_| usage_err("--boot-load-sectors must be 1-65535")))
            .transpose()?;
        opts.bios = Some(img);
    }
    if let Some(e) = a.get("boot-efi") {
        opts.efi = Some(boot(e)?);
    }
    let root = source_tree(&pos)?;
    if g.dry_run {
        o.ok_text(&format!("would build {} from {} source(s)", dest.display(), pos.len()));
        return Ok(());
    }
    let s = if xdvdfs { build_xiso(&root, &dest, |_, _| true) } else { build(&root, opts, &dest, |_, _| true) }
        .map_err(image_err)?;
    if o.json {
        o.ok_text(
            &json!({ "ok": true, "path": dest.display().to_string(), "bytes": s.sectors * 2048,
                     "files": s.files, "folders": s.dirs })
            .to_string(),
        );
    } else {
        o.ok_text(&format!(
            "Built {} ({} bytes, {} files, {} folders)",
            dest.display(),
            s.sectors * 2048,
            s.files,
            s.dirs
        ));
    }
    Ok(())
}

fn info(tail: &[String], o: &mut Out<'_>) -> R {
    let a = parse(tail, &[]).map_err(usage_err)?;
    let path = one_path(&a, "IMAGE")?;
    let v = open_volume(&path, None)?;
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let (format, img) = if_image::container::open_image(&path).map_err(image_err)?;
    let tracks: Vec<String> = match &img {
        if_image::container::Image::Cd(d) => d
            .disc()
            .tracks
            .iter()
            .map(|t| format!("{:02} {:?} at {} ({} sectors)", t.number, t.kind, t.start, t.length))
            .collect(),
        if_image::container::Image::Sectors(_) => Vec::new(),
    };
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
            &json!({ "ok": true, "path": path.display().to_string(), "format": format.name(), "tracks": tracks,
                     "bytes": size, "label": v.fs.label(),
                     "file_systems": fss, "boot": boot })
            .to_string(),
        );
    } else {
        o.ok_text(&format!("Image:        {}", path.display()));
        o.ok_text(&format!("Format:       {}", format.name()));
        o.ok_text(&format!("Size:         {size} bytes ({} data sectors)", v.src.sectors()));
        for t in &tracks {
            o.ok_text(&format!("Track         {t}"));
        }
        o.ok_text(&format!("Label:        {}", v.fs.label()));
        o.ok_text(&format!("File systems: {}", fss.join(", ")));
        o.ok_text(&format!("Bootable:     {}", if boot.is_empty() { "no".to_string() } else { boot.join(", ") }));
    }
    Ok(())
}

fn convert(tail: &[String], g: &Global, o: &mut Out<'_>) -> R {
    use if_image::convert::{convert, Output};
    let a = parse(tail, &["out", "to"]).map_err(usage_err)?;
    let src = one_path(&a, "IMAGE")?;
    if !src.is_file() {
        return Err((Exit::Io, format!("{} not found", src.display())));
    }
    let dest = PathBuf::from(a.get("out").ok_or_else(|| usage_err("missing --out FILE"))?);
    let to = match a.get("to") {
        Some(t) => Output::parse(t)
            .ok_or_else(|| usage_err(format!("unknown output format {t} (iso, cue, ccd, chd, chd-cd)")))?,
        None => Output::for_path(&dest).ok_or_else(|| usage_err("can't tell the output format; add --to"))?,
    };
    if g.dry_run {
        let (f, _) = if_image::container::detect(&src).map_err(image_err)?;
        o.ok_text(&format!("would convert {} ({}) to {:?} at {}", src.display(), f.name(), to, dest.display()));
        return Ok(());
    }
    let mut last = 0u64;
    let s = convert(&src, &dest, to, &mut |done, total| {
        let pct = (done * 100).checked_div(total).unwrap_or(100);
        if pct != last {
            last = pct;
            o.progress(pct as u32);
        }
        true
    })
    .map_err(image_err)?;
    let files: Vec<String> = s.files.iter().map(|p| p.display().to_string()).collect();
    if o.json {
        o.ok_text(
            &json!({ "ok": true, "source_format": s.source.name(), "files": files, "warnings": s.warnings })
                .to_string(),
        );
    } else {
        o.ok_text(&format!("Converted {} ({}) to:", src.display(), s.source.name()));
        for f in &files {
            o.ok_text(&format!("  {f}"));
        }
        for w in &s.warnings {
            o.ok_text(&format!("  note: {w}"));
        }
    }
    Ok(())
}

fn edit_image(tail: &[String], g: &Global, o: &mut Out<'_>) -> R {
    use if_image::edit::Edit;
    let a = parse(tail, &["out", "add", "remove", "rename", "mkdir", "label"]).map_err(usage_err)?;
    let src = one_path(&a, "IMAGE")?;
    if !src.is_file() {
        return Err((Exit::Io, format!("{} not found", src.display())));
    }
    let dest = PathBuf::from(a.get("out").ok_or_else(|| usage_err("missing --out NEW"))?);
    let pair = |v: &str, what: &str| -> Result<(String, String), (Exit, String)> {
        v.split_once('=')
            .map(|(x, y)| (x.to_string(), y.to_string()))
            .ok_or_else(|| usage_err(format!("{what} needs A=B")))
    };
    let mut edits = Vec::new();
    for (k, v) in &a.opts {
        edits.push(match k.as_str() {
            "add" => {
                let (dir, from) = pair(v, "--add")?;
                Edit::Add { dir, from: PathBuf::from(from) }
            }
            "remove" => Edit::Remove(v.clone()),
            "rename" => {
                let (path, to) = pair(v, "--rename")?;
                Edit::Rename { path, to }
            }
            "mkdir" => Edit::MkDir(v.clone()),
            "label" => Edit::Label(v.clone()),
            _ => continue,
        });
    }
    if g.dry_run {
        let mut p = if_image::edit::Project::open(&src).map_err(image_err)?;
        for e in &edits {
            p.apply(e).map_err(image_err)?;
        }
        o.ok_text(&format!("would apply {} change(s) and write {}", edits.len(), dest.display()));
        return Ok(());
    }
    let (sum, warnings) = if_image::edit::edit(&src, &dest, &edits, |_, _| true).map_err(image_err)?;
    if o.json {
        o.ok_text(&json!({ "ok": true, "out": dest.display().to_string(), "bytes": sum.sectors * 2048, "files": sum.files, "warnings": warnings }).to_string());
    } else {
        o.ok_text(&format!("Saved {} ({} bytes, {} files)", dest.display(), sum.sectors * 2048, sum.files));
        for w in &warnings {
            o.ok_text(&format!("  note: {w}"));
        }
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

    fn xiso_fixture() -> PathBuf {
        let d = tmp("xiso");
        std::fs::create_dir_all(&d).unwrap();
        let mut image = vec![0u8; 64 * if_image::SECTOR];
        let header = 32 * if_image::SECTOR;
        image[header..header + 20].copy_from_slice(b"MICROSOFT*XBOX*MEDIA");
        image[header + 20..header + 24].copy_from_slice(&40u32.to_le_bytes());
        image[header + 24..header + 28].copy_from_slice(&(if_image::SECTOR as u32).to_le_bytes());
        image[header + 0x7ec..header + 0x7ec + 20].copy_from_slice(b"MICROSOFT*XBOX*MEDIA");

        let root = 40 * if_image::SECTOR;
        image[root..root + if_image::SECTOR].fill(0xff);
        image[root..root + 4].fill(0);
        image[root + 4..root + 8].copy_from_slice(&42u32.to_le_bytes());
        image[root + 8..root + 12].copy_from_slice(&(if_image::SECTOR as u32).to_le_bytes());
        image[root + 12] = 0x10;
        image[root + 13] = 3;
        image[root + 14..root + 17].copy_from_slice(b"sub");

        let child = 42 * if_image::SECTOR;
        image[child..child + if_image::SECTOR].fill(0xff);
        image[child..child + 4].fill(0);
        image[child + 4..child + 8].copy_from_slice(&43u32.to_le_bytes());
        image[child + 8..child + 12].copy_from_slice(&5u32.to_le_bytes());
        image[child + 12] = 0x20;
        image[child + 13] = 9;
        image[child + 14..child + 23].copy_from_slice(b"hello.txt");
        image[43 * if_image::SECTOR..43 * if_image::SECTOR + 5].copy_from_slice(b"hello");

        let path = d.join("game.iso");
        std::fs::write(&path, image).unwrap();
        path
    }

    fn fat12_fixture(name: &str) -> PathBuf {
        let d = tmp(name);
        std::fs::create_dir_all(&d).unwrap();
        let mut image = vec![0; 2880 * 512];
        let boot = &mut image[..512];
        boot[..3].copy_from_slice(&[0xeb, 0x3c, 0x90]);
        boot[3..11].copy_from_slice(b"ISOFNDRY");
        boot[11..13].copy_from_slice(&512u16.to_le_bytes());
        boot[13] = 1;
        boot[14..16].copy_from_slice(&1u16.to_le_bytes());
        boot[16] = 2;
        boot[17..19].copy_from_slice(&32u16.to_le_bytes());
        boot[19..21].copy_from_slice(&2880u16.to_le_bytes());
        boot[21] = 0xf0;
        boot[22..24].copy_from_slice(&9u16.to_le_bytes());
        boot[38] = 0x29;
        boot[43..54].copy_from_slice(b"FAT CLI    ");
        boot[510..512].copy_from_slice(&[0x55, 0xaa]);
        for start in [512usize, 10 * 512] {
            image[start..start + 5].copy_from_slice(&[0xf0, 0xff, 0xff, 0xff, 0x0f]);
        }
        let root = &mut image[19 * 512..21 * 512];
        root[..11].copy_from_slice(b"HELLO   TXT");
        root[11] = 0x20;
        root[26..28].copy_from_slice(&2u16.to_le_bytes());
        root[28..32].copy_from_slice(&5u32.to_le_bytes());
        image[21 * 512..21 * 512 + 5].copy_from_slice(b"hello");
        let path = d.join("fat.img");
        std::fs::write(&path, image).unwrap();
        path
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
    fn image_info_and_extract_support_xiso() {
        let image = xiso_fixture();
        let (code, out, err) = call(&["image-info", image.to_str().unwrap(), "--json"]);
        assert_eq!(code, 0, "{err}");
        let info: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(info["file_systems"], json!(["XDVDFS"]));

        let dest = image.parent().unwrap().join("extracted");
        let (code, _, err) = call(&[
            "extract",
            image.to_str().unwrap(),
            "--out",
            dest.to_str().unwrap(),
            "--path",
            "sub",
            "--fs",
            "xdvdfs",
        ]);
        assert_eq!(code, 0, "{err}");
        assert_eq!(std::fs::read(dest.join("hello.txt")).unwrap(), b"hello");
        std::fs::remove_dir_all(image.parent().unwrap()).unwrap();
    }

    #[test]
    fn image_info_and_extract_support_fat() {
        let image = fat12_fixture("fat12-read");
        let (code, out, err) = call(&["image-info", image.to_str().unwrap(), "--json"]);
        assert_eq!(code, 0, "{err}");
        let info: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(info["file_systems"], json!(["FAT"]));
        assert_eq!(info["label"], "FAT CLI");

        let dest = image.parent().unwrap().join("extracted");
        let (code, _, err) = call(&[
            "extract",
            image.to_str().unwrap(),
            "--out",
            dest.to_str().unwrap(),
            "--path",
            "hello.txt",
            "--fs",
            "fat",
        ]);
        assert_eq!(code, 0, "{err}");
        assert_eq!(std::fs::read(dest.join("hello.txt")).unwrap(), b"hello");
        std::fs::remove_dir_all(image.parent().unwrap()).unwrap();
    }

    #[test]
    fn image_cli_supports_fixed_and_dynamic_vhds() {
        let image = fat12_fixture("vhd-cli");
        let raw = std::fs::read(&image).unwrap();
        let dir = image.parent().unwrap();

        for (index, dynamic) in [false, true].into_iter().enumerate() {
            let source = dir.join(format!("source-{index}.vhd"));
            let mut raw_source = if_image::source::MemSource(raw.clone());
            if dynamic {
                if_image::vhd_write::write_dynamic(&mut raw_source, &source, |_, _| true).unwrap();
            } else {
                if_image::vhd_write::write_fixed(&mut raw_source, &source, |_, _| true).unwrap();
            }
            let original = std::fs::read(&source).unwrap();

            let (code, out, err) = call(&["image-info", source.to_str().unwrap(), "--json"]);
            assert_eq!(code, 0, "{out}{err}");
            let info: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
            assert_eq!(info["file_systems"], json!(["FAT"]));
            assert_eq!(info["label"], "FAT CLI");

            let extracted = dir.join(format!("extracted-{index}"));
            let (code, out, err) = call(&[
                "extract",
                source.to_str().unwrap(),
                "--out",
                extracted.to_str().unwrap(),
                "--path",
                "hello.txt",
                "--fs",
                "fat",
            ]);
            assert_eq!(code, 0, "{out}{err}");
            assert_eq!(std::fs::read(extracted.join("hello.txt")).unwrap(), b"hello");

            let edited = dir.join(format!("edited-{index}.vhd"));
            let (code, out, err) = call(&[
                "edit-image",
                source.to_str().unwrap(),
                "--out",
                edited.to_str().unwrap(),
                "--rename",
                "hello.txt=renamed.txt",
                "--label",
                "EDITED",
            ]);
            assert_eq!(code, 0, "{out}{err}");
            assert_eq!(std::fs::read(&source).unwrap(), original);

            let edited_files = dir.join(format!("edited-files-{index}"));
            let (code, out, err) = call(&[
                "extract",
                edited.to_str().unwrap(),
                "--out",
                edited_files.to_str().unwrap(),
                "--path",
                "renamed.txt",
                "--fs",
                "fat",
            ]);
            assert_eq!(code, 0, "{out}{err}");
            assert_eq!(std::fs::read(edited_files.join("renamed.txt")).unwrap(), b"hello");

            std::fs::remove_dir_all(extracted).unwrap();
            std::fs::remove_dir_all(edited_files).unwrap();
            std::fs::remove_file(source).unwrap();
            std::fs::remove_file(edited).unwrap();
        }

        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn edit_image_supports_fat_and_keeps_the_source() {
        let image = fat12_fixture("fat12-edit");
        let output = image.parent().unwrap().join("edited.img");
        let before = std::fs::read(&image).unwrap();
        let (code, out, err) = call(&[
            "edit-image",
            image.to_str().unwrap(),
            "--out",
            output.to_str().unwrap(),
            "--rename",
            "hello.txt=renamed.txt",
            "--label",
            "EDITED",
        ]);
        assert_eq!(code, 0, "{out}{err}");
        assert_eq!(std::fs::read(&image).unwrap(), before);

        let (code, out, err) = call(&["image-info", output.to_str().unwrap(), "--json"]);
        assert_eq!(code, 0, "{err}");
        let info: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(info["label"], "EDITED");

        let extracted = image.parent().unwrap().join("edited-files");
        let (code, _, err) =
            call(&["extract", output.to_str().unwrap(), "--out", extracted.to_str().unwrap(), "--path", "renamed.txt"]);
        assert_eq!(code, 0, "{err}");
        assert_eq!(std::fs::read(extracted.join("renamed.txt")).unwrap(), b"hello");
        std::fs::remove_dir_all(image.parent().unwrap()).unwrap();
    }

    #[test]
    fn create_image_can_build_xdvdfs() {
        let d = tmp("create-xiso");
        std::fs::create_dir_all(d.join("nested")).unwrap();
        std::fs::write(d.join("nested").join("payload.bin"), b"Xbox").unwrap();
        let image = d.join("game.iso");
        let (code, out, err) =
            call(&["create-image", d.to_str().unwrap(), "--out", image.to_str().unwrap(), "--fs", "xdvdfs"]);
        assert_eq!(code, 0, "{out}{err}");

        let (code, out, err) = call(&["image-info", image.to_str().unwrap(), "--json"]);
        assert_eq!(code, 0, "{err}");
        let info: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(info["file_systems"], json!(["XDVDFS"]));

        let dest = d.join("extracted");
        let (code, _, err) =
            call(&["extract", image.to_str().unwrap(), "--out", dest.to_str().unwrap(), "--fs", "xiso"]);
        assert_eq!(code, 0, "{err}");
        assert_eq!(std::fs::read(dest.join("nested").join("payload.bin")).unwrap(), b"Xbox");
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn xdvdfs_create_rejects_unsupported_iso_options() {
        let d = tmp("xiso-options");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("payload.bin"), b"Xbox").unwrap();
        let output = d.join("game.iso");
        let (code, _, err) =
            call(&["create-image", d.to_str().unwrap(), "--out", output.to_str().unwrap(), "--fs", "xdvdfs,joliet"]);
        assert_eq!(code, 2, "{err}");
        assert!(err.contains("cannot be combined"));

        let (code, _, err) = call(&[
            "create-image",
            d.to_str().unwrap(),
            "--out",
            output.to_str().unwrap(),
            "--fs",
            "xdvdfs",
            "--label",
            "GAMES",
        ]);
        assert_eq!(code, 2, "{err}");
        assert!(err.contains("does not support volume labels"));
        assert!(!output.exists());
        std::fs::remove_dir_all(d).unwrap();
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
