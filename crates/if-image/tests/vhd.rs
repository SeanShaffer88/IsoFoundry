use if_image::source::SectorSource;
use if_image::vhd::VhdFile;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const SECTOR: usize = 2048;
const FOOTER_SIZE: usize = 512;
static NEXT_FILE: AtomicU64 = AtomicU64::new(0);

struct TempVhd(PathBuf);

impl TempVhd {
    fn new(bytes: &[u8]) -> Self {
        let id = NEXT_FILE.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("if-image-vhd-{}-{id}.vhd", std::process::id()));
        std::fs::write(&path, bytes).unwrap();
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for TempVhd {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn checksum(block: &mut [u8], offset: usize) {
    block[offset..offset + 4].fill(0);
    let sum: u32 = block.iter().map(|byte| u32::from(*byte)).sum();
    block[offset..offset + 4].copy_from_slice(&(!sum).to_be_bytes());
}

fn footer(virtual_size: u64, disk_type: u32, data_offset: u64) -> [u8; FOOTER_SIZE] {
    let mut footer = [0; FOOTER_SIZE];
    footer[..8].copy_from_slice(b"conectix");
    footer[8..12].copy_from_slice(&2u32.to_be_bytes());
    footer[12..16].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    footer[16..24].copy_from_slice(&data_offset.to_be_bytes());
    footer[40..48].copy_from_slice(&virtual_size.to_be_bytes());
    footer[48..56].copy_from_slice(&virtual_size.to_be_bytes());
    footer[60..64].copy_from_slice(&disk_type.to_be_bytes());
    footer[68..84].copy_from_slice(&[0x5a; 16]);
    checksum(&mut footer, 64);
    footer
}

fn fixed_vhd(payload: &[u8]) -> Vec<u8> {
    let mut bytes = payload.to_vec();
    bytes.extend_from_slice(&footer(payload.len() as u64, 2, u64::MAX));
    bytes
}

fn dynamic_vhd() -> Vec<u8> {
    let virtual_size = 8192u64;
    let mut bytes = vec![0; 2048 + 512 + 4096];

    let mut header = [0; 1024];
    header[..8].copy_from_slice(b"cxsparse");
    header[8..16].copy_from_slice(&u64::MAX.to_be_bytes());
    header[16..24].copy_from_slice(&1536u64.to_be_bytes());
    header[24..28].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    header[28..32].copy_from_slice(&2u32.to_be_bytes());
    header[32..36].copy_from_slice(&4096u32.to_be_bytes());
    checksum(&mut header, 36);
    bytes[512..1536].copy_from_slice(&header);

    bytes[1536..1540].copy_from_slice(&4u32.to_be_bytes());
    bytes[1540..1544].copy_from_slice(&u32::MAX.to_be_bytes());
    bytes[2048] = 0xff;
    for (index, byte) in bytes[2560..6656].iter_mut().enumerate() {
        *byte = (index % 251) as u8;
    }
    bytes.extend_from_slice(&footer(virtual_size, 3, 512));
    bytes
}

#[test]
fn reads_fixed_vhd_payload_as_logical_sectors() {
    let payload: Vec<u8> = (0..4096).map(|byte| (byte % 239) as u8).collect();
    let file = TempVhd::new(&fixed_vhd(&payload));
    let mut vhd = VhdFile::open(file.path()).unwrap();

    assert_eq!(vhd.sectors(), 2);
    let mut out = vec![0; SECTOR];
    vhd.read_sectors(1, &mut out).unwrap();
    assert_eq!(out, payload[SECTOR..]);
}

#[test]
fn reports_exact_virtual_size_and_pads_the_last_sector() {
    let payload: Vec<u8> = (0..4608).map(|byte| (byte % 239) as u8).collect();
    let file = TempVhd::new(&fixed_vhd(&payload));
    let mut vhd = VhdFile::open(file.path()).unwrap();

    assert_eq!(vhd.byte_len(), payload.len() as u64);
    assert_eq!(vhd.sectors(), 3);
    let mut out = vec![0xff; SECTOR];
    vhd.read_sectors(2, &mut out).unwrap();
    assert_eq!(&out[..512], &payload[4096..]);
    assert!(out[512..].iter().all(|byte| *byte == 0));
    assert!(vhd.read_sectors(3, &mut out).is_err());
}

#[test]
fn reads_allocated_and_sparse_dynamic_vhd_blocks() {
    let file = TempVhd::new(&dynamic_vhd());
    let mut vhd = VhdFile::open(file.path()).unwrap();
    let mut out = vec![0; 4 * SECTOR];

    vhd.read_sectors(0, &mut out).unwrap();
    assert!(out[..4096].iter().enumerate().all(|(index, byte)| *byte == (index % 251) as u8));
    assert!(out[4096..].iter().all(|byte| *byte == 0));
}

#[test]
fn reads_unallocated_sectors_as_zero_inside_an_allocated_dynamic_block() {
    let mut bytes = dynamic_vhd();
    bytes[2048] = 0xf0;
    let file = TempVhd::new(&bytes);
    let mut vhd = VhdFile::open(file.path()).unwrap();
    let mut out = vec![0xff; 4 * SECTOR];

    vhd.read_sectors(0, &mut out).unwrap();
    assert!(out[..2048].iter().enumerate().all(|(index, byte)| *byte == (index % 251) as u8));
    assert!(out[2048..].iter().all(|byte| *byte == 0));
}

#[test]
fn rejects_a_vhd_with_an_invalid_footer_checksum() {
    let mut bytes = fixed_vhd(&vec![0x31; 4096]);
    let footer_start = bytes.len() - FOOTER_SIZE;
    bytes[footer_start + 10] ^= 1;
    let file = TempVhd::new(&bytes);

    assert!(VhdFile::open(file.path()).is_err());
}

#[test]
fn rejects_a_truncated_vhd_footer() {
    let file = TempVhd::new(&[0; 511]);

    assert!(VhdFile::open(file.path()).is_err());
}

#[test]
fn rejects_a_dynamic_vhd_bat_entry_outside_the_file() {
    let mut bytes = dynamic_vhd();
    bytes[1536..1540].copy_from_slice(&u32::MAX.wrapping_sub(1).to_be_bytes());
    let file = TempVhd::new(&bytes);

    assert!(VhdFile::open(file.path()).is_err());
}

#[test]
fn rejects_overlapping_dynamic_vhd_blocks() {
    let mut bytes = dynamic_vhd();
    bytes[1540..1544].copy_from_slice(&4u32.to_be_bytes());
    let file = TempVhd::new(&bytes);

    assert!(VhdFile::open(file.path()).is_err());
}

#[test]
fn rejects_a_dynamic_vhd_with_an_invalid_header_checksum() {
    let mut bytes = dynamic_vhd();
    bytes[512 + 100] ^= 1;
    let file = TempVhd::new(&bytes);

    assert!(VhdFile::open(file.path()).is_err());
}

#[test]
fn rejects_a_dynamic_vhd_with_a_zero_block_size() {
    let mut bytes = dynamic_vhd();
    bytes[512 + 32..512 + 36].fill(0);
    checksum(&mut bytes[512..1536], 36);
    let file = TempVhd::new(&bytes);

    assert!(VhdFile::open(file.path()).is_err());
}

#[test]
fn rejects_a_dynamic_vhd_with_an_excessive_block_map() {
    let mut bytes = dynamic_vhd();
    bytes[512 + 28..512 + 32].copy_from_slice(&1_048_577u32.to_be_bytes());
    checksum(&mut bytes[512..1536], 36);
    let file = TempVhd::new(&bytes);

    assert!(VhdFile::open(file.path()).is_err());
}

#[test]
fn rejects_differencing_vhds() {
    let mut bytes = fixed_vhd(&vec![0x31; 4096]);
    let footer_start = bytes.len() - FOOTER_SIZE;
    bytes[footer_start + 60..footer_start + 64].copy_from_slice(&4u32.to_be_bytes());
    let mut footer = bytes[footer_start..].to_vec();
    checksum(&mut footer, 64);
    bytes[footer_start..].copy_from_slice(&footer);
    let file = TempVhd::new(&bytes);

    assert!(VhdFile::open(file.path()).is_err());
}
