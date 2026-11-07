//! File helpers: atomic replace and quarantine of unreadable data.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Writes `bytes` to `path` atomically: a sibling temp file is written and
/// flushed to disk, then renamed over the target (`MoveFileExW` with
/// replace-existing on Windows). A crash leaves either the old or the new file.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let tmp = dir.join(format!(".{name}.tmp-{}", std::process::id()));
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

pub fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Moves an unreadable file aside (`<name>.bad-<unix time>`) so the user's
/// data is kept for inspection instead of being overwritten. Returns the new path.
pub fn quarantine(path: &Path) -> std::io::Result<PathBuf> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let mut target = path.with_file_name(format!("{name}.bad-{}", unix_now()));
    let mut n = 1;
    while target.exists() {
        target = path.with_file_name(format!("{name}.bad-{}-{n}", unix_now()));
        n += 1;
    }
    std::fs::rename(path, &target)?;
    Ok(target)
}

#[cfg(test)]
pub(crate) fn temp_dir(tag: &str) -> PathBuf {
    let mut buf = [0u8; 8];
    getrandom::fill(&mut buf).expect("os random");
    let hex: String = buf.iter().map(|b| format!("{b:02x}")).collect();
    let d = std::env::temp_dir().join(format!("isofoundry-test-{tag}-{hex}"));
    std::fs::create_dir_all(&d).expect("temp dir");
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_and_leaves_no_temp() {
        let d = temp_dir("atomic");
        let p = d.join("a.txt");
        atomic_write(&p, b"one").unwrap();
        atomic_write(&p, b"two").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"two");
        let leftovers: Vec<_> = std::fs::read_dir(&d).unwrap().filter_map(|e| e.ok()).collect();
        assert_eq!(leftovers.len(), 1);
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn quarantine_keeps_bytes() {
        let d = temp_dir("quarantine");
        let p = d.join("settings.toml");
        std::fs::write(&p, b"garbage").unwrap();
        let q = quarantine(&p).unwrap();
        assert!(!p.exists());
        assert_eq!(std::fs::read(&q).unwrap(), b"garbage");
        std::fs::remove_dir_all(d).unwrap();
    }
}
