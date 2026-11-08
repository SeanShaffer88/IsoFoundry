//! The fake drive: a simulated recorder with honest semantics for
//! the job layer — capacity and blank checks before a burn, state
//! changes after one, content digests so verify-after-burn means
//! something, and a full operation log for tests and the UI.

use crate::{err, BurnOptions, BurnReport, BurnSource, DriveInfo, EraseKind, MediaInfo, OpticalDrive, Result};
use if_core::job::{CancelToken, Progress};
use std::path::Path;

/// How the fake drive is set up.
#[derive(Debug, Clone)]
pub struct FakeConfig {
    pub vendor: String,
    pub product: String,
    pub media_kind: String,
    pub capacity_bytes: u64,
    pub media_present: bool,
    pub media_blank: bool,
    /// Make verify-after-burn fail (for testing the failure path).
    pub corrupt_verify: bool,
    /// Sectors that fail to read on a loaded disc (M5 damage map).
    pub bad_sectors: Vec<u32>,
    /// Sectors that read correctly but slowly (surface scans).
    pub slow_sectors: Vec<u32>,
}

impl Default for FakeConfig {
    fn default() -> Self {
        FakeConfig {
            vendor: "FakeDrive".into(),
            product: "Simulated CD/DVD Writer".into(),
            media_kind: "CD-R".into(),
            capacity_bytes: 700 * 1024 * 1024,
            media_present: true,
            media_blank: true,
            corrupt_verify: false,
            bad_sectors: Vec::new(),
            slow_sectors: Vec::new(),
        }
    }
}

/// One completed fake burn, kept so verify can compare digests.
#[derive(Debug, Clone)]
struct Recorded {
    description: String,
    bytes: u64,
    digest: String,
}

pub struct FakeDrive {
    config: FakeConfig,
    present: bool,
    blank: bool,
    closed: bool,
    free_bytes: u64,
    recorded: Vec<Recorded>,
    pub log: Vec<String>,
    /// Loaded / burned disc content for the M5 read side: raw
    /// sectors, their main-payload size, and the track layout.
    disc_raw: Vec<u8>,
    disc_sector_size: usize,
    disc_tracks: Vec<crate::TrackInfo>,
    /// Titles recorded by an audio burn, parallel to disc_tracks.
    disc_closed_after_audio: bool,
}

impl FakeDrive {
    pub fn new(config: FakeConfig) -> FakeDrive {
        let free = if config.media_blank { config.capacity_bytes } else { config.capacity_bytes / 2 };
        FakeDrive {
            present: config.media_present,
            blank: config.media_blank,
            closed: false,
            free_bytes: free,
            config,
            recorded: Vec::new(),
            log: Vec::new(),
            disc_raw: Vec::new(),
            disc_sector_size: 2048,
            disc_tracks: Vec::new(),
            disc_closed_after_audio: false,
        }
    }

    /// Loads a data disc: the image bytes become the disc's sectors
    /// (2048-byte payloads, zero-padded to a whole sector).
    pub fn load_image(&mut self, bytes: &[u8]) {
        let sectors = bytes.len().div_ceil(2048);
        let mut raw = bytes.to_vec();
        raw.resize(sectors * 2048, 0);
        self.disc_raw = raw;
        self.disc_sector_size = 2048;
        self.disc_tracks = vec![crate::TrackInfo {
            number: 1,
            start_lba: 0,
            sectors: sectors as u32,
            audio: false,
            title: None,
            performer: None,
        }];
        self.present = true;
        self.blank = false;
        self.closed = true;
        self.free_bytes = 0;
        self.log.push(format!("load image disc ({sectors} sectors)"));
    }

    /// Loads an audio disc from (title, PCM bytes) tracks; pregaps
    /// between tracks are part of the track starts, CD-style.
    pub fn load_audio(&mut self, tracks: Vec<(String, Vec<u8>)>) {
        let mut raw = Vec::new();
        let mut layout = Vec::new();
        let mut lba = 0u32;
        for (i, (title, pcm)) in tracks.into_iter().enumerate() {
            let start = lba;
            if i > 0 {
                // 2 s pregap of silence belongs to the next track.
                raw.resize(raw.len() + 150 * 2352, 0);
                lba += 150;
            }
            let frames = (pcm.len() / 2352) as u32;
            layout.push(crate::TrackInfo {
                number: (i + 1) as u32,
                start_lba: start,
                sectors: frames + if i > 0 { 150 } else { 0 },
                audio: true,
                title: Some(title),
                performer: None,
            });
            raw.extend_from_slice(&pcm[..frames as usize * 2352]);
            lba += frames;
        }
        self.disc_raw = raw;
        self.disc_sector_size = 2352;
        self.disc_tracks = layout;
        self.present = true;
        self.blank = false;
        self.closed = true;
        self.free_bytes = 0;
        self.log.push(format!("load audio disc ({lba} sectors)"));
    }

