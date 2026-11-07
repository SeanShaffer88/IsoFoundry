//! MAME CHD v5 (D-039): writing CD and DVD images, reading what we write
//! and chdman's zlib-based output.
//!
//! Writer: CD images use the `cdzl` codec (deflate of sector data with
//! sync/ECC stripped where it regenerates, plus deflated subcode); DVD/ISO
//! images use `zlib`. Identical hunks are stored once (self references).
//! The compressed v5 hunk map and the metadata follow chdman's format so
//! `chdman verify` and `extractcd`/`extractdvd` accept the result.
//!
//! Reader: codecs `zlib` and `cdzl`, uncompressed and self-referenced
//! hunks. LZMA, FLAC, Huffman and Zstandard hunks (chdman's other codecs)
//! are reported as unsupported.

use crate::cd::{self, RAW, SUB};
use crate::disc::{Disc, RawDisc, Track, TrackKind};
use crate::source::SectorSource;
use crate::{bad, Error, Result, SECTOR};
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

const FRAME: usize = RAW + SUB; // 2448
const CD_HUNK_FRAMES: usize = 8;
const DVD_HUNK: usize = 4096;
const TRACK_PAD: u64 = 4;

const C_NONE: u8 = 4;
const C_SELF: u8 = 5;
const C_PARENT: u8 = 6;
const C_RLE_SMALL: u8 = 7;
const C_RLE_LARGE: u8 = 8;
const C_SELF_0: u8 = 9;
const C_SELF_1: u8 = 10;

const TAG_CHT2: u32 = u32::from_be_bytes(*b"CHT2");
const TAG_DVD: u32 = u32::from_be_bytes(*b"DVD ");

fn fourcc(s: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*s)
}

/// CRC-16-CCITT as MAME's crc16_creator (initial 0xFFFF).
pub fn crc16(data: &[u8]) -> u16 {
    let mut c: u16 = 0xFFFF;
    for &b in data {
        c ^= (b as u16) << 8;
        for _ in 0..8 {
            c = if c & 0x8000 != 0 { (c << 1) ^ 0x1021 } else { c << 1 };
        }
    }
    c
}

// ---------------------------------------------------------------- bits

struct BitOut {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl BitOut {
    fn new() -> Self {
        BitOut { out: Vec::new(), acc: 0, n: 0 }
    }
    fn write(&mut self, v: u64, bits: u32) {
        if bits == 0 {
            return;
        }
        self.acc = (self.acc << bits) | (v & ((1u64 << bits) - 1));
        self.n += bits;
        while self.n >= 8 {
            self.out.push((self.acc >> (self.n - 8)) as u8);
            self.n -= 8;
        }
    }
    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push((self.acc << (8 - self.n)) as u8);
        }
        self.out
    }
}

struct BitIn<'a> {
    b: &'a [u8],
    pos: usize,
}

impl BitIn<'_> {
    fn read(&mut self, bits: u32) -> u64 {
        let mut v = 0u64;
        for _ in 0..bits {
            let byte = self.b.get(self.pos / 8).copied().unwrap_or(0);
            v = (v << 1) | ((byte >> (7 - self.pos % 8)) & 1) as u64;
            self.pos += 1;
        }
        v
    }
}

fn bits_for(mut v: u64) -> u32 {
    let mut r = 0;
    while v != 0 {
        v >>= 1;
        r += 1;
    }
    r
}

// ------------------------------------------------------------ huffman

const CODES: usize = 16;
const MAXBITS: u32 = 8;

/// Code lengths for the 16-symbol map alphabet, at most 8 bits, built the
/// way MAME does (weight scaling until the tree fits).
fn huff_lengths(histo: &[u32; CODES]) -> [u8; CODES] {
    let total: u64 = histo.iter().map(|&h| h as u64).sum();
    let build = |weight: u64| -> ([u8; CODES], u32) {
        // Nodes: (weight, symbol order key, children / leaf).
        #[derive(Clone)]
        struct N {
            w: u64,
            key: u32,
            parent: Option<usize>,
        }
        let mut nodes: Vec<N> = Vec::new();
        let mut list: Vec<usize> = Vec::new();
        for (s, &h) in histo.iter().enumerate() {
            nodes.push(N { w: 0, key: s as u32, parent: None });
            if h != 0 {
                let w = (h as u64 * weight / total.max(1)).max(1);
                nodes[s].w = w;
                list.push(s);
            }
        }
        list.sort_by(|&a, &b| nodes[b].w.cmp(&nodes[a].w).then(nodes[a].key.cmp(&nodes[b].key)));
        while list.len() > 1 {
            let n1 = list.pop().unwrap();
            let n0 = list.pop().unwrap();
            let id = nodes.len();
            nodes.push(N { w: nodes[n0].w + nodes[n1].w, key: 0, parent: None });
            nodes[n0].parent = Some(id);
            nodes[n1].parent = Some(id);
            let pos = list.iter().position(|&x| nodes[id].w > nodes[x].w).unwrap_or(list.len());
            list.insert(pos, id);
        }
        let mut lens = [0u8; CODES];
        let mut max = 0;
        for s in 0..CODES {
            if nodes[s].w > 0 {
                let mut n = 0;
                let mut c = s;
                while let Some(p) = nodes[c].parent {
                    n += 1;
                    c = p;
                }
                let n = n.max(1);
                lens[s] = n as u8;
                max = max.max(n);
            }
        }
        (lens, max)
    };
    let (mut lo, mut hi) = (0u64, total * 2);
    let mut best = None;
    loop {
        let cur = (lo + hi) / 2;
        let (lens, max) = build(cur);
        if max <= MAXBITS {
            lo = cur;
            best = Some(lens);
            if cur == total || hi - lo <= 1 {
                break;
            }
        } else {
            hi = cur;
        }
        if hi <= lo + 1 && best.is_some() {
            break;
        }
    }
    best.unwrap_or_else(|| build(1).0)
}

