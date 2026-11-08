//! The M4 burn commands: `burn-image`, `burn-files`, `burn-mp3`,
//! `burn-video`, `erase` and `info` (drive info).
//!
//! All of them run against the `if-drive` layer. A real drive is
//! picked with `--drive E:` (letter) or `--drive 0` (index into the
//! enumerated list); `--fake` picks the simulated drive, which is
//! how these commands run end to end with no burner attached.
//! Real-drive burns and erases need `--yes`; without it the command
//! prints the plan and stops.

use crate::cli::{Global, Out};
use if_core::error::Exit;
use if_core::job::{CancelToken, Progress};
use serde_json::json;
use std::path::{Path, PathBuf};

#[derive(Default)]
struct Args {
    pos: Vec<String>,
    drive: Option<String>,
    fake: bool,
    yes: bool,
    simulate: bool,
    no_verify: bool,
    append: bool,
    close: bool,
    eject: bool,
    full: bool,
    label: Option<String>,
    speed: Option<u32>,
}

const BOOL_FLAGS: &[&str] = &["fake", "yes", "simulate", "no-verify", "append", "close", "eject", "full"];

fn parse_args(tail: &[String]) -> Result<Args, String> {
    let mut a = Args::default();
    let mut i = 0;
    while i < tail.len() {
        let t = &tail[i];
        if let Some(name) = t.strip_prefix("--") {
            if BOOL_FLAGS.contains(&name) {
                match name {
                    "fake" => a.fake = true,
                    "yes" => a.yes = true,
                    "simulate" => a.simulate = true,
                    "no-verify" => a.no_verify = true,
                    "append" => a.append = true,
                    "close" => a.close = true,
                    "eject" => a.eject = true,
                    "full" => a.full = true,
                    _ => unreachable!(),
                }
                i += 1;
                continue;
            }
            let value = tail.get(i + 1).ok_or_else(|| format!("--{name} needs a value"))?.clone();
            match name {
                "drive" => a.drive = Some(value),
                "label" => a.label = Some(value),
                "speed" => a.speed = Some(value.parse().map_err(|_| "--speed must be a number (kbit/s)".to_string())?),
                other => return Err(format!("unknown option --{other}")),
            }
            i += 2;
        } else {
            a.pos.push(t.clone());
            i += 1;
        }
    }
    Ok(a)
}

fn fake_info() -> if_drive::DriveInfo {
    if_drive::DriveInfo {
        id: "fake".into(),
        letter: Some('Z'),
        vendor: "FakeDrive".into(),
        product: "Simulated CD/DVD Writer".into(),
        revision: "1.0".into(),
        backend: "Fake".into(),
    }
}

/// Resolves the drive the user means, as plain info.
fn pick_drive(a: &Args) -> Result<if_drive::DriveInfo, String> {
    if a.fake {
        return Ok(fake_info());
    }
    let drives = if_drive::enumerate();
    if drives.is_empty() {
        return Err(
            "no optical drives were found — connect a burner, or use --fake for the simulated drive".to_string()
        );
    }
    match &a.drive {
        None => {
            if drives.len() == 1 {
                Ok(drives.into_iter().next().unwrap())
            } else {
                Err("more than one drive found — pick one with --drive <letter> or --drive <index>".to_string())
            }
        }
        Some(want) => {
            let w = want.trim().trim_end_matches(':');
            if let Ok(idx) = w.parse::<usize>() {
                return drives.into_iter().nth(idx).ok_or_else(|| format!("drive index {idx} is out of range"));
            }
            let letter = w.chars().next().ok_or("empty --drive value")?.to_ascii_uppercase();
            drives
                .into_iter()
                .find(|d| d.letter == Some(letter))
                .ok_or_else(|| format!("no drive with letter {letter}: was found"))
        }
    }
}

fn options(a: &Args) -> if_drive::BurnOptions {
    if_drive::BurnOptions {
        simulate: a.simulate,
        verify: !a.no_verify,
        close_disc: a.close,
        append: a.append,
        speed_kbps: a.speed,
        eject_when_done: a.eject,
    }
}

/// Prints the plan for a real-drive job and returns false when the
/// run must stop (no --yes). Fake-drive and dry runs always proceed.
fn confirm_real(a: &Args, g: &Global, o: &mut Out, info: &if_drive::DriveInfo, what: &str) -> bool {
    if info.id == "fake" || g.dry_run || a.yes {
        return true;
    }
    o.ok_text(&format!("plan: {what} on {} — re-run with --yes to do it", info.display_name()));
    false
}