    /// The recorded audio tracks (title, data sectors), for tests.
    pub fn audio_tracks(&self) -> Vec<(String, u32)> {
        self.disc_tracks.iter().filter(|t| t.audio).map(|t| (t.title.clone().unwrap_or_default(), t.sectors)).collect()
    }

    /// Test/inspection view of what has been "burned".
    pub fn recorded_burns(&self) -> usize {
        self.recorded.len()
    }

    /// Descriptions of the completed burns, oldest first.
    pub fn history(&self) -> Vec<String> {
        self.recorded.iter().map(|r| format!("{} ({} bytes, sha256 {})", r.description, r.bytes, r.digest)).collect()
    }

    fn require_media(&self) -> Result<()> {
        if !self.present {
            return err("no disc in the drive");
        }
        Ok(())
    }

    fn hasher() -> if_hash::Hasher {
        if_hash::Hasher::new(if_hash::Algorithms::parse("sha256").expect("static algorithm name"))
    }

    /// Streams a file through the SHA-256 hasher (bounded memory).
    fn hash_file(path: &Path) -> Result<(u64, String)> {
        use std::io::Read;
        let mut f = std::fs::File::open(path).map_err(|e| crate::Error(format!("{}: {e}", path.display())))?;
        let mut hasher = Self::hasher();
        let mut buf = [0u8; 1 << 16];
        let mut total = 0u64;
        loop {
            let n = f.read(&mut buf).map_err(|e| crate::Error(format!("read failed: {e}")))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            total += n as u64;
        }
        Ok((total, hasher.finish().sha256.unwrap_or_default()))
    }

    /// Walks a folder tree (sorted, so the digest is stable) and
    /// digests `path \0 len \0 bytes` for every file.
    fn hash_tree(root: &Path) -> Result<(u64, String)> {
        use std::io::Read;
        let mut files: Vec<(String, std::path::PathBuf)> = Vec::new();
        fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, std::path::PathBuf)>) {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            let mut entries: Vec<_> = rd.flatten().collect();
            entries.sort_by_key(|e| e.file_name());
            for e in entries {
                let rel = if prefix.is_empty() {
                    e.file_name().to_string_lossy().into_owned()
                } else {
                    format!("{prefix}/{}", e.file_name().to_string_lossy())
                };
                let p = e.path();
                if p.is_dir() {
                    walk(&p, &rel, out);
                } else {
                    out.push((rel, p));
                }
            }
        }
        walk(root, "", &mut files);
        if files.is_empty() {
            return err(format!("{}: the folder is empty — nothing to burn", root.display()));
        }
        let mut hasher = Self::hasher();
        let mut total = 0u64;
        let mut buf = [0u8; 1 << 16];
        for (rel, path) in &files {
            let mut f = std::fs::File::open(path).map_err(|e| crate::Error(format!("{}: {e}", path.display())))?;
            let len = f.metadata().map(|m| m.len()).unwrap_or(0);
            hasher.update(rel.as_bytes());
            hasher.update(&[0]);
            hasher.update(&len.to_le_bytes());
            hasher.update(&[0]);
            loop {
                let n = f.read(&mut buf).map_err(|e| crate::Error(format!("read failed: {e}")))?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
                total += n as u64;
            }
        }
        Ok((total, hasher.finish().sha256.unwrap_or_default()))
    }

    /// Advances progress in chunks, honouring cancel.
    fn run_progress(&self, total: u64, progress: &Progress, cancel: &CancelToken) -> Result<()> {
        progress.set_total(total.max(1));
        let chunks = 32u64;
        let per = total / chunks + 1;
        let mut done = 0u64;
        while done < total {
            if cancel.is_cancelled() {
                return err("cancelled");
            }
            let step = per.min(total - done);
            progress.add(step);
            done += step;
        }
        Ok(())
    }
}

impl OpticalDrive for FakeDrive {
    fn info(&self) -> Result<DriveInfo> {
        Ok(DriveInfo {
            id: "fake".into(),
            letter: Some('Z'),
            vendor: self.config.vendor.clone(),
            product: self.config.product.clone(),
            revision: "1.0".into(),
            backend: "Fake".into(),
        })
    }

