//! User settings: versioned TOML, validated on load, written atomically.
//! An unreadable file is quarantined (kept, renamed) and defaults are used.

use serde::{Deserialize, Serialize};
use std::path::Path;

pub const SCHEMA: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Error,
    Warn,
    #[default]
    Info,
    Debug,
}

/// What to do when a new disc appears in a drive (M6 auto-action).
/// Off unless the user turns it on; when on, the matching task window
/// opens — the job itself still starts with one click, because its
/// output location is a per-job choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AutoAction {
    #[default]
    Off,
    /// Data discs open Copy disc to image.
    Image,
    /// Audio CDs open Rip audio CD.
    Rip,
    /// Both of the above, routed by disc type.
    Both,
}

/// Last main-window placement in physical pixels. Validated against the
/// connected monitors before use (Directive V2.0 viewport rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowPlacement {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub maximized: bool,
}

impl WindowPlacement {
    /// Rejects absurd sizes from a hand-edited or corrupt file.
    pub fn is_plausible(&self) -> bool {
        (200..=32_000).contains(&self.width)
            && (200..=32_000).contains(&self.height)
            && (-32_000..=32_000).contains(&self.x)
            && (-32_000..=32_000).contains(&self.y)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub schema: u32,
    /// Extra UI zoom on top of Windows' own text size, 100–200 %.
    pub ui_scale_percent: u16,
    /// Follow Windows Settings → Accessibility → Text size.
    pub follow_windows_text_size: bool,
    /// Play the system sound when a task finishes.
    pub sound_on_finish: bool,
    /// Show a notification balloon when a task finishes (M6).
    pub toast_on_finish: bool,
    /// Shut the computer down after a task finishes successfully (M6).
    /// Off by default; the shutdown is announced and can be cancelled
    /// with `shutdown /a`.
    pub shutdown_when_done: bool,
    /// What to open when a new disc is inserted (M6).
    pub auto_action: AutoAction,
    /// Check GitHub for a newer IsoFoundry on start (M6). Off by
    /// default — IsoFoundry makes no network calls unless the user
    /// opts in here or runs `check-update` / a MusicBrainz lookup.
    pub check_updates: bool,
    /// Look audio CDs up on MusicBrainz when ripping (M6, opt-in
    /// online feature; rips work fully offline without it).
    pub musicbrainz_lookup: bool,
    pub log_level: LogLevel,
    /// UI language (BCP 47). Only "en-US" ships before 1.x.
    pub language: String,
    /// Main window placement; `None` = centered default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window: Option<WindowPlacement>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            schema: SCHEMA,
            ui_scale_percent: 100,
            follow_windows_text_size: true,
            sound_on_finish: true,
            toast_on_finish: false,
            shutdown_when_done: false,
            auto_action: AutoAction::Off,
            check_updates: false,
            musicbrainz_lookup: false,
            log_level: LogLevel::Info,
            language: "en-US".into(),
            window: None,
        }
    }
}

impl Settings {
    /// Clamps out-of-range values instead of rejecting the whole file.
    pub fn sanitized(mut self) -> Self {
        self.ui_scale_percent = self.ui_scale_percent.clamp(100, 200);
        if self.language != "en-US" {
            self.language = "en-US".into();
        }
        if self.window.is_some_and(|w| !w.is_plausible()) {
            self.window = None;
        }
        self.schema = SCHEMA;
        self
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).unwrap_or_default()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub settings: Settings,
    /// A message for the log when something was not as expected.
    pub note: Option<String>,
}

pub fn load(path: &Path) -> Loaded {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Loaded { settings: Settings::default(), note: None }
        }
        Err(e) => {
            return Loaded {
                settings: Settings::default(),
                note: Some(format!("settings unreadable, using defaults: {e}")),
            }
        }
    };
    match toml::from_str::<Settings>(&text) {
        Ok(s) if s.schema > SCHEMA => Loaded {
            settings: Settings::default(),
            note: Some(format!(
                "settings were written by a newer version (schema {}); using defaults and leaving the file untouched until you save",
                s.schema
            )),
        },
        Ok(s) => {
            let clean = s.clone().sanitized();
            let note = (clean != s && s.schema == SCHEMA).then(|| "some settings were out of range and were reset".into());
            Loaded { settings: clean, note }
        }
        Err(e) => {
            let note = match crate::fsutil::quarantine(path) {
                Ok(q) => format!("settings could not be parsed ({e}); kept as {}", q.display()),
                Err(qe) => format!("settings could not be parsed ({e}) and could not be moved aside ({qe})"),
            };
            Loaded { settings: Settings::default(), note: Some(note) }
        }
    }
}

