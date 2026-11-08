//! Burn image window (M4): pick an image and a drive, choose the
//! burn options, write it with the `if-drive` layer.

use super::super::taskwin::*;
use super::drivepick::DrivePane;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "IsoFoundryTaskBurnImage";
const ID_IMAGE: u16 = 1;
const ID_BROWSE: u16 = 2;
const ID_DRIVES: u16 = 3;
const ID_REFRESH: u16 = 4;
const ID_SIMULATE: u16 = 5;
const ID_VERIFY: u16 = 6;
const ID_APPEND: u16 = 7;
const ID_CLOSE: u16 = 8;
const ID_EJECT: u16 = 9;
const ID_BURN: u16 = 13;
const ID_CANCEL: u16 = 14;

struct State {
    image: HWND,
    drives: DrivePane,
    media: HWND,
    simulate: HWND,
    verify: HWND,
    append: HWND,
    close: HWND,
    eject: HWND,
    status: HWND,
    bar: HWND,
    burn: HWND,
    cancel: HWND,
    shared: std::sync::Arc<JobShared>,
}

pub fn open(owner: HWND) {
    register_class(CLASS, Some(wndproc));
    let state = Box::new(State {
        image: std::ptr::null_mut(),
        drives: DrivePane::empty(),
        media: std::ptr::null_mut(),
        simulate: std::ptr::null_mut(),
        verify: std::ptr::null_mut(),
        append: std::ptr::null_mut(),
        close: std::ptr::null_mut(),
        eject: std::ptr::null_mut(),
        status: std::ptr::null_mut(),
        bar: std::ptr::null_mut(),
        burn: std::ptr::null_mut(),
        cancel: std::ptr::null_mut(),
        shared: JobShared::new(),
    });
    create_window(owner, CLASS, "Burn image to disc", 664, 420, Box::into_raw(state).cast());
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
                label(hwnd, 16, 14, 200, "Image file:");
                s.image = edit(hwnd, ID_IMAGE, 16, 34, 500, "", false);
                button(hwnd, ID_BROWSE, 524, 32, 120, "Browse…");
                label(hwnd, 16, 66, 200, "Recorder:");
                s.drives.create(hwnd, ID_DRIVES, 16, 86, 500, 66);
                button(hwnd, ID_REFRESH, 524, 86, 120, "Refresh");
                s.media = label(hwnd, 16, 158, 628, "Media: —");
                set_text(s.media, &format!("Media: {}", s.drives.media_summary()));
                s.simulate = checkbox(hwnd, ID_SIMULATE, 16, 186, 300, "Simulate only (write nothing)");
                s.verify = checkbox(hwnd, ID_VERIFY, 16, 210, 300, "Verify after burn");
                set_checked(s.verify, true);
                s.append = checkbox(hwnd, ID_APPEND, 332, 186, 300, "Append to the disc (new session)");
                s.close = checkbox(hwnd, ID_CLOSE, 332, 210, 300, "Close the disc (no more sessions)");
                s.eject = checkbox(hwnd, ID_EJECT, 16, 234, 300, "Eject when done");
                s.bar = progress_bar(hwnd, 16, 268, 628);
                s.status = label(hwnd, 16, 292, 628, "Ready.");
                s.burn = button(hwnd, ID_BURN, 404, 322, 120, "Burn");
                s.cancel = button(hwnd, ID_CANCEL, 532, 322, 112, "Cancel");
                set_enabled(s.cancel, false);
                0
            }
            WM_COMMAND => {
                let s = st(hwnd);
                match ((wp & 0xFFFF) as u16, (wp >> 16) as u16) {
                    (ID_BROWSE, _) => {
                        if let Some(p) = pick_open(
                            hwnd,
                            "Open disc image",
                            "Disc images|*.iso;*.img;*.bin;*.ccd;*.chd|All files|*.*",
                        ) {
                            set_text(s.image, &p.to_string_lossy());
                        }
                        0
                    }
                    (ID_REFRESH, _) => {
                        s.drives.refresh();
                        set_text(s.media, &format!("Media: {}", s.drives.media_summary()));
                        0
                    }
                    (ID_DRIVES, 1) => {
                        set_text(s.media, &format!("Media: {}", s.drives.media_summary()));
                        0
                    }
                    (ID_BURN, _) => {
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
                set_enabled(s.burn, true);
                set_enabled(s.cancel, false);
                match s.shared.take_result() {
                    Some(Ok(text)) => {
                        progress_show(s.bar, Some(1.0));
                        set_text(s.status, &text);
                        crate::win::gui::note_log(&text, false);
                    }
                    Some(Err(e)) => {
                        set_text(s.status, &format!("Failed: {e}"));
                        crate::win::gui::note_log(&format!("Burn image failed: {e}"), true);
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
    let path = get_text(s.image);
    if path.is_empty() || !PathBuf::from(&path).is_file() {
        set_text(s.status, "Choose an image file first.");
        return;
    }
    let Some(info) = s.drives.selected() else {
        set_text(s.status, "No drive selected.");
        return;
    };
    let opts = if_drive::BurnOptions {
        simulate: is_checked(s.simulate),
        verify: is_checked(s.verify),
        close_disc: is_checked(s.close),
        append: is_checked(s.append),
        speed_kbps: None,
        eject_when_done: is_checked(s.eject),
    };
    let source = if_drive::BurnSource::Image(PathBuf::from(&path));
    set_enabled(s.burn, false);
    set_enabled(s.cancel, true);
    set_text(s.status, "Burning…");
    let shared = s.shared.clone();
    run_job(hwnd, &shared, move |sh| {
        let mut session = if_drive::open(&info).map_err(|e| e.to_string())?;
        let rep = session.burn(&source, &opts, &sh.progress(), &sh.cancel).map_err(|e| e.to_string())?;
        let mut msg = format!("Done: {} bytes written via {}.", rep.bytes_written, rep.backend);
        if rep.verified {
            msg.push_str(" Verified.");
        }
        for n in &rep.notes {
            msg.push_str(&format!(" {n}"));
        }
        Ok(msg)
    });
}
