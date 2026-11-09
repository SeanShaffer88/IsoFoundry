//! Image editing: read an image's tree, change it, write a new image.
//! The source is never modified; the result goes to a new file through
//! the normal writer (ISO 9660 + Joliet + UDF bridge), keeping the label
//! and El Torito no-emulation boot entries.

use crate::container;
use crate::eltorito::{Emulation, Platform};
use crate::fs::{Data, Entry, FsKind, Volume};
use crate::write::{self, BootImage, ImageData, Kind, Node, Options, SharedSource, Source, Summary};
use crate::{bad, Error, Result, SECTOR};
use std::cell::RefCell;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

struct TempImage(PathBuf);

impl TempImage {
    fn new(label: &str) -> Result<Self> {
        loop {
            let id = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("isofoundry-{label}-{}-{id}.img", std::process::id()));
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(_) => return Ok(Self(path)),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error.into()),
            }
        }
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn clear(&self) -> Result<()> {
        match fs::remove_file(&self.0) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    fn remove(self) -> Result<()> {
        self.clear()
    }
}

impl Drop for TempImage {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// One change. Paths are inside the image, `/`-separated, case-insensitive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Edit {
    /// Copy a file or folder from disk into folder `dir` of the image
    /// (replacing an item of the same name).
    Add {
        dir: String,
        from: PathBuf,
    },
    /// Create an empty folder (and missing parents).
    MkDir(String),
    Remove(String),
    Rename {
        path: String,
        to: String,
    },
    Label(String),
}

/// An image's tree loaded for editing.
pub struct Project {
    pub root: Node,
    pub opts: Options,
    pub warnings: Vec<String>,
    source_path: PathBuf,
    source_format: container::Format,
    vhd_dynamic: bool,
    fat_layout: Option<crate::fat::FatLayout>,
    _src: SharedSource,
}

fn split(path: &str) -> Vec<&str> {
    path.split(['/', '\\']).filter(|p| !p.is_empty() && *p != ".").collect()
}

fn find<'a>(kids: &'a mut [Node], name: &str) -> Option<&'a mut Node> {
    let i = kids
        .iter()
        .position(|k| k.name == name)
        .or_else(|| kids.iter().position(|k| k.name.eq_ignore_ascii_case(name)))?;
    Some(&mut kids[i])
}

fn children(n: &mut Node) -> Result<&mut Vec<Node>> {
    match &mut n.kind {
        Kind::Dir(k) => Ok(k),
        Kind::File { .. } => bad(format!("{} is not a folder", n.name)),
    }
}

impl Project {
    pub fn open(image: &Path) -> Result<Project> {
        let (source_format, src) = container::open(image)?;
        let vhd_dynamic = if source_format == container::Format::Vhd {
            crate::vhd::VhdFile::open(image)?.is_dynamic()
        } else {
            false
        };
        let mut v = Volume::open(src, None)?;
        if v.fs.kind() == FsKind::Xdvdfs {
            return bad("XDVDFS images are read-only and cannot be edited");
        }
        let mut warnings = Vec::new();
        let fat_layout = if v.fs.kind() == FsKind::Fat {
            warnings.push(
                "FAT edits preserve volume geometry and boot/partition sectors; per-file timestamps and attributes are reset"
                    .into(),
            );
            Some(
                crate::fat::Fat::probe(v.src.as_mut())?
                    .ok_or_else(|| crate::Error::Format("FAT volume disappeared while opening the image".into()))?
                    .layout(),
            )
        } else {
            None
        };
        let label = v.fs.label();
        let joliet = v.found.contains(&FsKind::Joliet) || v.found.contains(&FsKind::Udf);
        let udf = v.found.contains(&FsKind::Udf);
        let boot = v.boot.clone();
        let root_entry = v.root();
        // Load the whole tree first, then share the source with the writer.
        let mut files: Vec<(u64, u64, Data)> = Vec::new();
        let mut tmp = Vec::new();
        let root = load(&mut v, &root_entry, 0, &mut files, &mut tmp)?;
        let shared: SharedSource = Rc::new(RefCell::new(v.src));
        let mut root = attach(root, &mut tmp.into_iter(), &shared);
        if let Some(cat) = &boot {
            // The old catalog shows up as a file in some images; the writer
            // makes a new one.
            drop_at(&mut root, cat.lba);
        }
        let mut opts = Options { label, joliet, udf, ..Options::default() };
        if let Some(cat) = boot {
            for e in cat.entries.iter().filter(|e| e.bootable) {
                if e.emulation != Emulation::NoEmulation {
                    return bad("the image boots with floppy or hard-disk emulation, which can't be rebuilt yet");
                }
                // Prefer the file whose data starts at the boot image's sector.
                let (size, data) = match files.iter().find(|(lba, _, _)| *lba == e.lba as u64) {
                    Some((_, size, d)) => (*size, d.clone()),
                    None => {
                        let size = (e.sector_count.max(1) as u64 * 512).max(SECTOR as u64);
                        warnings.push(format!(
                            "{:?} boot image is not a visible file; kept its first {size} bytes",
                            e.platform
                        ));
                        (size, Data::Extents(vec![crate::fs::Extent { lba: e.lba as u64, len: size, sparse: false }]))
                    }
                };
                let source = Source::Image(ImageData { src: shared.clone(), data: data.clone() });
                let mut head = [0u8; 64];
                crate::fs::read_data(shared.borrow_mut().as_mut(), &data, size, 0, &mut head)?;
                let info_table = u32::from_le_bytes(head[8..12].try_into().unwrap()) == 16
                    && u32::from_le_bytes(head[12..16].try_into().unwrap()) == e.lba;
                let img = BootImage {
                    source,
                    size,
                    load_sectors: (e.platform == Platform::Bios).then_some(e.sector_count.max(1)),
                    info_table,
                };
                match e.platform {
                    Platform::Bios if opts.bios.is_none() => opts.bios = Some(img),
                    Platform::Efi if opts.efi.is_none() => opts.efi = Some(img),
                    p => warnings.push(format!("extra boot entry for {p:?} dropped")),
                }
            }
        }
        Ok(Project {
            root,
            opts,
            warnings,
            source_path: image.to_path_buf(),
            source_format,
            vhd_dynamic,
            fat_layout,
            _src: shared,
        })
    }

