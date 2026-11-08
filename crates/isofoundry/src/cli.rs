//! Command line (D-016). Every grid task has a subcommand; since M6
//! all of them are implemented (the `not_implemented` exit 40 remains
//! in the exit-code table for completeness).
//! Platform work (helper launch, OS version) comes in through [`Platform`].

use if_core::error::Exit;
use if_core::settings::LogLevel;
use if_core::{strings, tasks};
use serde_json::json;
use std::io::Write;

/// Result of a helper handshake.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HelperInfo {
    pub version: String,
    pub elevated: bool,
    pub pid: u32,
}

pub trait Platform {
    fn os_description(&self) -> String;
    fn user_profile(&self) -> Option<String>;
    fn helper_check(&self, elevated: bool) -> Result<HelperInfo, (Exit, String)>;

    /// Physical disks for the USB jobs (through the elevated helper).
    fn usb_list(&self) -> Result<Vec<usbwriter::device::DiskInfo>, (Exit, String)> {
        Err((Exit::Generic, "USB operations are not available on this platform".into()))
    }

    /// Runs one USB job on a physical disk (through the elevated helper).
    fn usb_job(
        &self,
        _job: &if_core::ipc::UsbJob,
        _progress: &if_core::job::Progress,
    ) -> Result<(usize, u64, Vec<String>), (Exit, String)> {
        Err((Exit::Generic, "USB operations are not available on this platform".into()))
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Global {
    pub json: bool,
    pub level: Option<LogLevel>,
    pub dry_run: bool,
    pub yes: bool,
}

/// Splits global flags from the rest. Unknown `--flags` stay in `rest` for
/// the subcommand to reject.
pub fn split_globals(args: &[String]) -> (Global, Vec<String>) {
    let mut g = Global::default();
    let mut rest = Vec::new();
    for a in args {
        match a.as_str() {
            "--json" => g.json = true,
            "--verbose" => g.level = Some(LogLevel::Info),
            "--debug" | "--trace" => g.level = Some(LogLevel::Debug),
            "--dry-run" => g.dry_run = true,
            "--yes" | "-y" => g.yes = true,
            _ => rest.push(a.clone()),
        }
    }
    (g, rest)
}

pub fn usage() -> String {
    let mut s = format!(
        "IsoFoundry {}\nOpen source disc burning, imaging and bootable USB suite.\n\n\
Usage: isofoundry [COMMAND] [OPTIONS]\n\
       (no command opens the window)\n\n\
Commands:\n\
  list                 List every task, its CLI name and when it ships\n\
  helper-check         Start the helper, check the handshake, stop it\n\
       --elevated      ...started with administrator rights (UAC prompt)\n\
  report --out FILE    Write a redacted drive report (JSON) for a GitHub issue\n\
  clean                Remove IsoFoundry's settings, logs and reports\n\
                       (--dry-run lists them; --yes deletes)\n\
  help, --help         Show this text\n\
  version, --version   Show the version\n\n\
USB commands (administrator rights via the helper; disk targets erase the disk):\n\
  usb list             List physical disks\n\
  usb write IMAGE --disk N [--mode files|raw] [--scheme mbr|gpt] [--verify]\n\
                       Make a bootable stick from an ISO (files: copy its tree;\n\
                       raw: copy the image byte for byte). --yes confirms the erase\n\
  usb read --disk N --out FILE [--bytes N]   Copy a disk back to an image file\n\
  usb write IMAGE --file STICK.img --dev-allow-vdisk   Same, onto a disk-image file\n\
  windows-to-usb IMAGE --disk N [--layout mbr|gpt|gpt-ntfs] [--yes]\n\
                       Windows installer stick (Microsoft's own boot files;\n\
                       gpt-ntfs for images whose install.wim is over 4 GB)\n\n\
Burning commands:\n\
  burn-image IMG [--drive E:|--fake|--all-drives] [--simulate] [--append] [--close] [--speed N] [--yes]\n\
  burn-files DIR [--label NAME]   Burn a folder as a data disc\n\
  burn-mp3 DIR                     Burn an MP3 disc (playlist generated)\n\
  burn-video DIR                   Burn a DVD-video folder (VIDEO_TS)\n\
  burn-audio FILES...              Burn an audio CD from audio files\n\
  erase [--full]                   Erase a rewritable disc\n\
  info                             List drives, inserted media and disc contents\n\
\n\
Backup commands:\n\
  backup FOLDER [--size cd|dvd|dvd-dl|bd] [--out-dir DIR] [--drive E:] [--fake] [--yes]\n\
                       Spanning backup of a folder, one catalog on every span\n\
  backup-restore DIR --from SPAN [--from SPAN ...] [--yes]\n\
                       Restore a spanning backup (folders or span ISOs), checksums verified\n\n\
Integration commands:\n\
  completions powershell|bash   Print a shell completion script\n\
  check-update         Ask GitHub for the newest release (online, on request only)\n\
  explorer-verbs install|uninstall|status   Right-click entries for image files\n\n\
Rip and copy commands:\n\
  copy-to-image IMG [--drive E:|--fake] [--raw] [--subchannel] [--retries N]\n\
                       Rip a disc to an image (+ CUE, dump report, hash file)\n\
  copy-disc [--from E:] [--to F:] [--yes]   Copy a disc to another drive\n\
  rip [--out DIR] [--format wav|flac|mp3] [--cue] [--accuraterip] [--musicbrainz]\n\
                       Rip an audio CD, securely, with AccurateRip checksums\n\
                       (--musicbrainz also looks the CD up online for titles)\n\
  surface-test             Read every sector once and grade the surface\n\
  recover DIR              Extract files from a damaged disc, with a report\n\
\n\
Audio commands:\n\
  convert-audio FILES... --to wav|flac|mp3|aac [--out DIR] [--bitrate N] [--replaygain] [--apply-gain]\n\
                       Convert audio files in a batch; tags are carried across\n\n\
Image commands:\n{}{}\n\
",
        if_core::APP_VERSION,
        crate::cli_image::USAGE,
        crate::cli_mount::USAGE
    );
    let pending: Vec<_> = tasks::all().filter(|t| !t.is_available()).collect();
    if !pending.is_empty() {
        s.push_str("Task commands not available yet (exit code 40 until their version ships):\n");
        for t in pending {
            s.push_str(&format!(
                "  {:<20} {} [{} / {}]\n",
                t.cli,
                strings::task_title(t.id),
                t.milestone.label(),
                t.milestone.release()
            ));
        }
    }
    s.push_str(
        "\nGlobal options:\n\
  --json               Machine-readable output on stdout\n\
  --verbose, --debug   More detail in the log\n\
  --dry-run            Show what would happen, change nothing\n\
  --yes                Confirm a destructive command without asking\n\n\
Exit codes: 0 ok, 1 error, 2 usage, 3 cancelled, 10 no drive/media,\n\
  11 media incompatible, 12 write failed, 13 verify mismatch,\n\
  20 elevation denied, 30 I/O, 40 not implemented yet.\n",
    );
    s
}

/// The roadmap as plain text (Help → Roadmap).
pub fn roadmap_text() -> String {
    use tasks::Milestone::*;
    let mut s = String::from("What each pre-release brings (0.x). 1.0 ships when every task works.\n\n");
    for m in [M1, M2, M3, M4, M5, M6] {
        s.push_str(&format!("{} \u{2014} version {}\n", m.label(), m.release()));
        for t in tasks::all().filter(|t| t.milestone == m) {
            s.push_str(&format!("  \u{2022} {}\n", strings::task_title(t.id)));
        }
        for t in tasks::all() {
            for v in t.variants.iter().filter(|v| v.milestone == m && v.milestone != t.milestone) {
                s.push_str(&format!(
                    "  \u{2022} {} \u{2192} {}\n",
                    strings::task_title(t.id),
                    strings::variant_label(v.id)
                ));
            }
        }
        s.push('\n');
    }
    s.push_str("0.1 (this build): the window, settings, logs, command line, drive report and the elevated helper.\n");
    s
}

fn list_json() -> serde_json::Value {
    let tasks: Vec<_> = tasks::all()
        .map(|t| {
            json!({
                "id": t.id,
                "cli": t.cli,
                "title": strings::task_title(t.id),
                "milestone": t.milestone.label(),
                "release": t.milestone.release(),
                "admin": t.admin,
                "available": matches!(t.status(), tasks::Status::Available),
                "variants": t.variants.iter().map(|v| json!({
                    "id": v.id,
                    "label": strings::variant_label(v.id),
                    "milestone": v.milestone.label(),
                    "admin": v.admin,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({ "version": if_core::APP_VERSION, "tasks": tasks })
}

pub(crate) struct Out<'a> {
    pub(crate) out: &'a mut dyn Write,
    pub(crate) err: &'a mut dyn Write,
    pub(crate) json: bool,
}

impl Out<'_> {
    pub(crate) fn ok_text(&mut self, text: &str) {
        let _ = writeln!(self.out, "{text}");
    }

    /// Progress on stderr in steps of 10% (text mode only).
    pub(crate) fn progress(&mut self, pct: u32) {
        if !self.json && pct % 10 == 0 {
            let _ = writeln!(self.err, "{pct}%");
        }
    }

    pub(crate) fn fail(&mut self, exit: Exit, message: &str) -> i32 {
        if self.json {
            let _ = writeln!(
                self.out,
                "{}",
                json!({ "ok": false, "exit": exit.name(), "code": exit.code(), "message": message })
            );
        } else {
            let _ = writeln!(self.err, "isofoundry: {message}");
        }
        exit.code()
    }
}

/// Runs a command line (without the program name). Returns the exit code.
pub fn run(
    args: &[String],
    platform: &dyn Platform,
    dirs: &if_core::paths::DataDirs,
    log: &if_core::log::Logger,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> i32 {
    let (g, rest) = split_globals(args);
    if let Some(l) = g.level {
        log.set_level(l);
    }
    let mut o = Out { out, err, json: g.json };
    let cmd = rest.first().map(String::as_str).unwrap_or("help");
    let tail = &rest[1.min(rest.len())..];
    log.info(format!("cli: {cmd}"));
    match cmd {
        "help" | "--help" | "-h" | "/?" => {
            o.ok_text(&usage());
            0
        }
        "version" | "--version" | "-V" => {
            if g.json {
                o.ok_text(&json!({ "version": if_core::APP_VERSION }).to_string());
            } else {
                o.ok_text(&format!("IsoFoundry {}", if_core::APP_VERSION));
            }
            0
        }
        "list" => {
            if let Some(extra) = tail.first() {
                return o.fail(Exit::Usage, &format!("unexpected argument: {extra}"));
            }
            if g.json {
                o.ok_text(&serde_json::to_string_pretty(&list_json()).unwrap_or_default());
            } else {
                for t in tasks::all() {
                    let shield = if t.admin { " (admin)" } else { "" };
                    o.ok_text(&format!(
                        "{:<20} {:<44} {} {}{shield}",
                        t.cli,
                        strings::task_title(t.id),
                        t.milestone.label(),
                        t.milestone.release()
                    ));
                }
            }
            0
        }
        "helper-check" => {
            let elevated = match tail {
                [] => false,
                [f] if f == "--elevated" => true,
                [x, ..] => return o.fail(Exit::Usage, &format!("unexpected argument: {x}")),
            };
            if g.dry_run {
                o.ok_text(&format!(
                    "would start the helper{}",
                    if elevated { " with administrator rights" } else { "" }
                ));
                return 0;
            }
            match platform.helper_check(elevated) {
                Ok(h) => {
                    log.info(format!("helper ok: v{} elevated={} pid={}", h.version, h.elevated, h.pid));
                    if g.json {
                        o.ok_text(
                            &json!({ "ok": true, "version": h.version, "elevated": h.elevated, "pid": h.pid })
                                .to_string(),
                        );
                    } else {
                        o.ok_text(&format!(
                            "Helper OK: version {}, {}",
                            h.version,
                            if h.elevated {
                                "running as administrator"
                            } else {
                                "running without administrator rights"
                            }
                        ));
                    }
                    0
                }
                Err((exit, m)) => {
                    log.error(format!("helper check failed: {m}"));
                    o.fail(exit, &m)
                }
            }
        }
        "report" => {
            let path = match tail {
                [f, p] if f == "--out" => std::path::PathBuf::from(p),
                _ => return o.fail(Exit::Usage, "usage: isofoundry report --out FILE"),
            };
            let report = build_report(platform, log_tail(dirs, log));
            if g.dry_run {
                o.ok_text(&report.to_json());
                return 0;
            }
            match if_core::fsutil::atomic_write(&path, report.to_json().as_bytes()) {
                Ok(()) => {
                    log.info("drive report written");
                    if g.json {
                        o.ok_text(&json!({ "ok": true, "path": path.display().to_string() }).to_string());
                    } else {
                        o.ok_text(&format!("Drive report written to {}", path.display()));
                    }
                    0
                }
                Err(e) => o.fail(Exit::Io, &format!("cannot write {}: {e}", path.display())),
            }
        }
        "clean" | "--clean" | "--uninstall" => {
            if let Some(extra) = tail.first() {
                return o.fail(Exit::Usage, &format!("unexpected argument: {extra}"));
            }
            clean(dirs, &g, &mut o)
        }
        "extract" | "checksum" | "compare" | "image-info" | "verify" | "create-image" | "convert-image"
        | "edit-image" => crate::cli_image::run(cmd, tail, &g, &mut o, log),
        "mount" | "unmount" => crate::cli_mount::run(cmd, tail, dirs, &mut o),
        "convert-audio" => crate::cli_audio::run(cmd, tail, &g, &mut o),
        "burn-image" => crate::cli_burn::run_burn_image(tail, &g, &mut o),
        "burn-files" => crate::cli_burn::run_burn_files(tail, &g, &mut o),
        "burn-mp3" => crate::cli_burn::run_burn_mp3(tail, &g, &mut o),
        "burn-video" => crate::cli_burn::run_burn_video(tail, &g, &mut o),
        "erase" => crate::cli_burn::run_erase(tail, &g, &mut o),
        "info" => crate::cli_burn::run_info(tail, &g, &mut o),
        "backup" => crate::cli_backup::run_backup(tail, &g, &mut o),
        "backup-restore" => crate::cli_backup::run_restore(tail, &g, &mut o),
        "completions" => crate::cli_meta::run_completions(tail, &g, &mut o),
        "check-update" => crate::cli_meta::run_check_update(tail, &g, &mut o),
        "explorer-verbs" => crate::cli_meta::run_explorer_verbs(tail, &g, &mut o),
        "copy-to-image" => crate::cli_rip::run_copy_to_image(tail, &g, &mut o),
        "copy-disc" => crate::cli_rip::run_copy_disc(tail, &g, &mut o),
        "rip" => crate::cli_rip::run_rip(tail, &g, &mut o),
        "burn-audio" => crate::cli_rip::run_burn_audio(tail, &g, &mut o),
        "surface-test" => crate::cli_rip::run_surface_test(tail, &g, &mut o),
        "recover" => crate::cli_rip::run_recover(tail, &g, &mut o),
        "usb" | "windows-to-usb" => crate::cli_usb::run(cmd, tail, &g, platform, &mut o),
        other => match tasks::by_cli(other) {
            Some(t) => {
                let msg = format!(
                    "{} ships in {} (version {}); not available in this build",
                    strings::task_title(t.id),
                    t.milestone.label(),
                    t.milestone.release()
                );
                o.fail(Exit::NotImplemented, &msg)
            }
            None => o.fail(Exit::Usage, &format!("unknown command: {other} (try --help)")),
        },
    }
}

/// Last 200 log lines: the log file in the data folder (earlier GUI
/// sessions) followed by this process's in-memory entries.
pub fn log_tail(dirs: &if_core::paths::DataDirs, log: &if_core::log::Logger) -> Vec<String> {
    let mut lines: Vec<String> = std::fs::read_to_string(dirs.log_dir().join("isofoundry.log"))
        .map(|t| t.lines().map(str::to_string).collect())
        .unwrap_or_default();
    if log.path().is_none() {
        lines.extend(log.entries().iter().map(|e| e.line()));
    }
    let skip = lines.len().saturating_sub(200);
    lines.split_off(skip)
}

pub fn build_report(platform: &dyn Platform, tail: Vec<String>) -> if_core::report::DriveReport {
    // No live drive enumeration here: the report is written from
    // contexts where probing hardware is not always safe (it is a
    // support file, often produced by scripts). `info` is the
    // command that lists drives.
    if_core::report::DriveReport::new(
        platform.os_description(),
        Vec::new(),
        "drive list not included in the report — run `isofoundry info` for live drives",
        tail,
    )
    .redacted(platform.user_profile().as_deref())
}

/// Directive V2.0 §19: remove everything IsoFoundry ever wrote. Today that
/// is only the data folder (settings, logs, journal, reports); no registry
/// keys, tasks or temp folders are registered yet.
fn clean(dirs: &if_core::paths::DataDirs, g: &Global, o: &mut Out<'_>) -> i32 {
    let root = &dirs.root;
    let name_ok = root
        .file_name()
        .map(|n| {
            let n = n.to_string_lossy().to_ascii_lowercase();
            n == "isofoundry" || n == "data" || n == "settings"
        })
        .unwrap_or(false);
    if !name_ok {
        return o.fail(Exit::Generic, &format!("refusing to delete unexpected folder {}", root.display()));
    }
    let exists = root.is_dir();
    if g.dry_run || !g.yes {
        let what = if exists { format!("would delete {}", root.display()) } else { "nothing to delete".into() };
        if g.json {
            o.ok_text(&json!({ "ok": true, "dry_run": true, "targets": if exists { vec![root.display().to_string()] } else { vec![] } }).to_string());
        } else {
            o.ok_text(&what);
            if exists && !g.dry_run {
                o.ok_text("Run again with --yes to delete.");
            }
        }
        return 0;
    }
    if !exists {
        o.ok_text(if o.json { r#"{"ok":true,"deleted":[]}"# } else { "nothing to delete" });
        return 0;
    }
    match std::fs::remove_dir_all(root) {
        Ok(()) => {
            if o.json {
                o.ok_text(&json!({ "ok": true, "deleted": [root.display().to_string()] }).to_string());
            } else {
                o.ok_text(&format!("deleted {}", root.display()));
            }
            0
        }
        Err(e) => o.fail(Exit::Io, &format!("cannot delete {}: {e}", root.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use if_core::log::Logger;
    use if_core::paths::DataDirs;

    struct Fake;
    impl Platform for Fake {
        fn os_description(&self) -> String {
            r"Windows 10.0 build 22631 (C:\Users\sean)".into()
        }
        fn user_profile(&self) -> Option<String> {
            Some(r"C:\Users\sean".into())
        }
        fn helper_check(&self, elevated: bool) -> Result<HelperInfo, (Exit, String)> {
            if elevated {
                Err((Exit::ElevationDenied, "UAC prompt was declined".into()))
            } else {
                Ok(HelperInfo { version: if_core::APP_VERSION.into(), elevated: false, pid: 7 })
            }
        }
    }

    fn tmp(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("isofoundry-cli-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn call(args: &[&str], dirs: &DataDirs) -> (i32, String, String) {
        let a: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(&a, &Fake, dirs, &Logger::memory(), &mut out, &mut err);
        (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
    }

    fn dirs() -> DataDirs {
        DataDirs { root: tmp("unused").join("IsoFoundry"), portable: false }
    }

    #[test]
    fn help_lists_every_task() {
        let (code, out, _) = call(&["--help"], &dirs());
        assert_eq!(code, 0);
        for t in tasks::all() {
            assert!(out.contains(t.cli), "{}", t.cli);
        }
    }

    #[test]
    fn backup_needs_a_source_folder() {
        // Every task command is implemented since M6; `backup` with
        // no folder is a plain usage error, not a not-implemented.
        let (code, _, _) = call(&["backup", "--json"], &dirs());
        assert_eq!(code, 2);
    }

    #[test]
    fn unknown_command_is_usage_error() {
        let (code, _, err) = call(&["frobnicate"], &dirs());
        assert_eq!(code, 2);
        assert!(err.contains("unknown command"));
    }

    #[test]
    fn list_json_shape() {
        let (code, out, _) = call(&["list", "--json"], &dirs());
        assert_eq!(code, 0);
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["tasks"].as_array().unwrap().len(), tasks::all().count());
        assert_eq!(v["tasks"][0]["id"], "burn-image");
    }

    #[test]
    fn helper_check_paths() {
        let (code, out, _) = call(&["helper-check", "--json"], &dirs());
        assert_eq!(code, 0);
        assert!(out.contains("\"ok\":true"));
        let (code, _, err) = call(&["helper-check", "--elevated"], &dirs());
        assert_eq!(code, 20);
        assert!(err.contains("declined"));
        let (code, _, _) = call(&["helper-check", "--bogus"], &dirs());
        assert_eq!(code, 2);
    }

    #[test]
    fn report_is_redacted_and_written() {
        let d = tmp("report");
        let p = d.join("r.json");
        let (code, _, _) = call(&["report", "--out", p.to_str().unwrap()], &dirs());
        assert_eq!(code, 0);
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(!text.contains("sean"));
        assert!(text.contains("%USERPROFILE%"));
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn clean_needs_yes_and_only_touches_our_folder() {
        let base = tmp("clean");
        let root = base.join("IsoFoundry");
        std::fs::create_dir_all(root.join("logs")).unwrap();
        let d = DataDirs { root: root.clone(), portable: false };
        let (code, out, _) = call(&["clean"], &d);
        assert_eq!(code, 0);
        assert!(out.contains("--yes"));
        assert!(root.exists());
        let (code, _, _) = call(&["clean", "--dry-run", "--yes"], &d);
        assert_eq!(code, 0);
        assert!(root.exists());
        let (code, _, _) = call(&["clean", "--yes"], &d);
        assert_eq!(code, 0);
        assert!(!root.exists());
        let odd = DataDirs { root: base.join("Documents"), portable: false };
        std::fs::create_dir_all(&odd.root).unwrap();
        let (code, _, _) = call(&["clean", "--yes"], &odd);
        assert_eq!(code, 1);
        assert!(odd.root.exists());
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn roadmap_mentions_every_task() {
        let r = roadmap_text();
        for t in tasks::all() {
            assert!(r.contains(strings::task_title(t.id)));
        }
    }
}