/// Canonical codes from lengths (MAME's assign_canonical_codes).
fn canonical(lens: &[u8; CODES]) -> Result<[u32; CODES]> {
    let mut histo = [0u32; 33];
    for &l in lens {
        histo[l as usize] += 1;
    }
    let mut start = 0u32;
    for len in (1..=32).rev() {
        let next = (start + histo[len]) >> 1;
        if len != 1 && next * 2 != start + histo[len] {
            return bad("CHD map Huffman table is inconsistent");
        }
        histo[len] = start;
        start = next;
    }
    let mut codes = [0u32; CODES];
    for (s, &l) in lens.iter().enumerate() {
        if l > 0 {
            codes[s] = histo[l as usize];
            histo[l as usize] += 1;
        }
    }
    Ok(codes)
}

fn export_tree(b: &mut BitOut, lens: &[u8; CODES]) {
    let nb = 4;
    let emit = |b: &mut BitOut, val: u8, mut rep: u32| {
        while rep > 0 {
            if val == 1 {
                b.write(1, nb);
                b.write(1, nb);
                rep -= 1;
            } else if rep <= 2 {
                b.write(val as u64, nb);
                rep -= 1;
            } else {
                let cur = (rep - 3).min(15);
                b.write(1, nb);
                b.write(val as u64, nb);
                b.write(cur as u64, nb);
                rep -= cur + 3;
            }
        }
    };
    let mut last = lens[0];
    let mut rep = 0u32;
    for &l in lens {
        if l == last {
            rep += 1;
        } else {
            emit(b, last, rep);
            last = l;
            rep = 1;
        }
    }
    emit(b, last, rep);
}

fn import_tree(b: &mut BitIn) -> [u8; CODES] {
    let mut lens = [0u8; CODES];
    let mut i = 0;
    while i < CODES {
        let v = b.read(4) as u8;
        if v != 1 {
            lens[i] = v;
            i += 1;
        } else {
            let v = b.read(4) as u8;
            if v == 1 {
                lens[i] = 1;
                i += 1;
            } else {
                let rep = b.read(4) as usize + 3;
                for _ in 0..rep {
                    if i < CODES {
                        lens[i] = v;
                        i += 1;
                    }
                }
            }
        }
    }
    lens
}

fn huff_decode(b: &mut BitIn, lens: &[u8; CODES], codes: &[u32; CODES]) -> Result<u8> {
    let mut code = 0u32;
    for len in 1..=MAXBITS {
        code = (code << 1) | b.read(1) as u32;
        if let Some(s) = (0..CODES).find(|&s| lens[s] as u32 == len && codes[s] == code) {
            return Ok(s as u8);
        }
    }
    bad("corrupt CHD map")
}

// ------------------------------------------------------------- map

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct MapEntry {
    comp: u8,
    len: u32,
    offset: u64,
    crc: u16,
}

fn raw_map_bytes(map: &[MapEntry]) -> Vec<u8> {
    let mut v = Vec::with_capacity(map.len() * 12);
    for e in map {
        v.push(e.comp);
        v.extend_from_slice(&e.len.to_be_bytes()[1..]);
        v.extend_from_slice(&e.offset.to_be_bytes()[2..]);
        v.extend_from_slice(&e.crc.to_be_bytes());
    }
    v
}

