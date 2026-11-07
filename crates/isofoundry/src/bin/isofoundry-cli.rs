//! Console twin of `isofoundry.exe` (D-048), shipped as `isofoundry.com`.
//! Windows tries `.com` before `.exe`, so `isofoundry list` typed in cmd or
//! PowerShell runs this console program: the shell waits for it, pipes and
//! redirection work, and the exit code is reported. With no arguments it
//! opens the window by starting `isofoundry.exe`.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if !args.is_empty() {
        return isofoundry::run_cli(&args, false);
    }
    let gui = std::env::current_exe().map(|p| p.with_file_name("isofoundry.exe"));
    match gui.and_then(|p| std::process::Command::new(p).spawn()) {
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("cannot open the IsoFoundry window: {e}");
            ExitCode::from(if_core::error::Exit::Generic.code() as u8)
        }
    }
}
