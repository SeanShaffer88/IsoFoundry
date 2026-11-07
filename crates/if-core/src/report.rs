//! Drive report (D-031): a redacted JSON file users attach to a "Drive
//! report" GitHub issue. Serial numbers and personal paths never leave the
//! machine. Drive probing arrives with the drive layer (M4); until then the
//! report carries the app and OS details plus the recent log.

use serde::{Deserialize, Serialize};

pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriveEntry {
    pub letter: String,
    pub vendor: String,
    pub model: String,
    pub firmware: String,
    /// Always removed by [`DriveReport::redacted`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub serial: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriveReport {
    pub schema: u32,
    pub app_version: String,
    pub os: String,
    pub arch: String,
    pub generated_unix: u64,
    pub drives: Vec<DriveEntry>,
    pub drive_probe: String,
    pub log_tail: Vec<String>,
}

impl DriveReport {
    pub fn new(os: String, drives: Vec<DriveEntry>, drive_probe: &str, log_tail: Vec<String>) -> Self {
        Self {
            schema: SCHEMA,
            app_version: crate::APP_VERSION.into(),
            os,
            arch: std::env::consts::ARCH.into(),
            generated_unix: crate::fsutil::unix_now(),
            drives,
            drive_probe: drive_probe.into(),
            log_tail,
        }
    }

    /// Drops serials and replaces the user's profile path in every string.
    pub fn redacted(mut self, profile: Option<&str>) -> Self {
        for d in &mut self.drives {
            d.serial = None;
        }
        self.log_tail = self.log_tail.iter().map(|l| crate::log::redact_paths(l, profile)).collect();
        self.os = crate::log::redact_paths(&self.os, profile);
        self
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_default()
    }

    /// Suggested file name, e.g. `IsoFoundry-drive-report-1793400000.json`.
    pub fn file_name(&self) -> String {
        format!("IsoFoundry-drive-report-{}.json", self.generated_unix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_removes_serials_and_profile() {
        let r = DriveReport::new(
            "Windows 11".into(),
            vec![DriveEntry {
                letter: "E:".into(),
                vendor: "V".into(),
                model: "M".into(),
                firmware: "1.0".into(),
                serial: Some("SECRET123".into()),
            }],
            "not available before M4",
            vec![r"opened C:\Users\sean\a.iso".into()],
        )
        .redacted(Some(r"C:\Users\sean"));
        let json = r.to_json();
        assert!(!json.contains("SECRET123"));
        assert!(!json.contains("sean"));
        assert!(json.contains("%USERPROFILE%"));
        let back: DriveReport = serde_json::from_str(&json).unwrap();
        assert_eq!(back.schema, SCHEMA);
    }
}
