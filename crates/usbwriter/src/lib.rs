//! usbwriter — bootable-USB engine for IsoFoundry (later shared with
//! WinISOPilot 2.0 through foundry-kit; D-010/D-038).
//!
//! Layers, bottom up:
//! - [`device`]: the block-device trait, a file-backed device (VHDX and
//!   plain image files — the hardware-free test target, D-029) and the
//!   target-safety assessment every write must pass first.
//! - [`part`]: MBR and GPT partition-table bytes (pure, unit-tested).
//! - [`fat32`]: a FAT32 formatter and file writer over any block device.
//! - [`raw`]: image <-> device streaming copies with optional verify.
//! - [`install`]: the two file-copy jobs — a generic bootable stick from an
//!   ISO's file tree, and a Windows installer stick using the layouts from
//!   DECISIONS/roadmap (MBR+FAT32 default, GPT+FAT32, GPT+NTFS), copying
//!   Microsoft's own signed boot files so Secure Boot stays intact.
//!
//! The crate never touches a physical disk itself: physical devices live
//! behind [`device::BlockDevice`], implemented for real disks by the
//! elevated helper. Everything here is testable against files.

pub mod device;
pub mod fat32;
pub mod install;
pub mod part;
pub mod raw;

use std::fmt;

/// Error type for the crate: a plain message, in the house style (the CLI
/// and GUI print it as-is).
#[derive(Debug)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error(e.to_string())
    }
}

impl From<if_image::Error> for Error {
    fn from(e: if_image::Error) -> Self {
        Error(e.to_string())
    }
}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Error(s)
    }
}

impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Error(s.to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error(msg.into()))
}
