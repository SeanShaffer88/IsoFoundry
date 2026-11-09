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
fn opens_fixed_and_dynamic_vhds_as_fat_volumes() {
    let dir = std::env::temp_dir().join(format!("if-image-vhd-fat-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let source_image = fat_fixture(FatType::Fat16).image;

    for (index, dynamic) in [false, true].into_iter().enumerate() {
        let path = dir.join(format!("fat-{index}.vhd"));
        let mut source = MemSource(source_image.clone());
        if dynamic {
            if_image::vhd_write::write_dynamic(&mut source, &path, |_, _| true).unwrap();
        } else {
            if_image::vhd_write::write_fixed(&mut source, &path, |_, _| true).unwrap();
        }

        let (format, source) = if_image::container::open(&path).unwrap();
        assert_eq!(format, if_image::container::Format::Vhd);
        let mut volume = Volume::open(source, None).unwrap();
        assert_eq!(volume.fs.kind().name(), "FAT");
        assert_eq!(volume.fs.label(), "FAT TEST");
        let file = volume.lookup("A very long filename.txt").unwrap();
        let mut contents = vec![0; file.size as usize];
        volume.read(&file, 0, &mut contents).unwrap();
        assert_eq!(&contents[..512], vec![b'a'; 512]);
        assert_eq!(&contents[512..], vec![b'b'; 88]);

        std::fs::remove_file(path).unwrap();
    }

    std::fs::remove_dir(dir).unwrap();
}

#[test]
fn edits_fixed_and_dynamic_vhds_as_new_vhd_images() {
    let dir = std::env::temp_dir().join(format!("if-image-vhd-edit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let original = fat_fixture(FatType::Fat16).image;
    let added_file = dir.join("added.txt");
    std::fs::write(&added_file, b"added through VHD editing").unwrap();

    for (index, dynamic) in [false, true].into_iter().enumerate() {
        let source_path = dir.join(format!("source-{index}.vhd"));
        let output_path = dir.join(format!("edited-{index}.vhd"));
        let mut source = MemSource(original.clone());
        if dynamic {
            if_image::vhd_write::write_dynamic(&mut source, &source_path, |_, _| true).unwrap();
        } else {
            if_image::vhd_write::write_fixed(&mut source, &source_path, |_, _| true).unwrap();
        }
        let original_vhd = std::fs::read(&source_path).unwrap();

        let mut project = if_image::edit::Project::open(&source_path).unwrap();
        project
            .apply(&if_image::edit::Edit::Rename { path: "A very long filename.txt".into(), to: "renamed.txt".into() })
            .unwrap();
        project.apply(&if_image::edit::Edit::Remove("SUBDIR/NESTED.TXT".into())).unwrap();
        project.apply(&if_image::edit::Edit::MkDir("created".into())).unwrap();
        project.apply(&if_image::edit::Edit::Add { dir: String::new(), from: added_file.clone() }).unwrap();
        project.apply(&if_image::edit::Edit::Label("EDITED".into())).unwrap();
        project.save(&output_path, |_, _| true).unwrap();

        assert_eq!(if_image::vhd::VhdFile::open(&output_path).unwrap().is_dynamic(), dynamic);
        let (format, source) = if_image::container::open(&output_path).unwrap();
        assert_eq!(format, if_image::container::Format::Vhd);
        let mut volume = Volume::open(source, None).unwrap();
        assert_eq!(volume.fs.label(), "EDITED");
        assert!(volume.lookup("renamed.txt").is_ok());
        assert!(volume.lookup("created").unwrap().dir);
        assert!(volume.lookup("SUBDIR/NESTED.TXT").is_err());
        let added = volume.lookup("added.txt").unwrap();
        let mut added_bytes = vec![0; added.size as usize];
        volume.read(&added, 0, &mut added_bytes).unwrap();
        assert_eq!(added_bytes, b"added through VHD editing");
        assert_eq!(std::fs::read(&source_path).unwrap(), original_vhd);

        let cancelled = dir.join(format!("cancelled-{index}.vhd"));
        let mut project = if_image::edit::Project::open(&source_path).unwrap();
        project.apply(&if_image::edit::Edit::Label("CANCEL".into())).unwrap();
        assert!(matches!(project.save(&cancelled, |_, _| false), Err(if_image::Error::Cancelled)));
        assert!(!cancelled.exists());

        let existing = dir.join(format!("existing-{index}.vhd"));
        std::fs::write(&existing, b"preserve").unwrap();
        let mut project = if_image::edit::Project::open(&source_path).unwrap();
        project.apply(&if_image::edit::Edit::Label("COLLISION".into())).unwrap();
        assert!(project.save(&existing, |_, _| true).is_err());
        assert_eq!(std::fs::read(&existing).unwrap(), b"preserve");

        std::fs::remove_file(source_path).unwrap();
        std::fs::remove_file(output_path).unwrap();
        std::fs::remove_file(existing).unwrap();
    }

    std::fs::remove_file(added_file).unwrap();
    std::fs::remove_dir(dir).unwrap();
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
fn edits_a_fat_image_without_modifying_the_source() {
    use if_image::edit::{Edit, Project};

    let dir = std::env::temp_dir().join(format!("if-image-fat-edit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("added.txt"), b"new file").unwrap();
    std::fs::write(dir.join("long filename alpha.txt"), b"alpha").unwrap();
    std::fs::write(dir.join("long filename beta.txt"), b"beta").unwrap();
    std::fs::write(dir.join("duplicate.txt"), b"first").unwrap();
    std::fs::create_dir(dir.join("replacement")).unwrap();
    std::fs::write(dir.join("replacement").join("duplicate.txt"), b"second").unwrap();

    for (index, kind) in [FatType::Fat12, FatType::Fat16, FatType::Fat32].into_iter().enumerate() {
        let fixture = fat_fixture(kind);
        let path = dir.join(format!("source-{index}.img"));
        let output = dir.join(format!("edited-{index}.img"));
        std::fs::write(&path, &fixture.image).unwrap();

        let mut project = Project::open(&path).unwrap();
        project.apply(&Edit::Rename { path: "A very long filename.txt".into(), to: "renamed.txt".into() }).unwrap();
        project.apply(&Edit::Remove("SUBDIR/NESTED.TXT".into())).unwrap();
        project.apply(&Edit::MkDir("New/Nested".into())).unwrap();
        project.apply(&Edit::Add { dir: "New/Nested".into(), from: dir.join("added.txt") }).unwrap();
        project.apply(&Edit::Add { dir: "".into(), from: dir.join("long filename alpha.txt") }).unwrap();
        project.apply(&Edit::Add { dir: "".into(), from: dir.join("long filename beta.txt") }).unwrap();
        project.apply(&Edit::Add { dir: "".into(), from: dir.join("duplicate.txt") }).unwrap();
        project.apply(&Edit::Add { dir: "".into(), from: dir.join("replacement").join("duplicate.txt") }).unwrap();
        project.apply(&Edit::Label("EDITED".into())).unwrap();
        project.save(&output, |_, _| true).unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), fixture.image);
        let mut volume = Volume::open(Box::new(MemSource(std::fs::read(&output).unwrap())), None).unwrap();
        assert_eq!(volume.fs.label(), "EDITED");
        let renamed = volume.lookup("renamed.txt").unwrap();
        let mut bytes = vec![0; renamed.size as usize];
        volume.read(&renamed, 0, &mut bytes).unwrap();
        assert_eq!(&bytes[..512], vec![b'a'; 512]);
        assert_eq!(&bytes[512..], vec![b'b'; 88]);
        assert!(volume.lookup("SUBDIR/NESTED.TXT").is_err());
        assert_eq!(volume.lookup("New/Nested/added.txt").unwrap().size, 8);
        for (name, expected) in [
            ("long filename alpha.txt", b"alpha".as_slice()),
            ("long filename beta.txt", b"beta".as_slice()),
            ("duplicate.txt", b"second".as_slice()),
        ] {
            let entry = volume.lookup(name).unwrap();
            let mut actual = vec![0; entry.size as usize];
            volume.read(&entry, 0, &mut actual).unwrap();
            assert_eq!(actual, expected);
        }
    }

    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn edits_a_partitioned_fat_volume_and_preserves_the_mbr() {
    use if_image::edit::{Edit, Project};

    let dir = std::env::temp_dir().join(format!("if-image-fat-mbr-edit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let source_bytes = partitioned(fat_fixture(FatType::Fat16));
    let source = dir.join("source.img");
    let output = dir.join("edited.img");
    std::fs::write(&source, &source_bytes).unwrap();

    let mut project = Project::open(&source).unwrap();
    project.apply(&Edit::Rename { path: "A very long filename.txt".into(), to: "changed.txt".into() }).unwrap();
    project.save(&output, |_, _| true).unwrap();

    let result = std::fs::read(&output).unwrap();
    assert_eq!(&result[..512], &source_bytes[..512]);
    let mut volume = Volume::open(Box::new(MemSource(result)), None).unwrap();
    assert!(volume.lookup("changed.txt").is_ok());
    assert_eq!(std::fs::read(&source).unwrap(), source_bytes);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn fat_edit_refuses_existing_destinations_and_cleans_up_on_cancel() {
    use if_image::edit::Project;

    let dir = std::env::temp_dir().join(format!("if-image-fat-safety-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("source.img");
    std::fs::write(&source, fat_fixture(FatType::Fat16).image).unwrap();
    let existing = dir.join("existing.img");
    std::fs::write(&existing, b"preserve").unwrap();

    let error = match Project::open(&source).unwrap().save(&existing, |_, _| true) {
        Ok(_) => panic!("editing should not overwrite an existing destination"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("destination already exists"));
    assert_eq!(std::fs::read(&existing).unwrap(), b"preserve");

    let cancelled = dir.join("cancelled.img");
    assert!(Project::open(&source).unwrap().save(&cancelled, |_, _| false).is_err());
    assert!(!cancelled.exists());
    assert!(!dir.join("cancelled.img.part").exists());
    std::fs::remove_dir_all(dir).unwrap();
}
