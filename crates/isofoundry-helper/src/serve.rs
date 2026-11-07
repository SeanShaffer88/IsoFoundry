//! Request loop and the Windows pipe client.

use if_core::ipc::{read_frame, write_frame, FrameError, Hello, Request, Response, PROTOCOL};
use std::io::{Read, Write};

/// Sends the hello, then answers requests until `Shutdown` or the pipe closes.
pub fn serve<R: Read, W: Write>(r: &mut R, w: &mut W, nonce: &str, elevated: bool) -> Result<(), String> {
    let hello =
        Hello { protocol: PROTOCOL, nonce: nonce.to_string(), helper_version: if_core::APP_VERSION.into(), elevated };
    write_frame(w, &hello).map_err(|e| e.to_string())?;
    loop {
        let req: Request = match read_frame(r) {
            Ok(q) => q,
            Err(FrameError::Closed) => return Ok(()),
            Err(e) => {
                // Malformed or unknown request: answer once and stop serving.
                let _ = write_frame(
                    w,
                    &Response::Error { code: if_core::error::Exit::Usage as u8, message: e.to_string() },
                );
                return Err(e.to_string());
            }
        };
        let resp = match req {
            Request::Ping => Response::Pong,
            Request::Version => Response::Version { version: if_core::APP_VERSION.into() },
            Request::Shutdown => {
                write_frame(w, &Response::Bye).map_err(|e| e.to_string())?;
                return Ok(());
            }
        };
        write_frame(w, &resp).map_err(|e| e.to_string())?;
    }
}

#[cfg(windows)]
pub fn is_elevated() -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    // SAFETY: plain Win32 calls on our own process token; the handle is closed
    // and the output buffer is a correctly sized TOKEN_ELEVATION.
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elev = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elev as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        CloseHandle(token);
        ok != 0 && elev.TokenIsElevated != 0
    }
}

#[cfg(not(windows))]
pub fn is_elevated() -> bool {
    false
}

/// Opens the app's pipe as a client. Impersonation is limited to
/// identification level, so a squatting server could not act as this
/// (possibly elevated) process, and the server's process id must be the app
/// that started us.
#[cfg(windows)]
pub fn connect_pipe(name: &str, app_pid: u32) -> Result<std::fs::File, String> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::SECURITY_IDENTIFICATION;
    use windows_sys::Win32::System::Pipes::GetNamedPipeServerProcessId;

    if !name.starts_with(r"\\.\pipe\IsoFoundry-") {
        return Err("refusing a pipe that is not an IsoFoundry pipe".into());
    }
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .security_qos_flags(SECURITY_IDENTIFICATION)
        .open(name)
        .map_err(|e| format!("cannot open {name}: {e}"))?;
    let mut server_pid = 0u32;
    // SAFETY: the handle is owned by `f` and valid for the duration of the call.
    let ok = unsafe { GetNamedPipeServerProcessId(f.as_raw_handle() as _, &mut server_pid) };
    if ok == 0 || server_pid != app_pid {
        return Err(format!("pipe server is process {server_pid}, expected {app_pid}"));
    }
    Ok(f)
}

#[cfg(not(windows))]
pub fn connect_pipe(_name: &str, _app_pid: u32) -> Result<std::fs::File, String> {
    Err("named pipes are only used on Windows; use --stdio".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use if_core::ipc::check_hello;

    fn frames(reqs: &[Request]) -> Vec<u8> {
        let mut b = Vec::new();
        for r in reqs {
            write_frame(&mut b, r).unwrap();
        }
        b
    }

    #[test]
    fn hello_then_answers_until_shutdown() {
        let nonce = if_core::ipc::new_nonce();
        let input = frames(&[Request::Ping, Request::Version, Request::Shutdown, Request::Ping]);
        let mut out = Vec::new();
        serve(&mut input.as_slice(), &mut out, &nonce, false).unwrap();
        let mut r = out.as_slice();
        let hello: Hello = read_frame(&mut r).unwrap();
        check_hello(&hello, &nonce).unwrap();
        assert_eq!(read_frame::<_, Response>(&mut r).unwrap(), Response::Pong);
        assert!(matches!(read_frame::<_, Response>(&mut r).unwrap(), Response::Version { .. }));
        assert_eq!(read_frame::<_, Response>(&mut r).unwrap(), Response::Bye);
        // Nothing is answered after shutdown.
        assert!(matches!(read_frame::<_, Response>(&mut r), Err(FrameError::Closed)));
    }

    #[test]
    fn garbage_request_ends_session_with_error() {
        let mut input = 5u32.to_le_bytes().to_vec();
        input.extend_from_slice(b"nope!");
        let mut out = Vec::new();
        assert!(serve(&mut input.as_slice(), &mut out, &"a".repeat(64), false).is_err());
        let mut r = out.as_slice();
        let _: Hello = read_frame(&mut r).unwrap();
        assert!(matches!(read_frame::<_, Response>(&mut r).unwrap(), Response::Error { .. }));
    }
}
