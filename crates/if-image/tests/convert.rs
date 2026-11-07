//! Conversion round trips. CHD output is pinned to the overall SHA1 that
//! MAME chdman 0.264 computes for the same input (see fixtures/README.md).

use if_image::container::{self, Format};
use if_image::convert::{convert, Output};
use if_image::fs::Volume;
use std::path::{Path, PathBuf};

fn fixture(n: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(n)
}

fn tmp(n: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("if-conv-{n}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(src: &Path, dest: &Path, o: Output) -> if_image::convert::Summary {
    convert(src, dest, o, &mut |_, _| true).unwrap()
}

fn chd_sha1(p: &Path) -> String {
    let b = std::fs::read(p).unwrap();
    b[84..104].iter().map(|x| format!("{x:02x}")).collect()
}

fn read(p: &Path) -> Vec<u8> {
    std::fs::read(p).unwrap()
}

#[test]
fn iso_to_cd_chd_matches_chdman() {
    let d = tmp("cdchd");
    run(&fixture("plain.iso"), &d.join("p.cue"), Output::BinCue);
    assert_eq!(read(&d.join("p.bin")).len(), read(&fixture("plain.iso")).len() / 2048 * 2352);
    run(&d.join("p.cue"), &d.join("p.chd"), Output::Chd);
    assert_eq!(chd_sha1(&d.join("p.chd")), "a2acc8b13372907b7ea10a9e14d577ce4594316b");
    // Direct ISO -> CD CHD gives the same disc.
    run(&fixture("plain.iso"), &d.join("q.chd"), Output::ChdCd);
    assert_eq!(chd_sha1(&d.join("q.chd")), "a2acc8b13372907b7ea10a9e14d577ce4594316b");
    // And back.
    run(&d.join("p.chd"), &d.join("back.iso"), Output::Iso);
    assert_eq!(read(&d.join("back.iso")), read(&fixture("plain.iso")));
}

#[test]
fn iso_to_dvd_chd_matches_chdman() {
    let d = tmp("dvdchd");
    run(&fixture("plain.iso"), &d.join("p.chd"), Output::Chd);
    assert_eq!(chd_sha1(&d.join("p.chd")), "9acf9b6d4fa58ee718e40717222d0f92728e57c0");
    run(&d.join("p.chd"), &d.join("back.iso"), Output::Iso);
    assert_eq!(read(&d.join("back.iso")), read(&fixture("plain.iso")));
}

#[test]
fn reads_chdman_output() {
    let d = tmp("chdman");
    for f in ["plain_cd.chd", "plain_dvd.chd"] {
        let s = run(&fixture(f), &d.join("x.iso"), Output::Iso);
        assert_eq!(s.source, Format::Chd);
        assert_eq!(read(&d.join("x.iso")), read(&fixture("plain.iso")), "{f}");
        let (_, src) = container::open(&fixture(f)).unwrap();
        let v = Volume::open(src, None).unwrap();
        assert_eq!(v.fs.label(), "PLAIN");
    }
}

#[test]
fn mixed_mode_through_chd_and_clonecd() {
    let d = tmp("mixed");
    run(&fixture("plain.iso"), &d.join("data.cue"), Output::BinCue);
    let mut bin = read(&d.join("data.bin"));
    let data_frames = bin.len() / 2352;
    // 150 stored pregap frames + 300 audio frames of a non-repeating pattern.
    bin.extend((0..450 * 2352u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8));
    std::fs::write(d.join("mix.bin"), &bin).unwrap();
    let msf = |n: usize| format!("{:02}:{:02}:{:02}", n / 4500, (n / 75) % 60, n % 75);
    std::fs::write(
        d.join("mix.cue"),
        format!(
            "FILE \"mix.bin\" BINARY\n  TRACK 01 MODE1/2352\n    INDEX 01 00:00:00\n  TRACK 02 AUDIO\n    INDEX 00 {}\n    INDEX 01 {}\n",
            msf(data_frames),
            msf(data_frames + 150)
        ),
    )
    .unwrap();
    run(&d.join("mix.cue"), &d.join("mix.chd"), Output::Chd);
    run(&d.join("mix.chd"), &d.join("back.cue"), Output::BinCue);
    assert_eq!(read(&d.join("back.bin")), bin);
    assert_eq!(
        std::fs::read_to_string(d.join("back.cue")).unwrap().replace("back.bin", "mix.bin"),
        std::fs::read_to_string(d.join("mix.cue")).unwrap()
    );
    run(&d.join("mix.chd"), &d.join("cc.ccd"), Output::CloneCd);
    assert_eq!(std::fs::metadata(d.join("cc.sub")).unwrap().len() as usize, bin.len() / 2352 * 96);
    run(&d.join("cc.ccd"), &d.join("cc2.cue"), Output::BinCue);
    assert_eq!(read(&d.join("cc2.bin")), bin);
    let s = run(&d.join("mix.cue"), &d.join("data.iso"), Output::Iso);
    assert!(s.warnings.iter().any(|w| w.contains("dropped")));
    assert_eq!(read(&d.join("data.iso")), read(&fixture("plain.iso")));
}

#[test]
fn cancel_leaves_nothing() {
    let d = tmp("cancel");
    for (name, o) in [("x.chd", Output::Chd), ("x.cue", Output::BinCue), ("x.iso", Output::Iso)] {
        let r = convert(&fixture("plain.iso"), &d.join(name), o, &mut |_, _| false);
        assert!(matches!(r, Err(if_image::Error::Cancelled)), "{name}");
    }
    assert_eq!(std::fs::read_dir(&d).unwrap().count(), 0);
}

#[test]
fn sheetless_raw_bin_is_detected() {
    let d = tmp("raw");
    run(&fixture("plain.iso"), &d.join("p.cue"), Output::BinCue);
    std::fs::remove_file(d.join("p.cue")).unwrap();
    let (f, _) = container::detect(&d.join("p.bin")).unwrap();
    assert_eq!(f, Format::RawBin);
    run(&d.join("p.bin"), &d.join("p.iso"), Output::Iso);
    assert_eq!(read(&d.join("p.iso")), read(&fixture("plain.iso")));
}
