#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! `IsoFoundryPortable.exe`: the PortableApps.com Format launcher (D-044).
//!
//! Layout it expects, relative to itself:
//!   App\IsoFoundry\isofoundry.exe   the program
//!   Data\                           created here on first run
//!
//! `isofoundry.exe` recognises this layout and keeps its settings and logs in
//! `Data\settings` (if_core::paths), so the launcher only has to make sure
//! `Data\` exists and start the program with the same arguments.

use std::process::ExitCode;

fn main() -> ExitCode {
    let Some(base) = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf())) else {
        return ExitCode::from(1);
    };
    let app = base.join("App").join("IsoFoundry").join("isofoundry.exe");
    if !app.is_file() {
        eprintln!("{} is missing; reinstall IsoFoundry Portable", app.display());
        return ExitCode::from(1);
    }
    if let Err(e) = std::fs::create_dir_all(base.join("Data")) {
        eprintln!("cannot create the Data folder: {e}");
        return ExitCode::from(1);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    match std::process::Command::new(&app).args(&args).current_dir(base.join("App").join("IsoFoundry")).spawn() {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("cannot start {}: {e}", app.display());
            ExitCode::from(1)
        }
    }
}
