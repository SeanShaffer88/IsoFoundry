//! The M6 backup commands: `backup` (spanning) and
//! `backup-restore`.
//!
//! `backup` stages the plan once (see [`crate::backup`]), then
//! either writes one ISO per span (`--out-dir`) or burns each span
//! as a files disc, waiting for a fresh blank disc between spans.
//! `--fake` burns to a fresh simulated drive per span. Real burns
//! need `--yes`, like every other burn command.

use crate::backup;
use crate::cli::{Global, Out};
use if_core::error::Exit;
use if_core::job::{CancelToken, Progress};
use serde_json::json;
use std::path::PathBuf;

#[derive(Default)]
struct Args {
    pos: Vec<String>,
    name: Option<String>,
    size: Option<String>,
    capacity_mb: Option<u64>,
    out_dir: Option<String>,
    drive: Option<String>,
    from: Vec<String>,
    fake: bool,
    yes: bool,
    keep: bool,
    shutdown_when_done: bool,
}

fn parse_args(tail: &[String]) -> Result<Args, String> {
    let mut a = Args::default();
    let mut i = 0;
    while i < tail.len() {
        let t = &tail[i];
        if let Some(name) = t.strip_prefix("--") {
            match name {
                "fake" => a.fake = true,
                "yes" => a.yes = true,
                "keep" => a.keep = true,
                "shutdown-when-done" => a.shutdown_when_done = true,
                "name" | "size" | "capacity-mb" | "out-dir" | "drive" | "from" => {
                    let v = tail.get(i + 1).ok_or_else(|| format!("--{name} needs a value"))?.clone();
                    match name {
                        "name" => a.name = Some(v),
                        "size" => a.size = Some(v),
                        "capacity-mb" => a.capacity_mb = Some(v.parse().map_err(|_| "--capacity-mb must be a number")?),
                        "out-dir" => a.out_dir = Some(v),
                        "drive" => a.drive = Some(v),
                        "from" => {
                            a.from.extend(v.split(',').map(|s| s.trim().to_string()));
                        }
                        _ => unreachable!(),
                    }
                    i += 1;
                }
                other => return Err(format!("unknown option --{other}")),
            }
            i += 1;
            continue;
        }
        a.pos.push(t.clone());
        i += 1;
    }
    Ok(a)
}

fn capacity_of(a: &Args) -> Result<Option<u64>, String> {
    if let Some(mb) = a.capacity_mb {
        return Ok(Some(mb * 1024 * 1024));
    }
    match a.size.as_deref() {
        None => Ok(None),
        Some("cd") => Ok(Some(backup::CAP_CD)),
        Some("dvd") => Ok(Some(backup::CAP_DVD)),
        Some("dvd-dl") => Ok(Some(backup::CAP_DVD_DL)),
        Some("bd") => Ok(Some(backup::CAP_BD)),
        Some(other) => Err(format!("unknown --size {other} (cd, dvd, dvd-dl, bd)")),
    }
}

fn pick_real(want: &Option<String>) -> Result<if_drive::DriveInfo, String> {
    let drives = if_drive::enumerate();
    if drives.is_empty() {
        return Err("no optical drives were found — connect a drive, or use --fake".into());
    }
    match want {
        None if drives.len() == 1 => Ok(drives.into_iter().next().unwrap()),
        None => Err("more than one drive found — pick one with --drive <letter>".into()),
        Some(w) => {
            let w = w.trim().trim_end_matches(':').to_ascii_uppercase();
            drives
                .into_iter()
                .find(|d| d.letter.map(|l| l.to_string()) == Some(w.clone()))
                .ok_or_else(|| format!("no drive with letter {w}"))
        }
    }
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

fn temp_work(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("isofoundry-{tag}-{}", std::process::id()))
}

