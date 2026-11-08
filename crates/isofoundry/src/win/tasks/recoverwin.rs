//! Recover files window (M5): rip tolerantly (retries, zero-fill
//! the unreadable sectors) and extract whatever the filesystem
//! still gives up, with a damage report.

use super::super::taskwin::*;
use super::drivepick::DrivePane;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "IsoFoundryTaskRecover";
const ID_DRIVES: u16 = 1;
const ID_REFRESH: u16 = 2;
const ID_OUT: u16 = 3;
const ID_BROWSE: u16 = 4;
const ID_LOG: u16 = 5;
const ID_GO: u16 = 6;
const ID_CANCEL: u16 = 7;

struct State {
    drives: DrivePane,
    media: HWND,
    out: HWND,
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
        out: std::ptr::null_mut(),
        log: std::ptr::null_mut(),
        status: std::ptr::null_mut(),
        bar: std::ptr::null_mut(),
        go: std::ptr::null_mut(),
        cancel: std::ptr::null_mut(),
        shared: JobShared::new(),
    });
    create_window(owner, CLASS, "Recover files from a damaged disc", 664, 470, Box::into_raw(state).cast());
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
                s.drives.create(hwnd, ID_DRIVES, 16, 34, 500, 60);
                button(hwnd, ID_REFRESH, 524, 34, 120, "Refresh");
                s.media = label(hwnd, 16, 100, 628, "Media: —");
                set_text(s.media, &format!("Media: {}", s.drives.media_summary()));
                label(hwnd, 16, 128, 200, "Recover into folder:");
                s.out = edit(hwnd, ID_OUT, 16, 148, 500, "", false);
                button(hwnd, ID_BROWSE, 524, 146, 120, "Browse…");
                s.log = multiline(hwnd, ID_LOG, 16, 182, 628, 110);
                s.bar = progress_bar(hwnd, 16, 304, 628);
                s.status = label(
                    hwnd,
                    16,
                    328,
                    628,
                    "Ready. The test drive's disc is deliberately damaged, so recovery can be tried end to end.",
                );
                s.go = button(hwnd, ID_GO, 404, 360, 120, "Recover");
                s.cancel = button(hwnd, ID_CANCEL, 532, 360, 112, "Cancel");
                set_enabled(s.cancel, false);
                0
            }
            WM_COMMAND => {
                let s = st(hwnd);
                match ((wp & 0xFFFF) as u16, (wp >> 16) as u16) {
                    (ID_BROWSE, _) => {
                        if let Some(p) = pick_folder(hwnd, "Choose the recovery folder") {
                            set_text(s.out, &p.to_string_lossy());
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
                set_text(s.status, &progress_text(done, total));
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
                        set_text(s.status, "Recovery complete.");
                        crate::win::gui::note_log("Recover files complete", false);
                    }
                    Some(Err(e)) => {
                        set_text(s.status, &format!("Failed: {e}"));
                        crate::win::gui::note_log(&format!("Recover files failed: {e}"), true);
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
    let out = get_text(s.out);
    if out.is_empty() {
        set_text(s.status, "Choose a recovery folder first.");
        return;
    }
    let Some(info) = s.drives.selected() else {
        set_text(s.status, "No drive selected.");
        return;
    };
    let dest = PathBuf::from(out);
    set_enabled(s.go, false);
    set_enabled(s.cancel, true);
    set_text(s.status, "Recovering…");
    let shared = s.shared.clone();
    run_job(hwnd, &shared, move |sh| {
        let mut drive: Box<dyn if_drive::OpticalDrive> = if info.id == "fake" {
            Box::new(crate::cli_rip::fake_data_drive(&[280, 281], &[290])?)
        } else {
            if_drive::open(&info).map_err(|e| e.to_string())?
        };
        let desc = info.display_name();
        let tmp = std::env::temp_dir().join(format!("isofoundry-recover-{}.iso", std::process::id()));
        let opts = if_drive::rip::RipImageOptions { retries: 5, ..Default::default() };
        let report = if_drive::rip::rip_image(&mut *drive, &tmp, &opts, &sh.progress(), &sh.cancel)
            .map_err(|e| e.to_string())?;
        let bad = report.bad_sectors.clone();
        let result = crate::cli_rip::extract_recovered(&tmp, &dest, &bad);
        std::fs::remove_file(&tmp).ok();
        let (files, damaged) = result?;
        Ok(format!(
            "Recovered from {desc}\nUnreadable sectors ({}): {}\nFiles extracted: {files}\nFiles with damaged (zero-filled) regions: {}",
            bad.len(),
            bad.iter().map(u32::to_string).collect::<Vec<_>>().join(", "),
            if damaged.is_empty() { "none".to_string() } else { damaged.join(", ") }
        ))
    });
}
