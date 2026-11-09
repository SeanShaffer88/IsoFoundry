//! CLI for the USB jobs (M2): `usb list|write|read` and
//! `windows-to-usb`.
//!
//! Physical disks go through the elevated helper (the Platform trait);
//! `--file` targets a disk-image file (VHDX route, D-029) and runs
//! in-process through `usbwriter`, gated by `--dev-allow-vdisk`.
//! Erasing a physical disk requires `--yes`; the summary of the safety
//! rules lives in `usbwriter::device::assess_target`, which runs on
//! both routes before anything is written.

use crate::cli::{Global, Out, Platform};
use if_core::error::Exit;
use if_core::ipc::{UsbJob, UsbJobKind};
use if_core::job::{CancelToken, Progress};
use serde_json::json;
use std::path::{Path, PathBuf};
use usbwriter::device::{assess_target, describe, DiskInfo, FileDevice};
use usbwriter::install::Scheme;

type Fail = (Exit, String);

fn fail<T>(exit: Exit, msg: impl Into<String>) -> Result<T, Fail> {
    Err((exit, msg.into()))
}

struct Args {
    pos: Vec<String>,
    flags: Vec<(String, Option<String>)>,
}

impl Args {
    fn flag(&self, name: &str) -> Option<&str> {
        self.flags.iter().find(|(n, _)| n == name).and_then(|(_, v)| v.as_deref())
    }

    fn has(&self, name: &str) -> bool {
        self.flags.iter().any(|(n, _)| n == name)
    }
}

fn parse_args(tail: &[String]) -> Args {
    let mut pos = Vec::new();
    let mut flags = Vec::new();
    let mut i = 0;
    while i < tail.len() {
        let a = &tail[i];
        if let Some(name) = a.strip_prefix("--") {
            // Flags that take a value: the next token, unless it also
            // starts with --.
            let value = if i + 1 < tail.len() && !tail[i + 1].starts_with("--") {
                i += 1;
                Some(tail[i].clone())
            } else {
                None
            };
            flags.push((name.to_string(), value));
        } else {
            pos.push(a.clone());
        }
        i += 1;
    }
    Args { pos, flags }
}

pub(crate) fn run(cmd: &str, tail: &[String], g: &Global, platform: &dyn Platform, o: &mut Out<'_>) -> i32 {
    match dispatch(cmd, tail, g, platform, o) {
        Ok(code) => code,
        Err((exit, msg)) => o.fail(exit, &msg),
    }
}

fn dispatch(cmd: &str, tail: &[String], g: &Global, platform: &dyn Platform, o: &mut Out<'_>) -> Result<i32, Fail> {
    if cmd == "windows-to-usb" {
        return windows_to_usb(tail, g, platform, o);
    }
    let args = parse_args(tail);
    match args.pos.first().map(String::as_str) {
        Some("list") => usb_list(platform, o),
        Some("write") => usb_write(&args, g, platform, o),
        Some("read") => usb_read(&args, g, platform, o),
        _ => fail(
            Exit::Usage,
            "usage: usb list | usb write <image> --disk N|--file F | usb read --disk N|--file F --out FILE",
        ),
    }
}

fn usb_list(platform: &dyn Platform, o: &mut Out<'_>) -> Result<i32, Fail> {
    let disks = platform.usb_list()?;
    if o.json {
        let arr: Vec<_> = disks
            .iter()
            .map(|d| {
                json!({
                    "number": d.number, "model": d.model, "size_bytes": d.size_bytes,
                    "removable": d.removable, "system": d.is_system,
                    "volumes": d.volume_labels,
                })
            })
            .collect();
        o.ok_text(&json!({ "disks": arr }).to_string());
    } else if disks.is_empty() {
        o.ok_text("no physical disks found");
    } else {
        for d in &disks {
            let refused = assess_target(d, false, false).err().map(|e| format!(" [{}]", e.0));
            o.ok_text(&format!("{}{}", describe(d), refused.unwrap_or_default()));
        }
    }
    Ok(0)
}

/// The chosen target: a physical disk number, or a disk-image file.
enum Target {
    Disk(u32),
    File(PathBuf),
}

fn pick_target(args: &Args) -> Result<Target, Fail> {
    if let Some(f) = args.flag("file") {
        return Ok(Target::File(PathBuf::from(f)));
    }
    if let Some(d) = args.flag("disk") {
        let n: u32 = d.parse().map_err(|_| (Exit::Usage, format!("--disk needs a number, got '{d}'")))?;
        return Ok(Target::Disk(n));
    }
    fail(Exit::Usage, "name the target: --disk N or --file PATH")
}

