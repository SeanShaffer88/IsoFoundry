//! Windows-only code: the Win32 window (D-043), console attach for the CLI
//! and the helper launcher (D-014).

pub mod console;
pub mod gui;
pub mod helper;
pub mod mount;
pub mod util;

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
}
