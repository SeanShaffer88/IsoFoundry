//! CLI for audio conversion (M3): `convert-audio <files>... --to
//! <format> [--out DIR] [--bitrate N] [--replaygain] [--apply-gain]`.
//!
//! Batch by design: every input is converted, a failure on one file
//! is reported and the rest still run. Tags are carried across, and
//! ReplayGain can be measured into the output's tags or baked in.

use crate::cli::{Global, Out};
use if_audio::{ConvertOptions, Format};
use if_core::error::Exit;
use if_core::job::{CancelToken, Progress};
use serde_json::json;
use std::path::{Path, PathBuf};

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

/// Flags that never take a value (so a following file name is not
/// swallowed as their value).
const BOOL_FLAGS: [&str; 2] = ["replaygain", "apply-gain"];

fn parse_args(tail: &[String]) -> Args {
    let mut pos = Vec::new();
    let mut flags = Vec::new();
    let mut i = 0;
    while i < tail.len() {
        let a = &tail[i];
        if let Some(name) = a.strip_prefix("--") {
            let value = if !BOOL_FLAGS.contains(&name) && i + 1 < tail.len() && !tail[i + 1].starts_with("--") {
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

pub(crate) fn run(_cmd: &str, tail: &[String], g: &Global, o: &mut Out<'_>) -> i32 {
    let args = parse_args(tail);
    if args.pos.is_empty() {
        return o.fail(Exit::Usage, "convert-audio needs at least one input file (try --help)");
    }
    let Some(to) = args.flag("to") else {
        return o.fail(Exit::Usage, "convert-audio needs --to <wav|flac|mp3|aac|opus|vorbis>");
    };
    let format = match Format::parse(to) {
        Ok(f) => f,
        Err(e) => return o.fail(Exit::Usage, &e.0),
    };
    let bitrate: u32 = match args.flag("bitrate") {
        Some(v) => match v.parse() {
            Ok(n) if (32..=512).contains(&n) => n,
            _ => return o.fail(Exit::Usage, "--bitrate must be a number of kbit/s between 32 and 512"),
        },
        None => 192,
    };
    let opts = ConvertOptions {
        format,
        bitrate_kbps: bitrate,
        replaygain: args.has("replaygain"),
        apply_gain: args.has("apply-gain"),
    };
    let out_dir = args.flag("out").map(PathBuf::from);

    // Plan first: every destination is computed (and checked against
    // overwriting its own input) before anything runs.
    let mut jobs: Vec<(PathBuf, PathBuf)> = Vec::new();
    for src in &args.pos {
        let src = PathBuf::from(src);
        let dir = out_dir.clone().unwrap_or_else(|| src.parent().map(|p| p.to_path_buf()).unwrap_or_default());
        let stem = src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "audio".into());
        let dst = dir.join(format!("{stem}.{}", format.extension()));
        if same_file(&src, &dst) {
            return o.fail(
                Exit::Usage,
                &format!("{}: the output would replace the input — pick another --out folder or format", src.display()),
            );
        }
        jobs.push((src, dst));
    }

    if g.dry_run {
        if o.json {
            let list: Vec<_> = jobs
                .iter()
                .map(|(s, d)| json!({ "from": s.display().to_string(), "to": d.display().to_string() }))
                .collect();
            o.ok_text(&json!({ "ok": true, "dry_run": true, "jobs": list }).to_string());
        } else {
            for (s, d) in &jobs {
                o.ok_text(&format!("{} -> {} ({})", s.display(), d.display(), format.name()));
            }
        }
        return 0;
    }

    if let Some(dir) = &out_dir {
        if let Err(e) = std::fs::create_dir_all(dir) {
            return o.fail(Exit::Io, &format!("cannot create {}: {e}", dir.display()));
        }
    }

    let mut results = Vec::new();
    let mut failed = 0usize;
    for (src, dst) in &jobs {
        if !o.json {
            o.ok_text(&format!("{} -> {}", src.display(), dst.display()));
        }
        let progress = Progress::default();
        let cancel = CancelToken::new();
        match if_audio::convert_file(src, dst, &opts, &progress, &cancel) {
            Ok(rep) => {
                results.push(json!({
                    "from": src.display().to_string(),
                    "to": dst.display().to_string(),
                    "ok": true,
                    "frames": rep.frames,
                    "sample_rate": rep.sample_rate,
                    "channels": rep.channels,
                    "replaygain_db": rep.gain_db,
                    "peak": rep.peak,
                }));
                if !o.json {
                    let mut line =
                        format!("  done: {} frames, {} Hz, {} ch", rep.frames, rep.sample_rate, rep.channels);
                    if let Some(gain) = rep.gain_db {
                        line.push_str(&format!(", ReplayGain {gain:+.2} dB"));
                    }
                    o.ok_text(&line);
                }
            }
            Err(e) => {
                failed += 1;
                results.push(json!({ "from": src.display().to_string(), "to": dst.display().to_string(), "ok": false, "error": e.0 }));
                let _ = writeln!(o.err, "  failed: {}", e.0);
            }
        }
    }

    if o.json {
        o.ok_text(
            &json!({ "ok": failed == 0, "converted": jobs.len() - failed, "failed": failed, "results": results })
                .to_string(),
        );
    } else if failed > 0 {
        o.ok_text(&format!("Converted {} of {} files; {} failed.", jobs.len() - failed, jobs.len(), failed));
    }
    if failed == 0 {
        0
    } else {
        Exit::Generic as i32
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(x), Ok(y)) => x == y,
        _ => a == b,
    }
}
