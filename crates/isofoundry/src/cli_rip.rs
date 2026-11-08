//! The M5 rip / copy / scan commands: `copy-to-image`, `copy-disc`,
//! `rip` (audio), `burn-audio`, `surface-test` and `recover`.
//!
//! All run on the if-drive layer. `--fake` on a read command uses a
//! simulated *source* disc (a small real ISO for data commands, a
//! generated audio CD for `rip`), so every flow runs end to end
//! with no drive attached. Real burns need `--yes`, like M4.

use crate::cli::{Global, Out};
use if_core::error::Exit;
use if_core::job::{CancelToken, Progress};
use if_drive::rip;
use serde_json::json;
use std::io::Read as _;
use std::path::{Path, PathBuf};

#[derive(Default)]
struct Args {
    pos: Vec<String>,
    drive: Option<String>,
    from: Option<String>,
    to: Option<String>,
    out: Option<String>,
    format: Option<String>,
    retries: Option<u32>,
    gap: Option<u32>,
    fake: bool,
    yes: bool,
    simulate: bool,
    raw: bool,
    subchannel: bool,
    cue: bool,
    no_secure: bool,
    accuraterip: bool,
    on_the_fly: bool,
}

const BOOL_FLAGS: &[&str] =
    &["fake", "yes", "simulate", "raw", "subchannel", "cue", "no-secure", "accuraterip", "on-the-fly"];

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
                    "raw" => a.raw = true,
                    "subchannel" => a.subchannel = true,
                    "cue" => a.cue = true,
                    "no-secure" => a.no_secure = true,
                    "accuraterip" => a.accuraterip = true,
                    "on-the-fly" => a.on_the_fly = true,
                    _ => unreachable!(),
                }
                i += 1;
                continue;
            }
            let value = tail.get(i + 1).ok_or_else(|| format!("--{name} needs a value"))?.clone();
            match name {
                "drive" => a.drive = Some(value),
                "from" => a.from = Some(value),
                "to" => a.to = Some(value),
                "out" => a.out = Some(value),
                "format" => a.format = Some(value),
                "retries" => a.retries = Some(value.parse().map_err(|_| "--retries must be a number".to_string())?),
                "gap" => a.gap = Some(value.parse().map_err(|_| "--gap must be a number of seconds".to_string())?),
                other => return Err(format!("unknown option --{other}")),
            }
            i += 2;
            continue;
        }
        a.pos.push(t.clone());
        i += 1;
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

/// Picks a real drive by letter/index (same rules as the M4 CLI).
fn pick_real(want: &Option<String>) -> Result<if_drive::DriveInfo, String> {
    let drives = if_drive::enumerate();
    if drives.is_empty() {
        return Err("no optical drives were found — connect a drive, or use --fake for the simulated drive".to_string());
    }
    match want {
        None => {
            if drives.len() == 1 {
                Ok(drives.into_iter().next().unwrap())
            } else {
                Err("more than one drive found — pick one with --drive <letter> or --drive <index>".to_string())
            }
        }
        Some(w) => {
            let w = w.trim().trim_end_matches(':');
            if let Ok(idx) = w.parse::<usize>() {
                return drives.into_iter().nth(idx).ok_or_else(|| format!("drive index {idx} is out of range"));
            }
            let letter = w.chars().next().ok_or("empty drive value")?.to_ascii_uppercase();
            drives
                .into_iter()
                .find(|d| d.letter == Some(letter))
                .ok_or_else(|| format!("no drive with letter {letter}: was found"))
        }
    }
}