fn compress_map(map: &[MapEntry]) -> Vec<u8> {
    let mapcrc = crc16(&raw_map_bytes(map));
    let mut rle: Vec<u8> = Vec::new();
    let mut histo = [0u32; CODES];
    let (mut max_self, mut last_self, mut max_len) = (0u64, 0u64, 0u32);
    let mut first_offs = u64::MAX;
    let mut last = 0u8;
    let mut count = 0u32;
    let mut types = Vec::with_capacity(map.len());
    let mut push = |rle: &mut Vec<u8>, v: u8| {
        histo[v as usize] += 1;
        rle.push(v);
    };
    let flush = |rle: &mut Vec<u8>, push: &mut dyn FnMut(&mut Vec<u8>, u8), last: u8, count: u32| {
        if count == 0 {
            return;
        }
        if count < 3 {
            for _ in 0..count {
                push(rle, last);
            }
        } else if count < 3 + 16 {
            push(rle, C_RLE_SMALL);
            push(rle, (count - 3) as u8);
        } else {
            push(rle, C_RLE_LARGE);
            push(rle, ((count - 3 - 16) >> 4) as u8);
            push(rle, ((count - 3 - 16) & 15) as u8);
        }
    };
    for e in map {
        let mut cur = e.comp;
        if cur == C_SELF {
            if e.offset == last_self {
                cur = C_SELF_0;
            } else if e.offset == last_self + 1 {
                cur = C_SELF_1;
            } else {
                max_self = max_self.max(e.offset);
            }
            last_self = e.offset;
        }
        if cur < C_SELF {
            max_len = max_len.max(e.len);
            first_offs = first_offs.min(e.offset);
        }
        types.push(cur);
        if cur == last && count < 3 + 16 + 255 {
            count += 1;
        } else {
            flush(&mut rle, &mut push, last, count);
            push(&mut rle, cur);
            last = cur;
            count = 0;
        }
    }
    flush(&mut rle, &mut push, last, count);
    if first_offs == u64::MAX {
        first_offs = 0;
    }
    let lens = huff_lengths(&histo);
    let codes = canonical(&lens).expect("valid lengths");
    let mut b = BitOut::new();
    export_tree(&mut b, &lens);
    for &v in &rle {
        b.write(codes[v as usize] as u64, lens[v as usize] as u32);
    }
    let (lb, sb) = (bits_for(max_len as u64), bits_for(max_self));
    for (e, &t) in map.iter().zip(&types) {
        match t {
            0..=3 => {
                b.write(e.len as u64, lb);
                b.write(e.crc as u64, 16);
            }
            C_NONE => b.write(e.crc as u64, 16),
            C_SELF => b.write(e.offset, sb),
            _ => {}
        }
    }
    let body = b.finish();
    let mut out = vec![0u8; 16];
    out[0..4].copy_from_slice(&(body.len() as u32).to_be_bytes());
    out[4..10].copy_from_slice(&first_offs.to_be_bytes()[2..]);
    out[10..12].copy_from_slice(&mapcrc.to_be_bytes());
    out[12] = lb as u8;
    out[13] = sb as u8;
    out[14] = 0;
    out.extend_from_slice(&body);
    out
}

fn decompress_map(data: &[u8], hunks: usize, hunkbytes: u32) -> Result<Vec<MapEntry>> {
    if data.len() < 16 {
        return bad("CHD map header truncated");
    }
    let first = u64::from_be_bytes([0, 0, data[4], data[5], data[6], data[7], data[8], data[9]]);
    let mapcrc = u16::from_be_bytes([data[10], data[11]]);
    let (lb, sb, pb) = (data[12] as u32, data[13] as u32, data[14] as u32);
    let mut b = BitIn { b: &data[16..], pos: 0 };
    let lens = import_tree(&mut b);
    let codes = canonical(&lens)?;
    let mut types = Vec::with_capacity(hunks);
    let (mut rep, mut last) = (0u32, 0u8);
    for _ in 0..hunks {
        if rep > 0 {
            types.push(last);
            rep -= 1;
            continue;
        }
        let v = huff_decode(&mut b, &lens, &codes)?;
        if v == C_RLE_SMALL {
            types.push(last);
            rep = 2 + huff_decode(&mut b, &lens, &codes)? as u32;
        } else if v == C_RLE_LARGE {
            types.push(last);
            rep = 2 + 16 + ((huff_decode(&mut b, &lens, &codes)? as u32) << 4);
            rep += huff_decode(&mut b, &lens, &codes)? as u32;
        } else {
            last = v;
            types.push(v);
        }
    }
    let mut map = Vec::with_capacity(hunks);
    let (mut cur, mut last_self) = (first, 0u64);
    for (i, &t) in types.iter().enumerate() {
        let e = match t {
            0..=3 => {
                let len = b.read(lb) as u32;
                let crc = b.read(16) as u16;
                let e = MapEntry { comp: t, len, offset: cur, crc };
                cur += len as u64;
                e
            }
            C_NONE => {
                let crc = b.read(16) as u16;
                let e = MapEntry { comp: C_NONE, len: hunkbytes, offset: cur, crc };
                cur += hunkbytes as u64;
                e
            }
            C_SELF => {
                last_self = b.read(sb);
                MapEntry { comp: C_SELF, len: 0, offset: last_self, crc: 0 }
            }
            C_SELF_1 | C_SELF_0 => {
                if t == C_SELF_1 {
                    last_self += 1;
                }
                MapEntry { comp: C_SELF, len: 0, offset: last_self, crc: 0 }
            }
            C_PARENT | 11..=13 => {
                let _ = b.read(if t == C_PARENT { pb } else { 0 });
                MapEntry { comp: C_PARENT, len: 0, offset: 0, crc: 0 }
            }
            _ => return bad(format!("unknown CHD map entry type {t} at hunk {i}")),
        };
        map.push(e);
    }
    if crc16(&raw_map_bytes(&map)) != mapcrc {
        return bad("CHD map checksum mismatch");
    }
    Ok(map)
}

// ----------------------------------------------------------- codecs

fn deflate(data: &[u8]) -> Vec<u8> {
    miniz_oxide::deflate::compress_to_vec(data, 9)
}

fn inflate(data: &[u8], size: usize) -> Result<Vec<u8>> {
    let v = miniz_oxide::inflate::decompress_to_vec_with_limit(data, size)
        .map_err(|e| Error::Format(format!("corrupt CHD hunk ({e:?})")))?;
    if v.len() != size {
        return bad("CHD hunk decompressed to the wrong size");
    }
    Ok(v)
}