pub(crate) fn run_backup(tail: &[String], g: &Global, o: &mut Out) -> i32 {
    let a = match parse_args(tail) {
        Ok(a) => a,
        Err(m) => return o.fail(Exit::Usage, &m),
    };
    let source = match a.pos.as_slice() {
        [p] => PathBuf::from(p),
        [] => {
            return o.fail(
                Exit::Usage,
                "usage: backup FOLDER [--size cd|dvd|dvd-dl|bd] [--out-dir DIR] [--drive E:] [--fake] [--yes]",
            )
        }
        [_, x, ..] => return o.fail(Exit::Usage, &format!("unexpected argument: {x}")),
    };
    let name = a.name.clone().unwrap_or_else(|| {
        source.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "backup".into())
    });
    let capacity = match capacity_of(&a) {
        Ok(c) => c,
        Err(m) => return o.fail(Exit::Usage, &m),
    };

    let mode = if a.out_dir.is_some() {
        "isos"
    } else if a.fake {
        "fake"
    } else {
        "drive"
    };
    // For a real drive, plan against the inserted blank's capacity
    // unless the user sized the spans themselves.
    let capacity = match (capacity, mode) {
        (Some(c), _) => c,
        (None, "isos") => backup::CAP_DVD,
        (None, "fake") => backup::CAP_CD,
        (None, _) => {
            if !a.yes {
                return o.fail(Exit::Generic, "burning a backup needs --yes (or use --out-dir to write span ISOs)");
            }
            let info = match pick_real(&a.drive) {
                Ok(i) => i,
                Err(m) => return o.fail(Exit::Generic, &m),
            };
            let drive = match if_drive::open(&info) {
                Ok(d) => d,
                Err(e) => return o.fail(Exit::Generic, &e.to_string()),
            };
            match drive.media() {
                Ok(m) if m.present && m.blank && m.capacity_bytes > 0 => m.capacity_bytes,
                Ok(m) if !m.present => {
                    return o.fail(Exit::NoDriveOrMedia, "no disc in the drive — insert a blank disc first")
                }
                Ok(_) => {
                    return o.fail(
                        Exit::MediaIncompatible,
                        "the disc in the drive is not blank — insert a blank disc first",
                    )
                }
                Err(e) => return o.fail(Exit::Generic, &e.to_string()),
            }
        }
    };

    let work = temp_work("backup");
    let progress = Progress::default();
    let cancel = CancelToken::new();
    let staged = match backup::create_backup(&source, &name, capacity, &work, &progress, &cancel) {
        Ok(s) => s,
        Err(m) => {
            let _ = std::fs::remove_dir_all(&work);
            return o.fail(Exit::Generic, &m);
        }
    };
    let spans = staged.catalog.spans;
    let files = staged.catalog.files.len();
    let bytes = staged.catalog.total_bytes();

    let finish = |o: &mut Out, mode: &str| -> i32 {
        if !a.keep {
            let _ = std::fs::remove_dir_all(&work);
        }
        if g.json {
            o.ok_text(
                &json!({ "ok": true, "name": name, "spans": spans, "files": files,
                          "bytes": bytes, "mode": mode })
                .to_string(),
            );
        } else {
            o.ok_text(&format!(
                "Backup '{name}' complete: {files} file(s), {} across {spans} span(s) ({mode}).",
                backup::human_bytes(bytes)
            ));
            if a.keep {
                o.ok_text(&format!("Staged spans kept in {}", work.display()));
            }
        }
        if a.shutdown_when_done
            && !crate::finish_actions::shutdown_after_finish(
                "IsoFoundry finished the backup and is shutting down, as requested.",
            )
        {
            o.ok_text("note: the shutdown could not be started");
        }
        0
    };

    match mode {
        "isos" => {
            let out_dir = PathBuf::from(a.out_dir.unwrap());
            if let Err(e) = std::fs::create_dir_all(&out_dir) {
                return o.fail(Exit::Io, &format!("cannot create {}: {e}", out_dir.display()));
            }
            for (i, dir) in staged.span_dirs.iter().enumerate() {
                let iso = out_dir.join(format!("{}-{:03}.iso", slug(&name), i + 1));
                let p2 = Progress::default();
                match backup::write_span_iso(dir, &iso, &name, &p2, &cancel) {
                    Ok(_) => {
                        if !g.json {
                            o.ok_text(&format!("span {} of {spans}: {}", i + 1, iso.display()));
                        }
                    }
                    Err(m) => return o.fail(Exit::Generic, &m),
                }
            }
            finish(o, "span ISOs")
        }
        "fake" => {
            let info = fake_info();
            for (i, dir) in staged.span_dirs.iter().enumerate() {
                let mut drive = match if_drive::open(&info) {
                    Ok(d) => d,
                    Err(e) => return o.fail(Exit::Generic, &e.to_string()),
                };
                let p2 = Progress::default();
                let src = if_drive::BurnSource::Files { root: dir.clone(), label: name.clone() };
                let opts = if_drive::BurnOptions { verify: true, close_disc: true, ..Default::default() };
                match drive.burn(&src, &opts, &p2, &cancel) {
                    Ok(r) => {
                        if !g.json {
                            o.ok_text(&format!(
                                "span {} of {spans}: burned to the simulated drive ({} written, verified: {})",
                                i + 1,
                                backup::human_bytes(r.bytes_written),
                                r.verified
                            ));
                        }
                    }
                    Err(e) => return o.fail(Exit::Generic, &e.to_string()),
                }
            }
            finish(o, "simulated drive")
        }
        _ => {
            if g.json && spans > 1 {
                let _ = std::fs::remove_dir_all(&work);
                return o.fail(
                    Exit::Usage,
                    "burning a multi-span backup needs a terminal (one prompt per disc); use --out-dir for unattended runs",
                );
            }
            let info = match pick_real(&a.drive) {
                Ok(i) => i,
                Err(m) => return o.fail(Exit::Generic, &m),
            };
            for (i, dir) in staged.span_dirs.iter().enumerate() {
                if i > 0 || spans > 1 {
                    o.ok_text(&format!(
                        "Insert blank disc {} of {spans} in drive {} …",
                        i + 1,
                        info.letter.map(|l| l.to_string()).unwrap_or_default()
                    ));
                }
                let mut drive = match wait_for_blank(&info, o) {
                    Ok(d) => d,
                    Err(m) => return o.fail(Exit::Generic, &m),
                };
                let p2 = Progress::default();
                let src = if_drive::BurnSource::Files { root: dir.clone(), label: name.clone() };
                let opts = if_drive::BurnOptions {
                    verify: false,
                    close_disc: true,
                    eject_when_done: true,
                    ..Default::default()
                };
                match drive.burn(&src, &opts, &p2, &cancel) {
                    Ok(r) => {
                        if !g.json {
                            o.ok_text(&format!(
                                "span {} of {spans}: burned ({})",
                                i + 1,
                                backup::human_bytes(r.bytes_written)
                            ));
                        }
                    }
                    Err(e) => return o.fail(Exit::Generic, &e.to_string()),
                }
            }
            finish(o, "discs")
        }
    }
}

