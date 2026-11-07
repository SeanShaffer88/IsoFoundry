//! The task catalog: every button in the main grid, its split-button
//! variants, the milestone that implements it and whether it needs the
//! elevated helper. The GUI and the CLI are both generated from this list.

/// Release milestone a task ships in (see docs/roadmap.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Milestone {
    M1,
    M2,
    M3,
    M4,
    M5,
    M6,
}

impl Milestone {
    pub fn label(self) -> &'static str {
        match self {
            Milestone::M1 => "M1",
            Milestone::M2 => "M2",
            Milestone::M3 => "M3",
            Milestone::M4 => "M4",
            Milestone::M5 => "M5",
            Milestone::M6 => "M6",
        }
    }

    /// Pre-release version the milestone ships as.
    pub fn release(self) -> &'static str {
        match self {
            Milestone::M1 => "0.2",
            Milestone::M2 => "0.3",
            Milestone::M3 => "0.4",
            Milestone::M4 => "0.5",
            Milestone::M5 => "0.6",
            Milestone::M6 => "0.7",
        }
    }
}

/// Implementation state, reported honestly in the UI and CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Planned(Milestone),
    Available,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Variant {
    /// Stable id; also the i18n key suffix (`variant-<id>`).
    pub id: &'static str,
    pub milestone: Milestone,
    pub admin: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Task {
    /// Stable id; also the i18n key suffix (`task-<id>`).
    pub id: &'static str,
    /// CLI subcommand name.
    pub cli: &'static str,
    pub milestone: Milestone,
    /// Needs the elevated helper (shown with a UAC shield).
    pub admin: bool,
    /// Split-button menu entries; empty for a plain button.
    pub variants: &'static [Variant],
}

/// Tasks whose core works in this build (GUI dialog and/or CLI).
pub const AVAILABLE: &[&str] = &["browse-image", "checksum", "create-image"];

impl Task {
    pub fn status(&self) -> Status {
        if AVAILABLE.contains(&self.id) {
            Status::Available
        } else {
            Status::Planned(self.milestone)
        }
    }

    pub fn is_available(&self) -> bool {
        self.status() == Status::Available
    }
}

const fn v(id: &'static str, milestone: Milestone, admin: bool) -> Variant {
    Variant { id, milestone, admin }
}

const fn t(
    id: &'static str,
    cli: &'static str,
    milestone: Milestone,
    admin: bool,
    variants: &'static [Variant],
) -> Task {
    Task { id, cli, milestone, admin, variants }
}

use Milestone::*;

/// Grid order, left column then right column, row by row (D-006).
/// `None` is an empty cell.
pub const GRID: [[Option<Task>; 2]; 11] = [
    [
        Some(t(
            "burn-image",
            "burn-image",
            M4,
            false,
            &[v("burn-image-multi", M4, false), v("burn-image-simulate", M4, false)],
        )),
        Some(t(
            "burn-files",
            "burn-files",
            M4,
            false,
            &[v("burn-files-multisession", M4, false), v("burn-files-boot", M4, false)],
        )),
    ],
    [
        Some(t("erase", "erase", M4, false, &[])),
        Some(t("copy-to-image", "copy-to-image", M5, false, &[v("copy-to-image-raw", M5, false)])),
    ],
    [
        Some(t(
            "copy-disc",
            "copy-disc",
            M5,
            false,
            &[v("copy-disc-multi", M5, false), v("copy-disc-on-the-fly", M5, false)],
        )),
        Some(t("rip-audio", "rip", M5, false, &[])),
    ],
    [Some(t("burn-audio", "burn-audio", M5, false, &[])), Some(t("burn-mp3", "burn-mp3", M4, false, &[]))],
    [
        Some(t("convert-audio", "convert-audio", M3, false, &[])),
        Some(t("convert-image", "convert-image", M1, false, &[])),
    ],
    [Some(t("create-image", "create-image", M1, false, &[])), Some(t("edit-image", "edit-image", M1, false, &[]))],
    [Some(t("browse-image", "extract", M1, false, &[])), Some(t("surface-test", "surface-test", M5, false, &[]))],
    [Some(t("drive-info", "info", M4, false, &[])), Some(t("install-windows-usb", "windows-to-usb", M2, true, &[]))],
    [
        Some(t("burn-video", "burn-video", M4, false, &[])),
        Some(t("backup", "backup", M6, false, &[v("backup-restore", M6, false)])),
    ],
    [Some(t("checksum", "checksum", M1, false, &[])), Some(t("recover-files", "recover", M5, false, &[]))],
    [
        Some(t("bootable-usb", "usb", M2, true, &[v("usb-image-to-usb", M2, true), v("usb-usb-to-image", M2, true)])),
        None,
    ],
];

/// All tasks in grid order.
pub fn all() -> impl Iterator<Item = Task> {
    GRID.iter().flat_map(|row| row.iter().flatten().copied())
}

pub fn by_id(id: &str) -> Option<Task> {
    all().find(|t| t.id == id)
}

pub fn by_cli(name: &str) -> Option<Task> {
    all().find(|t| t.cli == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_cli_names_are_unique() {
        let mut ids: Vec<&str> = all().map(|t| t.id).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n);
        let mut cli: Vec<&str> = all().map(|t| t.cli).collect();
        cli.sort_unstable();
        cli.dedup();
        assert_eq!(cli.len(), n);
        let mut vids: Vec<&str> = all().flat_map(|t| t.variants.iter().map(|v| v.id)).collect();
        let vn = vids.len();
        vids.sort_unstable();
        vids.dedup();
        assert_eq!(vids.len(), vn);
    }

    #[test]
    fn grid_has_all_anyburn_tasks_plus_new_rows() {
        // 17 AnyBurn tasks + 4 new ones (video, backup, checksum, recover).
        assert_eq!(all().count(), 21);
    }

    #[test]
    fn usb_tasks_need_admin() {
        assert!(by_id("bootable-usb").unwrap().admin);
        assert!(by_id("install-windows-usb").unwrap().admin);
        assert!(!by_id("burn-image").unwrap().admin);
    }

    #[test]
    fn variants_ship_no_earlier_than_their_task() {
        for t in all() {
            for v in t.variants {
                assert!(v.milestone >= t.milestone, "{} / {}", t.id, v.id);
            }
        }
    }

    #[test]
    fn lookup() {
        assert_eq!(by_cli("rip").unwrap().id, "rip-audio");
        assert!(by_cli("nope").is_none());
    }
}