/// ECC sectors whose P/Q parity regenerates exactly (Mode 1).
fn ecc_regenerates(s: &[u8; RAW]) -> bool {
    if s[..12] != cd::SYNC || s[15] != 1 {
        return false;
    }
    let lba = cd::msf_to_lba(cd::from_bcd(s[12]), cd::from_bcd(s[13]), cd::from_bcd(s[14]));
    let mut user = [0u8; SECTOR];
    user.copy_from_slice(&s[16..2064]);
    let regen = cd::mode1(lba, &user);
    regen[..] == s[..]
}

fn cdzl_compress(hunk: &[u8]) -> Vec<u8> {
    let frames = hunk.len() / FRAME;
    let ecc_bytes = frames.div_ceil(8);
    let mut base = Vec::with_capacity(frames * RAW);
    let mut sub = Vec::with_capacity(frames * SUB);
    let mut ecc = vec![0u8; ecc_bytes];
    for f in 0..frames {
        let fr = &hunk[f * FRAME..(f + 1) * FRAME];
        let mut s: [u8; RAW] = fr[..RAW].try_into().unwrap();
        if ecc_regenerates(&s) {
            ecc[f / 8] |= 1 << (f % 8);
            s[..12].fill(0);
            s[0x81C..0x81C + 172 + 104].fill(0);
        }
        base.extend_from_slice(&s);
        sub.extend_from_slice(&fr[RAW..]);
    }
    let cbase = deflate(&base);
    let csub = deflate(&sub);
    let complen_bytes = if hunk.len() < 65536 { 2 } else { 3 };
    let mut out = ecc;
    out.extend_from_slice(&(cbase.len() as u32).to_be_bytes()[4 - complen_bytes..]);
    out.extend_from_slice(&cbase);
    out.extend_from_slice(&csub);
    out
}

fn cdzl_decompress(data: &[u8], hunkbytes: usize) -> Result<Vec<u8>> {
    let frames = hunkbytes / FRAME;
    let ecc_bytes = frames.div_ceil(8);
    let cl = if hunkbytes < 65536 { 2 } else { 3 };
    if data.len() < ecc_bytes + cl {
        return bad("CHD CD hunk truncated");
    }
    let mut complen = 0usize;
    for &b in &data[ecc_bytes..ecc_bytes + cl] {
        complen = (complen << 8) | b as usize;
    }
    let start = ecc_bytes + cl;
    let base =
        inflate(data.get(start..start + complen).ok_or(Error::Format("CHD CD hunk truncated".into()))?, frames * RAW)?;
    let sub = inflate(&data[start + complen..], frames * SUB)?;
    let mut out = vec![0u8; hunkbytes];
    for f in 0..frames {
        let mut s: [u8; RAW] = base[f * RAW..(f + 1) * RAW].try_into().unwrap();
        if data[f / 8] & (1 << (f % 8)) != 0 {
            s[..12].copy_from_slice(&cd::SYNC);
            let lba = cd::msf_to_lba(cd::from_bcd(s[12]), cd::from_bcd(s[13]), cd::from_bcd(s[14]));
            let mut user = [0u8; SECTOR];
            user.copy_from_slice(&s[16..2064]);
            s = cd::mode1(lba, &user);
        }
        out[f * FRAME..f * FRAME + RAW].copy_from_slice(&s);
        out[f * FRAME + RAW..(f + 1) * FRAME].copy_from_slice(&sub[f * SUB..(f + 1) * SUB]);
    }
    Ok(out)
}

// ----------------------------------------------------------- writer

struct Writer {
    w: BufWriter<File>,
    pos: u64,
    hunkbytes: usize,
    cd: bool,
    map: Vec<MapEntry>,
    seen: HashMap<[u8; 20], u64>,
    raw_sha: Sha1,
    logical: u64,
    written: u64,
}

impl Writer {
    fn hunk(&mut self, data: &[u8]) -> Result<()> {
        let idx = self.map.len() as u64;
        let take = (self.logical - self.written).min(data.len() as u64) as usize;
        self.raw_sha.update(&data[..take]);
        self.written += take as u64;
        let digest: [u8; 20] = Sha1::digest(data).into();
        if let Some(&prev) = self.seen.get(&digest) {
            self.map.push(MapEntry { comp: C_SELF, len: 0, offset: prev, crc: 0 });
            return Ok(());
        }
        self.seen.insert(digest, idx);
        let crc = crc16(data);
        let c = if self.cd { cdzl_compress(data) } else { deflate(data) };
        let (comp, bytes): (u8, &[u8]) = if c.len() < self.hunkbytes { (0, &c) } else { (C_NONE, data) };
        self.w.write_all(bytes)?;
        self.map.push(MapEntry { comp, len: bytes.len() as u32, offset: self.pos, crc });
        self.pos += bytes.len() as u64;
        Ok(())
    }
}

fn track_type(t: &Track) -> &'static str {
    match t.kind {
        TrackKind::Audio => "AUDIO",
        TrackKind::Mode1 => "MODE1_RAW",
        TrackKind::Mode2 | TrackKind::Mode2Form1 => "MODE2_RAW",
    }
}

