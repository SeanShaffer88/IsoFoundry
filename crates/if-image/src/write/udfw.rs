//! UDF 1.02 descriptor builders (ECMA-167 3rd ed. / OSTA UDF 1.02).

pub const REVISION: u16 = 0x0102;

/// CRC-16-CCITT (x^16 + x^12 + x^5 + 1), initial value 0, as ECMA-167 7.2.6.
pub fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for &b in data {
        crc ^= (b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

/// Fills the 16-byte descriptor tag of `d` (whose length is the descriptor
/// length, CRC over d[16..]).
pub fn tag(d: &mut [u8], id: u16, location: u32) {
    d[0..2].copy_from_slice(&id.to_le_bytes());
    d[2..4].copy_from_slice(&2u16.to_le_bytes());
    d[5] = 0;
    d[6..8].copy_from_slice(&1u16.to_le_bytes());
    let crc_len = (d.len() - 16) as u16;
    let crc = crc16(&d[16..]);
    d[8..10].copy_from_slice(&crc.to_le_bytes());
    d[10..12].copy_from_slice(&crc_len.to_le_bytes());
    d[12..16].copy_from_slice(&location.to_le_bytes());
    d[4] = 0;
    let sum = d[..16].iter().fold(0u8, |s, v| s.wrapping_add(*v));
    d[4] = sum;
}

pub fn put16(d: &mut [u8], o: usize, v: u16) {
    d[o..o + 2].copy_from_slice(&v.to_le_bytes());
}

pub fn put32(d: &mut [u8], o: usize, v: u32) {
    d[o..o + 4].copy_from_slice(&v.to_le_bytes());
}

pub fn put64(d: &mut [u8], o: usize, v: u64) {
    d[o..o + 8].copy_from_slice(&v.to_le_bytes());
}

/// OSTA CS0 d-characters: compression id 8 when every char fits a byte,
/// else 16 (UCS-2 big endian).
pub fn dchars(s: &str) -> Vec<u8> {
    if s.chars().all(|c| (c as u32) < 256) {
        let mut v = vec![8u8];
        v.extend(s.chars().map(|c| c as u8));
        v
    } else {
        let mut v = vec![16u8];
        for u in s.encode_utf16() {
            v.extend_from_slice(&u.to_be_bytes());
        }
        v
    }
}

/// Fixed-size d-string field; last byte = used length.
pub fn dstring(d: &mut [u8], s: &str) {
    let len = d.len();
    let mut c = dchars(s);
    c.truncate(len - 1);
    if c.len() == 1 {
        c.clear();
    }
    d[..c.len()].copy_from_slice(&c);
    d[len - 1] = c.len() as u8;
}

pub fn charspec(d: &mut [u8]) {
    d[0] = 0;
    d[1..24].copy_from_slice(b"OSTA Compressed Unicode");
}

/// Entity identifier: flags 0, identifier, suffix.
pub fn regid(d: &mut [u8], id: &str, suffix: &[u8]) {
    d[0] = 0;
    let b = id.as_bytes();
    d[1..1 + b.len().min(23)].copy_from_slice(&b[..b.len().min(23)]);
    d[24..24 + suffix.len().min(8)].copy_from_slice(&suffix[..suffix.len().min(8)]);
}

pub fn udf_suffix() -> [u8; 3] {
    let r = REVISION.to_le_bytes();
    [r[0], r[1], 0]
}

pub fn impl_id(d: &mut [u8]) {
    regid(d, "*IsoFoundry", &[]);
}

/// UDF timestamp, UTC.
pub fn timestamp(d: &mut [u8], unix: i64) {
    let (y, mo, da, h, mi, s) = crate::fs::civil_from_unix(unix);
    put16(d, 0, 0x1000);
    put16(d, 2, y as u16);
    d[4] = mo as u8;
    d[5] = da as u8;
    d[6] = h as u8;
    d[7] = mi as u8;
    d[8] = s as u8;
    d[9] = 0;
    d[10] = 0;
    d[11] = 0;
}

/// long_ad: length, logical block, partition reference 0, implementation use.
pub fn long_ad(d: &mut [u8], len: u32, lbn: u32, unique: u32) {
    put32(d, 0, len);
    put32(d, 4, lbn);
    put16(d, 8, 0);
    // UDF 2.3.4.3: ADImpUse = flags (2 bytes) + UDF unique id (4 bytes).
    put16(d, 10, 0);
    put32(d, 12, unique);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc_known_value() {
        // ECMA-167 7.2.6 example: CRC of bytes 0x70 0x6A 0x77 is 0x3299.
        assert_eq!(crc16(&[0x70, 0x6A, 0x77]), 0x3299);
    }

    #[test]
    fn tag_checksum() {
        let mut d = vec![0u8; 512];
        tag(&mut d, 8, 33);
        let sum = d[..16].iter().enumerate().filter(|(i, _)| *i != 4).fold(0u8, |s, (_, v)| s.wrapping_add(*v));
        assert_eq!(sum, d[4]);
        assert_eq!(u16::from_le_bytes([d[10], d[11]]), 496);
    }
}