pub(crate) fn run_info(tail: &[String], _g: &Global, o: &mut Out) -> i32 {
    let a = match parse_args(tail) {
        Ok(a) => a,
        Err(e) => return o.fail(Exit::Usage, &e),
    };
    let drives: Vec<if_drive::DriveInfo> = if a.fake { vec![fake_info()] } else { if_drive::enumerate() };
    if o.json {
        let arr: Vec<serde_json::Value> = drives
            .iter()
            .map(|d| {
                json!({
                    "id": d.id,
                    "letter": d.letter.map(|c| c.to_string()),
                    "vendor": d.vendor.trim(),
                    "product": d.product.trim(),
                    "revision": d.revision.trim(),
                    "backend": d.backend,
                })
            })
            .collect();
        o.ok_text(&json!({ "drives": arr }).to_string());
        return 0;
    }
    if drives.is_empty() {
        o.ok_text("no optical drives found (use --fake for the simulated drive)");
        return 0;
    }
    for d in &drives {
        o.ok_text(&format!(
            "{} — {} {}, firmware {}",
            d.display_name(),
            d.vendor.trim(),
            d.product.trim(),
            d.revision.trim()
        ));
        if let Ok(session) = if_drive::open(d) {
            match session.media() {
                Ok(m) if m.present => {
                    o.ok_text(&format!(
                        "   media: {} — {} free of {} bytes{}",
                        m.kind,
                        m.free_bytes,
                        m.capacity_bytes,
                        if m.blank {
                            ", blank"
                        } else if m.appendable {
                            ", appendable"
                        } else {
                            ", closed"
                        }
                    ));
                }
                Ok(_) => o.ok_text("   media: none inserted"),
                Err(e) => o.ok_text(&format!("   media: could not be read ({e})")),
            }
        }
    }
    0
}

fn run_burn(source: if_drive::BurnSource, a: &Args, g: &Global, o: &mut Out, what: &str) -> i32 {
    let info = match pick_drive(a) {
        Ok(i) => i,
        Err(e) => return o.fail(Exit::Usage, &e),
    };
    if !confirm_real(a, g, o, &info, what) {
        return 0;
    }
    if g.dry_run {
        o.ok_text(&format!("dry run: {what} on {}", info.display_name()));
        return 0;
    }
    let mut session = match if_drive::open(&info) {
        Ok(s) => s,
        Err(e) => return o.fail(Exit::Generic, &format!("{e}")),
    };
    match session.burn(&source, &options(a), &Progress::default(), &CancelToken::new()) {
        Ok(rep) => {
            if o.json {
                o.ok_text(
                    &json!({ "ok": true, "bytes_written": rep.bytes_written, "verified": rep.verified, "backend": rep.backend, "notes": rep.notes })
                        .to_string(),
                );
            } else {
                o.ok_text(&format!("done: {} bytes written via {}", rep.bytes_written, rep.backend));
                if rep.verified {
                    o.ok_text("verify: the read-back matches the source");
                }
                for n in &rep.notes {
                    o.ok_text(&format!("note: {n}"));
                }
            }
            0
        }
        Err(e) => o.fail(Exit::Generic, &format!("{e}")),
    }
}

pub(crate) fn run_burn_image(tail: &[String], g: &Global, o: &mut Out) -> i32 {
    let a = match parse_args(tail) {
        Ok(a) => a,
        Err(e) => return o.fail(Exit::Usage, &e),
    };
    let Some(img) = a.pos.first() else {
        return o.fail(
            Exit::Usage,
            "usage: burn-image <image> [--drive E:|--fake] [--simulate] [--append] [--close] [--speed N] [--yes]",
        );
    };
    let path = PathBuf::from(img);
    if !path.is_file() {
        return o.fail(Exit::Usage, &format!("{}: no such file", path.display()));
    }
    run_burn(if_drive::BurnSource::Image(path.clone()), &a, g, o, &format!("burn image {}", path.display()))
}

fn label_or_default(a: &Args, dir: &Path) -> String {
    a.label.clone().unwrap_or_else(|| {
        dir.file_name()
            .map(|n| n.to_string_lossy().to_uppercase())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "ISOFOUNDRY".to_string())
    })
}

pub(crate) fn run_burn_files(tail: &[String], g: &Global, o: &mut Out) -> i32 {
    let a = match parse_args(tail) {
        Ok(a) => a,
        Err(e) => return o.fail(Exit::Usage, &e),
    };
    let Some(dir) = a.pos.first() else {
        return o.fail(Exit::Usage, "usage: burn-files <folder> [--label NAME] [--drive E:|--fake] [--append] [--yes]");
    };
    let root = PathBuf::from(dir);
    if !root.is_dir() {
        return o.fail(Exit::Usage, &format!("{}: no such folder", root.display()));
    }
    let label = label_or_default(&a, &root);
    run_burn(
        if_drive::BurnSource::Files { root: root.clone(), label },
        &a,
        g,
        o,
        &format!("burn files from {}", root.display()),
    )
}

