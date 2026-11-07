//! UDF reading (ECMA-167 / OSTA UDF 1.02–2.60), as used by Windows install
//! media, DVDs and Blu-rays. Physical, sparable (read without remapping)
//! and metadata partitions are supported; virtual (VAT, CD-R multisession
//! UDF) partitions are not.

use crate::fs::{days_from_civil, Data, Entry, Extent, FileSystem, FsKind};
use crate::source::{read_bytes, SectorSource};
use crate::{bad, le16, le32, le64, Result, SECTOR};

const TAG_AVDP: u16 = 2;
const TAG_PD: u16 = 5;
const TAG_LVD: u16 = 6;
const TAG_TD: u16 = 8;
const TAG_FSD: u16 = 256;
const TAG_FID: u16 = 257;
const TAG_AED: u16 = 258;
const TAG_FE: u16 = 261;
const TAG_EFE: u16 = 266;

#[derive(Debug, Clone)]
enum Map {
    Physical {
        part: u16,
    },
    /// Metadata partition (UDF 2.50+): (first metadata block, physical lbn, blocks).
    Metadata {
        part: u16,
        runs: Vec<(u64, u64, u64)>,
    },
}

#[derive(Debug, Clone)]
pub struct Udf {
    label: String,
    revision: u16,
    partitions: Vec<(u16, u64)>,
    maps: Vec<Map>,
    root: Entry,
}

fn tag_ok(b: &[u8], id: u16) -> bool {
    if b.len() < 16 || le16(b, 0) != id {
        return false;
    }
    let sum = b[..16].iter().enumerate().filter(|(i, _)| *i != 4).fold(0u8, |s, (_, v)| s.wrapping_add(*v));
    sum == b[4]
}

/// OSTA compressed Unicode (CS0) d-characters.
fn dchars(b: &[u8]) -> String {
    match b.first() {
        Some(8) | Some(254) => b[1..].iter().map(|&c| c as char).collect(),
        Some(16) | Some(255) => {
            let u: Vec<u16> = b[1..].chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            String::from_utf16_lossy(&u)
        }
        _ => String::new(),
    }
}

/// Fixed-length d-string: the last byte holds the used length.
fn dstring(b: &[u8]) -> String {
    let used = (*b.last().unwrap_or(&0) as usize).min(b.len().saturating_sub(1));
    dchars(&b[..used]).trim_end_matches('\0').to_string()
}

fn timestamp(b: &[u8]) -> Option<i64> {
    let tz_raw = le16(b, 0);
    let year = le16(b, 2) as i16 as i64;
    let (mo, d, h, mi, s) = (b[4] as i64, b[5] as i64, b[6] as i64, b[7] as i64, b[8] as i64);
    if !(1..=12).contains(&mo) || d < 1 || year < 1 {
        return None;
    }
    let mut t = days_from_civil(year, mo, d) * 86_400 + h * 3600 + mi * 60 + s;
    if tz_raw >> 12 == 1 {
        let mut tz = (tz_raw & 0x0FFF) as i64;
        if tz & 0x800 != 0 {
            tz -= 0x1000;
        }
        if tz != -2047 {
            t -= tz * 60;
        }
    }
    Some(t)
}

impl Udf {
    pub fn revision(&self) -> u16 {
        self.revision
    }

