//! Starts `isofoundry-helper.exe` (elevated when asked) and talks to it
//! over a named pipe this process owns (D-014, docs/ARCHITECTURE.md §1).
//!
//! Safety checks, in order:
//! 1. The pipe is created first, single instance, local clients only, with
//!    a DACL that admits only the current user, Administrators and SYSTEM.
//! 2. The client that connects must be the exact process we started.
//! 3. Its first message must carry the per-session nonce and our protocol.

use super::util::{wide, OwnedHandle};
use crate::cli::HelperInfo;
use if_core::error::Exit;
use if_core::ipc::{self, Hello, Request, Response};
use std::io::{Read, Write};
use std::time::Duration;
use windows_sys::Win32::Foundation::{
    GetLastError, LocalFree, ERROR_CANCELLED, ERROR_IO_PENDING, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE,
    WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, TokenUser, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
};
use windows_sys::Win32::Storage::FileSystem::{
    ReadFile, WriteFile, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX,
};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, GetNamedPipeClientProcessId, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, GetCurrentProcess, GetProcessId, OpenProcessToken, TerminateProcess, WaitForMultipleObjects,
    WaitForSingleObject,
};
use windows_sys::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::UI::Shell::{
    ShellExecuteExW, SEE_MASK_FLAG_NO_UI, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

/// Time for the user to answer the UAC prompt.
const CONNECT_TIMEOUT_ELEVATED: Duration = Duration::from_secs(120);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const IO_TIMEOUT: Duration = Duration::from_secs(10);

type Fail = (Exit, String);

fn fail(exit: Exit, m: impl Into<String>) -> Fail {
    (exit, m.into())
}

/// `isofoundry helper-check [--elevated]`: start, handshake, ping, stop.
pub fn check(elevated: bool) -> Result<HelperInfo, Fail> {
    let mut s = Session::start(elevated)?;
    s.call(&Request::Ping, |r| matches!(r, Response::Pong))?;
    let version = match s.request(&Request::Version)? {
        Response::Version { version } => version,
        other => return Err(fail(Exit::Generic, format!("unexpected reply: {other:?}"))),
    };
    s.call(&Request::Shutdown, |r| matches!(r, Response::Bye))?;
    Ok(HelperInfo { version, elevated: s.hello.elevated, pid: s.pid })
}

/// Lists physical disks through an elevated helper session.
pub fn usb_list() -> Result<Vec<usbwriter::device::DiskInfo>, Fail> {
    let mut s = Session::start(true)?;
    let disks = match s.request(&Request::UsbList)? {
        Response::UsbDisks { disks } => disks,
        other => return Err(fail(Exit::Generic, format!("unexpected reply: {other:?}"))),
    };
    let _ = s.request(&Request::Shutdown);
    Ok(disks.iter().map(disk_from_dto).collect())
}

/// Runs one USB job in an elevated helper session, reporting progress.
pub fn usb_job(job: &ipc::UsbJob, on_progress: impl FnMut(u64, u64)) -> Result<(usize, u64, Vec<String>), Fail> {
    let mut s = Session::start(true)?;
    let resp = s.request_streaming(&Request::UsbJob(job.clone()), on_progress)?;
    let _ = s.request(&Request::Shutdown);
    match resp {
        Response::UsbDone { files, bytes, notes } => Ok((files, bytes, notes)),
        other => Err(fail(Exit::Generic, format!("unexpected reply: {other:?}"))),
    }
}

/// Converts the wire DTO back into the engine's disk description.
pub fn disk_from_dto(d: &ipc::DiskDto) -> usbwriter::device::DiskInfo {
    use usbwriter::device::{Bus, DiskInfo};
    DiskInfo {
        number: d.number,
        model: d.model.clone(),
        size_bytes: d.size_bytes,
        bus: match d.bus.as_str() {
            "usb" => Bus::Usb,
            "nvme" => Bus::Nvme,
            "sata" => Bus::Sata,
            "scsi" => Bus::Scsi,
            "virtual" => Bus::Virtual,
            _ => Bus::Other,
        },
        removable: d.removable,
        is_system: d.is_system,
        holds_boot: d.holds_boot,
        holds_pagefile: d.holds_pagefile,
        volume_labels: d.volume_labels.clone(),
        is_file_target: false,
    }
}

pub struct Session {
    pipe: Pipe,
    process: OwnedHandle,
    pub pid: u32,
    pub hello: Hello,
}

impl Session {
    pub fn start(elevated: bool) -> Result<Session, Fail> {
        let exe = std::env::current_exe().map_err(|e| fail(Exit::Io, format!("cannot locate isofoundry.exe: {e}")))?;
        let helper = exe.with_file_name("isofoundry-helper.exe");
        if !helper.is_file() {
            return Err(fail(Exit::Generic, format!("{} is missing; reinstall IsoFoundry", helper.display())));
        }
        let nonce = ipc::new_nonce();
        let name = ipc::pipe_name(&nonce);
        let pipe = Pipe::create(&name)?;

        let params = format!("--pipe {name} --nonce {nonce} --app-pid {}", std::process::id());
        let process = launch(&helper, &params, elevated)?;
        // SAFETY: valid process handle owned by `process`.
        let pid = unsafe { GetProcessId(process.0) };

        let timeout = if elevated { CONNECT_TIMEOUT_ELEVATED } else { CONNECT_TIMEOUT };
        if let Err(e) = pipe.connect(&process, timeout) {
            kill(&process);
            return Err(e);
        }
        let client = pipe.client_pid().unwrap_or(0);
        if client != pid {
            kill(&process);
            return Err(fail(Exit::Generic, format!("pipe client is process {client}, expected helper {pid}")));
        }
        let mut s = Session { pipe, process, pid, hello: placeholder_hello() };
        let hello: Hello =
            ipc::read_frame(&mut s.pipe).map_err(|e| fail(Exit::Generic, format!("helper handshake: {e}")))?;
        if let Err(e) = ipc::check_hello(&hello, &nonce) {
            kill(&s.process);
            return Err(fail(Exit::Generic, format!("helper handshake rejected: {e}")));
        }
        if elevated && !hello.elevated {
            kill(&s.process);
            return Err(fail(Exit::ElevationDenied, "the helper started without administrator rights"));
        }
        s.hello = hello;
        Ok(s)
    }

    pub fn request(&mut self, req: &Request) -> Result<Response, Fail> {
        ipc::write_frame(&mut self.pipe, req).map_err(|e| fail(Exit::Generic, e.to_string()))?;
        let resp: Response = ipc::read_frame(&mut self.pipe).map_err(|e| fail(Exit::Generic, e.to_string()))?;
        if let Response::Error { message, .. } = &resp {
            return Err(fail(Exit::Generic, format!("helper error: {message}")));
        }
        Ok(resp)
    }

    /// Like `request`, but consumes streamed Progress frames first.
    pub fn request_streaming(
        &mut self,
        req: &Request,
        mut on_progress: impl FnMut(u64, u64),
    ) -> Result<Response, Fail> {
        ipc::write_frame(&mut self.pipe, req).map_err(|e| fail(Exit::Generic, e.to_string()))?;
        loop {
            let resp: Response = ipc::read_frame(&mut self.pipe).map_err(|e| fail(Exit::Generic, e.to_string()))?;
            match resp {
                Response::Progress { done, total } => on_progress(done, total),
                Response::Error { message, .. } => return Err(fail(Exit::Generic, format!("helper error: {message}"))),
                other => return Ok(other),
            }
        }
    }

    fn call(&mut self, req: &Request, ok: impl Fn(&Response) -> bool) -> Result<(), Fail> {
        let r = self.request(req)?;
        if ok(&r) {
            Ok(())
        } else {
            Err(fail(Exit::Generic, format!("unexpected reply to {req:?}: {r:?}")))
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        // A well-behaved helper has exited after Shutdown; give it a moment,
        // then make sure no elevated process outlives the session.
        // SAFETY: valid process handle owned by the session.
        if unsafe { WaitForSingleObject(self.process.0, 2000) } == WAIT_TIMEOUT {
            kill(&self.process);
        }
    }
}

fn placeholder_hello() -> Hello {
    Hello { protocol: 0, nonce: String::new(), helper_version: String::new(), elevated: false }
}

fn kill(p: &OwnedHandle) {
    // SAFETY: valid process handle; failure (already exited) is harmless.
    unsafe { TerminateProcess(p.0, 1) };
}

fn launch(helper: &std::path::Path, params: &str, elevated: bool) -> Result<OwnedHandle, Fail> {
    let verb = wide(if elevated { "runas" } else { "open" });
    let file = wide(&helper.to_string_lossy());
    let params = wide(params);
    // SAFETY: the struct is zeroed, sized, and every pointer outlives the call.
    unsafe {
        let mut sei: SHELLEXECUTEINFOW = std::mem::zeroed();
        sei.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        sei.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI;
        sei.lpVerb = verb.as_ptr();
        sei.lpFile = file.as_ptr();
        sei.lpParameters = params.as_ptr();
        sei.nShow = SW_HIDE;
        if ShellExecuteExW(&mut sei) == 0 {
            let err = GetLastError();
            if err == ERROR_CANCELLED {
                return Err(fail(Exit::ElevationDenied, "administrator rights were declined"));
            }
            return Err(fail(Exit::Generic, format!("cannot start the helper (Windows error {err})")));
        }
        if sei.hProcess.is_null() {
            return Err(fail(Exit::Generic, "the helper started but no process handle was returned"));
        }
        Ok(OwnedHandle(sei.hProcess))
    }
}

/// SDDL: protected DACL, full access for the current user, Administrators
/// (an elevated helper on an over-the-shoulder admin account) and SYSTEM.
fn pipe_sddl() -> Result<String, Fail> {
    // SAFETY: token handle closed by OwnedHandle; buffer sized by the first call;
    // the string SID is freed with LocalFree as documented.
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(fail(Exit::Generic, "cannot read the process token"));
        }
        let token = OwnedHandle(token);
        let mut len = 0u32;
        GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut len);
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        if GetTokenInformation(token.0, TokenUser, buf.as_mut_ptr().cast(), len, &mut len) == 0 {
            return Err(fail(Exit::Generic, "cannot read the user SID"));
        }
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s: *mut u16 = std::ptr::null_mut();
        if ConvertSidToStringSidW(user.User.Sid, &mut s) == 0 {
            return Err(fail(Exit::Generic, "cannot format the user SID"));
        }
        let mut n = 0;
        while *s.add(n) != 0 {
            n += 1;
        }
        let sid = String::from_utf16_lossy(std::slice::from_raw_parts(s, n));
        LocalFree(s.cast());
        Ok(format!("D:P(A;;GA;;;{sid})(A;;GA;;;BA)(A;;GA;;;SY)"))
    }
}

