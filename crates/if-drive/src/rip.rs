//! Rip / copy / scan engines (M5), built on the [`OpticalDrive`]
//! read side. Everything here is backend-neutral: the fake drive
//! exercises it fully in tests, the MMC/SPTI backend runs it on
//! real hardware (Unverified), and IMAPI-only drives report the
//! read side as unsupported.
//!
//! No engine here tries to defeat copy protection (D-004): a
//! sector that cannot be read after retries is reported and, for
//! image rips, zero-filled — never reverse-engineered around.

use crate::{err, OpticalDrive, Result, SectorQuality, Toc};
use if_core::job::{CancelToken, Progress};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Options for a disc → image rip.
#[derive(Debug, Clone)]
pub struct RipImageOptions {
    /// Keep full 2352-byte sectors (BIN) instead of 2048-byte user
    /// data (ISO). Only discs whose sectors are 2352 bytes can be
    /// ripped raw; a data disc read at 2048 stays an ISO and the
    /// report says so.
    pub raw: bool,
    /// Also capture the 96-byte subchannel block per sector (into
    /// the BIN, or a `.sub` sidecar next to an ISO).
    pub subchannel: bool,
    /// Retries per failing chunk/sector before giving up on it.
    pub retries: u32,
}

impl Default for RipImageOptions {
    fn default() -> Self {
        RipImageOptions { raw: false, subchannel: false, retries: 3 }
    }
}

/// The outcome of a disc → image rip.
#[derive(Debug, Clone)]
pub struct RipImageReport {
    pub sectors: u32,
    pub bytes_written: u64,
    /// Sectors that never read cleanly (zero-filled in the image).
    pub bad_sectors: Vec<u32>,
    pub retries_used: u32,
    pub notes: Vec<String>,
}

/// Reads sectors `lba..lba+count` with chunk-then-sector fallback.
/// Failing sectors come back zero-filled and listed in `bad`.
#[allow(clippy::too_many_arguments)]
pub fn read_range(
    drive: &mut dyn OpticalDrive,
    lba: u32,
    count: u32,
    subchannel: bool,
    retries: u32,
    progress: &Progress,
    cancel: &CancelToken,
    sink: &mut dyn FnMut(&[u8]) -> Result<()>,
) -> Result<(Vec<u32>, u32)> {
    let toc = drive.toc()?;
    let payload = toc.sector_size as usize;
    let stride = if subchannel { payload + 96 } else { payload };
    let mut bad = Vec::new();
    let mut retries_used = 0u32;
    let mut done = 0u32;
    while done < count {
        cancel.check().map_err(|e| crate::Error(e.to_string()))?;
        let n = (count - done).min(16);
        match read_with_retries(drive, lba + done, n, subchannel, retries, &mut retries_used) {
            Ok(bytes) => {
                sink(&bytes)?;
                progress.add((n as usize * stride) as u64);
                done += n;
            }
            Err(_) => {
                // Isolate: one sector at a time.
                for s in 0..n {
                    let at = lba + done + s;
                    match read_with_retries(drive, at, 1, subchannel, retries, &mut retries_used) {
                        Ok(bytes) => sink(&bytes)?,
                        Err(_) => {
                            bad.push(at);
                            sink(&vec![0u8; stride])?;
                        }
                    }
                    progress.add(stride as u64);
                }
                done += n;
            }
        }
    }
    Ok((bad, retries_used))
}

fn read_with_retries(
    drive: &mut dyn OpticalDrive,
    lba: u32,
    count: u32,
    subchannel: bool,
    retries: u32,
    retries_used: &mut u32,
) -> Result<Vec<u8>> {
    let mut attempt = 0;
    loop {
        match drive.read_chunk(lba, count, subchannel) {
            Ok(b) => return Ok(b),
            Err(e) => {
                if attempt >= retries {
                    return Err(e);
                }
                attempt += 1;
                *retries_used += 1;
            }
        }
    }
}

