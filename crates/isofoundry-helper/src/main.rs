#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
//! `isofoundry-helper.exe`: started by `isofoundry.exe` only for tasks that
//! need administrator rights (D-014). It connects back to the app's named
//! pipe, proves itself with the session nonce, answers typed requests and
//! exits when the app says so or the pipe closes.
//!
//! Usage (internal):
//!   isofoundry-helper --pipe <name> --nonce <hex> --app-pid <pid>
//!   isofoundry-helper --stdio --nonce <hex>          (tests only)

mod serve;
#[cfg(windows)]
mod usbwin;

use std::process::ExitCode;

struct Args {
    pipe: Option<String>,
    nonce: String,
    app_pid: Option<u32>,
    stdio: bool,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut pipe = None;
    let mut nonce = None;
    let mut app_pid = None;
    let mut stdio = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--pipe" => pipe = it.next().cloned(),
            "--nonce" => nonce = it.next().cloned(),
            "--app-pid" => app_pid = it.next().and_then(|p| p.parse().ok()),
            "--stdio" => stdio = true,
            "--version" => return Err(format!("isofoundry-helper {}", if_core::APP_VERSION)),
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    let nonce = nonce.filter(|n| n.len() == if_core::ipc::NONCE_BYTES * 2 && n.bytes().all(|b| b.is_ascii_hexdigit()));
    let nonce = nonce.ok_or("missing or malformed --nonce")?;
    if !stdio && (pipe.is_none() || app_pid.is_none()) {
        return Err("--pipe and --app-pid are required".into());
    }
    Ok(Args { pipe, nonce, app_pid, stdio })
}

fn main() -> ExitCode {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let args = match parse(&raw) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(if_core::error::Exit::Usage.code() as u8);
        }
    };
    let elevated = serve::is_elevated();
    let result = if args.stdio {
        let stdin = std::io::stdin();
        let stdout = std::io::stdout();
        serve::serve(&mut stdin.lock(), &mut stdout.lock(), &args.nonce, elevated)
    } else {
        serve::connect_pipe(args.pipe.as_deref().unwrap_or_default(), args.app_pid.unwrap_or(0)).and_then(|mut f| {
            let mut r = f.try_clone().map_err(|e| e.to_string())?;
            serve::serve(&mut r, &mut f, &args.nonce, elevated)
        })
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("helper: {e}");
            ExitCode::from(if_core::error::Exit::Generic.code() as u8)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn requires_wellformed_nonce() {
        assert!(parse(&s(&["--stdio"])).is_err());
        assert!(parse(&s(&["--stdio", "--nonce", "zz"])).is_err());
        let n = "a".repeat(64);
        assert!(parse(&s(&["--stdio", "--nonce", &n])).is_ok());
    }

    #[test]
    fn pipe_mode_requires_pid() {
        let n = "a".repeat(64);
        assert!(parse(&s(&["--pipe", "x", "--nonce", &n])).is_err());
        let a = parse(&s(&["--pipe", "x", "--nonce", &n, "--app-pid", "42"])).unwrap();
        assert_eq!(a.app_pid, Some(42));
    }

    #[test]
    fn rejects_unknown_args() {
        assert!(parse(&s(&["--run", "calc.exe"])).is_err());
    }
}
