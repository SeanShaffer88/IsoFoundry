//! Completion actions for finished jobs (M6): the sound, the
//! notification balloon and the optional shutdown, driven by the
//! settings the main window publishes here (worker threads read
//! atomics only; the balloon itself is shown by the main window in
//! answer to [`WM_FINISH_TOAST`]).

use super::util::wide;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, HWND, LPARAM, LUID};
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, LookupPrivilegeValueW, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows_sys::Win32::System::Diagnostics::Debug::MessageBeep;
use windows_sys::Win32::System::Shutdown::InitiateSystemShutdownExW;
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, MB_ICONEXCLAMATION, MB_OK, WM_APP};

/// Posted to the main window with a heap `Box<String>` summary in
/// LPARAM (the window reclaims it) and success in WPARAM.
pub(crate) const WM_FINISH_TOAST: u32 = WM_APP + 21;

static SOUND: AtomicBool = AtomicBool::new(true);
static TOAST: AtomicBool = AtomicBool::new(false);
static SHUTDOWN: AtomicBool = AtomicBool::new(false);
static MAIN_HWND: AtomicUsize = AtomicUsize::new(0);

pub(crate) fn set_main_hwnd(hwnd: HWND) {
    MAIN_HWND.store(hwnd as usize, Ordering::Relaxed);
}

/// Called by the main window whenever settings load or change.
pub(crate) fn set_prefs(sound: bool, toast: bool, shutdown: bool) {
    SOUND.store(sound, Ordering::Relaxed);
    TOAST.store(toast, Ordering::Relaxed);
    SHUTDOWN.store(shutdown, Ordering::Relaxed);
}

/// Runs the configured completion actions. Called from job worker
/// threads, so it touches no window state directly.
pub(crate) fn on_job_finished(ok: bool, summary: &str) {
    if SOUND.load(Ordering::Relaxed) {
        // SAFETY: a plain system sound, any thread.
        unsafe { MessageBeep(if ok { MB_OK } else { MB_ICONEXCLAMATION }) };
    }
    if TOAST.load(Ordering::Relaxed) {
        let hwnd = MAIN_HWND.load(Ordering::Relaxed);
        if hwnd != 0 {
            let boxed = Box::into_raw(Box::new(summary.to_string()));
            // SAFETY: the main window reclaims the box when it
            // handles the message; if the post fails we reclaim it
            // here so nothing leaks.
            let posted =
                unsafe { PostMessageW(hwnd as HWND, WM_FINISH_TOAST, if ok { 1 } else { 0 }, boxed as LPARAM) };
            if posted == 0 {
                unsafe { drop(Box::from_raw(boxed)) };
            }
        }
    }
    if ok && SHUTDOWN.load(Ordering::Relaxed) {
        shutdown_after_finish("IsoFoundry finished a task and is shutting down, as set in Settings (Options menu). Run shutdown /a to cancel.");
    }
}

/// Starts an announced, cancellable shutdown (20 s grace). Returns
/// whether Windows accepted it.
pub(crate) fn shutdown_after_finish(message: &str) -> bool {
    unsafe {
        enable_shutdown_privilege();
        let msg = wide(message);
        InitiateSystemShutdownExW(
            std::ptr::null(),
            msg.as_ptr(),
            20,
            0,
            0,
            0x0000_0001, // SHTDN_REASON_MAJOR_OTHER
        ) != 0
    }
}

unsafe fn enable_shutdown_privilege() {
    let mut token: HANDLE = std::ptr::null_mut();
    if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut token) == 0 {
        return;
    }
    let name = wide("SeShutdownPrivilege");
    let mut luid = LUID { LowPart: 0, HighPart: 0 };
    if LookupPrivilegeValueW(std::ptr::null(), name.as_ptr(), &mut luid) != 0 {
        let tp = TOKEN_PRIVILEGES {
            PrivilegeCount: 1,
            Privileges: [windows_sys::Win32::Security::LUID_AND_ATTRIBUTES {
                Luid: luid,
                Attributes: 2, // SE_PRIVILEGE_ENABLED
            }],
        };
        AdjustTokenPrivileges(token, 0, &tp, 0, std::ptr::null_mut(), std::ptr::null_mut());
    }
    CloseHandle(token);
}