/// Builds a tiny but real ISO (three small files) used as the
/// simulated data disc's content.
pub(crate) fn sample_iso_bytes() -> Result<Vec<u8>, String> {
    let dir = std::env::temp_dir().join(format!("isofoundry-sample-{}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let iso = dir.join("sample.iso");
    let result = (|| {
        use if_image::write::{build, Node, Options};
        let root = Node::dir(
            "",
            0,
            vec![
                Node::bytes(
                    "README.TXT",
                    0,
                    b"IsoFoundry simulated source disc.\nRipped by the M5 test gate.\n".to_vec(),
                ),
                Node::bytes("HELLO.TXT", 0, b"Hello from the fake drive.\n".to_vec()),
                Node::dir(
                    "DATA",
                    0,
                    vec![Node::bytes("PATTERN.BIN", 0, (0..50000u32).map(|i| (i % 253) as u8).collect())],
                ),
            ],
        );
        build(&root, Options { label: "ISOFOUNDRY_TEST".into(), ..Options::default() }, &iso, |_, _| true)
            .map_err(|e| e.to_string())?;
        std::fs::read(&iso).map_err(|e| e.to_string())
    })();
    std::fs::remove_dir_all(&dir).ok();
    result
}

/// The simulated data source drive (with optional damage map).
pub(crate) fn fake_data_drive(bad: &[u32], slow: &[u32]) -> Result<if_drive::FakeDrive, String> {
    let bytes = sample_iso_bytes()?;
    let mut d = if_drive::FakeDrive::new(if_drive::FakeConfig {
        media_blank: false,
        bad_sectors: bad.to_vec(),
        slow_sectors: slow.to_vec(),
        ..if_drive::FakeConfig::default()
    });
    d.load_image(&bytes);
    Ok(d)
}

/// Opens the source drive for a read command.
fn open_source(a: &Args) -> Result<(Box<dyn if_drive::OpticalDrive>, String), String> {
    if a.fake {
        let d = fake_data_drive(&[], &[])?;
        return Ok((Box::new(d), "simulated data disc".to_string()));
    }
    let info = pick_real(&a.drive)?;
    let desc = info.display_name();
    Ok((if_drive::open(&info).map_err(|e| e.to_string())?, desc))
}

fn hash_file(path: &Path) -> Result<if_hash::Digests, String> {
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut h = if_hash::Hasher::new(if_hash::Algorithms::parse("crc32,md5,sha1,sha256")?);
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finish())
}

fn dat_line(path: &Path, d: &if_hash::Digests) -> String {
    format!(
        "rom ( name \"{}\" size {} crc {} md5 {} sha1 {} sha256 {} )",
        path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
        d.size,
        d.crc32.as_deref().unwrap_or_default(),
        d.md5.as_deref().unwrap_or_default(),
        d.sha1.as_deref().unwrap_or_default(),
        d.sha256.as_deref().unwrap_or_default()
    )
}

/// Writes the CUE (raw rips), dump report and hash file that
/// belong next to a ripped image. Returns the paths written.
pub(crate) fn write_sidecars(
    dest: &Path,
    toc: &if_drive::Toc,
    desc: &str,
    report: &rip::RipImageReport,
    raw: bool,
) -> Vec<String> {
    let mut sidecars = Vec::new();
    if raw && toc.sector_size == 2352 {
        let cue = rip::cue_sheet(toc, &dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        let cue_path = rip::sidecar(dest, "cue");
        if std::fs::write(&cue_path, cue).is_ok() {
            sidecars.push(cue_path.display().to_string());
        }
    }
    let extra: Vec<String> = report.notes.clone();
    let report_text =
        rip::dump_report(desc, toc, &dest.display().to_string(), &report.bad_sectors, report.retries_used, &extra);
    let report_path = rip::sidecar(dest, "report.txt");
    if std::fs::write(&report_path, report_text).is_ok() {
        sidecars.push(report_path.display().to_string());
    }
    if let Ok(d) = hash_file(dest) {
        let dat = format!(
            "game (\n\tname \"IsoFoundry dump\"\n\tdescription \"dumped by IsoFoundry\"\n\t{}\n)\n",
            dat_line(dest, &d)
        );
        let dat_path = rip::sidecar(dest, "dat");
        if std::fs::write(&dat_path, dat).is_ok() {
            sidecars.push(dat_path.display().to_string());
        }
    }
    sidecars
}

pub(crate) fn run_copy_to_image(tail: &[String], g: &Global, o: &mut Out) -> i32 {
    let a = match parse_args(tail) {
        Ok(a) => a,
        Err(e) => return o.fail(Exit::Usage, &e),
    };
    let dest = match a.pos.first() {
        Some(p) => PathBuf::from(p),
        None => {
            return o.fail(
                Exit::Usage,
                "usage: copy-to-image <image> [--drive E:|--fake] [--raw] [--subchannel] [--retries N]",
            )
        }
    };
    let (mut drive, desc) = match open_source(&a) {
        Ok(d) => d,
        Err(e) => return o.fail(Exit::Generic, &e),
    };
    if g.dry_run {
        o.ok_text(&format!("plan: rip {desc} to {}", dest.display()));
        return 0;
    }
    let progress = Progress::default();
    let opts = rip::RipImageOptions { raw: a.raw, subchannel: a.subchannel, retries: a.retries.unwrap_or(3) };
    let toc = match drive.toc() {
        Ok(t) => t,
        Err(e) => return o.fail(Exit::Generic, &e.to_string()),
    };
    match rip::rip_image(&mut *drive, &dest, &opts, &progress, &CancelToken::new()) {
        Ok(report) => {
            let sidecars = write_sidecars(&dest, &toc, &desc, &report, opts.raw);
            let msg = format!(
                "ripped {} sectors ({} bytes) to {}{}",
                report.sectors,
                report.bytes_written,
                dest.display(),
                if report.bad_sectors.is_empty() {
                    String::new()
                } else {
                    format!(" — {} sectors were unreadable and zero-filled", report.bad_sectors.len())
                }
            );
            if o.json {
                o.ok_text(&json!({"sectors": report.sectors, "bytes": report.bytes_written, "bad_sectors": report.bad_sectors, "retries": report.retries_used, "image": dest.display().to_string(), "sidecars": sidecars}).to_string());
            } else {
                o.ok_text(&msg);
            }
            0
        }
        Err(e) => o.fail(Exit::Generic, &e.to_string()),
    }
}

pub(crate) fn run_copy_disc(tail: &[String], g: &Global, o: &mut Out) -> i32 {
    let a = match parse_args(tail) {
        Ok(a) => a,
        Err(e) => return o.fail(Exit::Usage, &e),
    };
    // Source.
    let (mut src, src_desc): (Box<dyn if_drive::OpticalDrive>, String) = if a.fake {
        match fake_data_drive(&[], &[]) {
            Ok(d) => (Box::new(d), "simulated data disc".to_string()),
            Err(e) => return o.fail(Exit::Generic, &e),
        }
    } else {
        match pick_real(&a.from) {
            Ok(info) => {
                let desc = info.display_name();
                match if_drive::open(&info) {
                    Ok(d) => (d, desc),
                    Err(e) => return o.fail(Exit::Generic, &e.to_string()),
                }
            }
            Err(e) => return o.fail(Exit::Generic, &e),
        }
    };
    // Destination.
    let (mut dst, dst_info): (Box<dyn if_drive::OpticalDrive>, if_drive::DriveInfo) = if a.fake {
        (Box::new(if_drive::fake_drive()), fake_info())
    } else {
        match pick_real(&a.to) {
            Ok(info) => match if_drive::open(&info) {
                Ok(d) => (d, info),
                Err(e) => return o.fail(Exit::Generic, &e.to_string()),
            },
            Err(e) => return o.fail(Exit::Generic, &e),
        }
    };
    if dst_info.id != "fake" && !g.dry_run && !a.yes {
        o.ok_text(&format!("plan: copy {src_desc} to {} — re-run with --yes to burn", dst_info.display_name()));
        return 0;
    }
    let tmp = std::env::temp_dir().join(format!("isofoundry-copy-{}.iso", std::process::id()));
    let progress = Progress::default();
    let rip_opts = rip::RipImageOptions::default();
    if let Err(e) = rip::rip_image(&mut *src, &tmp, &rip_opts, &progress, &CancelToken::new()) {
        return o.fail(Exit::Generic, &e.to_string());
    }
    if a.on_the_fly {
        // The sector stream went drive -> temp -> drive either
        // way; say so plainly rather than claiming a true
        // on-the-fly pipe (see LIMITATIONS).
        o.ok_text(
            "note: copying runs through a temporary image on this build (true on-the-fly streaming is not wired up)",
        );
    }
    if g.dry_run {
        std::fs::remove_file(&tmp).ok();
        o.ok_text("plan only (dry run)");
        return 0;
    }
    let burn_opts = if_drive::BurnOptions { verify: true, close_disc: true, ..Default::default() };
    let result = dst.burn(&if_drive::BurnSource::Image(tmp.clone()), &burn_opts, &progress, &CancelToken::new());
    std::fs::remove_file(&tmp).ok();
    match result {
        Ok(r) => {
            if o.json {
                o.ok_text(&json!({"bytes": r.bytes_written, "verified": r.verified, "backend": r.backend}).to_string());
            } else {
                o.ok_text(&format!(
                    "copied {src_desc} to {} ({} bytes{})",
                    dst_info.display_name(),
                    r.bytes_written,
                    if r.verified { ", verified" } else { "" }
                ));
            }
            0
        }
        Err(e) => o.fail(Exit::Generic, &e.to_string()),
    }
}

pub(crate) fn write_wav(path: &Path, pcm: &[u8]) -> Result<(), String> {
    let data_len = pcm.len() as u32;
    let mut out = Vec::with_capacity(44 + pcm.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&2u16.to_le_bytes()); // stereo
    out.extend_from_slice(&44100u32.to_le_bytes());
    out.extend_from_slice(&(44100u32 * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    out.extend_from_slice(pcm);
    std::fs::write(path, out).map_err(|e| e.to_string())
}

/// Writes ripped audio tracks into `out_dir` in the chosen
/// format, plus the rip log and (for WAV) an optional CUE sheet.
/// Returns the file paths written.
pub(crate) fn write_ripped_tracks(
    tracks: &[rip::RippedTrack],
    out_dir: &Path,
    format: &str,
    cue: bool,
    accuraterip_flag: bool,
    desc: &str,
) -> Result<Vec<String>, String> {
    std::fs::create_dir_all(out_dir).map_err(|e| e.to_string())?;
    let mut written = Vec::new();
    let mut log_lines = vec![format!("Ripped {desc}")];
    for t in tracks {
        let safe_title: String =
            t.title.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' { c } else { '_' }).collect();
        let stem = format!("{:02} - {}", t.number, safe_title.trim());
        let wav = out_dir.join(format!("{stem}.wav"));
        write_wav(&wav, &t.pcm)?;
        let final_path = if format == "wav" {
            wav.clone()
        } else {
            let target = if_audio::Format::parse(format).map_err(|e| e.to_string())?;
            let dest = out_dir.join(format!("{stem}.{}", target.extension()));
            let opts = if_audio::ConvertOptions { format: target, ..Default::default() };
            if_audio::convert_file(&wav, &dest, &opts, &Progress::default(), &CancelToken::new())
                .map_err(|e| format!("converting track {}: {e}", t.number))?;
            std::fs::remove_file(&wav).ok();
            dest
        };
        log_lines.push(format!(
            "Track {:02} {}: {} sectors, {} bad, AccurateRip v1 {:08x} v2 {:08x}{}",
            t.number,
            t.title,
            t.pcm.len() / 2352,
            t.bad_sectors.len(),
            t.accuraterip_v1,
            t.accuraterip_v2,
            if accuraterip_flag { "" } else { " (checksums computed; --accuraterip not set)" }
        ));
        written.push(final_path.display().to_string());
    }
    if cue && format == "wav" {
        let mut cue_text = String::new();
        for (i, t) in tracks.iter().enumerate() {
            let name = Path::new(&written[i]).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            cue_text += &format!("FILE \"{name}\" WAVE\n  TRACK {:02} AUDIO\n    INDEX 01 00:00:00\n", t.number);
        }
        std::fs::write(out_dir.join("disc.cue"), cue_text).map_err(|e| e.to_string())?;
    }
    std::fs::write(out_dir.join("rip.log"), log_lines.join("\n") + "\n").map_err(|e| e.to_string())?;
    Ok(written)
}

pub(crate) fn run_rip(tail: &[String], g: &Global, o: &mut Out) -> i32 {
    let a = match parse_args(tail) {
        Ok(a) => a,
        Err(e) => return o.fail(Exit::Usage, &e),
    };
    let format = a.format.as_deref().unwrap_or("wav").to_lowercase();
    if !["wav", "flac", "mp3"].contains(&format.as_str()) {
        return o.fail(Exit::Usage, "--format must be wav, flac or mp3");
    }
    let out_dir = PathBuf::from(a.out.clone().unwrap_or_else(|| "rip".to_string()));
    let (mut drive, desc): (Box<dyn if_drive::OpticalDrive>, String) = if a.fake {
        (Box::new(if_drive::fake_source_audio()), "simulated audio CD".to_string())
    } else {
        match pick_real(&a.drive) {
            Ok(info) => {
                let d = info.display_name();
                match if_drive::open(&info) {
                    Ok(x) => (x, d),
                    Err(e) => return o.fail(Exit::Generic, &e.to_string()),
                }
            }
            Err(e) => return o.fail(Exit::Generic, &e),
        }
    };
    if g.dry_run {
        o.ok_text(&format!("plan: rip {desc} to {}", out_dir.display()));
        return 0;
    }
    let toc = match drive.toc() {
        Ok(t) => t,
        Err(e) => return o.fail(Exit::Generic, &e.to_string()),
    };
    let opts = rip::RipAudioOptions { secure: !a.no_secure, retries: a.retries.unwrap_or(3) };
    let progress = Progress::default();
    match rip::rip_audio(&mut *drive, &opts, &progress, &CancelToken::new()) {
        Ok(tracks) => {
            let written = match write_ripped_tracks(&tracks, &out_dir, &format, a.cue, a.accuraterip, &desc) {
                Ok(w) => w,
                Err(e) => return o.fail(Exit::Generic, &e),
            };
            let _ = toc;
            if o.json {
                o.ok_text(
                    &json!({"tracks": written.len(), "files": written, "out": out_dir.display().to_string()})
                        .to_string(),
                );
            } else {
                o.ok_text(&format!("ripped {} tracks from {desc} to {}", written.len(), out_dir.display()));
            }
            0
        }
        Err(e) => o.fail(Exit::Generic, &e.to_string()),
    }
}

/// Decodes one audio file to CD-ready PCM (44.1 kHz, 16-bit,
/// stereo, whole 2352-byte frames), resampling linearly when the
/// source rate differs.
pub(crate) fn decode_for_cd(path: &Path) -> Result<(Vec<u8>, String), String> {
    let decoded =
        if_audio::decode::decode_all(path, &Progress::default(), &CancelToken::new()).map_err(|e| e.to_string())?;
    let channels = decoded.channels.max(1) as usize;
    let frames_in = decoded.samples.len() / channels;
    let frames_out = if decoded.sample_rate == 44100 {
        frames_in
    } else {
        (frames_in as f64 * 44100.0 / decoded.sample_rate as f64) as usize
    };
    let mut pcm = Vec::with_capacity(frames_out * 4);
    for i in 0..frames_out {
        let src_pos =
            if decoded.sample_rate == 44100 { i as f64 } else { i as f64 * decoded.sample_rate as f64 / 44100.0 };
        let i0 = (src_pos as usize).min(frames_in.saturating_sub(1));
        let i1 = (i0 + 1).min(frames_in.saturating_sub(1));
        let frac = (src_pos - i0 as f64) as f32;
        for ch in 0..2usize {
            let c = ch.min(channels - 1);
            let s0 = decoded.samples[i0 * channels + c];
            let s1 = decoded.samples[i1 * channels + c];
            let v = s0 + (s1 - s0) * frac;
            let s = (v.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            pcm.extend_from_slice(&s.to_le_bytes());
        }
    }
    // Pad to a whole number of CD frames with silence.
    let rem = pcm.len() % 2352;
    if rem != 0 {
        pcm.resize(pcm.len() + (2352 - rem), 0);
    }
    let title = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Track".to_string());
    Ok((pcm, title))
}

pub(crate) fn run_burn_audio(tail: &[String], g: &Global, o: &mut Out) -> i32 {
    let a = match parse_args(tail) {
        Ok(a) => a,
        Err(e) => return o.fail(Exit::Usage, &e),
    };
    if a.pos.is_empty() {
        return o.fail(Exit::Usage, "usage: burn-audio <audio files...> [--drive E:|--fake] [--gap N] [--yes]");
    }
    let gap_sectors = a.gap.unwrap_or(2) * 75;
    let mut tracks = Vec::new();
    for p in &a.pos {
        match decode_for_cd(Path::new(p)) {
            Ok((pcm, title)) => {
                tracks.push(if_drive::AudioTrackData { title, performer: String::new(), pcm, gap_sectors })
            }
            Err(e) => return o.fail(Exit::Generic, &format!("{p}: {e}")),
        }
    }
    let (mut drive, info): (Box<dyn if_drive::OpticalDrive>, if_drive::DriveInfo) = if a.fake {
        (Box::new(if_drive::fake_drive()), fake_info())
    } else {
        match pick_real(&a.drive) {
            Ok(info) => match if_drive::open(&info) {
                Ok(d) => (d, info),
                Err(e) => return o.fail(Exit::Generic, &e.to_string()),
            },
            Err(e) => return o.fail(Exit::Generic, &e),
        }
    };
    if info.id != "fake" && !g.dry_run && !a.yes {
        o.ok_text(&format!(
            "plan: burn {} audio tracks to {} — re-run with --yes to burn",
            tracks.len(),
            info.display_name()
        ));
        return 0;
    }
    if g.dry_run {
        o.ok_text(&format!("plan: burn {} audio tracks (dry run)", tracks.len()));
        return 0;
    }
    let opts = if_drive::AudioBurnOptions { simulate: a.simulate, ..Default::default() };
    let progress = Progress::default();
    match drive.burn_audio(&tracks, &opts, &progress, &CancelToken::new()) {
        Ok(r) => {
            if o.json {
                o.ok_text(
                    &json!({"tracks": tracks.len(), "bytes": r.bytes_written, "backend": r.backend, "notes": r.notes})
                        .to_string(),
                );
            } else {
                o.ok_text(&format!(
                    "burned {} audio tracks ({} bytes) on {}",
                    tracks.len(),
                    r.bytes_written,
                    r.backend
                ));
            }
            0
        }
        Err(e) => o.fail(Exit::Generic, &e.to_string()),
    }
}

pub(crate) fn run_surface_test(tail: &[String], _g: &Global, o: &mut Out) -> i32 {
    let a = match parse_args(tail) {
        Ok(a) => a,
        Err(e) => return o.fail(Exit::Usage, &e),
    };
    let (mut drive, desc) = match open_source(&a) {
        Ok(d) => d,
        Err(e) => return o.fail(Exit::Generic, &e),
    };
    let progress = Progress::default();
    match rip::surface_scan(&mut *drive, &progress, &CancelToken::new()) {
        Ok(r) => {
            if o.json {
                o.ok_text(&json!({"sectors": r.total_sectors, "good": r.good, "slow": r.slow, "bad": r.bad, "quality_score": r.quality_score, "quality_grade": r.quality_grade.to_string()}).to_string());
            } else {
                o.ok_text(&format!(
                    "surface of {desc}: {} sectors — {} good, {} slow, {} bad; quality {:.1}% (grade {})",
                    r.total_sectors,
                    r.good,
                    r.slow,
                    r.bad.len(),
                    r.quality_score,
                    r.quality_grade
                ));
            }
            0
        }
        Err(e) => o.fail(Exit::Generic, &e.to_string()),
    }
}

/// Extracts files from a tolerantly ripped image and reports
/// which files overlap the bad sectors (their regions are
/// zero-filled). Returns (file count, damaged file paths).
pub(crate) fn extract_recovered(image: &Path, dest: &Path, bad: &[u32]) -> Result<(usize, Vec<String>), String> {
    let (_format, src) = if_image::container::open(image).map_err(|e| e.to_string())?;
    let mut vol = if_image::fs::Volume::open(src, None).map_err(|e| e.to_string())?;
    let root = vol.root();
    let items = vol.walk(&root).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let mut damaged = Vec::new();
    let mut files = 0usize;
    for (rel, entry) in &items {
        if entry.dir {
            continue;
        }
        files += 1;
        if let if_image::fs::Data::Extents(extents) = &entry.data {
            let hit = extents.iter().any(|x| {
                let start = x.lba as u32;
                let end = (x.lba + x.len.div_ceil(2048)) as u32;
                bad.iter().any(|b| *b >= start && *b < end)
            });
            if hit {
                damaged.push(rel.clone());
            }
        }
    }
    vol.extract(&root, dest, |_, _| true).map_err(|e| e.to_string())?;
    Ok((files, damaged))
}

pub(crate) fn run_recover(tail: &[String], _g: &Global, o: &mut Out) -> i32 {
    let a = match parse_args(tail) {
        Ok(a) => a,
        Err(e) => return o.fail(Exit::Usage, &e),
    };
    let dest = match a.pos.first() {
        Some(p) => PathBuf::from(p),
        None => return o.fail(Exit::Usage, "usage: recover <folder> [--drive E:|--fake]"),
    };
    // The fake source for recover is deliberately damaged, so the
    // flow can be tried end to end anywhere.
    let (mut drive, desc): (Box<dyn if_drive::OpticalDrive>, String) = if a.fake {
        match fake_data_drive(&[280, 281], &[290]) {
            Ok(d) => (Box::new(d), "simulated damaged disc".to_string()),
            Err(e) => return o.fail(Exit::Generic, &e),
        }
    } else {
        match pick_real(&a.drive) {
            Ok(info) => {
                let d = info.display_name();
                match if_drive::open(&info) {
                    Ok(x) => (x, d),
                    Err(e) => return o.fail(Exit::Generic, &e.to_string()),
                }
            }
            Err(e) => return o.fail(Exit::Generic, &e),
        }
    };
    let tmp = std::env::temp_dir().join(format!("isofoundry-recover-{}.iso", std::process::id()));
    let progress = Progress::default();
    let opts = rip::RipImageOptions { retries: 5, ..Default::default() };
    let report = match rip::rip_image(&mut *drive, &tmp, &opts, &progress, &CancelToken::new()) {
        Ok(r) => r,
        Err(e) => return o.fail(Exit::Generic, &e.to_string()),
    };
    let bad = report.bad_sectors.clone();
    let result = extract_recovered(&tmp, &dest, &bad);
    std::fs::remove_file(&tmp).ok();
    match result {
        Ok((files, damaged)) => {
            let report_text = format!(
                "Recovered from {desc}\nUnreadable sectors ({}): {}\nFiles extracted: {files}\nFiles with damaged (zero-filled) regions: {}\n",
                bad.len(),
                bad.iter().map(u32::to_string).collect::<Vec<_>>().join(", "),
                if damaged.is_empty() { "none".to_string() } else { damaged.join(", ") }
            );
            std::fs::write(dest.join("recover-report.txt"), &report_text).ok();
            if o.json {
                o.ok_text(&json!({"files": files, "damaged_files": damaged, "bad_sectors": bad, "out": dest.display().to_string()}).to_string());
            } else {
                o.ok_text(&format!(
                    "recovered {files} files from {desc} to {} ({} damaged)",
                    dest.display(),
                    damaged.len()
                ));
            }
            0
        }
        Err(e) => o.fail(Exit::Generic, &e),
    }
}
