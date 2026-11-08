//! Surface test window (M5): scan every sector of the disc once
//! and grade the surface (the quality scan for drives without
//! vendor error reporting).

use super::super::taskwin::*;
use super::drivepick::DrivePane;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "IsoFoundryTaskSurfaceTest";
const ID_DRIVES: u16 = 1;
const ID_REFRESH: u16 = 2;
const ID_GO: u16 = 3;
const ID_CANCEL: u16 = 4;
const ID_LOG: u16 = 5;

struct State {
    drives: DrivePane,
    media: HWND,
    log: HWND,
    status: HWND,
    bar: HWND,
    go: HWND,
    cancel: HWND,
    shared: std::sync::Arc<JobShared>,
}

pub fn open(owner: HWND) {
    register_class(CLASS, Some(wndproc));
    let state = Box::new(State {
        drives: DrivePane::empty(),
        media: std::ptr::null_mut(),
        log: std::ptr::null_mut(),
        status: std::ptr::null_mut(),
        bar: std::ptr::null_mut(),
        go: std::ptr::null_mut(),
        cancel: std::ptr::null_mut(),
        shared: JobShared::new(),
    });
    create_window(owner, CLASS, "Surface test", 664, 430, Box::into_raw(state).cast());
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
                label(hwnd, 16, 14, 200, "Drive:");
                s.drives.create(hwnd, ID_DRIVES, 16, 34, 500, 66);
                button(hwnd, ID_REFRESH, 524, 34, 120, "Refresh");
                s.media = label(hwnd, 16, 106, 628, "Media: —");
                set_text(s.media, &format!("Media: {}", s.drives.media_summary()));
                s.log = multiline(hwnd, ID_LOG, 16, 134, 628, 140);
                s.bar = progress_bar(hwnd, 16, 286, 628);
                s.status = label(hwnd, 16, 310, 628, "Ready.");
                s.go = button(hwnd, ID_GO, 404, 342, 120, "Scan");
                s.cancel = button(hwnd, ID_CANCEL, 532, 342, 112, "Cancel");
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
                    (ID_GO, _) => {
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
                set_text(s.status, &format!("Scanning… {}", progress_text(done, total)));
                0
            }
            WM_JOB_DONE => {
                let s = st(hwnd);
                stop_timer(hwnd);
                set_enabled(s.go, true);
                set_enabled(s.cancel, false);
                match s.shared.take_result() {
                    Some(Ok(text)) => {
                        progress_show(s.bar, Some(1.0));
                        set_text(s.log, &text.replace('\n', "\r\n"));
                        set_text(s.status, "Scan complete.");
                        crate::win::gui::note_log("Surface test complete", false);
                    }
                    Some(Err(e)) => {
                        set_text(s.status, &format!("Failed: {e}"));
                        crate::win::gui::note_log(&format!("Surface test failed: {e}"), true);
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
    set_enabled(s.go, false);
    set_enabled(s.cancel, true);
    set_text(s.status, "Scanning…");
    let shared = s.shared.clone();
    run_job(hwnd, &shared, move |sh| {
        let mut drive: Box<dyn if_drive::OpticalDrive> = if info.id == "fake" {
            Box::new(crate::cli_rip::fake_data_drive(&[], &[])?)
        } else {
            if_drive::open(&info).map_err(|e| e.to_string())?
        };
        let r = if_drive::rip::surface_scan(&mut *drive, &sh.progress(), &sh.cancel).map_err(|e| e.to_string())?;
        let mut text = format!(
            "Surface: {} sectors\nGood: {}\nSlow: {}\nBad: {}\nQuality: {:.1}% (grade {})",
            r.total_sectors,
            r.good,
            r.slow,
            r.bad.len(),
            r.quality_score,
            r.quality_grade
        );
        if !r.bad.is_empty() {
            text.push_str(&format!(
                "\nBad sectors: {}",
                r.bad.iter().map(u32::to_string).collect::<Vec<_>>().join(", ")
            ));
        }
        Ok(text)
    });
}
