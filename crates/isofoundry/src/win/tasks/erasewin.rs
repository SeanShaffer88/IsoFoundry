//! Erase disc window (M4): pick a drive with rewritable media,
//! choose quick or full, erase through the `if-drive` layer.

use super::super::taskwin::*;
use super::drivepick::DrivePane;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "IsoFoundryTaskErase";
const ID_DRIVES: u16 = 1;
const ID_REFRESH: u16 = 2;
const ID_QUICK: u16 = 3;
const ID_FULL: u16 = 4;
const ID_ERASE: u16 = 8;
const ID_CANCEL: u16 = 9;

struct State {
    drives: DrivePane,
    media: HWND,
    quick: HWND,
    status: HWND,
    bar: HWND,
    erase: HWND,
    cancel: HWND,
    shared: std::sync::Arc<JobShared>,
}

pub fn open(owner: HWND) {
    register_class(CLASS, Some(wndproc));
    let state = Box::new(State {
        drives: DrivePane::empty(),
        media: std::ptr::null_mut(),
        quick: std::ptr::null_mut(),
        status: std::ptr::null_mut(),
        bar: std::ptr::null_mut(),
        erase: std::ptr::null_mut(),
        cancel: std::ptr::null_mut(),
        shared: JobShared::new(),
    });
    create_window(owner, CLASS, "Erase rewritable disc", 664, 400, Box::into_raw(state).cast());
}

fn st(hwnd: HWND) -> &'static mut State {
    unsafe { &mut *state_ptr::<State>(hwnd) }
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
                label(hwnd, 16, 14, 400, "Recorder (only rewritable media — CD-RW, DVD±RW, BD-RE — can be erased):");
                s.drives.create(hwnd, ID_DRIVES, 16, 36, 500, 66);
                button(hwnd, ID_REFRESH, 524, 36, 120, "Refresh");
                s.media = label(hwnd, 16, 108, 628, "Media: —");
                set_text(s.media, &format!("Media: {}", s.drives.media_summary()));
                s.quick = radio(hwnd, ID_QUICK, 16, 138, 300, "Quick erase (fast, clears the index)", true);
                radio(hwnd, ID_FULL, 16, 162, 300, "Full erase (slow, wipes every sector)", false);
                s.bar = progress_bar(hwnd, 16, 196, 628);
                s.status = label(hwnd, 16, 220, 628, "Erasing cannot be undone.");
                s.erase = button(hwnd, ID_ERASE, 404, 252, 120, "Erase disc");
                s.cancel = button(hwnd, ID_CANCEL, 532, 252, 112, "Cancel");
                set_enabled(s.cancel, false);
                0
            }
            WM_COMMAND => {
                let s = st(hwnd);
                match ((wp & 0xFFFF) as u16, (wp >> 16) as u16) {
                    (ID_REFRESH, _) => {
                        s.drives.refresh();
                        set_text(s.media, &format!("Media: {}", s.drives.media_summary()));
                        0
                    }
                    (ID_DRIVES, 1) => {
                        set_text(s.media, &format!("Media: {}", s.drives.media_summary()));
                        0
                    }
                    (ID_ERASE, _) => {
                        start(s, hwnd);
                        0
                    }
                    (ID_CANCEL, _) => {
                        s.shared.cancel.cancel();
                        0
                    }
                    _ => DefWindowProcW(hwnd, msg, wp, lp),
                }
            }
            WM_TIMER if wp == TIMER_JOB => {
                let s = st(hwnd);
                let (done, total) = s.shared.snapshot();
                progress_show(s.bar, s.shared.fraction());
                set_text(s.status, &progress_text(done, total));
                0
            }
            WM_JOB_DONE => {
                let s = st(hwnd);
                stop_timer(hwnd);
                set_enabled(s.erase, true);
                set_enabled(s.cancel, false);
                match s.shared.take_result() {
                    Some(Ok(text)) => {
                        progress_show(s.bar, Some(1.0));
                        set_text(s.status, &text);
                        set_text(s.media, &format!("Media: {}", s.drives.media_summary()));
                        crate::win::gui::note_log(&text, false);
                    }
                    Some(Err(e)) => {
                        set_text(s.status, &format!("Failed: {e}"));
                        crate::win::gui::note_log(&format!("Erase failed: {e}"), true);
                    }
                    None => {}
                }
                0
            }
            WM_CLOSE => {
                st(hwnd).shared.cancel.cancel();
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

fn start(s: &mut State, hwnd: HWND) {
    let Some(info) = s.drives.selected() else {
        set_text(s.status, "No drive selected.");
        return;
    };
    let kind = if is_checked(s.quick) { if_drive::EraseKind::Quick } else { if_drive::EraseKind::Full };
    set_enabled(s.erase, false);
    set_enabled(s.cancel, true);
    set_text(s.status, "Erasing…");
    let shared = s.shared.clone();
    run_job(hwnd, &shared, move |sh| {
        let mut session = if_drive::open(&info).map_err(|e| e.to_string())?;
        session.erase(kind, &sh.progress(), &sh.cancel).map_err(|e| e.to_string())?;
        Ok(format!("Done: disc erased ({kind:?})."))
    });
}