    fn dir_mut(&mut self, path: &str, create: bool) -> Result<&mut Vec<Node>> {
        let mut cur = &mut self.root;
        for part in split(path) {
            let kids = children(cur)?;
            if find(kids, part).is_none() {
                if !create {
                    return bad(format!("not found in image: {path}"));
                }
                kids.push(Node { name: part.to_string(), mtime: self.opts.created, kind: Kind::Dir(Vec::new()) });
            }
            cur = find(children(cur)?, part).expect("present");
        }
        children(cur)
    }

    pub fn apply(&mut self, e: &Edit) -> Result<()> {
        match e {
            Edit::Label(l) => self.opts.label = l.clone(),
            Edit::MkDir(p) => {
                self.dir_mut(p, true)?;
            }
            Edit::Add { dir, from } => {
                let n =
                    Node::from_path(from).map_err(|err| crate::Error::Format(format!("{}: {err}", from.display())))?;
                let kids = self.dir_mut(dir, false)?;
                kids.retain(|k| !k.name.eq_ignore_ascii_case(&n.name));
                kids.push(n);
            }
            Edit::Remove(p) => {
                let parts = split(p);
                let Some((last, parent)) = parts.split_last() else { return bad("can't remove the root folder") };
                let kids = self.dir_mut(&parent.join("/"), false)?;
                let before = kids.len();
                let exact = kids.iter().any(|k| k.name == *last);
                kids.retain(|k| if exact { k.name != *last } else { !k.name.eq_ignore_ascii_case(last) });
                if kids.len() == before {
                    return bad(format!("not found in image: {p}"));
                }
            }
            Edit::Rename { path, to } => {
                if to.is_empty() || to.contains(['/', '\\']) {
                    return bad("a new name can't be empty or contain / or \\");
                }
                let parts = split(path);
                let Some((last, parent)) = parts.split_last() else { return bad("can't rename the root folder") };
                let kids = self.dir_mut(&parent.join("/"), false)?;
                if kids.iter().any(|k| k.name.eq_ignore_ascii_case(to) && !k.name.eq_ignore_ascii_case(last)) {
                    return bad(format!("{to} already exists there"));
                }
                match find(kids, last) {
                    Some(n) => n.name = to.clone(),
                    None => return bad(format!("not found in image: {path}")),
                }
            }
        }
        Ok(())
    }

    /// Writes the edited image to `dest` (never the source).
    pub fn save(self, dest: &Path, progress: impl FnMut(u64, u64) -> bool + 'static) -> Result<Summary> {
        if let Some(layout) = self.fat_layout {
            if self.source_format == container::Format::Vhd {
                return self.save_vhd(layout, dest, progress);
            }
            crate::fat_write::build(&self.root, &self.opts.label, layout, &self.source_path, dest, progress)
        } else {
            write::build(&self.root, self.opts, dest, progress)
        }
    }