pub struct ChdSummary {
    pub hunks: u64,
    pub bytes: u64,
    pub logical: u64,
}

/// Writes a CD CHD from any raw disc.
pub fn write_cd(src: &mut dyn RawDisc, dest: &Path, progress: &mut dyn FnMut(u64, u64) -> bool) -> Result<ChdSummary> {
    let disc = src.disc().clone();
    let with_sub = disc.tracks.iter().any(|t| t.sub.is_some());
    let mut frames_total = 0u64;
    let mut meta = Vec::new();
    for (i, t) in disc.tracks.iter().enumerate() {
        let stored_gap = if i > 0 && t.pregap_stored { t.pregap } else { 0 };
        let frames = t.length + stored_gap;
        let pg = if i > 0 { t.pregap } else { 0 };
        // As chdman writes it: an unstored pregap is typed MODE1.
        let pgtype = if stored_gap > 0 { format!("V{}", track_type(t)) } else { "MODE1".to_string() };
        meta.push(format!(
            "TRACK:{} TYPE:{} SUBTYPE:{} FRAMES:{} PREGAP:{} PGTYPE:{} PGSUB:{} POSTGAP:0",
            i + 1,
            track_type(t),
            if with_sub { "RW_RAW" } else { "NONE" },
            frames,
            pg,
            pgtype,
            if with_sub { "RW_RAW" } else { "NONE" },
        ));
        frames_total += frames.div_ceil(TRACK_PAD) * TRACK_PAD;
    }
    let logical = frames_total * FRAME as u64;
    let hunkbytes = CD_HUNK_FRAMES * FRAME;
    let total_hunks = logical.div_ceil(hunkbytes as u64);
    let mut w = start(dest, hunkbytes, logical)?;
    w.cd = true;
    let mut buf = Vec::with_capacity(hunkbytes);
    let mut done = 0u64;
    for (i, t) in disc.tracks.iter().enumerate() {
        let stored_gap = if i > 0 && t.pregap_stored { t.pregap as i64 } else { 0 };
        let frames = t.length as i64 + stored_gap;
        let padded = (frames as u64).div_ceil(TRACK_PAD) * TRACK_PAD;
        for f in 0..padded as i64 {
            let mut s = [0u8; RAW];
            let mut sub = [0u8; SUB];
            if f < frames {
                let rel = f - stored_gap;
                src.read_raw(i, rel, &mut s)?;
                if with_sub {
                    sub = src.read_sub(i, rel)?;
                }
                if t.kind == TrackKind::Audio {
                    for p in s.chunks_exact_mut(2) {
                        p.swap(0, 1);
                    }
                }
            }
            buf.extend_from_slice(&s);
            buf.extend_from_slice(&sub);
            if buf.len() == hunkbytes {
                w.hunk(&buf)?;
                buf.clear();
                done += 1;
                if !progress(done, total_hunks) {
                    return Err(Error::Cancelled);
                }
            }
        }
    }
    if !buf.is_empty() {
        buf.resize(hunkbytes, 0);
        w.hunk(&buf)?;
    }
    let metas: Vec<(u32, Vec<u8>)> = meta.into_iter().map(|m| (TAG_CHT2, nul(m))).collect();
    finish(w, dest, *b"cdzl", 2448, metas)
}

/// Writes a DVD CHD from 2048-byte sectors.
pub fn write_dvd(
    src: &mut dyn SectorSource,
    dest: &Path,
    progress: &mut dyn FnMut(u64, u64) -> bool,
) -> Result<ChdSummary> {
    let logical = src.sectors() * SECTOR as u64;
    let mut w = start(dest, DVD_HUNK, logical)?;
    let hunks = logical.div_ceil(DVD_HUNK as u64);
    let per = (DVD_HUNK / SECTOR) as u64;
    let mut buf = vec![0u8; DVD_HUNK];
    for h in 0..hunks {
        let first = h * per;
        let n = per.min(src.sectors() - first) as usize;
        buf.fill(0);
        src.read_sectors(first, &mut buf[..n * SECTOR])?;
        w.hunk(&buf)?;
        if !progress(h + 1, hunks) {
            return Err(Error::Cancelled);
        }
    }
    finish(w, dest, *b"zlib", SECTOR as u32, vec![(TAG_DVD, vec![0])])
}

fn nul(s: String) -> Vec<u8> {
    let mut v = s.into_bytes();
    v.push(0);
    v
}

fn part_path(dest: &Path) -> PathBuf {
    let mut p = dest.as_os_str().to_owned();
    p.push(".part");
    PathBuf::from(p)
}

fn start(dest: &Path, hunkbytes: usize, logical: u64) -> Result<Writer> {
    let f = File::create(part_path(dest))?;
    let mut w = BufWriter::with_capacity(1 << 20, f);
    w.write_all(&[0u8; 124])?;
    Ok(Writer {
        w,
        pos: 124,
        hunkbytes,
        cd: false,
        map: Vec::new(),
        seen: HashMap::new(),
        raw_sha: Sha1::new(),
        logical,
        written: 0,
    })
}

