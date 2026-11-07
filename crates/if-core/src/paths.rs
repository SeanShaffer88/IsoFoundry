//! Where IsoFoundry keeps its data. Portable mode (a `IsoFoundry.portable`
//! marker next to the exe, or the PortableApps.com `Data` layout) keeps
//! everything beside the program in `data\` (Directive V2.0 §8); otherwise
//! the per-user local app data folder is used (installed builds).

use std::path::{Path, PathBuf};

pub const PORTABLE_MARKER: &str = "IsoFoundry.portable";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataDirs {
    pub root: PathBuf,
    pub portable: bool,
}

impl DataDirs {
    pub fn settings_file(&self) -> PathBuf {
        self.root.join("settings.toml")
    }

    pub fn log_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    pub fn journal_dir(&self) -> PathBuf {
        self.root.join("journal")
    }

    pub fn reports_dir(&self) -> PathBuf {
        self.root.join("reports")
    }
}

/// Resolves the data folder. `env` looks up environment variables (injected
/// for tests). `exe_dir` is the folder that holds the executable.
pub fn resolve(exe_dir: &Path, env: impl Fn(&str) -> Option<String>) -> DataDirs {
    if exe_dir.join(PORTABLE_MARKER).is_file() {
        return DataDirs { root: exe_dir.join("data"), portable: true };
    }
    // PortableApps.com launcher layout: App\IsoFoundry\isofoundry.exe with Data\ two levels up.
    if let Some(app) = exe_dir.parent() {
        if app.file_name().is_some_and(|n| n.eq_ignore_ascii_case("App")) {
            if let Some(base) = app.parent() {
                if base.join("Data").is_dir() {
                    return DataDirs { root: base.join("Data").join("settings"), portable: true };
                }
            }
        }
    }
    let root = if let Some(local) = env("LOCALAPPDATA").filter(|s| !s.is_empty()) {
        PathBuf::from(local).join(crate::APP_NAME)
    } else if let Some(xdg) = env("XDG_DATA_HOME").filter(|s| !s.is_empty()) {
        PathBuf::from(xdg).join(crate::APP_NAME)
    } else if let Some(home) = env("HOME").filter(|s| !s.is_empty()) {
        PathBuf::from(home).join(".local").join("share").join(crate::APP_NAME)
    } else {
        exe_dir.join("data")
    };
    DataDirs { root, portable: false }
}

/// Resolves using the running executable and the real environment.
pub fn current() -> DataDirs {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    resolve(&exe_dir, |k| std::env::var(k).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsutil::temp_dir;

    #[test]
    fn marker_selects_portable() {
        let d = temp_dir("paths-marker");
        std::fs::write(d.join(PORTABLE_MARKER), b"").unwrap();
        let r = resolve(&d, |_| Some("C:\\Users\\x\\AppData\\Local".into()));
        assert!(r.portable);
        assert_eq!(r.root, d.join("data"));
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn portableapps_layout() {
        let d = temp_dir("paths-paf");
        let exe_dir = d.join("App").join("IsoFoundry");
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::create_dir_all(d.join("Data")).unwrap();
        let r = resolve(&exe_dir, |_| None);
        assert!(r.portable);
        assert_eq!(r.root, d.join("Data").join("settings"));
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn localappdata_when_installed() {
        let d = temp_dir("paths-installed");
        let r = resolve(&d, |k| (k == "LOCALAPPDATA").then(|| "/la".into()));
        assert!(!r.portable);
        assert_eq!(r.root, PathBuf::from("/la").join("IsoFoundry"));
        assert_eq!(r.settings_file(), PathBuf::from("/la").join("IsoFoundry").join("settings.toml"));
        std::fs::remove_dir_all(d).unwrap();
    }
}
