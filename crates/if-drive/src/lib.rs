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
