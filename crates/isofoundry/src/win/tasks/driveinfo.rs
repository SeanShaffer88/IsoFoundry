//! Drive info window (M4): every recorder IMAPI2 can see (plus
//! the fake drive), with its media state. Read-only.

use super::super::taskwin::*;
use super::drivepick::DrivePane;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "IsoFoundryTaskDriveInfo";
const ID_DRIVES: u16 = 1;
const ID_REFRESH: u16 = 2;
const ID_TEXT: u16 = 3;

struct State {
    drives: DrivePane,
    text: HWND,
}

pub fn open(owner: HWND) {
    register_class(CLASS, Some(wndproc));
    let state = Box::new(State { drives: DrivePane::empty(), text: std::ptr::null_mut() });
    create_window(owner, CLASS, "Drive information", 664, 430, Box::into_raw(state).cast());
}

fn st(hwnd: HWND) -> &'static mut State {
    unsafe { &mut *state_ptr::<State>(hwnd) }
}

fn describe(s: &State) -> String {
    let Some(info) = s.drives.selected() else {
        return "No drives found.".to_string();
    };
    let mut out = format!(
        "Drive: {}\r\nVendor: {}\r\nModel: {}\r\nFirmware: {}\r\nBackend: {}",
        info.display_name(),
        info.vendor.trim(),
        info.product.trim(),
        info.revision.trim(),
        info.backend
    );
    match if_drive::open(&info).and_then(|session| session.media()) {
        Ok(m) if m.present => {
            out.push_str(&format!(
                "\r\n\r\nMedia: {}\r\nCapacity: {} bytes\r\nFree: {} bytes\r\nState: {}",
                m.kind,
                m.capacity_bytes,
                m.free_bytes,
                if m.blank {
                    "blank"
                } else if m.appendable {
                    "appendable (more sessions fit)"
                } else {
                    "closed"
                }
            ));
        }
        Ok(_) => out.push_str("\r\n\r\nMedia: none inserted"),
        Err(e) => out.push_str(&format!("\r\n\r\nMedia: could not be read ({e})")),
    }
    out
}

extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            WM_NCCREATE => {
                attach::<State>(hwnd, lp);
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_CREATE => {
                let s = st(hwnd);
                label(hwnd, 16, 14, 300, "Recorder:");
                s.drives.create(hwnd, ID_DRIVES, 16, 36, 500, 80);
                button(hwnd, ID_REFRESH, 524, 36, 120, "Refresh");
                s.text = multiline(hwnd, ID_TEXT, 16, 128, 628, 190);
                set_text(s.text, &describe(s));
                0
            }
            WM_COMMAND => {
                let s = st(hwnd);
                match ((wp & 0xFFFF) as u16, (wp >> 16) as u16) {
                    (ID_REFRESH, _) => {
                        s.drives.refresh();
                        set_text(s.text, &describe(s));
                        0
                    }
                    (ID_DRIVES, 1) => {
                        set_text(s.text, &describe(s));
                        0
                    }
                    _ => DefWindowProcW(hwnd, msg, wp, lp),
                }
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                0
            }
            WM_NCDESTROY => {
                let p = state_ptr::<State>(hwnd);
                if !p.is_null() {
                    // SAFETY: created with Box::into_raw in `open`.
                    drop(Box::from_raw(p));
                }
                DefWindowProcW(hwnd, msg, wp, lp)
            }
            WM_CTLCOLORSTATIC | WM_CTLCOLORBTN | WM_CTLCOLOREDIT | WM_CTLCOLORLISTBOX => ctl_color(wp),
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}