/// Runs a job on a file target in-process, or hands a physical-disk job
/// to the helper. Returns (files, bytes, notes).
fn execute(
    target: &Target,
    kind: &UsbJobKind,
    g: &Global,
    platform: &dyn Platform,
    o: &mut Out<'_>,
) -> Result<(usize, u64, Vec<String>), Fail> {
    match target {
        Target::Disk(n) => {
            if !g.yes && !g.dry_run {
                return fail(
                    Exit::Usage,
                    "erasing a physical disk needs --yes (run `usb list` first and check the disk number)",
                );
            }
            if g.dry_run {
                o.ok_text(&format!("would run {kind:?} on disk {n}"));
                return Ok((0, 0, vec![]));
            }
            let job = UsbJob { disk_number: *n, kind: kind.clone() };
            let progress = Progress::default();
            platform.usb_job(&job, &progress)
        }
        Target::File(path) => {
            // The --dev-allow-vdisk requirement is enforced by callers.
            let info = file_disk_info(path)?;
            assess_target(&info, false, true).map_err(|e| (Exit::Generic, e.0))?;
            if g.dry_run {
                o.ok_text(&format!("would run {kind:?} on {}", path.display()));
                return Ok((0, 0, vec![]));
            }
            run_on_file(path, kind, o)
        }
    }
}

fn file_disk_info(path: &Path) -> Result<DiskInfo, Fail> {
    let size = std::fs::metadata(path).map(|m| m.len()).map_err(|e| (Exit::Io, format!("{}: {e}", path.display())))?;
    Ok(DiskInfo {
        number: 0,
        model: format!("disk image file {}", path.display()),
        size_bytes: size,
        bus: usbwriter::device::Bus::Virtual,
        removable: false,
        is_system: false,
        holds_boot: false,
        holds_pagefile: false,
        volume_labels: vec![],
        is_file_target: true,
    })
}

pub(crate) fn run_on_file(path: &Path, kind: &UsbJobKind, o: &mut Out<'_>) -> Result<(usize, u64, Vec<String>), Fail> {
    run_on_file_with(path, kind, o, &Progress::default(), &CancelToken::new())
}

pub(crate) fn run_on_file_with(
    path: &Path,
    kind: &UsbJobKind,
    _o: &mut Out<'_>,
    progress: &Progress,
    cancel: &CancelToken,
) -> Result<(usize, u64, Vec<String>), Fail> {
    let map = |e: usbwriter::Error| (Exit::Generic, e.0);
    match kind {
        UsbJobKind::Files { image, scheme, windows_installer } => {
            let scheme = Scheme::parse(scheme).map_err(map)?;
            if scheme.is_ntfs() {
                return fail(
                    Exit::Generic,
                    "gpt-ntfs needs a physical disk (the helper formats with Windows' formatter)",
                );
            }
            if *windows_installer {
                usbwriter::install::check_windows_installer(Path::new(image)).map_err(map)?;
            }
            let dev = FileDevice::open(path, true).map_err(map)?;
            let rep =
                usbwriter::install::copy_iso_tree(dev, Path::new(image), scheme, *windows_installer, progress, cancel)
                    .map_err(map)?;
            Ok((rep.files, rep.bytes, rep.notes))
        }
        UsbJobKind::RawWrite { image, verify } => {
            let mut dev = FileDevice::open(path, true).map_err(map)?;
            let n = usbwriter::raw::image_to_device(Path::new(image), &mut dev, progress, cancel).map_err(map)?;
            let mut notes = Vec::new();
            if *verify {
                let total = std::fs::metadata(image).map(|m| m.len()).unwrap_or(0);
                let a = usbwriter::raw::hash_device_range(&dev, total).map_err(map)?;
                let b = usbwriter::raw::hash_file(Path::new(image)).map_err(map)?;
                if a != b {
                    return fail(Exit::VerifyMismatch, "verify failed: the written data does not match the image");
                }
                notes.push("write verified (SHA-256 match)".into());
            }
            Ok((0, n, notes))
        }
        UsbJobKind::ReadImage { out, limit_bytes } => {
            if let (Ok(source), Ok(destination)) = (path.canonicalize(), Path::new(out).canonicalize()) {
                if source == destination {
                    return fail(Exit::Usage, "the output image must differ from the disk-image source");
                }
            }
            let dev = FileDevice::open(path, false).map_err(map)?;
            let n =
                usbwriter::raw::device_to_image(&dev, Path::new(out), *limit_bytes, progress, cancel).map_err(map)?;
            Ok((0, n, vec![]))
        }
    }
}

fn report(o: &mut Out<'_>, what: &str, files: usize, bytes: u64, notes: &[String]) -> i32 {
    if o.json {
        o.ok_text(&json!({ "ok": true, "files": files, "bytes": bytes, "notes": notes }).to_string());
    } else {
        o.ok_text(&format!("{what}: {files} files, {bytes} bytes written"));
        for n in notes {
            o.ok_text(&format!("note: {n}"));
        }
    }
    0
}