pub fn save(path: &Path, s: &Settings) -> std::io::Result<()> {
    crate::fsutil::atomic_write(path, s.clone().sanitized().to_toml().as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsutil::temp_dir;

    #[test]
    fn missing_file_gives_defaults_silently() {
        let d = temp_dir("settings-missing");
        let l = load(&d.join("settings.toml"));
        assert_eq!(l.settings, Settings::default());
        assert!(l.note.is_none());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn m6_fields_roundtrip_and_default_off() {
        let d = temp_dir("settings-m6");
        let p = d.join("settings.toml");
        // Defaults: every M6 opt-in starts off.
        let def = Settings::default();
        assert_eq!(def.auto_action, AutoAction::Off);
        assert!(!def.check_updates && !def.musicbrainz_lookup);
        assert!(!def.toast_on_finish && !def.shutdown_when_done);
        let s = Settings {
            auto_action: AutoAction::Both,
            check_updates: true,
            musicbrainz_lookup: true,
            toast_on_finish: true,
            ..Default::default()
        };
        save(&p, &s).unwrap();
        assert_eq!(load(&p), Loaded { settings: s, note: None });
        // A pre-M6 settings file (no new keys) loads with defaults.
        std::fs::write(&p, "schema = 1\nsound_on_finish = false\n").unwrap();
        let l = load(&p);
        assert_eq!(l.settings.auto_action, AutoAction::Off);
        assert!(!l.settings.sound_on_finish);
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn roundtrip() {
        let d = temp_dir("settings-rt");
        let p = d.join("settings.toml");
        let s = Settings { ui_scale_percent: 150, sound_on_finish: false, ..Default::default() };
        save(&p, &s).unwrap();
        assert_eq!(load(&p), Loaded { settings: s, note: None });
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn out_of_range_is_clamped() {
        let d = temp_dir("settings-clamp");
        let p = d.join("settings.toml");
        std::fs::write(&p, "schema = 1\nui_scale_percent = 900\n").unwrap();
        let l = load(&p);
        assert_eq!(l.settings.ui_scale_percent, 200);
        assert!(l.note.is_some());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let d = temp_dir("settings-unknown");
        let p = d.join("settings.toml");
        std::fs::write(&p, "schema = 1\nfuture_option = true\n").unwrap();
        let l = load(&p);
        assert_eq!(l.settings, Settings::default());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn garbage_is_quarantined() {
        let d = temp_dir("settings-bad");
        let p = d.join("settings.toml");
        std::fs::write(&p, "this is = = not toml").unwrap();
        let l = load(&p);
        assert_eq!(l.settings, Settings::default());
        assert!(l.note.unwrap().contains("kept as"));
        assert!(!p.exists());
        let kept = std::fs::read_dir(&d).unwrap().count();
        assert_eq!(kept, 1);
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn window_placement_roundtrip_and_validation() {
        let d = temp_dir("settings-window");
        let p = d.join("settings.toml");
        let w = WindowPlacement { x: -1900, y: 40, width: 816, height: 700, maximized: false };
        save(&p, &Settings { window: Some(w), ..Default::default() }).unwrap();
        assert_eq!(load(&p).settings.window, Some(w));
        std::fs::write(&p, "schema = 1\n[window]\nx = 0\ny = 0\nwidth = 5\nheight = 99999\nmaximized = false\n")
            .unwrap();
        assert_eq!(load(&p).settings.window, None);
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn newer_schema_is_not_overwritten_on_load() {
        let d = temp_dir("settings-newer");
        let p = d.join("settings.toml");
        std::fs::write(&p, "schema = 99\n").unwrap();
        let l = load(&p);
        assert_eq!(l.settings, Settings::default());
        assert!(p.exists());
        std::fs::remove_dir_all(d).unwrap();
    }
}