    fn media(&self) -> Result<MediaInfo> {
        Ok(MediaInfo {
            present: self.present,
            kind: if self.present { self.config.media_kind.clone() } else { String::new() },
            blank: self.blank,
            appendable: !self.blank && !self.closed && self.free_bytes > 0,
            capacity_bytes: self.config.capacity_bytes,
            free_bytes: if self.present { self.free_bytes } else { 0 },
        })
    }

    fn burn(
        &mut self,
        source: &BurnSource,
        opts: &BurnOptions,
        progress: &Progress,
        cancel: &CancelToken,
    ) -> Result<BurnReport> {
        self.require_media()?;
        if !self.blank && !opts.append {
            return err("the disc is not blank — erase it first, or burn with append for another session");
        }
        if !self.blank && opts.append && self.closed {
            return err("the disc is closed — no more sessions can be added");
        }
        if !self.blank && opts.append && self.free_bytes == 0 {
            return err("the disc has no free space for another session");
        }
        // Measure and digest the source first (this is also what a
        // verify pass compares against).
        let (bytes, digest, description) = match source {
            BurnSource::Image(path) => {
                let (n, d) = Self::hash_file(path)?;
                (n, d, format!("image {}", path.display()))
            }
            BurnSource::Files { root, label } => {
                let (n, d) = Self::hash_tree(root)?;
                (n, d, format!("files from {} (label {label})", root.display()))
            }
        };
        if bytes > self.free_bytes {
            return err(format!("the source needs {} bytes but only {} are free on the disc", bytes, self.free_bytes));
        }
        let mut notes = Vec::new();
        self.run_progress(bytes, progress, cancel)?;
        if opts.simulate {
            self.log.push(format!("simulate burn: {description} ({bytes} bytes)"));
            notes.push("Simulated only — nothing was written.".to_string());
            return Ok(BurnReport { bytes_written: 0, verified: false, notes, backend: "Fake".into() });
        }
        self.log.push(format!("burn: {description} ({bytes} bytes)"));
        self.recorded.push(Recorded { description, bytes, digest: digest.clone() });
        if let BurnSource::Image(path) = source {
            if let Ok(bytes) = std::fs::read(path) {
                let sectors = bytes.len().div_ceil(2048);
                let mut raw = bytes;
                raw.resize(sectors * 2048, 0);
                self.disc_raw = raw;
                self.disc_sector_size = 2048;
                self.disc_tracks = vec![crate::TrackInfo {
                    number: 1,
                    start_lba: 0,
                    sectors: sectors as u32,
                    audio: false,
                    title: None,
                    performer: None,
                }];
            }
        }
        self.blank = false;
        self.free_bytes -= bytes;
        let mut verified = false;
        if opts.verify {
            // Recompute from the recording: the fake "reads back"
            // what it wrote, so a mismatch can only come from the
            // configured corruption (the failure-path test switch).
            let readback = if self.config.corrupt_verify { "corrupted".to_string() } else { digest };
            if readback == self.recorded.last().map(|r| r.digest.clone()).unwrap_or_default() {
                verified = true;
                self.log.push("verify: digest matches".to_string());
            } else {
                return err("verify after burn failed: the read-back digest does not match the source");
            }
        }
        if opts.close_disc {
            self.closed = true;
            self.log.push("disc closed".to_string());
        }
        if opts.eject_when_done {
            self.log.push("eject".to_string());
        }
        Ok(BurnReport { bytes_written: bytes, verified, notes, backend: "Fake".into() })
    }

    fn erase(&mut self, kind: EraseKind, progress: &Progress, cancel: &CancelToken) -> Result<()> {
        self.require_media()?;
        if !self.config.media_kind.contains("RW") && !self.config.media_kind.contains("RE") {
            return err(format!("{} media cannot be erased (only rewritable media can)", self.config.media_kind));
        }
        let total = match kind {
            EraseKind::Quick => 1_000,
            EraseKind::Full => self.config.capacity_bytes / 4096,
        };
        self.run_progress(total, progress, cancel)?;
        self.log.push(format!("erase ({kind:?})"));
        self.blank = true;
        self.closed = false;
        self.free_bytes = self.config.capacity_bytes;
        self.recorded.clear();
        self.disc_raw.clear();
        self.disc_tracks.clear();
        Ok(())
    }

    fn caps(&self) -> crate::Caps {
        crate::Caps { read: true, subchannel: true, audio_burn: true, dao_cd_text: true }
    }

