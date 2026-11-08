//! if-drive — the optical drive layer (M4).
//!
//! One trait, two backends:
//! - [`imapi`] on Windows: IMAPI2 (enumeration, drive/media info,
//!   burn image, burn files with optional multisession append,
//!   erase). Runtime behaviour is **Unverified** — the project's
//!   test gate for this milestone is the fake drive (roadmap M4).
//! - [`FakeDrive`]: a simulated drive with real semantics for the
//!   parts that matter to the job layer — capacity checks, blank /
//!   appendable state, content digests for verify-after-burn, and
//!   an operation log. Every burn/erase flow in the app and CLI can
//!   run end to end against it with no hardware.
//!
//! The MMC/SPTI backend (raw reads, DAO, quality scans) is M5 and
//! is not part of this crate yet (ARCHITECTURE §3, D-003).

pub mod fake;
#[cfg(windows)]
pub mod imapi;
pub mod rip;
#[cfg(windows)]
pub mod spti;

pub use fake::{FakeConfig, FakeDrive};

use if_core::job::{CancelToken, Progress};
use std::path::PathBuf;

#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error(msg.into()))
}

/// A recorder as the system reports it.
#[derive(Debug, Clone)]
pub struct DriveInfo {
    /// Backend-unique identifier (IMAPI recorder id, or "fake").
    pub id: String,
    /// Drive letter when the recorder owns one (e.g. 'E').
    pub letter: Option<char>,
    pub vendor: String,
    pub product: String,
    pub revision: String,
    /// Which engine is behind this drive ("IMAPI2" or "Fake").
    pub backend: String,
}

impl DriveInfo {
    pub fn display_name(&self) -> String {
        let drive = match self.letter {
            Some(l) => format!("{l}: "),
            None => String::new(),
        };
        format!("{drive}{} {} ({})", self.vendor.trim(), self.product.trim(), self.backend)
    }
}

/// The disc currently in a drive.
#[derive(Debug, Clone)]
pub struct MediaInfo {
    pub present: bool,
    /// "CD-R", "DVD+R", "BD-R", ... or a plain description.
    pub kind: String,
    pub blank: bool,
    /// Another session can be added.
    pub appendable: bool,
    pub capacity_bytes: u64,
    pub free_bytes: u64,
}

impl MediaInfo {
    pub fn absent() -> MediaInfo {
        MediaInfo {
            present: false,
            kind: String::new(),
            blank: false,
            appendable: false,
            capacity_bytes: 0,
            free_bytes: 0,
        }
    }
}

/// What to burn.
#[derive(Debug, Clone)]
pub enum BurnSource {
    /// A disc image file (ISO and friends), written as-is.
    Image(PathBuf),
    /// A folder whose contents become a data disc (its root is the
    /// disc root). `label` is the volume name.
    Files { root: PathBuf, label: String },
}

/// How to burn it.
#[derive(Debug, Clone)]
pub struct BurnOptions {
    /// Validate everything, write nothing.
    pub simulate: bool,
    /// Read the result back and compare (fake drive: digest check;
    /// the IMAPI path cannot read back yet and reports it).
    pub verify: bool,
    /// Close the disc so nothing more can be added.
    pub close_disc: bool,
    /// Append a new session to a non-blank, appendable disc.
    pub append: bool,
    /// Requested write speed in kbit/s (None = drive's choice).
    pub speed_kbps: Option<u32>,
    pub eject_when_done: bool,
}

impl Default for BurnOptions {
    fn default() -> Self {
        BurnOptions {
            simulate: false,
            verify: true,
            close_disc: false,
            append: false,
            speed_kbps: None,
            eject_when_done: false,
        }
    }
}

/// The outcome of a burn.
#[derive(Debug, Clone)]
pub struct BurnReport {
    pub bytes_written: u64,
    pub verified: bool,
    /// Plain-language notes (simulation, verify unavailable, ...).
    pub notes: Vec<String>,
    pub backend: String,
}

/// Erase depth for rewritable media.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EraseKind {
    Quick,
    Full,
}

/// An optical drive. Implemented by the IMAPI2 backend and the fake
/// drive; M5 adds the MMC backend behind the same trait.
pub trait OpticalDrive {
    fn info(&self) -> Result<DriveInfo>;
    fn media(&self) -> Result<MediaInfo>;
    fn burn(
        &mut self,
        source: &BurnSource,
        opts: &BurnOptions,
        progress: &Progress,
        cancel: &CancelToken,
    ) -> Result<BurnReport>;
    fn erase(&mut self, kind: EraseKind, progress: &Progress, cancel: &CancelToken) -> Result<()>;

    /// What this backend can do beyond burn/erase (M5).
    fn caps(&self) -> Caps {
        Caps::default()
    }

    /// The disc's table of contents, when the backend can read.
    fn toc(&self) -> Result<Toc> {
        err("this drive cannot read a table of contents")
    }

    /// Reads `count` consecutive sectors starting at `lba` in one
    /// backend call. With `subchannel`, each returned sector carries
    /// 96 extra bytes of raw subchannel data after its main payload.
    /// The engine in [`rip`] falls back to single sectors to isolate
    /// bad ones, so a chunk error should name no sector in
    /// particular.
    fn read_chunk(&mut self, _lba: u32, _count: u32, _subchannel: bool) -> Result<Vec<u8>> {
        err("this drive cannot read raw sectors")
    }

    /// How a sector behaved on this drive, when the backend knows
    /// without being asked twice (the fake drive's damage map; real
    /// backends return `Good` and let the engine time the reads).
    fn sector_quality(&self, _lba: u32) -> SectorQuality {
        SectorQuality::Good
    }

