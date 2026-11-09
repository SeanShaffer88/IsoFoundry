//! Builds ISO images: ISO 9660 + Joliet + UDF 1.02 bridge, optionally
//! bootable (El Torito BIOS no-emulation and UEFI entries). File data is
//! stored once and shared by all three directory trees, so Windows, macOS
//! and Linux each read the tree they prefer.
//!
//! The image is planned first (every sector assigned) and then written
//! front to back in one pass to a temporary file, renamed on success.

mod names;
pub(crate) mod udfw;
mod xiso;

use crate::{bad, Error, Result, SECTOR};
use std::collections::HashSet;
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use udfw::{put16, put32, put64};

/// ISO 9660 extents hold at most 4 GiB - 2048; UDF short_ad at most 1 GiB - 2048.
const ISO_MAX_EXTENT: u64 = 0xFFFF_F800;
const UDF_MAX_EXTENT: u64 = 0x3FFF_F800;
const S: u64 = SECTOR as u64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    File(PathBuf),
    Bytes(Vec<u8>),
    /// A file inside another image (image editing).
    Image(ImageData),
}

/// Shared handle on an opened image, for files copied out of it.
pub type SharedSource = std::rc::Rc<std::cell::RefCell<Box<dyn crate::source::SectorSource>>>;

#[derive(Clone)]
pub struct ImageData {
    pub src: SharedSource,
    pub data: crate::fs::Data,
}

impl std::fmt::Debug for ImageData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ImageData({:?})", self.data)
    }
}

impl PartialEq for ImageData {
    fn eq(&self, o: &Self) -> bool {
        std::rc::Rc::ptr_eq(&self.src, &o.src) && self.data == o.data
    }
}

impl Eq for ImageData {}

struct ImageReader {
    d: ImageData,
    size: u64,
    pos: u64,
}

impl Read for ImageReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = crate::fs::read_data(self.d.src.borrow_mut().as_mut(), &self.d.data, self.size, self.pos, buf)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        self.pos += n as u64;
        Ok(n)
    }
}

