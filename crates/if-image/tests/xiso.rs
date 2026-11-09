use if_image::fs::{FsKind, Volume};
use if_image::source::MemSource;

const SECTOR: usize = if_image::SECTOR;

fn xiso_with_one_file() -> MemSource {
    let mut image = vec![0u8; 64 * SECTOR];
    let header = 32 * SECTOR;
    image[header..header + 20].copy_from_slice(b"MICROSOFT*XBOX*MEDIA");
    image[header + 20..header + 24].copy_from_slice(&40u32.to_le_bytes());
    image[header + 24..header + 28].copy_from_slice(&(SECTOR as u32).to_le_bytes());
    image[header + 0x7ec..header + 0x7ec + 20].copy_from_slice(b"MICROSOFT*XBOX*MEDIA");

    let root = 40 * SECTOR;
    image[root..root + SECTOR].fill(0xff);
    image[root..root + 4].fill(0);
    image[root + 4..root + 8].copy_from_slice(&41u32.to_le_bytes());
    image[root + 8..root + 12].copy_from_slice(&5u32.to_le_bytes());
    image[root + 12] = 0x20;
    image[root + 13] = 9;
    image[root + 14..root + 23].copy_from_slice(b"hello.txt");
    image[41 * SECTOR..41 * SECTOR + 5].copy_from_slice(b"hello");
    MemSource(image)
}

#[test]
fn opens_xdvdfs_and_reads_file_extents() {
    let mut volume = Volume::open(Box::new(xiso_with_one_file()), None).unwrap();

    assert_eq!(volume.fs.kind(), FsKind::Xdvdfs);
    assert_eq!(volume.found, vec![FsKind::Xdvdfs]);
    let file = volume.lookup("hello.txt").unwrap();
    assert_eq!(file.size, 5);

    let mut contents = [0; 5];
    volume.read(&file, 0, &mut contents).unwrap();
    assert_eq!(&contents, b"hello");
}

#[test]
fn rejects_xdvdfs_root_extent_outside_image() {
    let mut source = xiso_with_one_file();
    let root_sector_offset = 32 * SECTOR + 20;
    source.0[root_sector_offset..root_sector_offset + 4].copy_from_slice(&u32::MAX.to_le_bytes());

    assert!(Volume::open(Box::new(source), None).is_err());
}

#[test]
fn rejects_invalid_xdvdfs_filename() {
    let mut source = xiso_with_one_file();
    let filename_offset = 40 * SECTOR + 14;
    source.0[filename_offset] = b'/';
    let mut volume = Volume::open(Box::new(source), None).unwrap();

    assert!(volume.list(&volume.root()).is_err());
}

#[test]
fn rejects_invalid_xdvdfs_descriptor_signature() {
    let mut source = xiso_with_one_file();
    source.0[32 * SECTOR + 0x7ec] = b'X';

    assert!(Volume::open(Box::new(source), None).is_err());
}

#[test]
fn reads_files_in_nested_xdvdfs_directories() {
    let mut source = xiso_with_one_file();
    let root = 40 * SECTOR;
    source.0[root + 17..root + SECTOR].fill(0xff);
    source.0[root + 4..root + 8].copy_from_slice(&42u32.to_le_bytes());
    source.0[root + 8..root + 12].copy_from_slice(&(SECTOR as u32).to_le_bytes());
    source.0[root + 12] = 0x10;
    source.0[root + 13] = 3;
    source.0[root + 14..root + 17].copy_from_slice(b"sub");

    let child = 42 * SECTOR;
    source.0[child..child + SECTOR].fill(0xff);
    source.0[child..child + 4].fill(0);
    source.0[child + 4..child + 8].copy_from_slice(&43u32.to_le_bytes());
    source.0[child + 8..child + 12].copy_from_slice(&5u32.to_le_bytes());
    source.0[child + 12] = 0x20;
    source.0[child + 13] = 9;
    source.0[child + 14..child + 23].copy_from_slice(b"child.txt");
    source.0[43 * SECTOR..43 * SECTOR + 5].copy_from_slice(b"child");

    let mut volume = Volume::open(Box::new(source), None).unwrap();
    let file = volume.lookup("sub/child.txt").unwrap();
    let mut contents = [0; 5];

    volume.read(&file, 0, &mut contents).unwrap();

    assert_eq!(&contents, b"child");
}

#[test]
fn lists_empty_xdvdfs_directory() {
    let mut source = xiso_with_one_file();
    let root = 40 * SECTOR;
    source.0[root + 17..root + SECTOR].fill(0xff);
    source.0[root + 4..root + 8].copy_from_slice(&42u32.to_le_bytes());
    source.0[root + 8..root + 12].copy_from_slice(&0u32.to_le_bytes());
    source.0[root + 12] = 0x10;
    source.0[root + 13] = 3;
    source.0[root + 14..root + 17].copy_from_slice(b"sub");

    let mut volume = Volume::open(Box::new(source), None).unwrap();
    let directory = volume.lookup("sub").unwrap();

    assert!(volume.list(&directory).unwrap().is_empty());
}