    pub fn open(src: &mut dyn SectorSource) -> Result<Udf> {
        let n = src.sectors();
        let mut avdp = None;
        for lba in [256, n.saturating_sub(257), n.saturating_sub(1), 512] {
            if lba == 0 || lba >= n {
                continue;
            }
            let b = read_bytes(src, lba, 0, SECTOR)?;
            if tag_ok(&b, TAG_AVDP) {
                avdp = Some(b);
                break;
            }
        }
        let Some(avdp) = avdp else { return bad("no UDF anchor") };
        let (vds_len, vds_loc) = (le32(&avdp, 16) as u64, le32(&avdp, 20) as u64);
        let count = (vds_len / SECTOR as u64).clamp(1, 64);
        let mut partitions = Vec::new();
        let mut lvd = None;
        let mut label = String::new();
        for i in 0..count {
            let b = read_bytes(src, vds_loc + i, 0, SECTOR)?;
            match le16(&b, 0) {
                1 if tag_ok(&b, 1) => label = dstring(&b[24..56]),
                TAG_PD if tag_ok(&b, TAG_PD) => partitions.push((le16(&b, 22), le32(&b, 188) as u64)),
                TAG_LVD if tag_ok(&b, TAG_LVD) => lvd = Some(b),
                TAG_TD => break,
                _ => {}
            }
        }
        let Some(lvd) = lvd else { return bad("UDF logical volume descriptor missing") };
        if le32(&lvd, 212) as usize != SECTOR {
            return bad(format!("unsupported UDF block size {}", le32(&lvd, 212)));
        }
        let lv_label = dstring(&lvd[84..212]);
        if !lv_label.is_empty() {
            label = lv_label;
        }
        let (fsd_lbn, fsd_part) = (le32(&lvd, 252) as u64, le16(&lvd, 256));
        let map_count = le32(&lvd, 268) as usize;
        let map_len = (le32(&lvd, 264) as usize).min(SECTOR - 440);
        let table = &lvd[440..440 + map_len];
        let mut maps = Vec::new();
        let mut p = 0;
        let mut meta_file = None;
        for _ in 0..map_count {
            if p + 2 > table.len() {
                break;
            }
            let (ty, len) = (table[p], table[p + 1] as usize);
            if len < 6 || p + len > table.len() {
                return bad("bad UDF partition map");
            }
            let m = &table[p..p + len];
            match ty {
                1 => maps.push(Map::Physical { part: le16(m, 4) }),
                2 if len >= 64 => {
                    let id = &m[5..28];
                    let part = le16(m, 38);
                    if id.starts_with(b"*UDF Metadata Partition") {
                        meta_file = Some((maps.len(), part, le32(m, 40) as u64));
                        maps.push(Map::Metadata { part, runs: Vec::new() });
                    } else if id.starts_with(b"*UDF Sparable Partition") {
                        maps.push(Map::Physical { part });
                    } else {
                        return bad(format!(
                            "unsupported UDF partition type {}",
                            String::from_utf8_lossy(id).trim_end_matches('\0')
                        ));
                    }
                }
                _ => return bad("unsupported UDF partition map"),
            }
            p += len;
        }
        let mut udf = Udf { label, revision: 0, partitions, maps, root: placeholder() };
        if let Some((idx, part, lbn)) = meta_file {
            // The metadata file lives in the physical partition.
            let phys = udf.maps.iter().position(|m| matches!(m, Map::Physical { part: p } if *p == part)).unwrap_or(0);
            let fe = udf.read_fe(src, phys as u16, lbn)?;
            let Data::Extents(exts) = fe.data else { return bad("inline UDF metadata file") };
            let start = udf.partition_start(part)?;
            let mut meta_block = 0;
            let mut runs = Vec::new();
            for e in exts {
                let blocks = e.len.div_ceil(SECTOR as u64);
                runs.push((meta_block, e.lba - start, blocks));
                meta_block += blocks;
            }
            udf.maps[idx] = Map::Metadata { part, runs };
        }
        let fsd = read_bytes(src, udf.translate(fsd_part, fsd_lbn)?, 0, SECTOR)?;
        if !tag_ok(&fsd, TAG_FSD) {
            return bad("UDF file set descriptor missing");
        }
        if udf.label.is_empty() {
            udf.label = dstring(&fsd[112..240]);
        }
        let (root_lbn, root_part) = (le32(&fsd, 404) as u64, le16(&fsd, 408));
        udf.revision = le16(&lvd, 216 + 25);
        let mut root = udf.read_fe(src, root_part, root_lbn)?;
        if !root.dir {
            return bad("UDF root is not a directory");
        }
        root.name = String::new();
        udf.root = root;
        Ok(udf)
    }

    fn partition_start(&self, part: u16) -> Result<u64> {
        self.partitions
            .iter()
            .find(|(n, _)| *n == part)
            .map(|(_, s)| *s)
            .ok_or_else(|| crate::Error::Format(format!("UDF partition {part} missing")))
    }

    fn translate(&self, partref: u16, lbn: u64) -> Result<u64> {
        Ok(self.translate_run(partref, lbn, 1)?[0].0)
    }

    /// Absolute (sector, blocks) runs for `blocks` logical blocks.
    fn translate_run(&self, partref: u16, lbn: u64, blocks: u64) -> Result<Vec<(u64, u64)>> {
        let map = self
            .maps
            .get(partref as usize)
            .ok_or_else(|| crate::Error::Format("bad UDF partition reference".into()))?;
        match map {
            Map::Physical { part } => Ok(vec![(self.partition_start(*part)? + lbn, blocks.max(1))]),
            Map::Metadata { part, runs } => {
                let start = self.partition_start(*part)?;
                let mut out = Vec::new();
                let (mut b, mut left) = (lbn, blocks.max(1));
                while left > 0 {
                    let Some(&(m0, phys, n)) = runs.iter().find(|(m0, _, n)| b >= *m0 && b < m0 + n) else {
                        return bad("UDF metadata block outside the metadata file");
                    };
                    let take = (m0 + n - b).min(left);
                    out.push((start + phys + (b - m0), take));
                    b += take;
                    left -= take;
                }
                Ok(out)
            }
        }
    }

