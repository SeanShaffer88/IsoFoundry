//! Builds images with the writer and reads them back through every file
//! system. External-tool checks (isoinfo, 7z, udfinfo, xorriso) are run by
//! hand and recorded in AUDIT.md.

use if_image::eltorito::Platform;
use if_image::fs::{FsKind, Volume};
use if_image::source::IsoFile;
use if_image::write::{build, BootImage, Node, Options, Source};
use std::path::PathBuf;

const UNICODE: &str = "Ünïcödé name – long file name.txt";

fn tree() -> Node {
    let big: Vec<u8> = (0..5000u32).map(|i| ((i * 31 + 7) % 256) as u8).collect();
    let many: Vec<Node> = (0..120).map(|i| Node::bytes(&format!("file{i:03}.txt"), 0, vec![b'x'; i])).collect();
    Node::dir(
        "",
        1_600_000_000,
        vec![
            Node::bytes("readme.txt", 1_623_760_496, b"hello world\n".to_vec()),
            Node::bytes("empty.txt", 0, vec![]),
            Node::bytes(UNICODE, 0, b"unicode\n".to_vec()),
            Node::dir(
                "Docs",
                0,
                vec![
                    Node::bytes("big.bin", 0, big),
                    Node::dir(
                        "Deep",
                        0,
                        vec![Node::dir("Deeper", 0, vec![Node::bytes("leaf.dat", 0, b"leaf".to_vec())])],
                    ),
                ],
            ),
            Node::dir("Empty Folder", 0, vec![]),
            Node::dir("Many", 0, many),
            Node::bytes("ReadMe.TXT", 0, b"clash".to_vec()),
        ],
    )
}

fn out(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("if-image-write-{name}-{}.iso", std::process::id()))
}

fn check(path: &PathBuf, kind: FsKind, unicode: bool) {
    let mut v = Volume::open(Box::new(IsoFile::open(path).unwrap()), Some(kind)).unwrap();
    let r = v.lookup("readme.txt").unwrap();
    let mut b = vec![0u8; r.size as usize];
    v.read(&r, 0, &mut b).unwrap();
    assert_eq!(b, b"hello world\n", "{kind:?}");
    let big = v.lookup("Docs/big.bin").unwrap();
    let mut b = vec![0u8; 5000];
    v.read(&big, 0, &mut b).unwrap();
    assert!(b.iter().enumerate().all(|(i, &x)| x as usize == (i * 31 + 7) % 256), "{kind:?}");
    let leaf = v.lookup("Docs/Deep/Deeper/leaf.dat").unwrap();
    assert_eq!(leaf.size, 4);
    let many = v.lookup("Many").unwrap();
    let kids = v.list(&many).unwrap();
    assert_eq!(kids.len(), 120, "{kind:?}");
    let f119 = kids.iter().find(|k| k.name.eq_ignore_ascii_case("file119.txt")).unwrap();
    assert_eq!(f119.size, 119);
    assert_eq!(v.lookup("empty.txt").unwrap().size, 0);
    if unicode {
        let u = v.lookup(UNICODE).unwrap();
        assert_eq!(u.size, 8, "{kind:?}");
        assert_eq!(v.lookup("readme.txt").unwrap().mtime, Some(1_623_760_496));
    }
}

#[test]
fn three_trees_read_back() {
    let p = out("trees");
    let s = build(&tree(), Options { label: "IsoFoundry Test".into(), ..Default::default() }, &p, |_, _| true).unwrap();
    assert_eq!(std::fs::metadata(&p).unwrap().len(), s.sectors * 2048);
    let v = Volume::open(Box::new(IsoFile::open(&p).unwrap()), None).unwrap();
    assert_eq!(v.found, vec![FsKind::Udf, FsKind::Joliet, FsKind::Iso9660]);
    assert_eq!(v.fs.label(), "IsoFoundry Test");
    check(&p, FsKind::Udf, true);
    check(&p, FsKind::Joliet, true);
    check(&p, FsKind::Iso9660, false);
    std::fs::remove_file(p).unwrap();
}

#[test]
fn iso9660_only() {
    let p = out("plain");
    build(&tree(), Options { joliet: false, udf: false, ..Default::default() }, &p, |_, _| true).unwrap();
    let v = Volume::open(Box::new(IsoFile::open(&p).unwrap()), None).unwrap();
    assert_eq!(v.found, vec![FsKind::Iso9660]);
    check(&p, FsKind::Iso9660, false);
    std::fs::remove_file(p).unwrap();
}

#[test]
fn bootable_bios_and_uefi() {
    let p = out("boot");
    let mut bios = vec![0xB0u8; 2048];
    bios[0] = 0xFA;
    let opts = Options {
        label: "BOOT".into(),
        bios: Some(BootImage { source: Source::Bytes(bios), size: 2048, load_sectors: Some(8), info_table: true }),
        efi: Some(BootImage {
            source: Source::Bytes(vec![0xEF; 3000]),
            size: 3000,
            load_sectors: None,
            info_table: false,
        }),
        ..Default::default()
    };
    build(&tree(), opts, &p, |_, _| true).unwrap();
    let mut v = Volume::open(Box::new(IsoFile::open(&p).unwrap()), None).unwrap();
    let cat = v.boot.clone().unwrap();
    assert!(cat.has(Platform::Bios) && cat.has(Platform::Efi));
    let b = &cat.entries[0];
    assert_eq!(b.sector_count, 8);
    let mut sec = vec![0u8; 2048];
    v.src.read_sectors(b.lba as u64, &mut sec).unwrap();
    assert_eq!(sec[0], 0xFA);
    // Boot information table: PVD at 16, own LBA, length.
    assert_eq!(u32::from_le_bytes(sec[8..12].try_into().unwrap()), 16);
    assert_eq!(u32::from_le_bytes(sec[12..16].try_into().unwrap()), b.lba);
    assert_eq!(u32::from_le_bytes(sec[16..20].try_into().unwrap()), 2048);
    let efi = &cat.entries[1];
    assert_eq!(efi.sector_count, 6);
    v.src.read_sectors(efi.lba as u64, &mut sec).unwrap();
    assert_eq!(sec[0], 0xEF);
    std::fs::remove_file(p).unwrap();
}

#[test]
fn cancel_removes_partial_file() {
    let p = out("cancel");
    let r = build(&tree(), Options::default(), &p, |done, _| done < 300 * 2048);
    assert!(matches!(r, Err(if_image::Error::Cancelled)));
    assert!(!p.exists());
    let mut part = p.as_os_str().to_owned();
    part.push(".part");
    assert!(!PathBuf::from(part).exists());
}

#[test]
fn from_disk_folder() {
    let src = std::env::temp_dir().join(format!("if-image-src-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&src);
    std::fs::create_dir_all(src.join("sub")).unwrap();
    std::fs::write(src.join("sub/x.txt"), b"x").unwrap();
    let node = Node::from_path(&src).unwrap();
    let p = out("disk");
    build(&node, Options::default(), &p, |_, _| true).unwrap();
    let mut v = Volume::open(Box::new(IsoFile::open(&p).unwrap()), None).unwrap();
    assert_eq!(v.lookup("sub/x.txt").unwrap().size, 1);
    std::fs::remove_file(p).unwrap();
    std::fs::remove_dir_all(src).unwrap();
}
