//! Raw CD sector layout (ECMA-130): sync, header, EDC and the P/Q
//! Reed-Solomon ECC of Mode 1 and Mode 2 Form 1 sectors, MSF addresses
//! and the Q subchannel.

pub const RAW: usize = 2352;
pub const SUB: usize = 96;
pub const SYNC: [u8; 12] = [0x00, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00];
/// Sectors before LBA 0 (the 2-second lead-in offset).
pub const MSF_OFFSET: i64 = 150;

fn edc_table() -> &'static [u32; 256] {
    static T: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut v = i as u32;
            for _ in 0..8 {
                v = if v & 1 != 0 { (v >> 1) ^ 0xD801_8001 } else { v >> 1 };
            }
            *e = v;
        }
        t
    })
}

/// CD-ROM EDC: CRC-32 with polynomial 0x8001801B (reflected), no inversion.
pub fn edc(data: &[u8]) -> u32 {
    let t = edc_table();
    data.iter().fold(0u32, |c, &b| (c >> 8) ^ t[((c ^ b as u32) & 0xFF) as usize])
}

fn ecc_luts() -> &'static ([u8; 256], [u8; 256]) {
    static L: std::sync::OnceLock<([u8; 256], [u8; 256])> = std::sync::OnceLock::new();
    L.get_or_init(|| {
        let (mut f, mut b) = ([0u8; 256], [0u8; 256]);
        for i in 0..256usize {
            let j = ((i << 1) ^ if i & 0x80 != 0 { 0x11D } else { 0 }) as u8;
            f[i] = j;
            b[i ^ j as usize] = i as u8;
        }
        (f, b)
    })
}

/// One ECC block (P or Q) over sector bytes 12.. into `out`.
fn ecc_block(
    s: &[u8; RAW],
    major_count: usize,
    minor_count: usize,
    major_mult: usize,
    minor_inc: usize,
    out: usize,
) -> Vec<u8> {
    let (f, b) = ecc_luts();
    let src = &s[12..];
    let size = major_count * minor_count;
    let mut res = vec![0u8; major_count * 2];
    for major in 0..major_count {
        let mut index = (major >> 1) * major_mult + (major & 1);
        let (mut ea, mut eb) = (0u8, 0u8);
        for _ in 0..minor_count {
            let t = src[index];
            index += minor_inc;
            if index >= size {
                index -= size;
            }
            ea ^= t;
            eb ^= t;
            ea = f[ea as usize];
        }
        ea = b[(f[ea as usize] ^ eb) as usize];
        res[major] = ea;
        res[major + major_count] = ea ^ eb;
    }
    let _ = out;
    res
}

fn ecc_generate(s: &mut [u8; RAW], zero_address: bool) {
    let saved = [s[12], s[13], s[14], s[15]];
    if zero_address {
        s[12..16].fill(0);
    }
    let p = ecc_block(s, 86, 24, 2, 86, 0x81C);
    s[0x81C..0x81C + 172].copy_from_slice(&p);
    let q = ecc_block(s, 52, 43, 86, 88, 0x8C8);
    s[0x8C8..0x8C8 + 104].copy_from_slice(&q);
    if zero_address {
        s[12..16].copy_from_slice(&saved);
    }
}

fn bcd(v: u8) -> u8 {
    ((v / 10) << 4) | (v % 10)
}

pub fn from_bcd(v: u8) -> u8 {
    (v >> 4) * 10 + (v & 0x0F)
}

/// (minute, second, frame) of an absolute sector address (LBA + 150).
pub fn msf(abs: i64) -> (u8, u8, u8) {
    let a = abs.max(0);
    ((a / 4500) as u8, ((a / 75) % 60) as u8, (a % 75) as u8)
}

pub fn msf_to_lba(m: u8, s: u8, f: u8) -> i64 {
    (m as i64 * 60 + s as i64) * 75 + f as i64 - MSF_OFFSET
}

fn header(s: &mut [u8; RAW], lba: i64, mode: u8) {
    s[..12].copy_from_slice(&SYNC);
    let (m, sec, f) = msf(lba + MSF_OFFSET);
    s[12] = bcd(m);
    s[13] = bcd(sec);
    s[14] = bcd(f);
    s[15] = mode;
}

/// A complete Mode 1 sector for 2048 bytes of user data.
pub fn mode1(lba: i64, user: &[u8]) -> [u8; RAW] {
    let mut s = [0u8; RAW];
    header(&mut s, lba, 1);
    s[16..2064].copy_from_slice(&user[..2048]);
    let e = edc(&s[..2064]);
    s[2064..2068].copy_from_slice(&e.to_le_bytes());
    ecc_generate(&mut s, false);
    s
}

/// A complete Mode 2 Form 1 sector (XA) with the given 4-byte subheader.
pub fn mode2_form1(lba: i64, subheader: [u8; 4], user: &[u8]) -> [u8; RAW] {
    let mut s = [0u8; RAW];
    header(&mut s, lba, 2);
    s[16..20].copy_from_slice(&subheader);
    s[20..24].copy_from_slice(&subheader);
    s[24..2072].copy_from_slice(&user[..2048]);
    let e = edc(&s[16..2072]);
    s[2072..2076].copy_from_slice(&e.to_le_bytes());
    ecc_generate(&mut s, true);
    s
}