    fn read_fe(&self, src: &mut dyn SectorSource, partref: u16, lbn: u64) -> Result<Entry> {
        let abs = self.translate(partref, lbn)?;
        let b = read_bytes(src, abs, 0, SECTOR)?;
        let id = le16(&b, 0);
        let (ea_off, ext) = match id {
            TAG_FE if tag_ok(&b, TAG_FE) => (168, false),
            TAG_EFE if tag_ok(&b, TAG_EFE) => (208, true),
            _ => return bad(format!("expected a UDF file entry at sector {abs}")),
        };
        let file_type = b[27];
        let flags = le16(&b, 34);
        let size = le64(&b, 56);
        let mtime = timestamp(if ext { &b[92..104] } else { &b[84..96] });
        let l_ea = le32(&b, ea_off) as usize;
        let l_ad = le32(&b, ea_off + 4) as usize;
        let ad_start = ea_off + 8 + l_ea;
        if ad_start + l_ad > SECTOR {
            return bad("UDF file entry allocation descriptors overflow the block");
        }
        let ads = &b[ad_start..ad_start + l_ad];
        let data = match flags & 7 {
            3 => Data::Inline(ads.to_vec()),
            0 => Data::Extents(self.parse_ads(src, ads, false, partref, 0)?),
            1 => Data::Extents(self.parse_ads(src, ads, true, partref, 0)?),
            t => return bad(format!("unsupported UDF allocation descriptor type {t}")),
        };
        Ok(Entry { name: String::new(), dir: file_type == 4, size, mtime, hidden: false, data })
    }

    fn parse_ads(
        &self,
        src: &mut dyn SectorSource,
        ads: &[u8],
        long: bool,
        partref: u16,
        depth: u32,
    ) -> Result<Vec<Extent>> {
        if depth > 32 {
            return bad("UDF allocation descriptor chain too long");
        }
        let step = if long { 16 } else { 8 };
        let mut out = Vec::new();
        for ad in ads.chunks_exact(step) {
            let raw = le32(ad, 0);
            let len = (raw & 0x3FFF_FFFF) as u64;
            let kind = raw >> 30;
            if len == 0 {
                break;
            }
            let lbn = le32(ad, 4) as u64;
            let pref = if long { le16(ad, 8) } else { partref };
            match kind {
                0 => {
                    let mut left = len;
                    for (abs, blocks) in self.translate_run(pref, lbn, len.div_ceil(SECTOR as u64))? {
                        let bytes = (blocks * SECTOR as u64).min(left);
                        out.push(Extent { lba: abs, len: bytes, sparse: false });
                        left -= bytes;
                    }
                }
                1 | 2 => out.push(Extent { lba: 0, len, sparse: true }),
                _ => {
                    let aed = read_bytes(src, self.translate(pref, lbn)?, 0, SECTOR)?;
                    if !tag_ok(&aed, TAG_AED) {
                        return bad("broken UDF allocation extent chain");
                    }
                    let l = (le32(&aed, 20) as usize).min(SECTOR - 24);
                    out.extend(self.parse_ads(src, &aed[24..24 + l], long, pref, depth + 1)?);
                    break;
                }
            }
        }
        Ok(out)
    }
}

fn placeholder() -> Entry {
    Entry { name: String::new(), dir: true, size: 0, mtime: None, hidden: false, data: Data::Inline(Vec::new()) }
}

impl FileSystem for Udf {
    fn kind(&self) -> FsKind {
        FsKind::Udf
    }

    fn label(&self) -> String {
        self.label.clone()
    }

    fn root(&self) -> Entry {
        self.root.clone()
    }

    fn list(&self, src: &mut dyn SectorSource, dir: &Entry) -> Result<Vec<Entry>> {
        let raw = crate::fs::read_all(src, dir, 64 << 20)?;
        let mut out = Vec::new();
        let mut p = 0;
        while p + 38 <= raw.len() {
            let f = &raw[p..];
            if le16(f, 0) != TAG_FID {
                break;
            }
            let chars = f[18];
            let l_fi = f[19] as usize;
            let (icb_lbn, icb_part) = (le32(f, 24) as u64, le16(f, 28));
            let l_iu = le16(f, 36) as usize;
            let total = (38 + l_iu + l_fi + 3) & !3;
            if p + 38 + l_iu + l_fi > raw.len() {
                return bad("truncated UDF directory");
            }
            if chars & 0x0C == 0 && l_fi > 0 {
                let name = dchars(&f[38 + l_iu..38 + l_iu + l_fi]);
                let mut e = self.read_fe(src, icb_part, icb_lbn)?;
                e.name = name;
                e.hidden = chars & 0x01 != 0;
                out.push(e);
            }
            p += total;
        }
        Ok(out)
    }
}