fn reader(src: &Source, size: u64) -> Result<Box<dyn Read>> {
    Ok(match src {
        Source::File(p) => Box::new(File::open(p).map_err(|e| Error::Format(format!("{}: {e}", p.display())))?),
        Source::Bytes(b) => Box::new(std::io::Cursor::new(b.clone())),
        Source::Image(d) => Box::new(ImageReader { d: d.clone(), size, pos: 0 }),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    Dir(Vec<Node>),
    File { source: Source, size: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub name: String,
    /// Unix seconds.
    pub mtime: i64,
    pub kind: Kind,
}

fn mtime_of(m: &std::fs::Metadata) -> i64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Node {
    pub fn dir(name: &str, mtime: i64, children: Vec<Node>) -> Node {
        Node { name: name.into(), mtime, kind: Kind::Dir(children) }
    }

    pub fn bytes(name: &str, mtime: i64, data: Vec<u8>) -> Node {
        let size = data.len() as u64;
        Node { name: name.into(), mtime, kind: Kind::File { source: Source::Bytes(data), size } }
    }

    /// A file or a whole folder from disk. Symbolic links to folders are not
    /// followed (no loops); links to files are.
    pub fn from_path(path: &Path) -> Result<Node> {
        let meta = std::fs::metadata(path)?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if meta.is_file() {
            return Ok(Node {
                name,
                mtime: mtime_of(&meta),
                kind: Kind::File { source: Source::File(path.to_path_buf()), size: meta.len() },
            });
        }
        let mut kids = Vec::new();
        let mut entries: Vec<_> = std::fs::read_dir(path)?.collect::<std::io::Result<_>>()?;
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            if e.file_type()?.is_symlink() && e.path().is_dir() {
                continue;
            }
            kids.push(Node::from_path(&e.path())?);
        }
        Ok(Node::dir(&name, mtime_of(&meta), kids))
    }

    pub fn is_dir(&self) -> bool {
        matches!(self.kind, Kind::Dir(_))
    }
}

/// An El Torito boot image, stored outside the directory trees.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootImage {
    pub source: Source,
    pub size: u64,
    /// 512-byte sectors the BIOS loads (no emulation). Default 4.
    pub load_sectors: Option<u16>,
    /// Patch an ISOLINUX/GRUB-style boot information table into bytes 8–63.
    pub info_table: bool,
}

impl BootImage {
    pub fn from_path(path: &Path) -> Result<BootImage> {
        let size = std::fs::metadata(path)?.len();
        if size == 0 {
            return bad(format!("boot image {} is empty", path.display()));
        }
        Ok(BootImage { source: Source::File(path.to_path_buf()), size, load_sectors: None, info_table: false })
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    pub label: String,
    pub joliet: bool,
    pub udf: bool,
    pub bios: Option<BootImage>,
    pub efi: Option<BootImage>,
    /// Volume creation time (Unix seconds).
    pub created: i64,
}

impl Default for Options {
    fn default() -> Self {
        let now =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        Options { label: "CDROM".into(), joliet: true, udf: true, bios: None, efi: None, created: now }
    }
}

/// Flattened node with every layout decision.
struct P {
    parent: usize,
    children: Vec<usize>,
    dir: bool,
    size: u64,
    source: Option<Source>,
    mtime: i64,
    iso: String,
    jol: String,
    udf: String,
    data_lba: u64,
    iso_dir: (u64, u64),
    jol_dir: (u64, u64),
    udf_fe: u64,
    udf_dir: (u64, u64),
}

pub struct Summary {
    pub sectors: u64,
    pub files: u64,
    pub dirs: u64,
}

pub use xiso::build_xiso;

struct Plan {
    nodes: Vec<P>,
    opts: Options,
    vd_bootrec: Option<u64>,
    vd_joliet: Option<u64>,
    vd_term: u64,
    vrs: Option<u64>,
    iso_pt: (u64, u64, u64), // L lba, M lba, size in bytes
    jol_pt: (u64, u64, u64),
    iso_dirs: Vec<usize>,
    jol_dirs: Vec<usize>,
    catalog: Option<u64>,
    bios_lba: u64,
    efi_lba: u64,
    data_order: Vec<usize>,
    part_start: u64,
    fe_start: u64,
    trailing_avdp: Option<u64>,
    total: u64,
}

const UDF_PART_START: u64 = 257;

fn flatten(root: &Node) -> Result<Vec<P>> {
    let mut nodes = Vec::new();
    let mk = |parent, dir, size, source, mtime| P {
        parent,
        children: Vec::new(),
        dir,
        size,
        source,
        mtime,
        iso: String::new(),
        jol: String::new(),
        udf: String::new(),
        data_lba: 0,
        iso_dir: (0, 0),
        jol_dir: (0, 0),
        udf_fe: 0,
        udf_dir: (0, 0),
    };
    nodes.push(mk(0, true, 0, None, root.mtime));
    let mut stack = vec![(0usize, root, 0u32)];
    while let Some((idx, n, depth)) = stack.pop() {
        if depth > 64 {
            return bad("folders nested deeper than 64 levels");
        }
        let Kind::Dir(kids) = &n.kind else { continue };
        let (mut si, mut sj, mut su) = (HashSet::new(), HashSet::new(), HashSet::new());
        for k in kids {
            let (dir, size, source) = match &k.kind {
                Kind::Dir(_) => (true, 0, None),
                Kind::File { source, size } => (false, *size, Some(source.clone())),
            };
            let mut p = mk(idx, dir, size, source, k.mtime);
            p.iso = names::iso(&k.name, dir, &mut si);
            p.jol = names::joliet(&k.name, dir, &mut sj);
            p.udf = names::udf(&k.name, dir, &mut su);
            let id = nodes.len();
            nodes.push(p);
            nodes[idx].children.push(id);
            if dir {
                stack.push((id, k, depth + 1));
            }
        }
    }
    Ok(nodes)
}

/// Directories in path-table order: breadth first, siblings by name.
fn bfs(nodes: &[P], key: impl Fn(&P) -> Vec<u16>) -> Vec<usize> {
    let mut order = vec![0usize];
    let mut i = 0;
    while i < order.len() {
        let mut kids: Vec<usize> = nodes[order[i]].children.iter().copied().filter(|&c| nodes[c].dir).collect();
        kids.sort_by_key(|&c| key(&nodes[c]));
        order.extend(kids);
        i += 1;
    }
    order
}

fn iso_ident(p: &P, joliet: bool) -> Vec<u8> {
    let name = if joliet { &p.jol } else { &p.iso };
    let full = if p.dir { name.clone() } else { format!("{name};1") };
    if joliet {
        full.encode_utf16().flat_map(|u| u.to_be_bytes()).collect()
    } else {
        full.into_bytes()
    }
}

fn record_len(ident_len: usize) -> usize {
    let l = 33 + ident_len;
    l + (l & 1)
}

/// Sorted child list for a directory in one tree.
fn sorted_children(nodes: &[P], d: usize, joliet: bool) -> Vec<usize> {
    let mut c = nodes[d].children.clone();
    c.sort_by_key(|&k| iso_ident(&nodes[k], joliet));
    c
}

fn extents_of(size: u64, max: u64) -> u64 {
    size.div_ceil(max).max(1)
}

/// Bytes a directory occupies (whole sectors; records never cross sectors).
fn dir_size(nodes: &[P], d: usize, joliet: bool) -> u64 {
    let mut used = 0usize;
    let mut sectors = 1u64;
    let mut add = |len: usize| {
        if used + len > SECTOR {
            sectors += 1;
            used = 0;
        }
        used += len;
    };
    add(34);
    add(34);
    for c in sorted_children(nodes, d, joliet) {
        let len = record_len(iso_ident(&nodes[c], joliet).len());
        let n = if nodes[c].dir { 1 } else { extents_of(nodes[c].size, ISO_MAX_EXTENT) };
        for _ in 0..n {
            add(len);
        }
    }
    sectors * S
}

fn path_table_size(nodes: &[P], order: &[usize], joliet: bool) -> u64 {
    order
        .iter()
        .map(|&d| {
            let l = if d == 0 { 1 } else { iso_ident(&nodes[d], joliet).len() };
            (8 + l + (l & 1)) as u64
        })
        .sum()
}

fn fid_len(name: &str) -> u64 {
    let l = if name.is_empty() { 0 } else { udfw::dchars(name).len() };
    ((38 + l + 3) & !3) as u64
}

fn plan(root: &Node, opts: Options) -> Result<Plan> {
    if !root.is_dir() {
        return bad("the image root must be a folder");
    }
    let mut nodes = flatten(root)?;
    // Volume descriptors from sector 16.
    let mut cur = 17u64;
    let vd_bootrec = (opts.bios.is_some() || opts.efi.is_some()).then(|| {
        cur += 1;
        cur - 1
    });
    let vd_joliet = opts.joliet.then(|| {
        cur += 1;
        cur - 1
    });
    let vd_term = cur;
    cur += 1;
    let vrs = opts.udf.then(|| {
        cur += 3;
        cur - 3
    });
    let (part_start, fe_start) = if opts.udf {
        // VDS 32, reserve 48, LVID 64-65, anchor 256, partition from 257:
        // FSD at lbn 0, its terminator at lbn 1, then one file entry per node.
        cur = UDF_PART_START + 2;
        let fe_start = cur;
        cur += nodes.len() as u64;
        (UDF_PART_START, fe_start)
    } else {
        (0, 0)
    };
    if opts.udf {
        // Directory FID streams.
        for d in 0..nodes.len() {
            if !nodes[d].dir {
                continue;
            }
            let len = fid_len("") + nodes[d].children.iter().map(|&c| fid_len(&nodes[c].udf)).sum::<u64>();
            nodes[d].udf_dir = (cur, len);
            cur += len.div_ceil(S);
        }
        for (i, n) in nodes.iter_mut().enumerate() {
            n.udf_fe = fe_start + i as u64;
        }
    }
    let iso_dirs = bfs(&nodes, |p| iso_ident(p, false).iter().map(|&b| b as u16).collect());
    let pt = path_table_size(&nodes, &iso_dirs, false);
    let iso_pt = (cur, cur + pt.div_ceil(S), pt);
    cur += 2 * pt.div_ceil(S);
    for &d in &iso_dirs {
        let sz = dir_size(&nodes, d, false);
        nodes[d].iso_dir = (cur, sz);
        cur += sz / S;
    }
    let mut jol_dirs = Vec::new();
    let mut jol_pt = (0, 0, 0);
    if opts.joliet {
        jol_dirs = bfs(&nodes, |p| p.jol.encode_utf16().collect());
        let pt = path_table_size(&nodes, &jol_dirs, true);
        jol_pt = (cur, cur + pt.div_ceil(S), pt);
        cur += 2 * pt.div_ceil(S);
        for &d in &jol_dirs {
            let sz = dir_size(&nodes, d, true);
            nodes[d].jol_dir = (cur, sz);
            cur += sz / S;
        }
    }
    let mut catalog = None;
    let (mut bios_lba, mut efi_lba) = (0, 0);
    if vd_bootrec.is_some() {
        catalog = Some(cur);
        cur += 1;
        if let Some(b) = &opts.bios {
            bios_lba = cur;
            cur += b.size.div_ceil(S);
        }
        if let Some(e) = &opts.efi {
            efi_lba = cur;
            cur += e.size.div_ceil(S);
        }
    }
    // File data in directory order, so related files sit together.
    let mut data_order = Vec::new();
    let mut stack = vec![0usize];
    while let Some(d) = stack.pop() {
        let kids = sorted_children(&nodes, d, false);
        for &k in &kids {
            if !nodes[k].dir && nodes[k].size > 0 {
                data_order.push(k);
            }
        }
        for &k in kids.iter().rev() {
            if nodes[k].dir {
                stack.push(k);
            }
        }
    }
    for &f in &data_order {
        nodes[f].data_lba = cur;
        cur += nodes[f].size.div_ceil(S);
    }
    let trailing_avdp = opts.udf.then(|| {
        cur += 1;
        cur - 1
    });
    if cur > u32::MAX as u64 {
        return bad("image larger than 8 TB");
    }
    Ok(Plan {
        nodes,
        opts,
        vd_bootrec,
        vd_joliet,
        vd_term,
        vrs,
        iso_pt,
        jol_pt,
        iso_dirs,
        jol_dirs,
        catalog,
        bios_lba,
        efi_lba,
        data_order,
        part_start,
        fe_start,
        trailing_avdp,
        total: cur,
    })
}

fn both16(d: &mut [u8], o: usize, v: u16) {
    d[o..o + 2].copy_from_slice(&v.to_le_bytes());
    d[o + 2..o + 4].copy_from_slice(&v.to_be_bytes());
}

fn both32(d: &mut [u8], o: usize, v: u32) {
    d[o..o + 4].copy_from_slice(&v.to_le_bytes());
    d[o + 4..o + 8].copy_from_slice(&v.to_be_bytes());
}

fn iso_date7(d: &mut [u8], t: i64) {
    let (y, mo, da, h, mi, s) = crate::fs::civil_from_unix(t);
    d[0] = (y - 1900).clamp(0, 255) as u8;
    d[1] = mo as u8;
    d[2] = da as u8;
    d[3] = h as u8;
    d[4] = mi as u8;
    d[5] = s as u8;
    d[6] = 0;
}

fn iso_date17(d: &mut [u8], t: i64) {
    let (y, mo, da, h, mi, s) = crate::fs::civil_from_unix(t);
    let txt = format!("{y:04}{mo:02}{da:02}{h:02}{mi:02}{s:02}00");
    d[..16].copy_from_slice(txt.as_bytes());
    d[16] = 0;
}

fn dir_record(rec: &mut Vec<u8>, ident: &[u8], lba: u64, size: u64, mtime: i64, flags: u8) {
    let len = record_len(ident.len());
    let start = rec.len();
    rec.resize(start + len, 0);
    let r = &mut rec[start..];
    r[0] = len as u8;
    both32(r, 2, lba as u32);
    both32(r, 10, size as u32);
    iso_date7(&mut r[18..25], mtime);
    r[25] = flags;
    both16(r, 28, 1);
    r[32] = ident.len() as u8;
    r[33..33 + ident.len()].copy_from_slice(ident);
}

impl Plan {
    fn label_iso(&self) -> String {
        let l: String = self
            .opts
            .label
            .chars()
            .map(|c| c.to_ascii_uppercase())
            .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
            .take(32)
            .collect();
        if l.is_empty() {
            "CDROM".into()
        } else {
            l
        }
    }

    fn volume_descriptor(&self, joliet: bool) -> Vec<u8> {
        let mut d = vec![0u8; SECTOR];
        d[0] = if joliet { 2 } else { 1 };
        d[1..6].copy_from_slice(b"CD001");
        d[6] = 1;
        let pad = |d: &mut [u8], o: usize, n: usize, s: &str| {
            if joliet {
                let u: Vec<u8> = s.encode_utf16().flat_map(|u| u.to_be_bytes()).take(n).collect();
                for (i, ch) in d[o..o + n].chunks_mut(2).enumerate() {
                    let (a, b) = (u.get(i * 2).copied(), u.get(i * 2 + 1).copied());
                    ch[0] = a.unwrap_or(0);
                    if ch.len() > 1 {
                        ch[1] = b.unwrap_or(if a.is_none() { 0x20 } else { 0 });
                    }
                }
            } else {
                d[o..o + n].fill(b' ');
                let b = s.as_bytes();
                d[o..o + b.len().min(n)].copy_from_slice(&b[..b.len().min(n)]);
            }
        };
        pad(&mut d, 8, 32, "");
        let label = if joliet { self.opts.label.chars().take(16).collect() } else { self.label_iso() };
        pad(&mut d, 40, 32, &label);
        both32(&mut d, 80, self.total as u32);
        if joliet {
            d[88..91].copy_from_slice(&[0x25, 0x2F, 0x45]);
        }
        both16(&mut d, 120, 1);
        both16(&mut d, 124, 1);
        both16(&mut d, 128, SECTOR as u16);
        let (pt, root) =
            if joliet { (self.jol_pt, self.nodes[0].jol_dir) } else { (self.iso_pt, self.nodes[0].iso_dir) };
        both32(&mut d, 132, pt.2 as u32);
        d[140..144].copy_from_slice(&(pt.0 as u32).to_le_bytes());
        d[148..152].copy_from_slice(&(pt.1 as u32).to_be_bytes());
        let mut rr = Vec::new();
        dir_record(&mut rr, &[0], root.0, root.1, self.nodes[0].mtime, 2);
        d[156..190].copy_from_slice(&rr);
        for (o, n) in [(190, 128), (318, 128), (446, 128)] {
            pad(&mut d, o, n, "");
        }
        pad(&mut d, 574, 128, &format!("ISOFOUNDRY {}", env!("CARGO_PKG_VERSION")));
        for (o, n) in [(702, 37), (739, 37), (776, 37)] {
            pad(&mut d, o, n, "");
        }
        iso_date17(&mut d[813..830], self.opts.created);
        iso_date17(&mut d[830..847], self.opts.created);
        d[847..863].fill(b'0');
        iso_date17(&mut d[864..881], self.opts.created);
        d[881] = 1;
        d
    }

    fn path_table(&self, joliet: bool, big_endian: bool) -> Vec<u8> {
        let order = if joliet { &self.jol_dirs } else { &self.iso_dirs };
        let num_of = |d: usize| order.iter().position(|&x| x == d).unwrap() as u16 + 1;
        let mut t = Vec::new();
        for &d in order {
            let ident = if d == 0 { vec![0u8] } else { iso_ident(&self.nodes[d], joliet) };
            let lba = if joliet { self.nodes[d].jol_dir.0 } else { self.nodes[d].iso_dir.0 } as u32;
            let parent = if d == 0 { 1 } else { num_of(self.nodes[d].parent) };
            t.push(ident.len() as u8);
            t.push(0);
            t.extend_from_slice(&if big_endian { lba.to_be_bytes() } else { lba.to_le_bytes() });
            t.extend_from_slice(&if big_endian { parent.to_be_bytes() } else { parent.to_le_bytes() });
            t.extend_from_slice(&ident);
            if ident.len() % 2 == 1 {
                t.push(0);
            }
        }
        t
    }

    fn directory(&self, d: usize, joliet: bool) -> Vec<u8> {
        let n = &self.nodes;
        let loc = |i: usize| if joliet { n[i].jol_dir } else { n[i].iso_dir };
        let (_, size) = loc(d);
        let mut out = vec![0u8; size as usize];
        let mut sec = 0usize;
        let mut used = 0usize;
        let mut push = |rec: &[u8]| {
            if used + rec.len() > SECTOR {
                sec += 1;
                used = 0;
            }
            let o = sec * SECTOR + used;
            out[o..o + rec.len()].copy_from_slice(rec);
            used += rec.len();
        };
        let mut r = Vec::new();
        dir_record(&mut r, &[0], loc(d).0, loc(d).1, n[d].mtime, 2);
        push(&r);
        r.clear();
        dir_record(&mut r, &[1], loc(n[d].parent).0, loc(n[d].parent).1, n[n[d].parent].mtime, 2);
        push(&r);
        for c in sorted_children(n, d, joliet) {
            let ident = iso_ident(&n[c], joliet);
            if n[c].dir {
                r.clear();
                dir_record(&mut r, &ident, loc(c).0, loc(c).1, n[c].mtime, 2);
                push(&r);
                continue;
            }
            let parts = extents_of(n[c].size, ISO_MAX_EXTENT);
            for i in 0..parts {
                let off = i * ISO_MAX_EXTENT;
                let len = (n[c].size - off.min(n[c].size)).min(ISO_MAX_EXTENT);
                let lba = if n[c].size == 0 { 0 } else { n[c].data_lba + off / S };
                r.clear();
                dir_record(&mut r, &ident, lba, len, n[c].mtime, if i + 1 < parts { 0x80 } else { 0 });
                push(&r);
            }
        }
        out
    }

    fn boot_catalog(&self) -> Vec<u8> {
        let mut c = vec![0u8; SECTOR];
        let bios = self.opts.bios.as_ref();
        let efi = self.opts.efi.as_ref();
        c[0] = 1;
        c[1] = if bios.is_some() { 0 } else { 0xEF };
        c[4..14].copy_from_slice(b"ISOFOUNDRY");
        c[30] = 0x55;
        c[31] = 0xAA;
        let sum = (0..16).fold(0u16, |s, i| s.wrapping_add(u16::from_le_bytes([c[i * 2], c[i * 2 + 1]])));
        c[28..30].copy_from_slice(&0u16.wrapping_sub(sum).to_le_bytes());
        let entry = |e: &mut [u8], count: u16, lba: u64| {
            e[0] = 0x88;
            e[6..8].copy_from_slice(&count.to_le_bytes());
            e[8..12].copy_from_slice(&(lba as u32).to_le_bytes());
        };
        let efi_count = |b: &BootImage| b.size.div_ceil(512).min(0xFFFF) as u16;
        match (bios, efi) {
            (Some(b), e) => {
                entry(&mut c[32..64], b.load_sectors.unwrap_or(4), self.bios_lba);
                if let Some(e) = e {
                    c[64] = 0x91;
                    c[65] = 0xEF;
                    c[66..68].copy_from_slice(&1u16.to_le_bytes());
                    entry(&mut c[96..128], efi_count(e), self.efi_lba);
                }
            }
            (None, Some(e)) => entry(&mut c[32..64], efi_count(e), self.efi_lba),
            (None, None) => {}
        }
        c
    }

    fn boot_record_vd(&self) -> Vec<u8> {
        let mut d = vec![0u8; SECTOR];
        d[1..6].copy_from_slice(b"CD001");
        d[6] = 1;
        d[7..30].copy_from_slice(b"EL TORITO SPECIFICATION");
        d[71..75].copy_from_slice(&(self.catalog.unwrap_or(0) as u32).to_le_bytes());
        d
    }

    // ---- UDF ----

    fn lbn(&self, abs: u64) -> u32 {
        (abs - self.part_start) as u32
    }

    fn unique(&self, i: usize) -> u64 {
        if i == 0 {
            0
        } else {
            15 + i as u64
        }
    }

    fn part_len(&self) -> u32 {
        (self.trailing_avdp.unwrap_or(self.total) - self.part_start) as u32
    }

    fn udf_vds(&self, base: u64) -> Vec<Vec<u8>> {
        let t = self.opts.created;
        let label = &self.opts.label;
        let mut out = Vec::new();
        // Primary volume descriptor.
        let mut d = vec![0u8; 512];
        put32(&mut d, 16, 0);
        put32(&mut d, 20, 0);
        udfw::dstring(&mut d[24..56], label);
        put16(&mut d, 56, 1);
        put16(&mut d, 58, 1);
        put16(&mut d, 60, 2);
        put16(&mut d, 62, 2);
        put32(&mut d, 64, 1);
        put32(&mut d, 68, 1);
        udfw::dstring(&mut d[72..200], &format!("{:016X}{label}", t as u64));
        udfw::charspec(&mut d[200..264]);
        udfw::charspec(&mut d[264..328]);
        udfw::regid(&mut d[344..376], "*IsoFoundry", &[]);
        udfw::timestamp(&mut d[376..388], t);
        udfw::impl_id(&mut d[388..420]);
        udfw::tag(&mut d, 1, base as u32);
        out.push(d);
        // Implementation use volume descriptor.
        let mut d = vec![0u8; 512];
        put32(&mut d, 16, 1);
        udfw::regid(&mut d[20..52], "*UDF LV Info", &udfw::udf_suffix());
        udfw::charspec(&mut d[52..116]);
        udfw::dstring(&mut d[116..244], label);
        udfw::impl_id(&mut d[352..384]);
        udfw::tag(&mut d, 4, base as u32 + 1);
        out.push(d);
        // Partition descriptor.
        let mut d = vec![0u8; 512];
        put32(&mut d, 16, 2);
        put16(&mut d, 20, 1);
        put16(&mut d, 22, 0);
        udfw::regid(&mut d[24..56], "+NSR02", &[]);
        put32(&mut d, 184, 1);
        put32(&mut d, 188, self.part_start as u32);
        put32(&mut d, 192, self.part_len());
        udfw::impl_id(&mut d[196..228]);
        udfw::tag(&mut d, 5, base as u32 + 2);
        out.push(d);
        // Logical volume descriptor.
        let mut d = vec![0u8; 446];
        put32(&mut d, 16, 3);
        udfw::charspec(&mut d[20..84]);
        udfw::dstring(&mut d[84..212], label);
        put32(&mut d, 212, SECTOR as u32);
        udfw::regid(&mut d[216..248], "*OSTA UDF Compliant", &udfw::udf_suffix());
        udfw::long_ad(&mut d[248..264], SECTOR as u32, 0, 0);
        put32(&mut d, 264, 6);
        put32(&mut d, 268, 1);
        udfw::impl_id(&mut d[272..304]);
        put32(&mut d, 432, 2 * SECTOR as u32);
        put32(&mut d, 436, 64);
        d[440] = 1;
        d[441] = 6;
        put16(&mut d, 442, 1);
        put16(&mut d, 444, 0);
        udfw::tag(&mut d, 6, base as u32 + 3);
        out.push(d);
        // Unallocated space descriptor (none: the disc is read-only).
        let mut d = vec![0u8; 24];
        put32(&mut d, 16, 4);
        udfw::tag(&mut d, 7, base as u32 + 4);
        out.push(d);
        let mut d = vec![0u8; 512];
        udfw::tag(&mut d, 8, base as u32 + 5);
        out.push(d);
        out
    }

    fn udf_lvid(&self) -> Vec<u8> {
        let mut d = vec![0u8; 134];
        udfw::timestamp(&mut d[16..28], self.opts.created);
        put32(&mut d, 28, 1);
        put64(&mut d, 40, self.unique(self.nodes.len()));
        put32(&mut d, 72, 1);
        put32(&mut d, 76, 46);
        put32(&mut d, 80, 0);
        put32(&mut d, 84, self.part_len());
        udfw::impl_id(&mut d[88..120]);
        let files = self.nodes.iter().filter(|n| !n.dir).count() as u32;
        let dirs = self.nodes.iter().filter(|n| n.dir).count() as u32;
        put32(&mut d, 120, files);
        put32(&mut d, 124, dirs);
        put16(&mut d, 128, udfw::REVISION);
        put16(&mut d, 130, udfw::REVISION);
        put16(&mut d, 132, udfw::REVISION);
        udfw::tag(&mut d, 9, 64);
        d
    }

    fn udf_avdp(&self, at: u64) -> Vec<u8> {
        let mut d = vec![0u8; 512];
        put32(&mut d, 16, 16 * SECTOR as u32);
        put32(&mut d, 20, 32);
        put32(&mut d, 24, 16 * SECTOR as u32);
        put32(&mut d, 28, 48);
        udfw::tag(&mut d, 2, at as u32);
        d
    }

    fn udf_fsd(&self) -> Vec<u8> {
        let mut d = vec![0u8; 512];
        udfw::timestamp(&mut d[16..28], self.opts.created);
        put16(&mut d, 28, 3);
        put16(&mut d, 30, 3);
        put32(&mut d, 32, 1);
        put32(&mut d, 36, 1);
        udfw::charspec(&mut d[48..112]);
        udfw::dstring(&mut d[112..240], &self.opts.label);
        udfw::charspec(&mut d[240..304]);
        udfw::dstring(&mut d[304..336], &self.opts.label);
        udfw::long_ad(&mut d[400..416], SECTOR as u32, self.lbn(self.nodes[0].udf_fe), 0);
        udfw::regid(&mut d[416..448], "*OSTA UDF Compliant", &udfw::udf_suffix());
        udfw::tag(&mut d, 256, 0);
        d
    }

    fn udf_fe(&self, i: usize) -> Vec<u8> {
        let n = &self.nodes[i];
        let mut ads = Vec::new();
        let (len, data_lba) = if n.dir { (n.udf_dir.1, n.udf_dir.0) } else { (n.size, n.data_lba) };
        let mut off = 0u64;
        while off < len {
            let l = (len - off).min(UDF_MAX_EXTENT);
            let mut ad = [0u8; 8];
            put32(&mut ad, 0, l as u32);
            put32(&mut ad, 4, self.lbn(data_lba + off / S));
            ads.extend_from_slice(&ad);
            off += l;
        }
        let mut d = vec![0u8; 176 + ads.len()];
        // ICB tag: strategy 4, max entries 1, file type, flags = short_ad.
        put16(&mut d, 16 + 4, 4);
        put16(&mut d, 16 + 8, 1);
        d[16 + 11] = if n.dir { 4 } else { 5 };
        put16(&mut d, 16 + 18, 0);
        put32(&mut d, 36, u32::MAX);
        put32(&mut d, 40, u32::MAX);
        put32(&mut d, 44, 0x14A5);
        let links = if n.dir { 1 + n.children.iter().filter(|&&c| self.nodes[c].dir).count() as u16 } else { 1 };
        put16(&mut d, 48, links);
        put64(&mut d, 56, len);
        put64(&mut d, 64, len.div_ceil(S));
        for o in [72, 84, 96] {
            udfw::timestamp(&mut d[o..o + 12], n.mtime);
        }
        put32(&mut d, 108, 1);
        udfw::impl_id(&mut d[128..160]);
        put64(&mut d, 160, self.unique(i));
        put32(&mut d, 168, 0);
        put32(&mut d, 172, ads.len() as u32);
        d[176..].copy_from_slice(&ads);
        udfw::tag(&mut d, 261, self.lbn(n.udf_fe));
        d
    }

    fn udf_fids(&self, dir: usize) -> Vec<u8> {
        let n = &self.nodes;
        let (start, len) = n[dir].udf_dir;
        let mut out = Vec::with_capacity(len.div_ceil(S) as usize * SECTOR);
        let one = |name: &str, target: usize, chars: u8, out: &mut Vec<u8>| {
            let id = if name.is_empty() { Vec::new() } else { udfw::dchars(name) };
            let total = fid_len(name) as usize;
            let mut f = vec![0u8; total];
            put16(&mut f, 16, 1);
            f[18] = chars;
            f[19] = id.len() as u8;
            udfw::long_ad(&mut f[20..36], SECTOR as u32, self.lbn(n[target].udf_fe), self.unique(target) as u32);
            put16(&mut f, 36, 0);
            f[38..38 + id.len()].copy_from_slice(&id);
            let loc = self.lbn(start + out.len() as u64 / S);
            udfw::tag(&mut f, 257, loc);
            out.extend_from_slice(&f);
        };
        let parent = if dir == 0 { 0 } else { n[dir].parent };
        one("", parent, 0x0A, &mut out);
        let mut kids = n[dir].children.clone();
        kids.sort_by(|&a, &b| n[a].udf.cmp(&n[b].udf));
        for k in kids {
            one(&n[k].udf, k, if n[k].dir { 0x02 } else { 0 }, &mut out);
        }
        debug_assert_eq!(out.len() as u64, len);
        out.resize(len.div_ceil(S) as usize * SECTOR, 0);
        out
    }
}

struct Out<W: Write> {
    w: W,
    at: u64,
    total: u64,
    progress: Box<dyn FnMut(u64, u64) -> bool>,
}

impl<W: Write> Out<W> {
    /// Writes `data` at sector `lba` (zero-filling any gap), padded to a sector.
    fn put(&mut self, lba: u64, data: &[u8]) -> Result<()> {
        if lba < self.at {
            return bad(format!("internal layout error: sector {lba} written after {}", self.at));
        }
        self.zero(lba)?;
        self.w.write_all(data)?;
        let pad = (S - data.len() as u64 % S) % S;
        self.w.write_all(&vec![0u8; pad as usize])?;
        self.at += (data.len() as u64 + pad) / S;
        self.tick()
    }

    fn zero(&mut self, to: u64) -> Result<()> {
        let z = [0u8; SECTOR];
        while self.at < to {
            self.w.write_all(&z)?;
            self.at += 1;
        }
        Ok(())
    }

    fn tick(&mut self) -> Result<()> {
        if !(self.progress)(self.at * S, self.total * S) {
            return Err(Error::Cancelled);
        }
        Ok(())
    }
}

fn copy_source(out: &mut Out<impl Write>, lba: u64, src: &Source, size: u64, patch: Option<[u8; 56]>) -> Result<()> {
    out.zero(lba)?;
    let mut r = reader(src, size)?;
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    let mut first = true;
    while done < size {
        let want = ((size - done) as usize).min(buf.len());
        let mut n = 0;
        while n < want {
            let k = r.read(&mut buf[n..want])?;
            if k == 0 {
                break;
            }
            n += k;
        }
        if n < want {
            let what = match src {
                Source::File(p) => p.display().to_string(),
                Source::Bytes(_) | Source::Image(_) => "data".into(),
            };
            return bad(format!("{what} got shorter while the image was being built"));
        }
        if first {
            if let Some(p) = patch {
                let end = 64.min(n);
                buf[8..end].copy_from_slice(&p[..end - 8]);
            }
            first = false;
        }
        out.w.write_all(&buf[..n])?;
        done += n as u64;
        out.at += n as u64 / S;
        if n as u64 % S != 0 {
            let pad = S - n as u64 % S;
            out.w.write_all(&vec![0u8; pad as usize])?;
            out.at += 1;
        }
        out.tick()?;
    }
    Ok(())
}

/// Boot information table (as mkisofs -boot-info-table): PVD lba, file lba,
/// file length, 32-bit sum of the file from byte 64.
fn info_table(src: &Source, size: u64, lba: u64) -> Result<[u8; 56]> {
    if size > 64 << 20 {
        return bad("boot image too large for a boot information table");
    }
    let mut data = Vec::with_capacity(size as usize);
    reader(src, size)?.take(size).read_to_end(&mut data)?;
    let sum = data.get(64..).unwrap_or(&[]).chunks(4).fold(0u32, |s, c| {
        let mut w = [0u8; 4];
        w[..c.len()].copy_from_slice(c);
        s.wrapping_add(u32::from_le_bytes(w))
    });
    let mut t = [0u8; 56];
    t[0..4].copy_from_slice(&16u32.to_le_bytes());
    t[4..8].copy_from_slice(&(lba as u32).to_le_bytes());
    t[8..12].copy_from_slice(&(size as u32).to_le_bytes());
    t[12..16].copy_from_slice(&sum.to_le_bytes());
    Ok(t)
}

/// Writes the image to `dest` (via `dest.part`, renamed when complete).
/// `progress(done, total)` returns false to cancel.
pub fn build(
    root: &Node,
    opts: Options,
    dest: &Path,
    progress: impl FnMut(u64, u64) -> bool + 'static,
) -> Result<Summary> {
    let p = plan(root, opts)?;
    let mut part = dest.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let file = File::create(&part)?;
    let result = (|| -> Result<()> {
        let mut o =
            Out { w: BufWriter::with_capacity(1 << 20, &file), at: 0, total: p.total, progress: Box::new(progress) };
        o.put(16, &p.volume_descriptor(false))?;
        if let Some(l) = p.vd_bootrec {
            o.put(l, &p.boot_record_vd())?;
        }
        if let Some(l) = p.vd_joliet {
            o.put(l, &p.volume_descriptor(true))?;
        }
        let mut term = vec![0u8; SECTOR];
        term[0] = 255;
        term[1..6].copy_from_slice(b"CD001");
        term[6] = 1;
        o.put(p.vd_term, &term)?;
        if let Some(l) = p.vrs {
            for (i, id) in [b"BEA01", b"NSR02", b"TEA01"].iter().enumerate() {
                let mut v = vec![0u8; SECTOR];
                v[1..6].copy_from_slice(*id);
                v[6] = 1;
                o.put(l + i as u64, &v)?;
            }
            for base in [32u64, 48] {
                for (i, d) in p.udf_vds(base).iter().enumerate() {
                    o.put(base + i as u64, d)?;
                }
            }
            o.put(64, &p.udf_lvid())?;
            let mut td = vec![0u8; 512];
            udfw::tag(&mut td, 8, 65);
            o.put(65, &td)?;
            o.put(256, &p.udf_avdp(256))?;
            o.put(p.part_start, &p.udf_fsd())?;
            let mut td = vec![0u8; 512];
            udfw::tag(&mut td, 8, 1);
            o.put(p.part_start + 1, &td)?;
            for i in 0..p.nodes.len() {
                o.put(p.fe_start + i as u64, &p.udf_fe(i))?;
            }
            for d in 0..p.nodes.len() {
                if p.nodes[d].dir {
                    o.put(p.nodes[d].udf_dir.0, &p.udf_fids(d))?;
                }
            }
        }
        o.put(p.iso_pt.0, &p.path_table(false, false))?;
        o.put(p.iso_pt.1, &p.path_table(false, true))?;
        for &d in &p.iso_dirs {
            o.put(p.nodes[d].iso_dir.0, &p.directory(d, false))?;
        }
        if p.opts.joliet {
            o.put(p.jol_pt.0, &p.path_table(true, false))?;
            o.put(p.jol_pt.1, &p.path_table(true, true))?;
            for &d in &p.jol_dirs {
                o.put(p.nodes[d].jol_dir.0, &p.directory(d, true))?;
            }
        }
        if let Some(c) = p.catalog {
            o.put(c, &p.boot_catalog())?;
            if let Some(b) = &p.opts.bios {
                let patch = if b.info_table { Some(info_table(&b.source, b.size, p.bios_lba)?) } else { None };
                copy_source(&mut o, p.bios_lba, &b.source, b.size, patch)?;
            }
            if let Some(e) = &p.opts.efi {
                copy_source(&mut o, p.efi_lba, &e.source, e.size, None)?;
            }
        }
        for &f in &p.data_order {
            let n = &p.nodes[f];
            copy_source(&mut o, n.data_lba, n.source.as_ref().expect("file"), n.size, None)?;
        }
        if let Some(l) = p.trailing_avdp {
            o.put(l, &p.udf_avdp(l))?;
        }
        o.zero(p.total)?;
        o.w.flush()?;
        drop(o);
        file.sync_all()?;
        Ok(())
    })();
    drop(file);
    match result {
        Ok(()) => {
            std::fs::rename(&part, dest)?;
            Ok(Summary {
                sectors: p.total,
                files: p.nodes.iter().filter(|n| !n.dir).count() as u64,
                dirs: p.nodes.iter().filter(|n| n.dir).count() as u64,
            })
        }
        Err(e) => {
            let _ = std::fs::remove_file(&part);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_regions_do_not_overlap() {
        let root = Node::dir(
            "",
            0,
            vec![
                Node::bytes("a.txt", 0, vec![1; 10]),
                Node::dir("d", 0, vec![Node::bytes("b.bin", 0, vec![2; 5000]), Node::bytes("e", 0, vec![])]),
            ],
        );
        let o = Options {
            bios: Some(BootImage {
                source: Source::Bytes(vec![0xB; 2048]),
                size: 2048,
                load_sectors: None,
                info_table: true,
            }),
            ..Default::default()
        };
        let p = plan(&root, o).unwrap();
        assert!(p.vrs.unwrap() + 3 <= 32);
        assert!(p.iso_pt.0 > p.fe_start);
        let a = p.nodes.iter().find(|n| n.iso == "A.TXT").unwrap();
        let b = p.nodes.iter().find(|n| n.iso == "B.BIN").unwrap();
        assert!(b.data_lba > a.data_lba && a.data_lba > p.bios_lba);
        assert_eq!(p.trailing_avdp, Some(p.total - 1));
    }
}