fn usb_write(args: &Args, g: &Global, platform: &dyn Platform, o: &mut Out<'_>) -> Result<i32, Fail> {
    let image =
        args.pos.get(1).cloned().ok_or((Exit::Usage, "usage: usb write <image> --disk N|--file F".to_string()))?;
    let target = pick_target(args)?;
    if let Target::File(_) = target {
        if !args.has("dev-allow-vdisk") {
            return fail(Exit::Usage, "file targets need --dev-allow-vdisk (they exist for VHDX testing, D-029)");
        }
    }
    let mode = args.flag("mode").unwrap_or("files");
    let kind = match mode {
        "files" => {
            let scheme = args.flag("scheme").unwrap_or("mbr");
            UsbJobKind::Files { image, scheme: scheme.to_string(), windows_installer: false }
        }
        "raw" => UsbJobKind::RawWrite { image, verify: args.has("verify") },
        other => return fail(Exit::Usage, format!("unknown --mode '{other}' (files or raw)")),
    };
    let (files, bytes, notes) = execute(&target, &kind, g, platform, o)?;
    Ok(report(o, "usb write", files, bytes, &notes))
}

fn usb_read(args: &Args, g: &Global, platform: &dyn Platform, o: &mut Out<'_>) -> Result<i32, Fail> {
    let target = pick_target(args)?;
    if let Target::File(_) = target {
        if !args.has("dev-allow-vdisk") {
            return fail(Exit::Usage, "file targets need --dev-allow-vdisk (they exist for VHDX testing, D-029)");
        }
    }
    let out = args.flag("out").ok_or((Exit::Usage, "usb read needs --out FILE".to_string()))?;
    let limit = match args.flag("bytes") {
        Some(b) => Some(b.parse::<u64>().map_err(|_| (Exit::Usage, format!("--bytes needs a number, got '{b}'")))?),
        None => None,
    };
    let kind = UsbJobKind::ReadImage { out: out.to_string(), limit_bytes: limit };
    let (files, bytes, notes) = execute(&target, &kind, g, platform, o)?;
    Ok(report(o, "usb read", files, bytes, &notes))
}

fn windows_to_usb(tail: &[String], g: &Global, platform: &dyn Platform, o: &mut Out<'_>) -> Result<i32, Fail> {
    let args = parse_args(tail);
    let image = args
        .pos
        .first()
        .cloned()
        .ok_or((Exit::Usage, "usage: windows-to-usb <image> --disk N|--file F".to_string()))?;
    let target = pick_target(&args)?;
    if let Target::File(_) = target {
        if !args.has("dev-allow-vdisk") {
            return fail(Exit::Usage, "file targets need --dev-allow-vdisk (they exist for VHDX testing, D-029)");
        }
    }
    let layout = args.flag("layout").unwrap_or("mbr");
    let kind = UsbJobKind::Files { image, scheme: layout.to_string(), windows_installer: true };
    let (files, bytes, notes) = execute(&target, &kind, g, platform, o)?;
    Ok(report(o, "windows-to-usb", files, bytes, &notes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usb_read_from_disk_image_honors_the_optional_limit() {
        let dir = std::env::temp_dir().join(format!("isofoundry-usb-read-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.img");
        let output = dir.join("backup.img");
        std::fs::write(&source, b"0123456789abcdef").unwrap();
        let kind = UsbJobKind::ReadImage { out: output.to_string_lossy().into_owned(), limit_bytes: Some(6) };
        let mut stdout = std::io::sink();
        let mut stderr = std::io::sink();
        let mut out = Out { out: &mut stdout, err: &mut stderr, json: false };

        let (_, bytes, _) =
            run_on_file_with(&source, &kind, &mut out, &Progress::default(), &CancelToken::new()).unwrap();

        assert_eq!(bytes, 6);
        assert_eq!(std::fs::read(&output).unwrap(), b"012345");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn usb_read_does_not_overwrite_its_disk_image_source() {
        let dir = std::env::temp_dir().join(format!("isofoundry-usb-read-source-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.img");
        let original = b"keep this disk image intact";
        std::fs::write(&source, original).unwrap();
        let kind = UsbJobKind::ReadImage { out: source.to_string_lossy().into_owned(), limit_bytes: Some(6) };
        let mut stdout = std::io::sink();
        let mut stderr = std::io::sink();
        let mut out = Out { out: &mut stdout, err: &mut stderr, json: false };

        assert!(run_on_file_with(&source, &kind, &mut out, &Progress::default(), &CancelToken::new()).is_err());
        assert_eq!(std::fs::read(&source).unwrap(), original);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
