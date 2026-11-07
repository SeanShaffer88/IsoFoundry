use if_image::container;
use if_image::edit::{edit, Edit};
use if_image::eltorito::Platform;
use if_image::fs::{read_all, Volume};
use std::path::{Path, PathBuf};

fn fixture(n: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(n)
}

fn tmp(n: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("if-edit-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn open(p: &Path) -> Volume {
    let (_, s) = container::open(p).unwrap();
    Volume::open(s, None).unwrap()
}

fn file(v: &mut Volume, p: &str) -> Vec<u8> {
    let e = v.lookup(p).unwrap();
    read_all(v.src.as_mut(), &e, 1 << 30).unwrap()
}

#[test]
fn add_remove_rename_label() {
    let d = tmp("ops");
    std::fs::write(d.join("new.txt"), b"added").unwrap();
    std::fs::create_dir_all(d.join("folder/sub")).unwrap();
    std::fs::write(d.join("folder/sub/x.bin"), vec![7u8; 10_000]).unwrap();
    let out = d.join("out.iso");
    let edits = [
        Edit::Add { dir: "Docs".into(), from: d.join("new.txt") },
        Edit::Add { dir: "".into(), from: d.join("folder") },
        Edit::Remove("empty.txt".into()),
        Edit::Rename { path: "readme.txt".into(), to: "README-renamed.txt".into() },
        Edit::MkDir("A/B".into()),
        Edit::Label("EDITED".into()),
    ];
    let (_, warn) = edit(&fixture("udf.iso"), &out, &edits, |_, _| true).unwrap();
    assert!(warn.is_empty(), "{warn:?}");
    let mut v = open(&out);
    assert_eq!(v.fs.label(), "EDITED");
    assert_eq!(file(&mut v, "Docs/new.txt"), b"added");
    assert_eq!(file(&mut v, "folder/sub/x.bin"), vec![7u8; 10_000]);
    assert_eq!(file(&mut v, "README-renamed.txt"), b"hello world\n");
    assert!(v.lookup("empty.txt").is_err());
    assert!(v.lookup("A/B").unwrap().dir);
    // Untouched data copied from the source image.
    let big: Vec<u8> = (0..5000u32).map(|i| ((i * 31 + 7) % 256) as u8).collect();
    assert_eq!(file(&mut v, "Docs/big.bin"), big);
    assert_eq!(file(&mut v, "Docs/Deep/Deeper/leaf.dat"), b"leaf");
    assert!(v.lookup("Ünïcödé name – long file name.txt").is_ok());
}

#[test]
fn keeps_boot_entries() {
    let d = tmp("boot");
    let out = d.join("out.iso");
    edit(&fixture("boot.iso"), &out, &[Edit::Remove("readme.txt".into())], |_, _| true).unwrap();
    let mut src = open(&fixture("boot.iso"));
    let mut v = open(&out);
    let cat = v.boot.clone().expect("boot catalog");
    assert!(cat.has(Platform::Bios) && cat.has(Platform::Efi));
    for p in [Platform::Bios, Platform::Efi] {
        let a = src.boot.as_ref().unwrap().entries.iter().find(|e| e.platform == p).unwrap().clone();
        let b = cat.entries.iter().find(|e| e.platform == p).unwrap().clone();
        assert_eq!(a.sector_count, b.sector_count);
        let mut x = vec![0u8; 2048];
        let mut y = vec![0u8; 2048];
        src.src.read_sectors(a.lba as u64, &mut x).unwrap();
        v.src.read_sectors(b.lba as u64, &mut y).unwrap();
        assert_eq!(x, y, "{p:?} boot image");
    }
}

#[test]
fn refuses_to_overwrite_source_and_reports_missing() {
    let d = tmp("refuse");
    let copy = d.join("c.iso");
    std::fs::copy(fixture("plain.iso"), &copy).unwrap();
    assert!(edit(&copy, &copy, &[], |_, _| true).is_err());
    assert!(edit(&copy, &d.join("o.iso"), &[Edit::Remove("nope".into())], |_, _| true).is_err());
    assert!(!d.join("o.iso").exists());
}

#[test]
fn edits_inside_a_chd() {
    let d = tmp("chd");
    let out = d.join("o.iso");
    edit(&fixture("plain_cd.chd"), &out, &[Edit::Label("FROMCHD".into())], |_, _| true).unwrap();
    let mut v = open(&out);
    assert_eq!(v.fs.label(), "FROMCHD");
    assert_eq!(file(&mut v, "readme.txt").to_ascii_lowercase(), b"hello world\n");
}

#[test]
fn old_boot_catalog_file_is_not_copied() {
    let d = tmp("cat");
    let out = d.join("o.iso");
    edit(&fixture("boot.iso"), &out, &[], |_, _| true).unwrap();
    assert!(open(&fixture("boot.iso")).lookup("boot.catalog").is_ok());
    let mut v = open(&out);
    assert!(v.lookup("boot.catalog").is_err());
    assert!(v.lookup("boot.img").is_ok());
}
