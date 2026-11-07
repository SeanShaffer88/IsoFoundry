//! Small Win32 helpers: UTF-16 strings, owned handles, OS version.

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};

/// NUL-terminated UTF-16 for `PCWSTR` parameters.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// UTF-16 buffer (possibly NUL-terminated) to `String`.
pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// A kernel handle closed on drop (Directive V2.0 §13: RAII for handles).
pub struct OwnedHandle(pub HANDLE);

// SAFETY: kernel handles may be used from any thread.
unsafe impl Send for OwnedHandle {}
unsafe impl Sync for OwnedHandle {}

impl OwnedHandle {
    pub fn is_valid(&self) -> bool {
        !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if self.is_valid() {
            // SAFETY: we own the handle and close it exactly once.
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// "Windows 10.0 build 22631 (x86_64)". The app manifest declares Windows
/// 10/11 support, so the API reports the real version.
pub fn os_description() -> String {
    use windows_sys::Win32::System::SystemInformation::{GetVersionExW, OSVERSIONINFOW};
    // SAFETY: the struct is zeroed and its size field set as the API requires.
    let v = unsafe {
        let mut v: OSVERSIONINFOW = std::mem::zeroed();
        v.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;
        if GetVersionExW(&mut v) == 0 {
            return format!("Windows (version unknown) ({})", std::env::consts::ARCH);
        }
        v
    };
    let name = if v.dwMajorVersion == 10 && v.dwBuildNumber >= 22000 { "Windows 11" } else { "Windows 10" };
    format!("{name} ({}.{} build {}) ({})", v.dwMajorVersion, v.dwMinorVersion, v.dwBuildNumber, std::env::consts::ARCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wide_roundtrip() {
        let w = wide("Burn – ISO");
        assert_eq!(*w.last().unwrap(), 0);
        assert_eq!(from_wide(&w), "Burn – ISO");
    }
}
