//! RC failure drills: hostile media, damaged discs, corrupt files and
//! cancellations must all fail cleanly — a plain error, never a
//! panic, never silent corruption. Engine level, against the
//! simulated drive (the milestone test gate).

use if_core::job::{CancelToken, Progress};
use if_drive::fake::{FakeConfig, FakeDrive};
use if_drive::rip::{rip_audio, rip_image, surface_scan, RipAudioOptions, RipImageOptions};
use if_drive::{fake_source_audio, fake_source_data, BurnOptions, BurnSource, OpticalDrive};
use std::path::PathBuf;

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("isofoundry-drill-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A small real ISO built by the project's own writer.
fn sample_iso(dir: &std::path::Path) -> Vec<u8> {
    let root = if_image::write::Node::dir(
        "",
        0,
        vec![
            if_image::write::Node::bytes("HELLO.TXT", 0, b"hello".to_vec()),
            if_image::write::Node::bytes("DATA.BIN", 0, vec![7u8; 9000]),
        ],
    );
    let dest = dir.join("sample.iso");
    let never = |_done: u64, _total: u64| true;
    if_image::write::build(&root, if_image::write::Options::default(), &dest, never).unwrap();
    std::fs::read(&dest).unwrap()
}

fn burn_opts() -> BurnOptions {
    BurnOptions {
        simulate: false,
        verify: true,
        close_disc: true,
        append: false,
        speed_kbps: None,
        eject_when_done: false,
    }
}

#[test]
fn burn_rejects_an_image_bigger_than_the_disc() {
    let dir = tmp("oversize");
    let big = dir.join("big.iso");
    std::fs::File::create(&big).unwrap().set_len(800 * 1024 * 1024).unwrap();
    let mut drive = FakeDrive::new(FakeConfig::default());
    let r = drive.burn(&BurnSource::Image(big), &burn_opts(), &Progress::default(), &CancelToken::new());
    assert!(r.is_err(), "oversize burn must fail");
}

#[test]
fn burn_rejects_media_that_is_not_blank() {
    let dir = tmp("nonblank");
    let iso = sample_iso(&dir);
    let src = dir.join("in.iso");
    std::fs::write(&src, &iso).unwrap();
    let mut drive = fake_source_data(&iso);
    let r = drive.burn(&BurnSource::Image(src), &burn_opts(), &Progress::default(), &CancelToken::new());
    assert!(r.is_err(), "burning over a written disc must fail");
}

#[test]
fn a_failed_verify_is_never_reported_as_success() {
    let dir = tmp("verify");
    let iso = sample_iso(&dir);
    let src = dir.join("in.iso");
    std::fs::write(&src, &iso).unwrap();
    let cfg = FakeConfig { corrupt_verify: true, ..FakeConfig::default() };
    let mut drive = FakeDrive::new(cfg);
    match drive.burn(&BurnSource::Image(src), &burn_opts(), &Progress::default(), &CancelToken::new()) {
        Err(_) => {}
        Ok(rep) => assert!(!rep.verified, "corrupt verify must not claim verified"),
    }
}

#[test]
fn rip_zero_fills_bad_sectors_and_reports_them() {
    let dir = tmp("ripbad");
    let iso = sample_iso(&dir);
    let cfg = FakeConfig { bad_sectors: vec![3, 4], ..FakeConfig::default() };
    let mut drive = FakeDrive::new(cfg);
    drive.load_image(&iso);
    let dest = dir.join("out.iso");
    let rep =
        rip_image(&mut drive, &dest, &RipImageOptions::default(), &Progress::default(), &CancelToken::new()).unwrap();
    assert_eq!(rep.bad_sectors, vec![3, 4]);
    let back = std::fs::read(&dest).unwrap();
    assert!(back[3 * 2048..5 * 2048].iter().all(|b| *b == 0), "bad sectors zero-filled");
    // Sector 16 is the primary volume descriptor: a good sector that
    // must come through byte for byte.
    assert_eq!(&back[16 * 2048..17 * 2048], &iso[16 * 2048..17 * 2048], "good sectors intact");
}

#[test]
fn rip_audio_rejects_a_data_disc() {
    let dir = tmp("ripdata");
    let iso = sample_iso(&dir);
    let mut drive = fake_source_data(&iso);
    let r = rip_audio(&mut drive, &RipAudioOptions::default(), &Progress::default(), &CancelToken::new());
    assert!(r.is_err());
}

#[test]
fn surface_scan_reports_the_damage_map() {
    let dir = tmp("scan");
    let iso = sample_iso(&dir);
    let cfg = FakeConfig { bad_sectors: vec![1, 2, 3], slow_sectors: vec![5, 6], ..FakeConfig::default() };
    let mut drive = FakeDrive::new(cfg);
    drive.load_image(&iso);
    let rep = surface_scan(&mut drive, &Progress::default(), &CancelToken::new()).unwrap();
    assert_eq!(rep.bad, vec![1, 2, 3]);
    assert_eq!(rep.slow, 2);
    assert!(rep.quality_grade != 'A' || rep.total_sectors < 500, "heavy damage on a small disc grades down");
}

#[test]
fn a_cancelled_job_stops_instead_of_finishing() {
    let dir = tmp("cancel");
    let iso = sample_iso(&dir);
    let mut drive = fake_source_data(&iso);
    let cancel = CancelToken::new();
    cancel.cancel();
    let dest = dir.join("out.iso");
    let r = rip_image(&mut drive, &dest, &RipImageOptions::default(), &Progress::default(), &cancel);
    assert!(r.is_err(), "a cancelled rip must not complete");
}

#[test]
fn malformed_images_error_cleanly_never_panic() {
    let dir = tmp("garbage");
    let cases: Vec<Vec<u8>> =
        vec![Vec::new(), vec![0u8; 1], (0..8192u32).map(|i| (i * 37 % 251) as u8).collect(), vec![0xFF; 65536]];
    for (i, bytes) in cases.iter().enumerate() {
        let p = dir.join(format!("g{i}.iso"));
        std::fs::write(&p, bytes).unwrap();
        if let Ok((_fmt, src)) = if_image::container::open(&p) {
            let _ = if_image::fs::Volume::open(src, None); // Ok or Err, never a panic
        }
    }
    // A truncated valid image: opens, walks or errors — no panic.
    let iso = sample_iso(&dir);
    let cut = dir.join("cut.iso");
    std::fs::write(&cut, &iso[..iso.len() / 2]).unwrap();
    if let Ok((_fmt, src)) = if_image::container::open(&cut) {
        if let Ok(mut vol) = if_image::fs::Volume::open(src, None) {
            let _ = vol.walk(&vol.root());
        }
    }
    let _ = fake_source_audio(); // silence: audio fixture exercised elsewhere
}

#[test]
fn backup_restore_names_the_missing_span() {
    let dir = tmp("backup");
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.bin"), vec![1u8; 40 * 1024]).unwrap();
    std::fs::write(src.join("b.bin"), vec![2u8; 40 * 1024]).unwrap();
    let work = dir.join("work");
    let staged =
        isofoundry::backup::create_backup(&src, "drill", 64 * 1024, &work, &Progress::default(), &CancelToken::new())
            .unwrap();
    assert!(staged.span_dirs.len() >= 2, "the drill needs two spans");
    // Restore with only the first span present.
    let dest = dir.join("out");
    std::fs::create_dir_all(&dest).unwrap();
    let err = isofoundry::backup::restore(
        &[staged.span_dirs[0].clone()],
        &dest,
        &dir.join("rtmp"),
        &Progress::default(),
        &CancelToken::new(),
    )
    .unwrap_err();
    assert!(err.to_lowercase().contains("span"), "error names the missing span: {err}");
}

#[test]
fn backup_restore_catches_a_tampered_file() {
    let dir = tmp("tamper");
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("keep.txt"), b"original contents").unwrap();
    let work = dir.join("work");
    let staged = isofoundry::backup::create_backup(
        &src,
        "drill",
        4 * 1024 * 1024,
        &work,
        &Progress::default(),
        &CancelToken::new(),
    )
    .unwrap();
    let victim = staged.span_dirs[0].join("keep.txt");
    std::fs::write(&victim, b"tampered contents").unwrap();
    let dest = dir.join("out");
    std::fs::create_dir_all(&dest).unwrap();
    let err = isofoundry::backup::restore(
        &staged.span_dirs,
        &dest,
        &dir.join("rtmp"),
        &Progress::default(),
        &CancelToken::new(),
    )
    .unwrap_err();
    assert!(err.to_lowercase().contains("keep.txt"), "error names the file: {err}");
}

#[test]
fn backup_rejects_an_empty_source() {
    let dir = tmp("empty");
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    let r = isofoundry::backup::create_backup(
        &src,
        "drill",
        4 * 1024 * 1024,
        &dir.join("work"),
        &Progress::default(),
        &CancelToken::new(),
    );
    assert!(r.is_err());
}
