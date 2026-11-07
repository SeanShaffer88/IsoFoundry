//! One-pass hashing (CRC32, MD5, SHA-1, SHA-256) and `.dat` hash files
//! (Logiqx XML as used by redump.org and ClrMamePro). Pure Rust, no I/O
//! policy: callers feed bytes.

pub mod dat;

use md5::Digest;
use std::io::Read;

/// Which digests to compute. All four by default; CRC32 is nearly free.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Algorithms {
    pub crc32: bool,
    pub md5: bool,
    pub sha1: bool,
    pub sha256: bool,
}

impl Default for Algorithms {
    fn default() -> Self {
        Self { crc32: true, md5: true, sha1: true, sha256: true }
    }
}

impl Algorithms {
    /// Parses a comma list such as `crc32,sha256` (case-insensitive) or `all`.
    pub fn parse(list: &str) -> Result<Self, String> {
        if list.eq_ignore_ascii_case("all") {
            return Ok(Self::default());
        }
        let mut a = Self { crc32: false, md5: false, sha1: false, sha256: false };
        for part in list.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_ascii_lowercase().as_str() {
                "crc32" | "crc" => a.crc32 = true,
                "md5" => a.md5 = true,
                "sha1" | "sha-1" => a.sha1 = true,
                "sha256" | "sha-256" => a.sha256 = true,
                other => return Err(format!("unknown hash algorithm: {other}")),
            }
        }
        if a == (Self { crc32: false, md5: false, sha1: false, sha256: false }) {
            return Err("no hash algorithm selected".into());
        }
        Ok(a)
    }
}

/// Finished digests as lowercase hex; `None` for algorithms not computed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Digests {
    pub size: u64,
    pub crc32: Option<String>,
    pub md5: Option<String>,
    pub sha1: Option<String>,
    pub sha256: Option<String>,
}

impl Digests {
    /// True when every digest present in both sides is equal and sizes match.
    pub fn matches(&self, other: &Digests) -> bool {
        let eq = |a: &Option<String>, b: &Option<String>| match (a, b) {
            (Some(x), Some(y)) => x.eq_ignore_ascii_case(y),
            _ => true,
        };
        let shared = [
            self.crc32.is_some() && other.crc32.is_some(),
            self.md5.is_some() && other.md5.is_some(),
            self.sha1.is_some() && other.sha1.is_some(),
            self.sha256.is_some() && other.sha256.is_some(),
        ]
        .iter()
        .any(|b| *b);
        shared
            && self.size == other.size
            && eq(&self.crc32, &other.crc32)
            && eq(&self.md5, &other.md5)
            && eq(&self.sha1, &other.sha1)
            && eq(&self.sha256, &other.sha256)
    }
}

/// Incremental multi-hasher.
pub struct Hasher {
    size: u64,
    crc: Option<crc32fast::Hasher>,
    md5: Option<md5::Md5>,
    sha1: Option<sha1::Sha1>,
    sha256: Option<sha2::Sha256>,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

impl Hasher {
    pub fn new(a: Algorithms) -> Self {
        Self {
            size: 0,
            crc: a.crc32.then(crc32fast::Hasher::new),
            md5: a.md5.then(md5::Md5::new),
            sha1: a.sha1.then(sha1::Sha1::new),
            sha256: a.sha256.then(sha2::Sha256::new),
        }
    }

    pub fn update(&mut self, data: &[u8]) {
        self.size += data.len() as u64;
        if let Some(h) = &mut self.crc {
            h.update(data);
        }
        if let Some(h) = &mut self.md5 {
            h.update(data);
        }
        if let Some(h) = &mut self.sha1 {
            h.update(data);
        }
        if let Some(h) = &mut self.sha256 {
            h.update(data);
        }
    }

    pub fn finish(self) -> Digests {
        Digests {
            size: self.size,
            crc32: self.crc.map(|h| format!("{:08x}", h.finalize())),
            md5: self.md5.map(|h| hex(&h.finalize())),
            sha1: self.sha1.map(|h| hex(&h.finalize())),
            sha256: self.sha256.map(|h| hex(&h.finalize())),
        }
    }
}

/// Hashes a reader to the end. `progress` gets the running byte count and
/// returns `false` to cancel (then `Err(Interrupted)` is returned).
pub fn hash_reader<R: Read>(
    r: &mut R,
    a: Algorithms,
    mut progress: impl FnMut(u64) -> bool,
) -> std::io::Result<Digests> {
    let mut h = Hasher::new(a);
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        h.update(&buf[..n]);
        if !progress(h.size) {
            return Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "cancelled"));
        }
    }
    Ok(h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        let mut h = Hasher::new(Algorithms::default());
        h.update(b"abc");
        let d = h.finish();
        assert_eq!(d.size, 3);
        assert_eq!(d.crc32.as_deref(), Some("352441c2"));
        assert_eq!(d.md5.as_deref(), Some("900150983cd24fb0d6963f7d28e17f72"));
        assert_eq!(d.sha1.as_deref(), Some("a9993e364706816aba3e25717850c26c9cd0d89d"));
        assert_eq!(d.sha256.as_deref(), Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
    }

    #[test]
    fn chunking_does_not_matter() {
        let data: Vec<u8> = (0..300_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let whole = hash_reader(&mut data.as_slice(), Algorithms::default(), |_| true).unwrap();
        let mut h = Hasher::new(Algorithms::default());
        for c in data.chunks(4097) {
            h.update(c);
        }
        assert_eq!(whole, h.finish());
    }

    #[test]
    fn parse_and_match() {
        let a = Algorithms::parse("CRC32, sha256").unwrap();
        assert!(a.crc32 && a.sha256 && !a.md5 && !a.sha1);
        assert!(Algorithms::parse("whirlpool").is_err());
        assert!(Algorithms::parse("").is_err());
        let mut x = Hasher::new(a);
        x.update(b"x");
        let x = x.finish();
        let mut y = x.clone();
        y.md5 = Some("ff".into());
        assert!(x.matches(&y));
        y.sha256 = Some("00".into());
        assert!(!x.matches(&y));
    }

    #[test]
    fn cancel() {
        let data = vec![0u8; 3 << 20];
        let e = hash_reader(&mut data.as_slice(), Algorithms::default(), |n| n < (1 << 20)).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::Interrupted);
    }
}
