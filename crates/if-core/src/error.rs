//! Stable exit codes shared by the CLI and the GUI's job reports.
//! Values are part of the public CLI contract: never renumber, only add.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Exit {
    Ok = 0,
    Generic = 1,
    Usage = 2,
    Cancelled = 3,
    NoDriveOrMedia = 10,
    MediaIncompatible = 11,
    WriteFailed = 12,
    VerifyMismatch = 13,
    ElevationDenied = 20,
    Io = 30,
    /// The task exists in the catalog but its milestone has not shipped yet.
    NotImplemented = 40,
}

impl Exit {
    pub const ALL: [Exit; 11] = [
        Exit::Ok,
        Exit::Generic,
        Exit::Usage,
        Exit::Cancelled,
        Exit::NoDriveOrMedia,
        Exit::MediaIncompatible,
        Exit::WriteFailed,
        Exit::VerifyMismatch,
        Exit::ElevationDenied,
        Exit::Io,
        Exit::NotImplemented,
    ];

    pub fn code(self) -> i32 {
        self as u8 as i32
    }

    /// Short machine-readable name, used in `--json` output.
    pub fn name(self) -> &'static str {
        match self {
            Exit::Ok => "ok",
            Exit::Generic => "error",
            Exit::Usage => "usage",
            Exit::Cancelled => "cancelled",
            Exit::NoDriveOrMedia => "no_drive_or_media",
            Exit::MediaIncompatible => "media_incompatible",
            Exit::WriteFailed => "write_failed",
            Exit::VerifyMismatch => "verify_mismatch",
            Exit::ElevationDenied => "elevation_denied",
            Exit::Io => "io",
            Exit::NotImplemented => "not_implemented",
        }
    }
}

/// An error carrying the exit code it maps to and a human message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    pub exit: Exit,
    pub message: String,
}

impl Error {
    pub fn new(exit: Exit, message: impl Into<String>) -> Self {
        Self { exit, message: message.into() }
    }

    pub fn io(context: &str, e: std::io::Error) -> Self {
        Self::new(Exit::Io, format!("{context}: {e}"))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.message, self.exit.name())
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable() {
        let codes: Vec<i32> = Exit::ALL.iter().map(|e| e.code()).collect();
        assert_eq!(codes, vec![0, 1, 2, 3, 10, 11, 12, 13, 20, 30, 40]);
    }

    #[test]
    fn names_are_unique() {
        let mut names: Vec<&str> = Exit::ALL.iter().map(|e| e.name()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), Exit::ALL.len());
    }
}