/// Opens the drive repeatedly until a blank disc is present
/// (10-minute budget, polled every 2 s).
fn wait_for_blank(info: &if_drive::DriveInfo, o: &mut Out) -> Result<Box<dyn if_drive::OpticalDrive>, String> {
    let mut waited = 0;
    loop {
        let drive = if_drive::open(info).map_err(|e| e.to_string())?;
        match drive.media() {
            Ok(m) if m.present && m.blank => return Ok(drive),
            Ok(m) if m.present && !m.blank => {
                return Err("the inserted disc is not blank".into());
            }
            _ => {}
        }
        if waited >= 600 {
            return Err("timed out waiting for a blank disc".into());
        }
        if waited == 0 {
            o.ok_text("waiting for a blank disc…");
        }
        waited += 2;
        std::thread::sleep(std::time::Duration::from_secs(2));
    }
}

pub(crate) fn run_restore(tail: &[String], g: &Global, o: &mut Out) -> i32 {
    let a = match parse_args(tail) {
        Ok(a) => a,
        Err(m) => return o.fail(Exit::Usage, &m),
    };
    let dest = match a.pos.as_slice() {
        [p] => PathBuf::from(p),
        [] => return o.fail(Exit::Usage, "usage: backup-restore FOLDER --from SPAN [--from SPAN ...] [--yes]"),
        [_, x, ..] => return o.fail(Exit::Usage, &format!("unexpected argument: {x}")),
    };
    if a.from.is_empty() {
        return o.fail(Exit::Usage, "give at least one span with --from (a folder or an ISO file)");
    }
    if dest.is_dir() && std::fs::read_dir(&dest).map(|mut d| d.next().is_some()).unwrap_or(false) && !a.yes {
        return o.fail(Exit::Generic, "the restore folder is not empty — pick an empty folder or pass --yes");
    }
    let sources: Vec<PathBuf> = a.from.iter().map(PathBuf::from).collect();
    let work = temp_work("restore");
    let progress = Progress::default();
    let cancel = CancelToken::new();
    let result = backup::restore(&sources, &dest, &work, &progress, &cancel);
    let _ = std::fs::remove_dir_all(&work);
    match result {
        Ok(r) => {
            if g.json {
                o.ok_text(
                    &json!({ "ok": true, "name": r.backup_name, "files": r.files,
                              "bytes": r.bytes, "spans": r.spans_used, "verified": true })
                    .to_string(),
                );
            } else {
                o.ok_text(&format!(
                    "Restored '{}': {} file(s), {}, every checksum verified.",
                    r.backup_name,
                    r.files,
                    backup::human_bytes(r.bytes)
                ));
            }
            0
        }
        Err(m) => o.fail(Exit::Generic, &m),
    }
}

fn slug(name: &str) -> String {
    let s: String =
        name.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    let s = s.trim_matches('-');
    if s.is_empty() {
        "backup".into()
    } else {
        s.to_string()
    }
}