fn finish(mut w: Writer, dest: &Path, codec: [u8; 4], unit: u32, metas: Vec<(u32, Vec<u8>)>) -> Result<ChdSummary> {
    let map_off = w.pos;
    let cmap = compress_map(&w.map);
    w.w.write_all(&cmap)?;
    w.pos += cmap.len() as u64;
    let meta_off = w.pos;
    let n = metas.len();
    for (i, (tag, data)) in metas.iter().enumerate() {
        let next = if i + 1 < n { w.pos + 16 + data.len() as u64 } else { 0 };
        let mut h = [0u8; 16];
        h[0..4].copy_from_slice(&tag.to_be_bytes());
        h[4] = 1; // checksummed
        h[5..8].copy_from_slice(&(data.len() as u32).to_be_bytes()[1..]);
        h[8..16].copy_from_slice(&next.to_be_bytes());
        w.w.write_all(&h)?;
        w.w.write_all(data)?;
        w.pos += 16 + data.len() as u64;
    }
    let raw_sha: [u8; 20] = w.raw_sha.clone().finalize().into();
    let mut hashes: Vec<[u8; 24]> = metas
        .iter()
        .map(|(tag, d)| {
            let mut e = [0u8; 24];
            e[..4].copy_from_slice(&tag.to_be_bytes());
            let h: [u8; 20] = Sha1::digest(d).into();
            e[4..].copy_from_slice(&h);
            e
        })
        .collect();
    hashes.sort();
    let mut overall = Sha1::new();
    overall.update(raw_sha);
    for h in &hashes {
        overall.update(h);
    }
    let overall: [u8; 20] = overall.finalize().into();
    let mut hdr = [0u8; 124];
    hdr[0..8].copy_from_slice(b"MComprHD");
    hdr[8..12].copy_from_slice(&124u32.to_be_bytes());
    hdr[12..16].copy_from_slice(&5u32.to_be_bytes());
    hdr[16..20].copy_from_slice(&fourcc(&codec).to_be_bytes());
    hdr[32..40].copy_from_slice(&w.logical.to_be_bytes());
    hdr[40..48].copy_from_slice(&map_off.to_be_bytes());
    hdr[48..56].copy_from_slice(&meta_off.to_be_bytes());
    hdr[56..60].copy_from_slice(&(w.hunkbytes as u32).to_be_bytes());
    hdr[60..64].copy_from_slice(&unit.to_be_bytes());
    hdr[64..84].copy_from_slice(&raw_sha);
    hdr[84..104].copy_from_slice(&overall);
    w.w.flush()?;
    let mut f = w.w.into_inner().map_err(|e| Error::Io(e.into_error()))?;
    f.seek(SeekFrom::Start(0))?;
    f.write_all(&hdr)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(part_path(dest), dest)?;
    let bytes = std::fs::metadata(dest)?.len();
    Ok(ChdSummary { hunks: w.map.len() as u64, bytes, logical: w.logical })
}

/// Removes a half-written CHD after an error or cancel.
pub fn discard(dest: &Path) {
    let _ = std::fs::remove_file(part_path(dest));
}

// ----------------------------------------------------------- reader

pub struct Chd {
    f: File,
    path: PathBuf,
    hunkbytes: usize,
    unit: u32,
    logical: u64,
    codecs: [u32; 4],
    map: Vec<MapEntry>,
    cache: Option<(usize, Vec<u8>)>,
    pub metadata: Vec<(u32, Vec<u8>)>,
    disc: Option<Disc>,
}

impl Chd {
    pub fn open(path: &Path) -> Result<Chd> {
        let mut f = File::open(path)?;
        let mut h = [0u8; 124];
        f.read_exact(&mut h).map_err(|_| Error::Format("file too small for a CHD".into()))?;
        if &h[0..8] != b"MComprHD" {
            return bad("not a CHD file");
        }
        let version = u32::from_be_bytes(h[12..16].try_into().unwrap());
        if version != 5 {
            return bad(format!("CHD version {version} is not supported (convert it with chdman copy)"));
        }
        let be32 = |o: usize| u32::from_be_bytes(h[o..o + 4].try_into().unwrap());
        let be64 = |o: usize| u64::from_be_bytes(h[o..o + 8].try_into().unwrap());
        if h[104..124].iter().any(|&b| b != 0) {
            return bad("CHD files with a parent are not supported");
        }
        let codecs = [be32(16), be32(20), be32(24), be32(28)];
        let logical = be64(32);
        let (map_off, meta_off) = (be64(40), be64(48));
        let hunkbytes = be32(56) as usize;
        let unit = be32(60);
        if hunkbytes == 0 || hunkbytes > 1 << 24 || unit == 0 {
            return bad("CHD header has invalid sizes");
        }
        let hunks = logical.div_ceil(hunkbytes as u64) as usize;
        let len = f.metadata()?.len();
        let map = if codecs[0] == 0 {
            // Uncompressed CHD: 4-byte big-endian hunk offsets (in hunks).
            let mut m = vec![0u8; hunks * 4];
            f.seek(SeekFrom::Start(map_off))?;
            f.read_exact(&mut m)?;
            m.chunks_exact(4)
                .map(|c| {
                    let off = u32::from_be_bytes(c.try_into().unwrap()) as u64 * hunkbytes as u64;
                    MapEntry { comp: C_NONE, len: hunkbytes as u32, offset: off, crc: 0 }
                })
                .collect()
        } else {
            let mut hdr = [0u8; 16];
            f.seek(SeekFrom::Start(map_off))?;
            f.read_exact(&mut hdr)?;
            let mlen = u32::from_be_bytes(hdr[0..4].try_into().unwrap()) as u64;
            if map_off + 16 + mlen > len {
                return bad("CHD map outside the file");
            }
            let mut m = hdr.to_vec();
            m.resize(16 + mlen as usize, 0);
            f.read_exact(&mut m[16..])?;
            decompress_map(&m, hunks, hunkbytes as u32)?
        };
        let mut metadata = Vec::new();
        let mut off = meta_off;
        while off != 0 && metadata.len() < 1000 {
            let mut mh = [0u8; 16];
            f.seek(SeekFrom::Start(off))?;
            f.read_exact(&mut mh)?;
            let tag = u32::from_be_bytes(mh[0..4].try_into().unwrap());
            let mlen = u32::from_be_bytes([0, mh[5], mh[6], mh[7]]) as usize;
            let mut d = vec![0u8; mlen];
            f.read_exact(&mut d)?;
            metadata.push((tag, d));
            off = u64::from_be_bytes(mh[8..16].try_into().unwrap());
        }
        let mut chd = Chd {
            f,
            path: path.to_path_buf(),
            hunkbytes,
            unit,
            logical,
            codecs,
            map,
            cache: None,
            metadata,
            disc: None,
        };
        if unit as usize == FRAME {
            chd.disc = Some(chd.parse_tracks()?);
        }
        Ok(chd)
    }