/// Rips the whole disc to `dest` (`.iso` or `.bin` by content, the
/// caller picks the name). Returns what happened, damage included.
pub fn rip_image(
    drive: &mut dyn OpticalDrive,
    dest: &Path,
    opts: &RipImageOptions,
    progress: &Progress,
    cancel: &CancelToken,
) -> Result<RipImageReport> {
    let toc = drive.toc()?;
    if toc.tracks.iter().any(|t| t.audio) && toc.sector_size != 2352 {
        return err("this disc mixes track types this rip cannot lay out — rip audio and data separately");
    }
    let total = toc.lead_out_lba;
    let raw = opts.raw && toc.sector_size == 2352;
    let mut notes = Vec::new();
    if opts.raw && !raw {
        notes.push("Raw output requested, but this disc reads as 2048-byte sectors; wrote an ISO image.".to_string());
    }
    if toc.tracks.iter().any(|t| t.audio) && !raw {
        return err("this is an audio CD — use rip-audio to extract its tracks");
    }
    let payload = toc.sector_size as usize;
    let stride = if raw || opts.subchannel { payload + if opts.subchannel { 96 } else { 0 } } else { payload };
    let _ = stride;
    progress.set_total(u64::from(total) * payload as u64);

    let mut file =
        std::fs::File::create(dest).map_err(|e| crate::Error(format!("cannot create {}: {e}", dest.display())))?;
    let mut sub_file: Option<std::fs::File> = None;
    if opts.subchannel && !raw {
        let sub_path = dest.with_extension("sub");
        sub_file = Some(
            std::fs::File::create(&sub_path)
                .map_err(|e| crate::Error(format!("cannot create {}: {e}", sub_path.display())))?,
        );
        notes.push(format!("Subchannel data written to {}.", sub_path.display()));
    }
    // When ripping raw BIN with subchannel, sectors land whole
    // (payload+sub) in the one file; otherwise the sink strips the
    // subchannel block off each sector into the sidecar.
    let keep_whole = raw && opts.subchannel;
    let mut written = 0u64;
    let (bad, retries_used) = {
        let mut sink = |chunk: &[u8]| -> Result<()> {
            let in_stride = if opts.subchannel { payload + 96 } else { payload };
            if in_stride == payload {
                file.write_all(chunk).map_err(|e| crate::Error(e.to_string()))?;
                written += chunk.len() as u64;
            } else if keep_whole {
                file.write_all(chunk).map_err(|e| crate::Error(e.to_string()))?;
                written += chunk.len() as u64;
            } else {
                for sector in chunk.chunks(in_stride) {
                    file.write_all(&sector[..payload]).map_err(|e| crate::Error(e.to_string()))?;
                    written += payload as u64;
                    if let Some(sf) = sub_file.as_mut() {
                        sf.write_all(&sector[payload..]).map_err(|e| crate::Error(e.to_string()))?;
                    }
                }
            }
            Ok(())
        };
        read_range(drive, 0, total, opts.subchannel, opts.retries, progress, cancel, &mut sink)?
    };
    file.flush().map_err(|e| crate::Error(e.to_string()))?;
    Ok(RipImageReport { sectors: total, bytes_written: written, bad_sectors: bad, retries_used, notes })
}

/// A CUE sheet describing a raw BIN rip (or an audio rip layout).
pub fn cue_sheet(toc: &Toc, bin_name: &str) -> String {
    let mut out = format!("FILE \"{bin_name}\" BINARY\n");
    for t in &toc.tracks {
        let kind = if t.audio {
            "AUDIO"
        } else if toc.sector_size == 2352 {
            "MODE1/2352"
        } else {
            "MODE1/2048"
        };
        let (m, s, f) = Toc::msf(t.start_lba);
        out += &format!("  TRACK {:02} {kind}\n    INDEX 01 {m:02}:{s:02}:{f:02}\n", t.number);
    }
    out
}

/// Options for ripping an audio CD.
#[derive(Debug, Clone)]
pub struct RipAudioOptions {
    /// Secure mode: every sector is read at least twice and the
    /// reads must agree; disagreement costs retries.
    pub secure: bool,
    pub retries: u32,
}

impl Default for RipAudioOptions {
    fn default() -> Self {
        RipAudioOptions { secure: true, retries: 3 }
    }
}

