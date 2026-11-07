#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![cfg_attr(not(windows), allow(dead_code))] // the window code that uses layout/theme is Windows-only
//! `isofoundry.exe`: with no arguments it opens the task window; with
//! arguments it runs the command line (D-016) and attaches to the parent
//! console for output.

mod cli;
mod layout;
mod theme;
#[cfg(windows)]
mod win;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dirs = if_core::paths::current();

    if !args.is_empty() {
        #[cfg(windows)]
        win::console::attach();
        // The CLI logs to memory only so commands like `clean` never hold
        // the log file open; the drive report reads the file's tail instead.
        let log = if_core::log::Logger::memory();
        let code =
            cli::run(&args, &platform(), &dirs, &log, &mut std::io::stdout().lock(), &mut std::io::stderr().lock());
        return ExitCode::from(code.clamp(0, 255) as u8);
    }

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
