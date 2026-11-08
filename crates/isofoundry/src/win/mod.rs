//! Windows-only code: the Win32 window (D-043), console attach for the CLI
//! and the helper launcher (D-014).

pub mod console;
pub mod finish;
pub mod gui;
pub mod helper;
pub mod http;
pub mod mount;
pub mod tasks;
pub mod taskwin;
pub mod util;
pub mod verbs;

use crate::cli::{HelperInfo, Platform};
use if_core::error::Exit;

pub struct WinPlatform;

impl Platform for WinPlatform {
    fn os_description(&self) -> String {
        util::os_description()
    }

    fn user_profile(&self) -> Option<String> {
        std::env::var("USERPROFILE").ok()
    }

    fn helper_check(&self, elevated: bool) -> Result<HelperInfo, (Exit, String)> {
        helper::check(elevated)
    }

    fn usb_list(&self) -> Result<Vec<usbwriter::device::DiskInfo>, (Exit, String)> {
        helper::usb_list()
    }

    fn usb_job(
        &self,
        job: &if_core::ipc::UsbJob,
        progress: &if_core::job::Progress,
    ) -> Result<(usize, u64, Vec<String>), (Exit, String)> {
        let last = std::cell::Cell::new(0u64);
        helper::usb_job(job, |done, total| {
            progress.set_total(total);
            progress.add(done.saturating_sub(last.get()));
            last.set(done);
        })
    }
}