/// Stages an MP3 disc: playlist + the audio files, hard-linked
/// where possible, in a temp folder.
pub(crate) fn stage_mp3(root: &Path) -> Result<PathBuf, String> {
    let mut audio: Vec<PathBuf> = Vec::new();
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        let mut entries: Vec<_> = rd.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else if matches!(
                p.extension().and_then(|x| x.to_str()).map(|x| x.to_ascii_lowercase()).as_deref(),
                Some("mp3" | "flac" | "ogg" | "m4a" | "wav")
            ) {
                out.push(p);
            }
        }
    }
    walk(root, &mut audio);
    if audio.is_empty() {
        return Err(format!("{}: no audio files found", root.display()));
    }
    let stage = std::env::temp_dir().join(format!("isofoundry-mp3-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&stage);
    std::fs::create_dir_all(&stage).map_err(|e| format!("staging folder: {e}"))?;
    let mut names = Vec::new();
    for (i, src) in audio.iter().enumerate() {
        let name = format!("{:03}-{}", i + 1, src.file_name().unwrap_or_default().to_string_lossy());
        let dst = stage.join(&name);
        if std::fs::hard_link(src, &dst).is_err() {
            std::fs::copy(src, &dst).map_err(|e| format!("copying {}: {e}", src.display()))?;
        }
        names.push(name);
    }
    let playlist = names.iter().map(|n| format!("{n}\r\n")).collect::<String>();
    std::fs::write(stage.join("playlist.m3u"), playlist).map_err(|e| format!("writing playlist: {e}"))?;
    Ok(stage)
}

pub(crate) fn run_burn_mp3(tail: &[String], g: &Global, o: &mut Out) -> i32 {
    let a = match parse_args(tail) {
        Ok(a) => a,
        Err(e) => return o.fail(Exit::Usage, &e),
    };
    let Some(dir) = a.pos.first() else {
        return o.fail(Exit::Usage, "usage: burn-mp3 <folder> [--label NAME] [--drive E:|--fake] [--yes]");
    };
    let root = PathBuf::from(dir);
    if !root.is_dir() {
        return o.fail(Exit::Usage, &format!("{}: no such folder", root.display()));
    }
    let stage = match stage_mp3(&root) {
        Ok(s) => s,
        Err(e) => return o.fail(Exit::Usage, &e),
    };
    let label = a.label.clone().unwrap_or_else(|| "MP3 DISC".to_string());
    let code = run_burn(
        if_drive::BurnSource::Files { root: stage.clone(), label },
        &a,
        g,
        o,
        &format!("burn MP3 disc from {}", root.display()),
    );
    let _ = std::fs::remove_dir_all(&stage);
    code
}

pub(crate) fn run_burn_video(tail: &[String], g: &Global, o: &mut Out) -> i32 {
    let a = match parse_args(tail) {
        Ok(a) => a,
        Err(e) => return o.fail(Exit::Usage, &e),
    };
    let Some(dir) = a.pos.first() else {
        return o.fail(
            Exit::Usage,
            "usage: burn-video <folder> [--drive E:|--fake] [--yes] — the folder must contain a VIDEO_TS folder",
        );
    };
    let root = PathBuf::from(dir);
    if !root.join("VIDEO_TS").join("VIDEO_TS.IFO").is_file() {
        return o.fail(
            Exit::Usage,
            &format!("{}: not a DVD-video folder (VIDEO_TS/VIDEO_TS.IFO is missing)", root.display()),
        );
    }
    let label = a.label.clone().unwrap_or_else(|| "DVD VIDEO".to_string());
    run_burn(
        if_drive::BurnSource::Files { root: root.clone(), label },
        &a,
        g,
        o,
        &format!("burn DVD-video from {}", root.display()),
    )
}

pub(crate) fn run_erase(tail: &[String], g: &Global, o: &mut Out) -> i32 {
    let a = match parse_args(tail) {
        Ok(a) => a,
        Err(e) => return o.fail(Exit::Usage, &e),
    };
    let info = match pick_drive(&a) {
        Ok(i) => i,
        Err(e) => return o.fail(Exit::Usage, &e),
    };
    let kind = if a.full { if_drive::EraseKind::Full } else { if_drive::EraseKind::Quick };
    if !confirm_real(&a, g, o, &info, &format!("erase the disc ({kind:?})")) {
        return 0;
    }
    if g.dry_run {
        o.ok_text(&format!("dry run: erase ({kind:?}) on {}", info.display_name()));
        return 0;
    }
    let mut session = match if_drive::open(&info) {
        Ok(s) => s,
        Err(e) => return o.fail(Exit::Generic, &format!("{e}")),
    };
    match session.erase(kind, &Progress::default(), &CancelToken::new()) {
        Ok(()) => {
            o.ok_text(&format!("done: disc erased ({kind:?}) on {}", info.display_name()));
            0
        }
        Err(e) => o.fail(Exit::Generic, &format!("{e}")),
    }
}