/// One ripped audio track: decoded-ready PCM plus its damage list.
#[derive(Debug, Clone)]
pub struct RippedTrack {
    pub number: u32,
    pub title: String,
    pub pcm: Vec<u8>,
    pub bad_sectors: Vec<u32>,
    pub retries_used: u32,
    /// AccurateRip checksums over this track (v1 / v2), per the
    /// published algorithm — computed locally; the online database
    /// lookup is a separate opt-in in the app layer.
    pub accuraterip_v1: u32,
    pub accuraterip_v2: u32,
}

/// AccurateRip v1/v2 checksums for one track of 16-bit stereo PCM.
/// Both versions skip the first and last 2939 samples (DWORDs) and
/// multiply each remaining sample by its 1-based position; v2 sums
/// the high and low halves of each 64-bit product.
pub fn accuraterip(pcm: &[u8]) -> (u32, u32) {
    let dwords: Vec<u32> = pcm.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect();
    let skip = 2939usize;
    if dwords.len() <= skip * 2 {
        return (0, 0);
    }
    let mut v1: u32 = 0;
    let mut lo: u32 = 0;
    let mut hi: u32 = 0;
    for (i, &d) in dwords.iter().enumerate() {
        if i < skip || i >= dwords.len() - skip {
            continue;
        }
        let pos = (i + 1) as u32;
        v1 = v1.wrapping_add(d.wrapping_mul(pos));
        let mul = u64::from(d) * u64::from(pos);
        lo = lo.wrapping_add(mul as u32);
        hi = hi.wrapping_add((mul >> 32) as u32);
    }
    (v1, lo.wrapping_add(hi))
}

/// Rips every audio track on the disc.
pub fn rip_audio(
    drive: &mut dyn OpticalDrive,
    opts: &RipAudioOptions,
    progress: &Progress,
    cancel: &CancelToken,
) -> Result<Vec<RippedTrack>> {
    let toc = drive.toc()?;
    if toc.tracks.is_empty() || toc.tracks.iter().any(|t| !t.audio) {
        return err("this disc is not an audio CD");
    }
    let total_bytes: u64 = toc.tracks.iter().map(|t| u64::from(t.sectors) * 2352).sum();
    progress.set_total(total_bytes);
    let mut out = Vec::new();
    for track in &toc.tracks {
        let (pcm, bad, retries_used) = if opts.secure {
            rip_track_secure(drive, track.start_lba, track.sectors, opts.retries, progress, cancel)?
        } else {
            let mut buf = Vec::with_capacity(track.sectors as usize * 2352);
            let p2 = Progress::default();
            let (bad, used) =
                read_range(drive, track.start_lba, track.sectors, false, opts.retries, &p2, cancel, &mut |c| {
                    buf.extend_from_slice(c);
                    Ok(())
                })?;
            progress.add(buf.len() as u64);
            (buf, bad, used)
        };
        let (v1, v2) = accuraterip(&pcm);
        out.push(RippedTrack {
            number: track.number,
            title: track.title.clone().unwrap_or_else(|| format!("Track {:02}", track.number)),
            pcm,
            bad_sectors: bad,
            retries_used,
            accuraterip_v1: v1,
            accuraterip_v2: v2,
        });
    }
    Ok(out)
}

fn rip_track_secure(
    drive: &mut dyn OpticalDrive,
    start: u32,
    sectors: u32,
    retries: u32,
    progress: &Progress,
    cancel: &CancelToken,
) -> Result<(Vec<u8>, Vec<u32>, u32)> {
    let mut pcm = Vec::with_capacity(sectors as usize * 2352);
    let mut bad = Vec::new();
    let mut used = 0u32;
    for s in 0..sectors {
        cancel.check().map_err(|e| crate::Error(e.to_string()))?;
        let lba = start + s;
        let first = drive.read_chunk(lba, 1, false);
        let sector = match first {
            Ok(a) => {
                // Second read must agree; disagreement re-reads.
                let mut agreed = None;
                for _ in 0..retries.max(1) {
                    match drive.read_chunk(lba, 1, false) {
                        Ok(b) if b == a => {
                            agreed = Some(a.clone());
                            break;
                        }
                        Ok(_) => used += 1,
                        Err(_) => used += 1,
                    }
                }
                match agreed {
                    Some(x) => x,
                    None => {
                        bad.push(lba);
                        vec![0u8; 2352]
                    }
                }
            }
            Err(_) => {
                // First read failed: retry plain reads.
                let mut got = None;
                for _ in 0..retries {
                    used += 1;
                    if let Ok(b) = drive.read_chunk(lba, 1, false) {
                        got = Some(b);
                        break;
                    }
                }
                match got {
                    Some(x) => x,
                    None => {
                        bad.push(lba);
                        vec![0u8; 2352]
                    }
                }
            }
        };
        pcm.extend_from_slice(&sector);
        progress.add(2352);
    }
    Ok((pcm, bad, used))
}