/// Server end of the pipe, opened for overlapped I/O so every wait has a
/// timeout and can be cancelled.
pub struct Pipe {
    h: OwnedHandle,
    ev: OwnedHandle,
}

impl Pipe {
    fn create(name: &str) -> Result<Pipe, Fail> {
        let sddl = wide(&pipe_sddl()?);
        // SAFETY: SDDL string is NUL-terminated; the descriptor is freed with
        // LocalFree after CreateNamedPipeW copied it.
        unsafe {
            let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
            if ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut sd,
                std::ptr::null_mut(),
            ) == 0
            {
                return Err(fail(Exit::Generic, "cannot build the pipe security descriptor"));
            }
            let sa = SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: sd,
                bInheritHandle: 0,
            };
            let n = wide(name);
            let h = CreateNamedPipeW(
                n.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                ipc::MAX_FRAME as u32,
                ipc::MAX_FRAME as u32,
                0,
                &sa,
            );
            LocalFree(sd);
            if h == INVALID_HANDLE_VALUE {
                return Err(fail(Exit::Generic, format!("cannot create pipe (Windows error {})", GetLastError())));
            }
            let h = OwnedHandle(h);
            let ev = OwnedHandle(CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()));
            if !ev.is_valid() {
                return Err(fail(Exit::Generic, "cannot create an event"));
            }
            Ok(Pipe { h, ev })
        }
    }

    fn overlapped(&self) -> OVERLAPPED {
        // SAFETY: OVERLAPPED is plain data; zeroed is its documented initial state.
        let mut o: OVERLAPPED = unsafe { std::mem::zeroed() };
        o.hEvent = self.ev.0;
        o
    }

    /// Waits for the helper to connect, giving up if it exits first or the
    /// timeout passes.
    fn connect(&self, process: &OwnedHandle, timeout: Duration) -> Result<(), Fail> {
        let mut o = self.overlapped();
        // SAFETY: `o` and the handles outlive the operation; it is cancelled
        // and drained before returning on every non-success path.
        unsafe {
            if ConnectNamedPipe(self.h.0, &mut o) != 0 {
                return Ok(());
            }
            match GetLastError() {
                ERROR_PIPE_CONNECTED => return Ok(()),
                ERROR_IO_PENDING => {}
                e => return Err(fail(Exit::Generic, format!("pipe connect failed (Windows error {e})"))),
            }
            let handles = [self.ev.0, process.0];
            let w = WaitForMultipleObjects(2, handles.as_ptr(), 0, timeout.as_millis() as u32);
            let mut n = 0u32;
            if w == WAIT_OBJECT_0 && GetOverlappedResult(self.h.0, &o, &mut n, 0) != 0 {
                return Ok(());
            }
            CancelIoEx(self.h.0, &o);
            GetOverlappedResult(self.h.0, &o, &mut n, 1);
            if w == WAIT_OBJECT_0 + 1 {
                Err(fail(Exit::Generic, "the helper exited before connecting"))
            } else {
                Err(fail(Exit::Generic, "the helper did not connect in time"))
            }
        }
    }

    fn client_pid(&self) -> Option<u32> {
        let mut pid = 0u32;
        // SAFETY: valid pipe handle owned by `self`.
        (unsafe { GetNamedPipeClientProcessId(self.h.0, &mut pid) } != 0).then_some(pid)
    }

    fn io(&mut self, buf_ptr: *mut u8, len: usize, write: bool) -> std::io::Result<usize> {
        let len = len.min(u32::MAX as usize) as u32;
        let mut o = self.overlapped();
        let mut n = 0u32;
        // SAFETY: the buffer is valid for `len` bytes and, like `o`, outlives
        // the operation, which completes or is cancelled and drained here.
        unsafe {
            let ok = if write {
                WriteFile(self.h.0, buf_ptr, len, std::ptr::null_mut(), &mut o)
            } else {
                ReadFile(self.h.0, buf_ptr, len, std::ptr::null_mut(), &mut o)
            };
            if ok == 0 && GetLastError() != ERROR_IO_PENDING {
                return Err(std::io::Error::last_os_error());
            }
            if WaitForSingleObject(self.ev.0, IO_TIMEOUT.as_millis() as u32) != WAIT_OBJECT_0 {
                CancelIoEx(self.h.0, &o);
                GetOverlappedResult(self.h.0, &o, &mut n, 1);
                return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "helper did not answer in time"));
            }
            if GetOverlappedResult(self.h.0, &o, &mut n, 0) == 0 {
                let e = std::io::Error::last_os_error();
                // Broken pipe on read = the helper closed its end.
                if !write && e.raw_os_error() == Some(109) {
                    return Ok(0);
                }
                return Err(e);
            }
        }
        Ok(n as usize)
    }
}

impl Read for Pipe {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.io(buf.as_mut_ptr(), buf.len(), false)
    }
}

impl Write for Pipe {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.io(buf.as_ptr() as *mut u8, buf.len(), true)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
