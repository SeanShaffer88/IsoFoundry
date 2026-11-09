use if_image::fs::{FsKind, Volume};
use if_image::source::IsoFile;
use if_image::write::{build_xiso, Kind, Node, Source};
use std::path::{Path, PathBuf};

fn output(name: &str) -> PathBuf {
    std::env::temp_dir().join(format!("if-image-xiso-{name}-{}.iso", std::process::id()))
}

fn tree() -> Node {
    let files =
        (0..31).map(|i| Node::bytes(&format!("file-{i:02}.txt"), 0, format!("data-{i}").into_bytes())).collect();
    Node::dir(
        "",
        0,
        vec![
            Node::bytes("readme.txt", 0, b"hello XISO\n".to_vec()),
            Node::dir("nested", 0, vec![Node::dir("empty", 0, vec![]), Node::dir("files", 0, files)]),
        ],
    )
}

fn read_file(volume: &mut Volume, path: &str) -> Vec<u8> {
    let entry = volume.lookup(path).unwrap();
    let mut bytes = vec![0; entry.size as usize];
    volume.read(&entry, 0, &mut bytes).unwrap();
    bytes
}

fn table_names(image: &[u8], sector: u32, size: u32) -> Vec<String> {
    let start = sector as usize * 2048;
    let table = &image[start..start + size as usize];
    let mut pending = vec![0usize];
    let mut visited = std::collections::HashSet::new();
    let mut names = Vec::new();
    while let Some(offset) = pending.pop() {
        assert!(visited.insert(offset), "directory tree contains a cycle");
        assert!(offset + 14 <= table.len(), "entry offset is outside the directory table");
        let left = u16::from_le_bytes(table[offset..offset + 2].try_into().unwrap());
        let right = u16::from_le_bytes(table[offset + 2..offset + 4].try_into().unwrap());
        let len = usize::from(table[offset + 13]);
        assert!(offset + 14 + len <= table.len(), "filename is outside the directory table");
        names.push(String::from_utf8(table[offset + 14..offset + 14 + len].to_vec()).unwrap());
        if right != 0 {
            pending.push(usize::from(right) * 4);
        }
        if left != 0 {
            pending.push(usize::from(left) * 4);
        }
    }
    names
}

#[test]
fn creates_xdvdfs_image_with_nested_files_and_empty_directories() {
    let path = output("roundtrip");
    let summary = build_xiso(&tree(), &path, |_, _| true).unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().len(), summary.sectors * 2048);
    assert_eq!(summary.sectors % 32, 0);

    let mut volume = Volume::open(Box::new(IsoFile::open(&path).unwrap()), None).unwrap();
    assert_eq!(volume.fs.kind(), FsKind::Xdvdfs);
    assert_eq!(read_file(&mut volume, "readme.txt"), b"hello XISO\n");
    for i in 0..31 {
        assert_eq!(read_file(&mut volume, &format!("nested/files/file-{i:02}.txt")), format!("data-{i}").as_bytes());
    }
    let empty = volume.lookup("nested/empty").unwrap();
    assert!(empty.dir);
    assert!(volume.list(&empty).unwrap().is_empty());

    let bytes = std::fs::read(&path).unwrap();
    let root_sector = u32::from_le_bytes(bytes[32 * 2048 + 20..32 * 2048 + 24].try_into().unwrap());
    let root_size = u32::from_le_bytes(bytes[32 * 2048 + 24..32 * 2048 + 28].try_into().unwrap());
    let root_names = table_names(&bytes, root_sector, root_size);
    assert_eq!(root_names.len(), 2);
    assert!(root_names.iter().any(|name| name == "nested"));
    assert!(root_names.iter().any(|name| name == "readme.txt"));

    std::fs::remove_file(path).unwrap();
}

#[test]
fn rejects_xdvdfs_names_that_cannot_be_encoded() {
    let path = output("invalid-name");
    let source = Node::dir("", 0, vec![Node::bytes("bad/name", 0, vec![])]);

    assert!(build_xiso(&source, &path, |_, _| true).is_err());
    assert!(!Path::new(&path).exists());
}

#[test]
fn rejects_files_larger_than_xdvdfs_size_fields() {
    let path = output("oversized-file");
    let source = Node::dir(
        "",
        0,
        vec![Node {
            name: "large.bin".into(),
            mtime: 0,
            kind: Kind::File { source: Source::Bytes(Vec::new()), size: u64::from(u32::MAX) + 1 },
        }],
    );

    assert!(build_xiso(&source, &path, |_, _| true).is_err());
    assert!(!path.exists());
}

#[test]
fn rejects_directories_that_exceed_xdvdfs_pointer_range() {
    let path = output("oversized-directory");
    let files = (0..13_500).map(|i| Node::bytes(&format!("f{i:05}"), 0, Vec::new())).collect();
    let source = Node::dir("", 0, files);

    assert!(build_xiso(&source, &path, |_, _| true).is_err());
    assert!(!path.exists());
}

#[test]
fn removes_partial_output_when_final_rename_fails() {
    let path = output("rename-failure");
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("keep.txt"), b"keep").unwrap();
    let part = PathBuf::from(format!("{}.part", path.display()));

    assert!(build_xiso(&tree(), &path, |_, _| true).is_err());
    assert_eq!(std::fs::read(path.join("keep.txt")).unwrap(), b"keep");
    assert!(!part.exists());

    std::fs::remove_dir_all(path).unwrap();
}