    /// Burns an audio CD from decoded PCM tracks (M5). The fake
    /// drive writes DAO with CD-Text and per-track pregaps; the
    /// IMAPI2 path uses track-at-once (2 s gaps, no CD-Text — see
    /// docs/LIMITATIONS.md).
    fn burn_audio(
        &mut self,
        _tracks: &[AudioTrackData],
        _opts: &AudioBurnOptions,
        _progress: &Progress,
        _cancel: &CancelToken,
    ) -> Result<BurnReport> {
        err("audio burning is not supported by this drive")
    }
}

/// M5 capabilities of a drive backend.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Caps {
    /// Raw sector reads (MMC/SPTI or simulated).
    pub read: bool,
    /// Raw subchannel data alongside sector reads.
    pub subchannel: bool,
    /// Audio CD burning.
    pub audio_burn: bool,
    /// Disc-at-once audio with CD-Text and exact pregaps.
    pub dao_cd_text: bool,
}

/// One track from a disc's table of contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackInfo {
    pub number: u32,
    pub start_lba: u32,
    pub sectors: u32,
    pub audio: bool,
    /// CD-Text / recorded title when known (fake burns; else None).
    pub title: Option<String>,
    pub performer: Option<String>,
}

/// A disc's table of contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toc {
    pub tracks: Vec<TrackInfo>,
    /// First sector past the last track.
    pub lead_out_lba: u32,
    /// Main-payload bytes per sector on this disc: 2352 for audio /
    /// raw data, 2048 for cooked data.
    pub sector_size: u32,
}

impl Toc {
    pub fn total_sectors(&self) -> u32 {
        self.lead_out_lba
    }
    pub fn is_audio(&self) -> bool {
        !self.tracks.is_empty() && self.tracks.iter().all(|t| t.audio)
    }
    /// MSF (minutes, seconds, frames) for a sector address, the CD
    /// frame being one sector at 75 per second.
    pub fn msf(lba: u32) -> (u32, u32, u32) {
        (lba / 4500, (lba / 75) % 60, lba % 75)
    }
}

/// How one sector read behaved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectorQuality {
    Good,
    Slow,
    Bad,
}

/// One decoded audio track ready to burn: 44.1 kHz 16-bit stereo
/// PCM, a whole number of 2352-byte CD frames.
#[derive(Debug, Clone)]
pub struct AudioTrackData {
    pub title: String,
    pub performer: String,
    pub pcm: Vec<u8>,
    /// Silence before this track, in sectors (150 = 2 s standard).
    pub gap_sectors: u32,
}

impl AudioTrackData {
    pub fn sectors(&self) -> u32 {
        (self.pcm.len() / 2352) as u32
    }
}

/// How to burn an audio CD.
#[derive(Debug, Clone)]
pub struct AudioBurnOptions {
    pub simulate: bool,
    /// Finalize the disc (DAO backends and IMAPI alike).
    pub close_disc: bool,
    pub speed_kbps: Option<u32>,
    pub eject_when_done: bool,
}

impl Default for AudioBurnOptions {
    fn default() -> Self {
        AudioBurnOptions { simulate: false, close_disc: true, speed_kbps: None, eject_when_done: false }
    }
}

/// Lists the drives the real backend can see, as plain data. On
/// Windows this is IMAPI2; elsewhere (and on error) the list is
/// empty — use [`fake_drive`] for hardware-free work.
pub fn enumerate() -> Vec<DriveInfo> {
    #[cfg(windows)]
    {
        imapi::enumerate().unwrap_or_default()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Opens a working session on the drive described by `info`. The
/// session's COM objects (on the IMAPI path) belong to the calling
/// thread's apartment: open it on the thread that will burn.
pub fn open(info: &DriveInfo) -> Result<Box<dyn OpticalDrive>> {
    if info.id == "fake" {
        return Ok(Box::new(fake_drive()));
    }
    #[cfg(windows)]
    {
        Ok(Box::new(imapi::ImapiSession::open(info.clone())?))
    }
    #[cfg(not(windows))]
    {
        err(format!("drive {} is not available on this platform", info.id))
    }
}

/// The simulated drive used by `--fake` in the CLI and the task
/// windows' test route: a blank 700 MB CD-R.
pub fn fake_drive() -> FakeDrive {
    FakeDrive::new(FakeConfig::default())
}

/// A simulated drive loaded with a data disc whose content is the
/// given image bytes (2048-byte sectors) — the read-side `--fake`.
pub fn fake_source_data(image: &[u8]) -> FakeDrive {
    let mut d = FakeDrive::new(FakeConfig { media_blank: false, ..FakeConfig::default() });
    d.load_image(image);
    d
}

/// A simulated drive holding a small generated audio CD: three
/// short sine-tone tracks, so ripping can be tried anywhere.
pub fn fake_source_audio() -> FakeDrive {
    let mut d = FakeDrive::new(FakeConfig { media_blank: false, ..FakeConfig::default() });
    let mut tracks = Vec::new();
    for (i, freq) in [440.0f32, 554.37, 659.25].iter().enumerate() {
        let frames = 75 * 3; // 3 seconds per track
        let mut pcm = Vec::with_capacity(frames * 2352);
        for f in 0..frames * 588 {
            let t = f as f32 / 44100.0;
            let v = (t * freq * std::f32::consts::TAU).sin() * 0.5;
            let s = (v * i16::MAX as f32) as i16;
            pcm.extend_from_slice(&s.to_le_bytes());
            pcm.extend_from_slice(&s.to_le_bytes());
        }
        tracks.push((format!("Tone {} ({} Hz)", i + 1, *freq as u32), pcm));
    }
    d.load_audio(tracks);
    d
}
