//! Shared target picking for the two USB windows: the physical-disk
//! list (fetched through the elevated helper), a disk-image-file
//! alternative (the VHDX test route, D-029), the typed-confirmation
//! rule (ARCHITECTURE.md §7) and the job runner both windows share.

use super::super::helper;
use super::super::taskwin::*;
use if_core::ipc::{UsbJob, UsbJobKind};
use std::path::Path;
use std::sync::Arc;
use usbwriter::device::{describe, needs_typed_confirmation, DiskInfo};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::{SendMessageW, LB_SETCURSEL};

/// Disks + the optional file target shown in one list box.
#[derive(Default)]
pub struct UsbPane {
    pub disks: Vec<DiskInfo>,
    pub file: Option<(String, DiskInfo)>,
}

pub enum Choice {
    Disk(DiskInfo),
    File(String, DiskInfo),
}

impl UsbPane {
    /// (Re)fills the list box from the current state.
    pub fn fill(&self, list: HWND) {
        list_clear(list);
        for d in &self.disks {
            list_add(list, &describe(d));
        }
        if let Some((path, info)) = &self.file {
            list_add(list, &format!("File: {} ({:.1} GB)", path, info.size_bytes as f64 / 1_000_000_000.0));
        }
    }

    /// The current selection, if any.
    pub fn selection(&self, list: HWND) -> Option<Choice> {
        let sel = list_selected(list).into_iter().next()?;
        if sel < self.disks.len() {
            return Some(Choice::Disk(self.disks[sel].clone()));
        }
        self.file.as_ref().map(|(p, i)| Choice::File(p.clone(), i.clone()))
    }
}

/// Selects a row (used to pre-select the file target after picking it).
///
/// # Safety
/// `list` must be one of our own list boxes on the window thread.
pub unsafe fn select_row(list: HWND, index: usize) {
    // SAFETY: our own list box.
    unsafe { SendMessageW(list, LB_SETCURSEL, index, 0) };
}

/// Fetches the disk list through an elevated helper session (this is
/// the UAC prompt the window warns about).
pub fn refresh_disks() -> Result<Vec<DiskInfo>, String> {
    helper::usb_list().map_err(|(_, m)| m)
}

/// Synthetic DiskInfo for a disk-image file.
pub fn file_info(path: &str) -> Result<DiskInfo, String> {
    let size = std::fs::metadata(path).map(|m| m.len()).map_err(|e| format!("{path}: {e}"))?;
    Ok(DiskInfo {
        number: 0,
        model: format!("disk image file {path}"),
        size_bytes: size,
        bus: usbwriter::device::Bus::Virtual,
        removable: false,
        is_system: false,
        holds_boot: false,
        holds_pagefile: false,
        volume_labels: vec![],
        is_file_target: true,
    })
}

/// True when the chosen target needs the typed ERASE confirmation:
/// physical disks with mounted volumes or over 64 GB.
pub fn confirm_required(choice: &Choice) -> bool {
    match choice {
        Choice::Disk(info) => needs_typed_confirmation(info),
        Choice::File(_, _) => false,
    }
}

/// Runs the job for the chosen target. Physical disks go through the
/// helper; file targets run in-process (the same code as the CLI).
/// Cancelling a helper job only takes effect when the helper session
/// ends — the token is honoured by the in-process route.
pub fn run_choice(choice: &Choice, kind: UsbJobKind, sh: &Arc<JobShared>) -> Result<(usize, u64, Vec<String>), String> {
    match choice {
        Choice::Disk(info) => {
            let job = UsbJob { disk_number: info.number, kind };
            let last = std::cell::Cell::new(0u64);
            let progress = sh.progress();
            helper::usb_job(&job, |done, total| {
                progress.set_total(total);
                progress.add(done.saturating_sub(last.get()));
                last.set(done);
            })
            .map_err(|(_, m)| m)
        }
        Choice::File(path, _) => crate::cli_usb::run_on_file_with(
            Path::new(path),
            &kind,
            &mut crate::cli::Out { out: &mut std::io::sink(), err: &mut std::io::sink(), json: false },
            &sh.progress(),
            &sh.cancel,
        )
        .map_err(|(_, m)| m),
    }
}