    fn toc(&self) -> Result<crate::Toc> {
        self.require_media()?;
        if self.disc_tracks.is_empty() {
            return err("the disc is blank — there is no table of contents to read");
        }
        let lead_out = self.disc_tracks.last().map(|t| t.start_lba + t.sectors).unwrap_or(0);
        Ok(crate::Toc {
            tracks: self.disc_tracks.clone(),
            lead_out_lba: lead_out,
            sector_size: self.disc_sector_size as u32,
        })
    }

    fn read_chunk(&mut self, lba: u32, count: u32, subchannel: bool) -> Result<Vec<u8>> {
        self.require_media()?;
        if self.disc_tracks.is_empty() {
            return err("the disc is blank — nothing to read");
        }
        let size = self.disc_sector_size;
        let total = (self.disc_raw.len() / size) as u32;
        if lba + count > total {
            return err(format!("read past the end of the disc (sector {} of {total})", lba + count));
        }
        for s in lba..lba + count {
            if self.config.bad_sectors.contains(&s) {
                return err(format!("read error at sector {s} (simulated damage)"));
            }
        }
        let stride = if subchannel { size + 96 } else { size };
        let mut out = vec![0u8; count as usize * stride];
        for i in 0..count as usize {
            let src = (lba as usize + i) * size;
            out[i * stride..i * stride + size].copy_from_slice(&self.disc_raw[src..src + size]);
            if subchannel {
                // Deterministic pseudo-subchannel: the Q channel
                // carries the sector address in BCD-ish bytes.
                let a = lba as usize + i;
                let sub = &mut out[i * stride + size..i * stride + size + 96];
                sub[13] = (a / 4500) as u8;
                sub[14] = ((a / 75) % 60) as u8;
                sub[15] = (a % 75) as u8;
            }
        }
        Ok(out)
    }

    fn sector_quality(&self, lba: u32) -> crate::SectorQuality {
        if self.config.bad_sectors.contains(&lba) {
            crate::SectorQuality::Bad
        } else if self.config.slow_sectors.contains(&lba) {
            crate::SectorQuality::Slow
        } else {
            crate::SectorQuality::Good
        }
    }