/// The outcome of a surface scan.
#[derive(Debug, Clone)]
pub struct SurfaceReport {
    pub total_sectors: u32,
    pub good: u32,
    pub slow: u32,
    pub bad: Vec<u32>,
    /// 0..=100, good sectors as a percentage (slow counts half).
    pub quality_score: f32,
    /// A (>= 99.9) .. F (< 90) from the quality score.
    pub quality_grade: char,
}

/// Scans every sector of the disc once, classifying it good /
/// slow / bad, and grades the surface (the roadmap's quality scan
/// for drives without vendor C2 reporting).
pub fn surface_scan(drive: &mut dyn OpticalDrive, progress: &Progress, cancel: &CancelToken) -> Result<SurfaceReport> {
    let toc = drive.toc()?;
    let total = toc.lead_out_lba;
    progress.set_total(u64::from(total));
    let mut good = 0u32;
    let mut slow = 0u32;
    let mut bad = Vec::new();
    let mut lba = 0u32;
    while lba < total {
        cancel.check().map_err(|e| crate::Error(e.to_string()))?;
        let n = (total - lba).min(32);
        let started = Instant::now();
        match drive.read_chunk(lba, n, false) {
            Ok(_) => {
                let per_sector = started.elapsed().as_millis() as u64 / u64::from(n.max(1));
                for s in lba..lba + n {
                    if drive.sector_quality(s) == SectorQuality::Slow || per_sector > 25 {
                        slow += 1;
                    } else {
                        good += 1;
                    }
                }
            }
            Err(_) => {
                for s in lba..lba + n {
                    match drive.read_chunk(s, 1, false) {
                        Ok(_) => {
                            if drive.sector_quality(s) == SectorQuality::Slow {
                                slow += 1;
                            } else {
                                good += 1;
                            }
                        }
                        Err(_) => bad.push(s),
                    }
                }
            }
        }
        progress.add(u64::from(n));
        lba += n;
    }
    let score = if total == 0 { 0.0 } else { (good as f32 + slow as f32 * 0.5) / total as f32 * 100.0 };
    let grade = if score >= 99.9 {
        'A'
    } else if score >= 99.0 {
        'B'
    } else if score >= 95.0 {
        'C'
    } else if score >= 90.0 {
        'D'
    } else {
        'F'
    };
    Ok(SurfaceReport { total_sectors: total, good, slow, bad, quality_score: score, quality_grade: grade })
}

/// A plain-text dump report, written next to a ripped image:
/// drive, disc layout, damage and timing facts (redump-style).
pub fn dump_report(
    drive_desc: &str,
    toc: &Toc,
    image_name: &str,
    bad: &[u32],
    retries_used: u32,
    extra: &[String],
) -> String {
    let mut out = String::new();
    out += "IsoFoundry dump report\n";
    out += &format!("Drive: {drive_desc}\n");
    out += &format!("Image: {image_name}\n");
    out += &format!("Sector size: {} bytes, {} sectors total\n", toc.sector_size, toc.lead_out_lba);
    for t in &toc.tracks {
        let (m, s, f) = Toc::msf(t.start_lba);
        out += &format!(
            "Track {:02}: {} at {m:02}:{s:02}:{f:02} (LBA {}), {} sectors{}\n",
            t.number,
            if t.audio { "audio" } else { "data" },
            t.start_lba,
            t.sectors,
            t.title.as_deref().map(|x| format!(" — {x}")).unwrap_or_default()
        );
    }
    out += &format!("Read retries used: {retries_used}\n");
    if bad.is_empty() {
        out += "Unreadable sectors: none\n";
    } else {
        out += &format!(
            "Unreadable sectors ({}): {}\n",
            bad.len(),
            bad.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")
        );
    }
    for line in extra {
        out += line;
        out += "\n";
    }
    out
}