/// What kind of data a raw sector holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectorMode {
    Mode0,
    Mode1,
    Mode2Form1,
    Mode2Form2,
    Mode2Formless,
    Unknown,
}

pub fn sector_mode(s: &[u8]) -> SectorMode {
    if s.len() < RAW || s[..12] != SYNC {
        return SectorMode::Unknown;
    }
    match s[15] {
        0 => SectorMode::Mode0,
        1 => SectorMode::Mode1,
        2 if s[16..20] == s[20..24] && s[18] & 0x20 != 0 => SectorMode::Mode2Form2,
        2 if s[16..20] == s[20..24] => SectorMode::Mode2Form1,
        2 => SectorMode::Mode2Formless,
        _ => SectorMode::Unknown,
    }
}

/// The 2048 user bytes of a raw data sector (Mode 1 or Mode 2 Form 1).
pub fn user_data(s: &[u8]) -> Option<&[u8]> {
    match sector_mode(s) {
        SectorMode::Mode1 => Some(&s[16..2064]),
        SectorMode::Mode2Form1 => Some(&s[24..2072]),
        _ => None,
    }
}

/// True when the stored EDC matches the sector contents.
pub fn edc_ok(s: &[u8]) -> bool {
    match sector_mode(s) {
        SectorMode::Mode1 => edc(&s[..2064]).to_le_bytes() == s[2064..2068],
        SectorMode::Mode2Form1 => edc(&s[16..2072]).to_le_bytes() == s[2072..2076],
        SectorMode::Mode2Form2 => {
            let stored = &s[2348..2352];
            stored == [0, 0, 0, 0] || edc(&s[16..2348]).to_le_bytes() == stored
        }
        _ => false,
    }
}

/// CRC-16 of the Q subchannel (CCITT, inverted), ECMA-130 22.3.6.
fn q_crc(q: &[u8]) -> u16 {
    !crate::write::udfw::crc16(q)
}

/// 96-byte subchannel for one sector, in CloneCD layout (P, Q, then R–W,
/// 12 bytes each, not interleaved). P is set in the pregap; Q carries the
/// mode-1 position.
pub fn subchannel(control: u8, track: u8, index: u8, rel: i64, lba: i64) -> [u8; SUB] {
    let mut sub = [0u8; SUB];
    if index == 0 {
        sub[..12].fill(0xFF);
    }
    let q = &mut sub[12..24];
    q[0] = (control << 4) | 1;
    q[1] = bcd(track);
    q[2] = bcd(index);
    let (rm, rs, rf) = msf(rel.abs());
    q[3] = bcd(rm);
    q[4] = bcd(rs);
    q[5] = bcd(rf);
    q[6] = 0;
    let (am, as_, af) = msf(lba + MSF_OFFSET);
    q[7] = bcd(am);
    q[8] = bcd(as_);
    q[9] = bcd(af);
    let crc = q_crc(&q[..10]);
    q[10..12].copy_from_slice(&crc.to_be_bytes());
    sub
}

/// Checks a subchannel's Q CRC.
pub fn q_ok(sub: &[u8]) -> bool {
    let q = &sub[12..24];
    q_crc(&q[..10]).to_be_bytes() == q[10..12]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn msf_conversions() {
        assert_eq!(msf(150), (0, 2, 0));
        assert_eq!(msf_to_lba(0, 2, 0), 0);
        assert_eq!(msf_to_lba(79, 59, 74), 359_849);
    }

    #[test]
    fn mode1_roundtrip_and_checks() {
        let user: Vec<u8> = (0..2048).map(|i| (i * 13 % 256) as u8).collect();
        let s = mode1(16, &user);
        assert_eq!(sector_mode(&s), SectorMode::Mode1);
        assert_eq!(user_data(&s).unwrap(), &user[..]);
        assert!(edc_ok(&s));
        assert_eq!(&s[12..16], &[0x00, 0x02, 0x16, 0x01]);
        let mut bad = s;
        bad[100] ^= 1;
        assert!(!edc_ok(&bad));
    }

    /// The all-zero Mode 1 sector at LBA 0 has P/Q parity of zero except
    /// where the header contributes; regenerating twice is stable and the
    /// ECC changes with the address.
    #[test]
    fn ecc_depends_on_address_only_for_mode1() {
        let z = vec![0u8; 2048];
        let a = mode1(0, &z);
        let b = mode1(1, &z);
        assert_ne!(a[0x81C..], b[0x81C..]);
        let x1 = mode2_form1(0, [0, 0, 8, 0], &z);
        let x2 = mode2_form1(1, [0, 0, 8, 0], &z);
        assert_eq!(x1[0x81C..], x2[0x81C..]);
        assert!(edc_ok(&x1));
        assert_eq!(user_data(&x1).unwrap(), &z[..]);
    }

    #[test]
    fn q_subchannel_crc() {
        let s = subchannel(4, 1, 1, 0, 0);
        assert!(q_ok(&s));
        assert_eq!(s[12], 0x41);
        assert_eq!(&s[19..22], &[0x00, 0x02, 0x00]);
    }
}
