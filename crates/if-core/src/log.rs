//! Local logging: an in-memory ring for the log viewer plus a text file in
//! the data folder. Nothing leaves the machine.

use crate::settings::LogLevel;
use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

const RING: usize = 2000;
const ROTATE_BYTES: u64 = 5 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub unix: u64,
    pub level: LogLevel,
    pub message: String,
}

impl Entry {
    pub fn line(&self) -> String {
        let lvl = match self.level {
            LogLevel::Error => "ERROR",
            LogLevel::Warn => "WARN ",
            LogLevel::Info => "INFO ",
            LogLevel::Debug => "DEBUG",
        };
        format!("{} {lvl} {}", self.unix, self.message)
    }
}

fn rank(l: LogLevel) -> u8 {
    match l {
        LogLevel::Error => 0,
        LogLevel::Warn => 1,
        LogLevel::Info => 2,
        LogLevel::Debug => 3,
    }
}

pub struct Logger {
    ring: Mutex<VecDeque<Entry>>,
    file: Mutex<Option<std::fs::File>>,
    path: Option<PathBuf>,
    level: Mutex<LogLevel>,
}

impl Logger {
    /// Memory-only logger (tests, CLI `--no-log`).
    pub fn memory() -> Self {
        Self {
            ring: Mutex::new(VecDeque::new()),
            file: Mutex::new(None),
            path: None,
            level: Mutex::new(LogLevel::Info),
        }
    }

    /// Opens `dir/isofoundry.log`, rotating it to `.1` past 5 MB. Falls back
    /// to memory-only if the folder cannot be written.
    pub fn open(dir: &Path, level: LogLevel) -> Self {
        let path = dir.join("isofoundry.log");
        let file = std::fs::create_dir_all(dir).ok().and_then(|_| {
            if std::fs::metadata(&path).map(|m| m.len() > ROTATE_BYTES).unwrap_or(false) {
                let _ = std::fs::rename(&path, dir.join("isofoundry.log.1"));
            }
            std::fs::OpenOptions::new().create(true).append(true).open(&path).ok()
        });
        let opened = file.is_some();
        Self {
            ring: Mutex::new(VecDeque::new()),
            file: Mutex::new(file),
            path: opened.then_some(path),
            level: Mutex::new(level),
        }
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    pub fn set_level(&self, level: LogLevel) {
        if let Ok(mut l) = self.level.lock() {
            *l = level;
        }
    }

    pub fn log(&self, level: LogLevel, message: impl Into<String>) {
        let max = self.level.lock().map(|l| *l).unwrap_or(LogLevel::Info);
        if rank(level) > rank(max) {
            return;
        }
        let e = Entry { unix: crate::fsutil::unix_now(), level, message: message.into() };
        if let Ok(mut f) = self.file.lock() {
            if let Some(f) = f.as_mut() {
                let _ = writeln!(f, "{}", e.line());
            }
        }
        if let Ok(mut r) = self.ring.lock() {
            if r.len() == RING {
                r.pop_front();
            }
            r.push_back(e);
        }
    }

    pub fn info(&self, m: impl Into<String>) {
        self.log(LogLevel::Info, m)
    }

    pub fn warn(&self, m: impl Into<String>) {
        self.log(LogLevel::Warn, m)
    }

    pub fn error(&self, m: impl Into<String>) {
        self.log(LogLevel::Error, m)
    }

    pub fn entries(&self) -> Vec<Entry> {
        self.ring.lock().map(|r| r.iter().cloned().collect()).unwrap_or_default()
    }
}

/// Replaces the user's profile folder with `%USERPROFILE%` so exported logs
/// and reports don't carry the account name.
pub fn redact_paths(text: &str, profile: Option<&str>) -> String {
    match profile.filter(|p| p.len() > 3) {
        Some(p) => {
            let mut out = text.replace(p, "%USERPROFILE%");
            let alt = p.replace('\\', "/");
            if alt != p {
                out = out.replace(&alt, "%USERPROFILE%");
            }
            out
        }
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsutil::temp_dir;

    #[test]
    fn ring_is_bounded_and_filtered() {
        let l = Logger::memory();
        l.log(LogLevel::Debug, "hidden");
        for i in 0..(RING + 5) {
            l.info(format!("m{i}"));
        }
        let e = l.entries();
        assert_eq!(e.len(), RING);
        assert_eq!(e[0].message, "m5");
    }

    #[test]
    fn writes_file() {
        let d = temp_dir("log");
        let l = Logger::open(&d, LogLevel::Info);
        l.warn("hello");
        drop(l);
        let text = std::fs::read_to_string(d.join("isofoundry.log")).unwrap();
        assert!(text.contains("WARN  hello"));
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn redaction() {
        let s = r"opened C:\Users\sean\Downloads\a.iso and C:/Users/sean/x";
        assert_eq!(
            redact_paths(s, Some(r"C:\Users\sean")),
            r"opened %USERPROFILE%\Downloads\a.iso and %USERPROFILE%/x"
        );
        assert_eq!(redact_paths(s, None), s);
    }
}
