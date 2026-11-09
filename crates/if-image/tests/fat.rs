use if_image::fs::Volume;
use if_image::source::MemSource;

#[derive(Clone, Copy)]
enum FatType {
    Fat12,
    Fat16,
    Fat32,
}

struct Fixture {
    image: Vec<u8>,
    kind: FatType,
    cluster_count: u32,
}

fn set_fat_entry(fat: &mut [u8], kind: FatType, cluster: u32, value: u32) {
    match kind {
        FatType::Fat12 => {
            let offset = cluster as usize + cluster as usize / 2;
            let mut pair = u16::from_le_bytes([fat[offset], fat[offset + 1]]);
            pair = if cluster & 1 == 0 {
                (pair & 0xf000) | (value as u16 & 0x0fff)
            } else {
                (pair & 0x000f) | ((value as u16 & 0x0fff) << 4)
            };
            fat[offset..offset + 2].copy_from_slice(&pair.to_le_bytes());
        }
        FatType::Fat16 => {
            let offset = cluster as usize * 2;
            fat[offset..offset + 2].copy_from_slice(&(value as u16).to_le_bytes());
        }
        FatType::Fat32 => {
            let offset = cluster as usize * 4;
            fat[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
    }
}

fn checksum(short: &[u8; 11]) -> u8 {
    short
        .iter()
        .fold(0u8, |sum, byte| (if sum & 1 != 0 { 0x80u8 } else { 0u8 }).wrapping_add(sum >> 1).wrapping_add(*byte))
}

fn fat_fixture(kind: FatType) -> Fixture {
    let (total_sectors, reserved, fat_sectors, root_entries) = match kind {
        FatType::Fat12 => (2880usize, 1usize, 9usize, 32usize),
        FatType::Fat16 => (8192, 1, 32, 32),
        FatType::Fat32 => (66_594, 32, 513, 0),
    };
    let fats = 2usize;
    let root_sectors = (root_entries * 32).div_ceil(512);
    let first_data_sector = reserved + fats * fat_sectors + root_sectors;
    let cluster_count = (total_sectors - first_data_sector) as u32;
    let mut image = vec![0; (total_sectors * 512).div_ceil(2048) * 2048];
    let boot = &mut image[..512];
    boot[..3].copy_from_slice(&[0xeb, 0x3c, 0x90]);
    boot[3..11].copy_from_slice(b"ISOFNDRY");
    boot[11..13].copy_from_slice(&512u16.to_le_bytes());
    boot[13] = 1;
    boot[14..16].copy_from_slice(&(reserved as u16).to_le_bytes());
    boot[16] = fats as u8;
    boot[17..19].copy_from_slice(&(root_entries as u16).to_le_bytes());
    if total_sectors < 65_536 {
        boot[19..21].copy_from_slice(&(total_sectors as u16).to_le_bytes());
    } else {
        boot[32..36].copy_from_slice(&(total_sectors as u32).to_le_bytes());
    }
    boot[21] = 0xf8;
    if matches!(kind, FatType::Fat32) {
        boot[36..40].copy_from_slice(&(fat_sectors as u32).to_le_bytes());
        boot[44..48].copy_from_slice(&2u32.to_le_bytes());
        boot[66] = 0x29;
        boot[71..82].copy_from_slice(b"FAT TEST   ");
    } else {
        boot[22..24].copy_from_slice(&(fat_sectors as u16).to_le_bytes());
        boot[38] = 0x29;
        boot[43..54].copy_from_slice(b"FAT TEST   ");
    }
    boot[510..512].copy_from_slice(&[0x55, 0xaa]);

    for fat_index in 0..fats {
        let start = (reserved + fat_index * fat_sectors) * 512;
        let fat = &mut image[start..start + fat_sectors * 512];
        match kind {
            FatType::Fat12 => {
                fat[..3].copy_from_slice(&[0xf8, 0xff, 0xff]);
                set_fat_entry(fat, kind, 3, 5);
                set_fat_entry(fat, kind, 5, 0xfff);
                set_fat_entry(fat, kind, 7, 0xfff);
                set_fat_entry(fat, kind, 8, 0xfff);
            }
            FatType::Fat16 => {
                set_fat_entry(fat, kind, 0, 0xfff8);
                set_fat_entry(fat, kind, 1, 0xffff);
                set_fat_entry(fat, kind, 3, 5);
                set_fat_entry(fat, kind, 5, 0xffff);
                set_fat_entry(fat, kind, 7, 0xffff);
                set_fat_entry(fat, kind, 8, 0xffff);
            }
            FatType::Fat32 => {
                set_fat_entry(fat, kind, 0, 0x0fff_fff8);
                set_fat_entry(fat, kind, 1, 0xffff_ffff);
                set_fat_entry(fat, kind, 2, 0x0fff_ffff);
                set_fat_entry(fat, kind, 3, 5);
                set_fat_entry(fat, kind, 5, 0x0fff_ffff);
                set_fat_entry(fat, kind, 7, 0x0fff_ffff);
                set_fat_entry(fat, kind, 8, 0x0fff_ffff);
            }
        }
    }

    let root_offset =
        if matches!(kind, FatType::Fat32) { first_data_sector * 512 } else { (reserved + fats * fat_sectors) * 512 };
    let root =
        &mut image[root_offset..root_offset + if matches!(kind, FatType::Fat32) { 512 } else { root_sectors * 512 }];
    let short: [u8; 11] = *b"AVERYL~1TXT";
    let name: Vec<u16> = "A very long filename.txt".encode_utf16().collect();
    let lfn_count = name.len().div_ceil(13);
    for sequence in (1..=lfn_count).rev() {
        let start = (lfn_count - sequence) * 32;
        let lfn = &mut root[start..start + 32];
        lfn[0] = sequence as u8 | if sequence == lfn_count { 0x40 } else { 0 };
        lfn[11] = 0x0f;
        lfn[13] = checksum(&short);
        let mut units = [0xffffu16; 13];
        let unit_start = (sequence - 1) * 13;
        let count = (name.len() - unit_start).min(13);
        units[..count].copy_from_slice(&name[unit_start..unit_start + count]);
        if count < 13 {
            units[count] = 0;
        }
        for (index, unit) in units.iter().enumerate() {
            let offset = match index {
                0..=4 => 1 + index * 2,
                5..=10 => 14 + (index - 5) * 2,
                _ => 28 + (index - 11) * 2,
            };
            lfn[offset..offset + 2].copy_from_slice(&unit.to_le_bytes());
        }
    }
    let entry_offset = lfn_count * 32;
    let entry = &mut root[entry_offset..entry_offset + 32];
    entry[..11].copy_from_slice(&short);
    entry[11] = 0x20;
    entry[26..28].copy_from_slice(&3u16.to_le_bytes());
    entry[28..32].copy_from_slice(&600u32.to_le_bytes());
    let nested = &mut root[entry_offset + 32..entry_offset + 64];
    nested[..11].copy_from_slice(b"SUBDIR     ");
    nested[11] = 0x10;
    nested[26..28].copy_from_slice(&7u16.to_le_bytes());

    let first_file_sector = first_data_sector + (3 - 2);
    let second_file_sector = first_data_sector + (5 - 2);
    image[first_file_sector * 512..first_file_sector * 512 + 512].fill(b'a');
    image[second_file_sector * 512..second_file_sector * 512 + 88].fill(b'b');
    let subdir_sector = first_data_sector + (7 - 2);
    let subdir = &mut image[subdir_sector * 512..(subdir_sector + 1) * 512];
    subdir[..11].copy_from_slice(b"NESTED  TXT");
    subdir[11] = 0x20;
    subdir[26..28].copy_from_slice(&8u16.to_le_bytes());
    subdir[28..32].copy_from_slice(&3u32.to_le_bytes());
    let nested_file_sector = first_data_sector + (8 - 2);
    image[nested_file_sector * 512..nested_file_sector * 512 + 3].copy_from_slice(b"sub");
    Fixture { image, kind, cluster_count }
}

fn partitioned(fixture: Fixture) -> Vec<u8> {
    let start_sector = 1u32;
    let length_sectors = (fixture.image.len() / 512) as u32;
    let mut disk = vec![0; (512 + fixture.image.len()).div_ceil(2048) * 2048];
    let entry = &mut disk[446..462];
    entry[4] = match fixture.kind {
        FatType::Fat12 => 0x01,
        FatType::Fat16 => 0x16,
        FatType::Fat32 => 0x0c,
    };
    entry[8..12].copy_from_slice(&start_sector.to_le_bytes());
    entry[12..16].copy_from_slice(&length_sectors.to_le_bytes());
    disk[510..512].copy_from_slice(&[0x55, 0xaa]);
    let start = start_sector as usize * 512;
    disk[start..start + fixture.image.len()].copy_from_slice(&fixture.image);
    disk
}

fn read_long_file(mut image: Vec<u8>) -> Vec<u8> {
    let mut volume = Volume::open(Box::new(MemSource(std::mem::take(&mut image))), None).unwrap();
    assert_eq!(volume.fs.kind().name(), "FAT");
    assert_eq!(volume.fs.label(), "FAT TEST");
    let file = volume.lookup("A very long filename.txt").unwrap();
    let mut bytes = vec![0; file.size as usize];
    volume.read(&file, 0, &mut bytes).unwrap();
    bytes
}

#[test]
fn reads_fat12_fat16_and_fat32_long_names_and_fragmented_files() {
    for kind in [FatType::Fat12, FatType::Fat16, FatType::Fat32] {
        let fixture = fat_fixture(kind);
        let mut volume = Volume::open(Box::new(MemSource(fixture.image)), None).unwrap();
        assert_eq!(volume.fs.kind().name(), "FAT");
        assert_eq!(volume.fs.label(), "FAT TEST");
        let file = volume.lookup("A very long filename.txt").unwrap();
        let mut bytes = vec![0; file.size as usize];
        volume.read(&file, 0, &mut bytes).unwrap();
        assert_eq!(&bytes[..512], vec![b'a'; 512]);
        assert_eq!(&bytes[512..], vec![b'b'; 88]);
        let nested = volume.lookup("SUBDIR/NESTED.TXT").unwrap();
        let mut nested_bytes = vec![0; nested.size as usize];
        volume.read(&nested, 0, &mut nested_bytes).unwrap();
        assert_eq!(nested_bytes, b"sub");
    }
}

#[test]
fn reads_a_fat_partition_at_a_non_2048_byte_aligned_mbr_offset() {
    let disk = partitioned(fat_fixture(FatType::Fat16));
    let bytes = read_long_file(disk);
    assert_eq!(&bytes[..512], vec![b'a'; 512]);
    assert_eq!(&bytes[512..], vec![b'b'; 88]);
}

#[test]
fn rejects_fat_file_chains_that_point_outside_the_volume() {
    let mut fixture = fat_fixture(FatType::Fat16);
    let first_fat = 512;
    let invalid = fixture.cluster_count + 2;
    set_fat_entry(&mut fixture.image[first_fat..], fixture.kind, 3, invalid);

    let mut volume = Volume::open(Box::new(MemSource(fixture.image)), None).unwrap();
    assert!(volume.lookup("Long File.txt").is_err());
}

#[test]
fn rejects_cyclic_fat_file_chains() {
    let mut fixture = fat_fixture(FatType::Fat16);
    let first_fat = 512;
    set_fat_entry(&mut fixture.image[first_fat..], fixture.kind, 3, 5);
    set_fat_entry(&mut fixture.image[first_fat..], fixture.kind, 5, 3);
    let root = (1 + 2 * 32) * 512;
    fixture.image[root + 64 + 28..root + 64 + 32].copy_from_slice(&1200u32.to_le_bytes());

    let mut volume = Volume::open(Box::new(MemSource(fixture.image)), None).unwrap();
    assert!(volume.lookup("A very long filename.txt").is_err());
}

#[test]
fn rejects_fat_volumes_with_invalid_cluster_geometry() {
    let mut fixture = fat_fixture(FatType::Fat12);
    fixture.image[13] = 3;

    assert!(Volume::open(Box::new(MemSource(fixture.image)), None).is_err());
}

#[test]
fn rejects_fat_mbr_partitions_outside_the_image() {
    let mut disk = vec![0; 2048];
    disk[450] = 0x0c;
    disk[454..458].copy_from_slice(&2u32.to_le_bytes());
    disk[458..462].copy_from_slice(&8u32.to_le_bytes());
    disk[510..512].copy_from_slice(&[0x55, 0xaa]);

    assert!(Volume::open(Box::new(MemSource(disk)), None).is_err());
}

#[test]
fn reports_exfat_as_unsupported() {
    let mut image = vec![0; 2048];
    image[..3].copy_from_slice(&[0xeb, 0x76, 0x90]);
    image[3..11].copy_from_slice(b"EXFAT   ");

    let error = Volume::open(Box::new(MemSource(image)), None).err().unwrap().to_string();
    assert!(error.contains("exFAT volumes are not supported"));
}

#[test]
fn refuses_to_edit_fat_images_until_fat_writing_is_supported() {
    let path = std::env::temp_dir().join(format!("if-image-fat-readonly-{}.img", std::process::id()));
    std::fs::write(&path, fat_fixture(FatType::Fat16).image).unwrap();

    let error = if_image::edit::Project::open(&path).err().unwrap().to_string();
    assert!(error.contains("FAT images are read-only"));
    std::fs::remove_file(path).unwrap();
}