    fn burn_audio(
        &mut self,
        tracks: &[crate::AudioTrackData],
        opts: &crate::AudioBurnOptions,
        progress: &Progress,
        cancel: &CancelToken,
    ) -> Result<BurnReport> {
        self.require_media()?;
        if tracks.is_empty() {
            return err("no audio tracks to burn");
        }
        for t in tracks {
            if t.pcm.len() % 2352 != 0 {
                return err(format!("track '{}' is not a whole number of CD frames", t.title));
            }
        }
        let total_sectors: u64 = tracks.iter().map(|t| u64::from(t.sectors() + t.gap_sectors)).sum();
        let total_bytes = total_sectors * 2352;
        if total_bytes > self.free_bytes {
            return err(format!(
                "the tracks need {} bytes but only {} are free on the disc",
                total_bytes, self.free_bytes
            ));
        }
        if !self.blank {
            return err("the disc is not blank — audio burning needs a blank disc");
        }
        self.run_progress(total_bytes, progress, cancel)?;
        if opts.simulate {
            self.log.push(format!("simulate audio burn: {} tracks", tracks.len()));
            return Ok(BurnReport {
                bytes_written: 0,
                verified: false,
                notes: vec!["Simulated only — nothing was written.".to_string()],
                backend: "Fake".into(),
            });
        }
        // Materialize the disc exactly as a DAO write lays it out:
        // each track's pregap is silence attributed to that track.
        let mut raw = Vec::with_capacity(total_bytes as usize);
        let mut layout = Vec::new();
        let mut lba = 0u32;
        for (i, t) in tracks.iter().enumerate() {
            let gap = if i == 0 { 0 } else { t.gap_sectors };
            raw.resize(raw.len() + gap as usize * 2352, 0);
            layout.push(crate::TrackInfo {
                number: (i + 1) as u32,
                start_lba: lba,
                sectors: gap + t.sectors(),
                audio: true,
                title: Some(t.title.clone()),
                performer: Some(t.performer.clone()),
            });
            raw.extend_from_slice(&t.pcm);
            lba += gap + t.sectors();
        }
        self.disc_raw = raw;
        self.disc_sector_size = 2352;
        self.disc_tracks = layout;
        self.blank = false;
        self.disc_closed_after_audio = opts.close_disc;
        self.closed = opts.close_disc;
        self.free_bytes -= total_bytes;
        self.log.push(format!("burn audio (DAO, CD-Text): {} tracks, {total_sectors} sectors", tracks.len()));
        if opts.eject_when_done {
            self.log.push("eject".to_string());
        }
        Ok(BurnReport {
            bytes_written: total_bytes,
            verified: false,
            notes: vec!["Audio CD written disc-at-once with CD-Text titles.".to_string()],
            backend: "Fake".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use if_core::job::{CancelToken, Progress};
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("if-drive-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn burn_image_updates_state_and_verifies() {
        let dir = temp_dir("img");
        let img = dir.join("test.iso");
        std::fs::write(&img, vec![7u8; 100_000]).unwrap();
        let mut d = FakeDrive::new(FakeConfig::default());
        let rep = d
            .burn(&BurnSource::Image(img), &BurnOptions::default(), &Progress::default(), &CancelToken::new())
            .unwrap();
        assert_eq!(rep.bytes_written, 100_000);
        assert!(rep.verified);
        assert_eq!(d.recorded_burns(), 1);
        let m = d.media().unwrap();
        assert!(!m.blank);
        assert!(m.appendable);
        // A second burn without append is refused.
        let img2 = dir.join("test2.iso");
        std::fs::write(&img2, vec![1u8; 10]).unwrap();
        assert!(d
            .burn(&BurnSource::Image(img2.clone()), &BurnOptions::default(), &Progress::default(), &CancelToken::new())
            .is_err());
        // With append it fits.
        let rep2 = d
            .burn(
                &BurnSource::Image(img2),
                &BurnOptions { append: true, ..Default::default() },
                &Progress::default(),
                &CancelToken::new(),
            )
            .unwrap();
        assert_eq!(rep2.bytes_written, 10);
    }

    #[test]
    fn oversized_image_is_refused() {
        let dir = temp_dir("big");
        let img = dir.join("big.iso");
        std::fs::write(&img, vec![0u8; 4096]).unwrap();
        let cfg = FakeConfig { capacity_bytes: 2048, ..Default::default() };
        let mut d = FakeDrive::new(cfg);
        assert!(d
            .burn(&BurnSource::Image(img), &BurnOptions::default(), &Progress::default(), &CancelToken::new())
            .is_err());
    }

    #[test]
    fn simulate_writes_nothing() {
        let dir = temp_dir("sim");
        let img = dir.join("s.iso");
        std::fs::write(&img, vec![3u8; 5000]).unwrap();
        let mut d = FakeDrive::new(FakeConfig::default());
        let rep = d
            .burn(
                &BurnSource::Image(img),
                &BurnOptions { simulate: true, ..Default::default() },
                &Progress::default(),
                &CancelToken::new(),
            )
            .unwrap();
        assert_eq!(rep.bytes_written, 0);
        assert!(d.media().unwrap().blank);
        assert_eq!(d.recorded_burns(), 0);
    }

    #[test]
    fn files_burn_and_erase_cycle() {
        let dir = temp_dir("files");
        std::fs::write(dir.join("a.txt"), b"hello").unwrap();
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("sub/b.bin"), vec![9u8; 2048]).unwrap();
        let cfg = FakeConfig { media_kind: "DVD+RW".into(), ..Default::default() };
        let mut d = FakeDrive::new(cfg);
        let rep = d
            .burn(
                &BurnSource::Files { root: dir.clone(), label: "TEST".into() },
                &BurnOptions::default(),
                &Progress::default(),
                &CancelToken::new(),
            )
            .unwrap();
        assert_eq!(rep.bytes_written, 5 + 2048);
        d.erase(EraseKind::Quick, &Progress::default(), &CancelToken::new()).unwrap();
        let m = d.media().unwrap();
        assert!(m.blank);
        assert_eq!(m.free_bytes, m.capacity_bytes);
    }

    #[test]
    fn cd_r_cannot_be_erased() {
        let mut d = FakeDrive::new(FakeConfig::default());
        assert!(d.erase(EraseKind::Quick, &Progress::default(), &CancelToken::new()).is_err());
    }

    #[test]
    fn corrupt_verify_fails_the_burn() {
        let dir = temp_dir("corrupt");
        let img = dir.join("c.iso");
        std::fs::write(&img, vec![5u8; 100]).unwrap();
        let cfg = FakeConfig { corrupt_verify: true, ..Default::default() };
        let mut d = FakeDrive::new(cfg);
        assert!(d
            .burn(&BurnSource::Image(img), &BurnOptions::default(), &Progress::default(), &CancelToken::new())
            .is_err());
    }
}