    fn save_vhd(
        self,
        layout: crate::fat::FatLayout,
        dest: &Path,
        mut progress: impl FnMut(u64, u64) -> bool + 'static,
    ) -> Result<Summary> {
        if dest.exists() {
            return bad(format!("destination already exists: {}", dest.display()));
        }
        let bytes = self._src.borrow().byte_len();
        let total = bytes.saturating_mul(3).max(1);
        let raw_source = TempImage::new("source")?;
        let raw_output = TempImage::new("edited")?;
        let result = (|| -> Result<Summary> {
            {
                let mut src = self._src.borrow_mut();
                let mut file = OpenOptions::new().write(true).truncate(true).open(raw_source.path())?;
                let mut offset = 0u64;
                let mut buffer = vec![0; 1 << 20];
                while offset < bytes {
                    let len = (bytes - offset).min(buffer.len() as u64) as usize;
                    let read_len = len.div_ceil(SECTOR) * SECTOR;
                    src.read_sectors(offset / SECTOR as u64, &mut buffer[..read_len])?;
                    file.write_all(&buffer[..len])?;
                    offset += len as u64;
                    if !progress(offset, total) {
                        return Err(Error::Cancelled);
                    }
                }
                file.sync_all()?;
            }
            raw_output.clear()?;
            let summary = crate::fat_write::build(
                &self.root,
                &self.opts.label,
                layout,
                raw_source.path(),
                raw_output.path(),
                |done, stage_total| {
                    let scaled = if stage_total == 0 {
                        0
                    } else {
                        ((u128::from(done.min(stage_total)) * u128::from(bytes)) / u128::from(stage_total)) as u64
                    };
                    progress(bytes.saturating_add(scaled), total)
                },
            )?;
            let mut raw = crate::source::IsoFile::open(raw_output.path())?;
            let write_progress =
                |done: u64, _stage_total: u64| progress(bytes.saturating_mul(2).saturating_add(done), total);
            if self.vhd_dynamic {
                crate::vhd_write::write_dynamic(&mut raw, dest, write_progress)?;
            } else {
                crate::vhd_write::write_fixed(&mut raw, dest, write_progress)?;
            }
            Ok(summary)
        })();
        let cleanup = raw_source.remove().and_then(|()| raw_output.remove());
        match (result, cleanup) {
            (Err(error), Err(cleanup)) => bad(format!("{error}; additionally, {cleanup}")),
            (Err(error), _) => Err(error),
            (Ok(_), Err(cleanup)) => Err(cleanup),
            (Ok(summary), Ok(())) => Ok(summary),
        }
    }
}

/// Tree with placeholder sources; `tmp` collects each file's data in order.
fn load(
    v: &mut Volume,
    dir: &Entry,
    depth: u32,
    files: &mut Vec<(u64, u64, Data)>,
    tmp: &mut Vec<Data>,
) -> Result<Node> {
    if depth > 64 {
        return bad("folders nested deeper than 64 levels (corrupt image?)");
    }
    let mut kids = v.list(dir)?;
    kids.sort_by(|a, b| a.name.cmp(&b.name));
    let mut out = Vec::new();
    for k in kids {
        if k.dir {
            out.push(load(v, &k, depth + 1, files, tmp)?);
        } else {
            if let Data::Extents(x) = &k.data {
                if let Some(first) = x.first() {
                    files.push((first.lba, k.size, k.data.clone()));
                }
            }
            tmp.push(k.data.clone());
            out.push(Node {
                name: k.name.clone(),
                mtime: k.mtime.unwrap_or(0),
                kind: Kind::File { source: Source::Bytes(Vec::new()), size: k.size },
            });
        }
    }
    Ok(Node { name: dir.name.clone(), mtime: dir.mtime.unwrap_or(0), kind: Kind::Dir(out) })
}

fn attach(mut n: Node, data: &mut impl Iterator<Item = Data>, src: &SharedSource) -> Node {
    n.kind = match n.kind {
        Kind::Dir(kids) => Kind::Dir(kids.into_iter().map(|k| attach(k, data, src)).collect()),
        Kind::File { size, .. } => {
            Kind::File { source: Source::Image(ImageData { src: src.clone(), data: data.next().expect("data") }), size }
        }
    };
    n
}

fn drop_at(n: &mut Node, lba: u64) {
    if let Kind::Dir(kids) = &mut n.kind {
        kids.retain(|k| match &k.kind {
            Kind::File { source: Source::Image(d), .. } => {
                !matches!(&d.data, Data::Extents(x) if x.first().map(|e| e.lba) == Some(lba))
            }
            _ => true,
        });
        for k in kids {
            drop_at(k, lba);
        }
    }
}

/// Applies `edits` to `image` and writes the result to `dest`.
pub fn edit(
    image: &Path,
    dest: &Path,
    edits: &[Edit],
    progress: impl FnMut(u64, u64) -> bool + 'static,
) -> Result<(Summary, Vec<String>)> {
    if dest.exists() && std::fs::canonicalize(image).ok() == std::fs::canonicalize(dest).ok() {
        return bad("save the edited image to a new file, not over the original");
    }
    let mut p = Project::open(image)?;
    for e in edits {
        p.apply(e)?;
    }
    let w = std::mem::take(&mut p.warnings);
    Ok((p.save(dest, progress)?, w))
}
