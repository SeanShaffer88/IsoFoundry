//! The release exe uses the GUI subsystem, so a command typed in a console
//! has no stdout until we attach to the parent's console. Redirected
//! handles (pipes, files) are inherited and left alone.

use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::System::Console::{AttachConsole, GetStdHandle, ATTACH_PARENT_PROCESS, STD_OUTPUT_HANDLE};

pub fn attach() {
    // SAFETY: plain Win32 calls without pointers; failure just means no console.
    unsafe {
        let h = GetStdHandle(STD_OUTPUT_HANDLE);
        if h.is_null() || h == INVALID_HANDLE_VALUE {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}
