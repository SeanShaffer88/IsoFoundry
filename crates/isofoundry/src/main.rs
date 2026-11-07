#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! `isofoundry.exe`: with no arguments it opens the task window; with
//! arguments it runs the command line (D-016) and attaches to the parent
//! console for output. Scripts should call `isofoundry.com` (D-048), which
//! the shell finds first and which waits and redirects normally.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        isofoundry::run_gui()
    } else {
        isofoundry::run_cli(&args, true)
    }
}
