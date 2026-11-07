//! Reading images made by other tools (genisoimage, mkudffs). Fixture
//! sources: tests/fixtures/README.md.

use if_image::eltorito::Platform;
use if_image::fs::{FsKind, Volume};
use if_image::source::IsoFile;
use std::path::PathBuf;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(name)
}

fn open(name: &str, prefer: Option<FsKind>) -> Volume {
    Volume::open(Box::new(IsoFile::open(&fixture(name)).unwrap()), prefer).unwrap()
}

fn names(v: &mut Volume) -> Vec<String> {
    let root = v.root();
    v.walk(&root).unwrap().into_iter().map(|(p, e)| if e.dir { format!("{p}/") } else { p }).collect()
}

const UNICODE: &str = "Ünïcödé name – long file name.txt";

fn check_tree(v: &mut Volume, unicode: bool) {
    let n = names(v);
    assert!(n.contains(&"Docs/".to_string()), "{n:?}");
    assert!(n.contains(&"Docs/Deep/Deeper/leaf.dat".to_string()), "{n:?}");
    assert!(n.contains(&"Empty Folder/".to_string()) || n.iter().any(|x| x.eq_ignore_ascii_case("EMPTY_FO/")), "{n:?}");
    if unicode {
        assert!(n.contains(&UNICODE.to_string()), "{n:?}");
    }
    let big = v.lookup("Docs/big.bin").unwrap();
    assert_eq!(big.size, 5000);
    let mut buf = vec![0u8; 5000];
    v.read(&big, 0, &mut buf).unwrap();
    assert!(buf.iter().enumerate().all(|(i, &b)| b as usize == (i * 31 + 7) % 256));
    let r = v.lookup("readme.txt").unwrap();
    let mut s = vec![0u8; r.size as usize];
    v.read(&r, 0, &mut s).unwrap();
    assert_eq!(s, b"hello world\n");
    assert_eq!(v.lookup("empty.txt").unwrap().size, 0);
}

#[test]
fn joliet_and_rock_ridge() {
    let mut v = open("joliet_rr.iso", None);
    assert_eq!(v.found, vec![FsKind::Joliet, FsKind::Iso9660]);
    assert_eq!(v.fs.label(), "JOLIET RR");
    check_tree(&mut v, true);
    let r = v.lookup("readme.txt").unwrap();
    assert_eq!(r.mtime, Some(1_623_760_496)); // 2021-06-15 12:34:56 UTC
                                              // The primary tree with Rock Ridge names gives the same tree.
    let mut p = open("joliet_rr.iso", Some(FsKind::Iso9660));
    check_tree(&mut p, true);
}

#[test]
fn plain_iso9660_names() {
    let mut v = open("plain.iso", None);
    assert_eq!(v.found, vec![FsKind::Iso9660]);
    let n = names(&mut v);
    assert!(n.contains(&"README.TXT".to_string()), "{n:?}");
    let r = v.lookup("readme.txt").unwrap(); // case-insensitive fallback
    assert_eq!(r.size, 12);
}

#[test]
fn udf_bridge() {
    let mut v = open("udf.iso", None);
    assert_eq!(v.found, vec![FsKind::Udf, FsKind::Joliet, FsKind::Iso9660]);
    assert_eq!(v.fs.label(), "UDF VOL");
    check_tree(&mut v, true);
}

#[test]
fn el_torito_bios_and_efi() {
    let v = open("boot.iso", None);
    let cat = v.boot.expect("boot catalog");
    assert!(cat.has(Platform::Bios));
    assert!(cat.has(Platform::Efi));
    assert_eq!(cat.entries[0].sector_count, 4);
}

#[test]
fn extract_everything() {
    let mut v = open("udf.iso", None);
    let dest = std::env::temp_dir().join(format!("if-image-extract-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dest);
    let root = v.root();
    let mut calls = 0;
    let rep = v
        .extract(&root, &dest, |_, _| {
            calls += 1;
            true
        })
        .unwrap();
    assert!(rep.files >= 5 && calls > 0);
    assert_eq!(std::fs::read(dest.join("readme.txt")).unwrap(), b"hello world\n");
    assert_eq!(std::fs::read(dest.join("Docs/Deep/Deeper/leaf.dat")).unwrap(), b"leaf");
    assert!(dest.join(UNICODE).is_file());
    assert!(dest.join("Empty Folder").is_dir());
    std::fs::remove_dir_all(&dest).unwrap();
}

#[test]
fn extract_cancel_leaves_no_partial_file() {
    let mut v = open("joliet_rr.iso", None);
    let dest = std::env::temp_dir().join(format!("if-image-cancel-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dest);
    let big = v.lookup("Docs/big.bin").unwrap();
    assert!(matches!(v.extract(&big, &dest, |_, _| false), Err(if_image::Error::Cancelled)));
    assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 0);
    std::fs::remove_dir_all(&dest).unwrap();
}

#[test]
fn not_an_image() {
    let p = std::env::temp_dir().join(format!("if-image-junk-{}", std::process::id()));
    std::fs::write(&p, vec![0xA5u8; 600 * 2048]).unwrap();
    assert!(Volume::open(Box::new(IsoFile::open(&p).unwrap()), None).is_err());
    std::fs::remove_file(p).unwrap();
}
