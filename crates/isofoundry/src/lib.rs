#![cfg_attr(not(windows), allow(dead_code))] // the window code that uses layout/theme is Windows-only
//! Shared entry points for `isofoundry.exe` (window, GUI subsystem) and
//! `isofoundry.com` (console twin for scripts, D-048).

pub mod cli;
pub mod cli_image;
pub mod cli_mount;
pub mod cli_usb;
pub mod layout;
pub mod theme;
#[cfg(windows)]
pub mod win;

use std::process::ExitCode;

/// Runs a command line and returns its exit code. `attach` asks the GUI
/// build to borrow the parent console for output.
pub fn run_cli(args: &[String], attach: bool) -> ExitCode {
    #[cfg(windows)]
    if attach {
        win::console::attach();
    }
    #[cfg(not(windows))]
    let _ = attach;
    let dirs = if_core::paths::current();
    // The CLI logs to memory only so commands like `clean` never hold
    // the log file open; the drive report reads the file's tail instead.
    let log = if_core::log::Logger::memory();
    let code = cli::run(args, &platform(), &dirs, &log, &mut std::io::stdout().lock(), &mut std::io::stderr().lock());
    ExitCode::from(code.clamp(0, 255) as u8)
}

/// Opens the task window.
pub fn run_gui() -> ExitCode {
    let dirs = if_core::paths::current();
    let loaded = if_core::settings::load(&dirs.settings_file());
    let log = if_core::log::Logger::open(&dirs.log_dir(), loaded.settings.log_level);
    log.info(format!("IsoFoundry {} starting; data in {}", if_core::APP_VERSION, dirs.root.display()));
    if let Some(note) = &loaded.note {
        log.warn(note.clone());
    }

    #[cfg(windows)]
    {
        win::gui::run(dirs, loaded.settings, log)
    }
    #[cfg(not(windows))]
    {
        eprintln!("The IsoFoundry window needs Windows. Command line help:\n\n{}", cli::usage());
        ExitCode::from(if_core::error::Exit::Usage.code() as u8)
    }
}

#[cfg(windows)]
fn platform() -> win::WinPlatform {
    win::WinPlatform
}

#[cfg(not(windows))]
fn platform() -> NoPlatform {
    NoPlatform
}

#[cfg(not(windows))]
struct NoPlatform;

#[cfg(not(windows))]
impl cli::Platform for NoPlatform {
    fn os_description(&self) -> String {
        format!("{} (not Windows)", std::env::consts::OS)
    }
    fn user_profile(&self) -> Option<String> {
        std::env::var("HOME").ok()
    }
    fn helper_check(&self, _elevated: bool) -> Result<cli::HelperInfo, (if_core::error::Exit, String)> {
        Err((if_core::error::Exit::Generic, "the elevated helper only exists on Windows".into()))
    }
}