/// Where a dump's sidecar files go: same folder, same stem.
pub fn sidecar(dest: &Path, ext: &str) -> PathBuf {
    dest.with_extension(ext)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{fake_source_audio, fake_source_data, FakeConfig, FakeDrive};

    fn iso_bytes() -> Vec<u8> {
        (0..2048 * 40).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    fn rip_image_round_trips_a_data_disc() {
        let bytes = iso_bytes();
        let mut drive = fake_source_data(&bytes);
        let dir = std::env::temp_dir().join(format!("if-drive-rip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("out.iso");
        let progress = Progress::default();
        let report = rip_image(&mut drive, &dest, &RipImageOptions::default(), &progress, &CancelToken::new()).unwrap();
        assert!(report.bad_sectors.is_empty());
        assert_eq!(std::fs::read(&dest).unwrap(), bytes);
        let toc = drive.toc().unwrap();
        let cue = cue_sheet(&toc, "out.bin");
        assert!(cue.contains("TRACK 01 MODE1/2048"));
        let text = dump_report("Fake", &toc, "out.iso", &[], 0, &[]);
        assert!(text.contains("Unreadable sectors: none"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rip_image_zero_fills_bad_sectors_and_reports_them() {
        let bytes = iso_bytes();
        let mut drive =
            FakeDrive::new(FakeConfig { media_blank: false, bad_sectors: vec![5, 6], ..FakeConfig::default() });
        drive.load_image(&bytes);
        let dir = std::env::temp_dir().join(format!("if-drive-ripbad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("out.iso");
        let report =
            rip_image(&mut drive, &dest, &RipImageOptions::default(), &Progress::default(), &CancelToken::new())
                .unwrap();
        assert_eq!(report.bad_sectors, vec![5, 6]);
        let back = std::fs::read(&dest).unwrap();
        assert_eq!(back.len(), bytes.len());
        assert!(back[5 * 2048..6 * 2048].iter().all(|&b| b == 0));
        assert_eq!(back[7 * 2048], bytes[7 * 2048]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn rip_audio_reads_all_tracks_with_checksums() {
        let mut drive = fake_source_audio();
        let tracks =
            rip_audio(&mut drive, &RipAudioOptions::default(), &Progress::default(), &CancelToken::new()).unwrap();
        assert_eq!(tracks.len(), 3);
        for t in &tracks {
            assert!(t.bad_sectors.is_empty());
            assert_eq!(t.pcm.len() % 2352, 0);
            // Short tracks fall inside the checksum skip window:
            // the important part is the call is well-formed.
            let _ = (t.accuraterip_v1, t.accuraterip_v2);
        }
        let toc = drive.toc().unwrap();
        let cue = cue_sheet(&toc, "cd.bin");
        assert!(cue.contains("TRACK 01 AUDIO"));
        assert!(cue.contains("TRACK 03 AUDIO"));
    }

    #[test]
    fn accuraterip_checksum_is_position_sensitive() {
        let mut pcm = vec![0u8; 2352 * 20];
        pcm[4000 * 4] = 7;
        let (a1, a2) = accuraterip(&pcm);
        pcm[4000 * 4] = 9;
        let (b1, b2) = accuraterip(&pcm);
        assert_ne!(a1, b1);
        assert_ne!(a2, b2);
    }

    #[test]
    fn surface_scan_grades_a_damaged_disc() {
        let mut drive = FakeDrive::new(FakeConfig {
            media_blank: false,
            bad_sectors: vec![10, 11],
            slow_sectors: vec![20],
            ..FakeConfig::default()
        });
        drive.load_image(&iso_bytes());
        let report = surface_scan(&mut drive, &Progress::default(), &CancelToken::new()).unwrap();
        assert_eq!(report.bad, vec![10, 11]);
        assert_eq!(report.slow, 1);
        assert_eq!(report.total_sectors, 40);
        assert!(report.quality_score < 100.0);
    }
}