    pub fn is_cd(&self) -> bool {
        self.disc.is_some()
    }

    fn parse_tracks(&self) -> Result<Disc> {
        let mut tracks = Vec::new();
        let mut frame = 0u64; // CHD frame index (with padding)
        let mut lba = 0i64;
        for (tag, d) in &self.metadata {
            if *tag != TAG_CHT2 && *tag != u32::from_be_bytes(*b"CHTR") {
                continue;
            }
            let text = String::from_utf8_lossy(d).trim_end_matches('\0').to_string();
            let field = |k: &str| {
                text.split_whitespace()
                    .find_map(|kv| kv.strip_prefix(&format!("{k}:")).map(str::to_string))
                    .unwrap_or_default()
            };
            let num = |k: &str| field(k).parse::<u64>().unwrap_or(0);
            let ty = field("TYPE");
            let kind = match ty.as_str() {
                "AUDIO" => TrackKind::Audio,
                "MODE1" | "MODE1_RAW" => TrackKind::Mode1,
                "MODE2_FORM1" => TrackKind::Mode2Form1,
                _ => TrackKind::Mode2,
            };
            let frames = num("FRAMES");
            let pregap = num("PREGAP");
            let stored = field("PGTYPE").starts_with('V');
            let stored_gap = if stored { pregap } else { 0 };
            if tracks.is_empty() {
                lba = 0;
            } else {
                lba += pregap as i64;
            }
            tracks.push(Track {
                number: num("TRACK") as u8,
                kind,
                start: lba,
                length: frames - stored_gap,
                pregap,
                pregap_stored: stored,
                file: self.path.clone(),
                offset: frame,
                layout: crate::disc::Layout::Raw,
                sub: None,
            });
            lba += (frames - stored_gap) as i64;
            frame += frames.div_ceil(TRACK_PAD) * TRACK_PAD;
        }
        if tracks.is_empty() {
            return bad("CD CHD without track metadata");
        }
        Ok(Disc { tracks, notes: Vec::new() })
    }

    fn read_hunk(&mut self, idx: usize) -> Result<&[u8]> {
        if self.cache.as_ref().map(|(i, _)| *i == idx) != Some(true) {
            let data = self.load_hunk(idx, 0)?;
            self.cache = Some((idx, data));
        }
        Ok(&self.cache.as_ref().expect("cached").1)
    }

    fn load_hunk(&mut self, idx: usize, depth: u32) -> Result<Vec<u8>> {
        let e = *self.map.get(idx).ok_or(Error::Format("hunk outside the CHD".into()))?;
        match e.comp {
            C_NONE => {
                let mut d = vec![0u8; self.hunkbytes];
                self.f.seek(SeekFrom::Start(e.offset))?;
                self.f.read_exact(&mut d)?;
                Ok(d)
            }
            C_SELF if depth < 8 => self.load_hunk(e.offset as usize, depth + 1),
            0..=3 => {
                let mut d = vec![0u8; e.len as usize];
                self.f.seek(SeekFrom::Start(e.offset))?;
                self.f.read_exact(&mut d)?;
                let codec = self.codecs[e.comp as usize].to_be_bytes();
                let out = match &codec {
                    b"zlib" => inflate(&d, self.hunkbytes)?,
                    b"cdzl" => cdzl_decompress(&d, self.hunkbytes)?,
                    other => {
                        return bad(format!(
                            "CHD codec '{}' is not supported yet (zlib and cdzl are); recompress with chdman -c cdzl or zlib",
                            String::from_utf8_lossy(other)
                        ))
                    }
                };
                if crc16(&out) != e.crc {
                    return bad(format!("CHD hunk {idx} failed its checksum"));
                }
                Ok(out)
            }
            _ => bad("CHD parent references are not supported"),
        }
    }

    fn read_frame(&mut self, frame: u64, out: &mut [u8; FRAME]) -> Result<()> {
        let per = (self.hunkbytes / FRAME) as u64;
        let (h, o) = ((frame / per) as usize, (frame % per) as usize * FRAME);
        let data = self.read_hunk(h)?;
        out.copy_from_slice(&data[o..o + FRAME]);
        Ok(())
    }

    pub fn logical_bytes(&self) -> u64 {
        self.logical
    }
}

impl RawDisc for Chd {
    fn disc(&self) -> &Disc {
        self.disc.as_ref().expect("CD CHD")
    }

    fn read_raw(&mut self, t: usize, rel: i64, out: &mut [u8; RAW]) -> Result<()> {
        let tr = self.disc().tracks[t].clone();
        let stored = if t > 0 && tr.pregap_stored { tr.pregap as i64 } else { 0 };
        if rel < -stored {
            *out = match tr.kind {
                TrackKind::Audio => [0u8; RAW],
                _ => cd::mode1(tr.start + rel, &[0u8; SECTOR]),
            };
            return Ok(());
        }
        let mut f = [0u8; FRAME];
        self.read_frame(tr.offset + (rel + stored) as u64, &mut f)?;
        out.copy_from_slice(&f[..RAW]);
        if tr.kind == TrackKind::Audio {
            for p in out.chunks_exact_mut(2) {
                p.swap(0, 1);
            }
        }
        Ok(())
    }

    fn read_sub(&mut self, t: usize, rel: i64) -> Result<[u8; SUB]> {
        let tr = self.disc().tracks[t].clone();
        let stored = if t > 0 && tr.pregap_stored { tr.pregap as i64 } else { 0 };
        if rel >= -stored {
            let mut f = [0u8; FRAME];
            self.read_frame(tr.offset + (rel + stored) as u64, &mut f)?;
            if f[RAW..].iter().any(|&b| b != 0) {
                return Ok(f[RAW..].try_into().unwrap());
            }
        }
        Ok(cd::subchannel(tr.kind.control(), tr.number, if rel < 0 { 0 } else { 1 }, rel, tr.start + rel))
    }
}

/// DVD (2048-byte unit) CHD as a sector source.
impl SectorSource for Chd {
    fn sectors(&self) -> u64 {
        self.logical / SECTOR as u64
    }

    fn read_sectors(&mut self, lba: u64, buf: &mut [u8]) -> Result<()> {
        if self.unit as usize != SECTOR {
            return bad("not a DVD CHD");
        }
        let per = (self.hunkbytes / SECTOR) as u64;
        for (i, chunk) in buf.chunks_mut(SECTOR).enumerate() {
            let s = lba + i as u64;
            let (h, o) = ((s / per) as usize, (s % per) as usize * SECTOR);
            let data = self.read_hunk(h)?;
            chunk.copy_from_slice(&data[o..o + SECTOR]);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc16_matches_mame() {
        // CRC-16/CCITT-FALSE check value.
        assert_eq!(crc16(b"123456789"), 0x29B1);
    }

    #[test]
    fn map_roundtrip_with_runs_and_self_refs() {
        let mut map = Vec::new();
        let mut off = 124u64;
        for i in 0..300u64 {
            if i % 7 == 3 {
                map.push(MapEntry { comp: C_SELF, len: 0, offset: i / 2, crc: 0 });
            } else if i % 11 == 0 {
                map.push(MapEntry { comp: C_NONE, len: 4096, offset: off, crc: (i * 31) as u16 });
                off += 4096;
            } else {
                let len = 100 + (i as u32 * 37) % 3000;
                map.push(MapEntry { comp: 0, len, offset: off, crc: (i * 7) as u16 });
                off += len as u64;
            }
        }
        let c = compress_map(&map);
        let back = decompress_map(&c, map.len(), 4096).unwrap();
        assert_eq!(back, map);
    }

    #[test]
    fn single_symbol_map() {
        let map: Vec<MapEntry> = (0..5).map(|i| MapEntry { comp: 0, len: 10, offset: 124 + i * 10, crc: 1 }).collect();
        assert_eq!(decompress_map(&compress_map(&map), 5, 4096).unwrap(), map);
    }

    #[test]
    fn cdzl_roundtrip() {
        let mut hunk = vec![0u8; CD_HUNK_FRAMES * FRAME];
        for f in 0..CD_HUNK_FRAMES {
            let user: Vec<u8> = (0..2048).map(|i| ((i + f * 7) % 251) as u8).collect();
            let s = if f == 5 { [3u8; RAW] } else { cd::mode1(100 + f as i64, &user) };
            hunk[f * FRAME..f * FRAME + RAW].copy_from_slice(&s);
            hunk[f * FRAME + RAW] = f as u8;
        }
        let c = cdzl_compress(&hunk);
        assert_eq!(c[0], 0b1101_1111);
        assert_eq!(cdzl_decompress(&c, hunk.len()).unwrap(), hunk);
    }
}
