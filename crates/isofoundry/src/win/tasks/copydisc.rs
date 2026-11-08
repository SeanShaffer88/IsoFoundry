//! Copy disc window (M5): pick a source and a destination drive;
//! the source is ripped and burned to the destination (through a
//! temporary image; verified after burn on drives that read back).

use super::super::taskwin::*;
use super::drivepick::DrivePane;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "IsoFoundryTaskCopyDisc";
const ID_SRC: u16 = 1;
const ID_DST: u16 = 2;
const ID_SRC_REFRESH: u16 = 3;
const ID_DST_REFRESH: u16 = 4;
const ID_CLOSE: u16 = 5;
const ID_GO: u16 = 6;
const ID_CANCEL: u16 = 7;

struct State {
    src: DrivePane,
    dst: DrivePane,
    close: HWND,
    status: HWND,
    bar: HWND,
    go: HWND,
    cancel: HWND,
    shared: std::sync::Arc<JobShared>,
}

pub fn open(owner: HWND) {
    register_class(CLASS, Some(wndproc));
    let state = Box::new(State {
        src: DrivePane::empty(),
        dst: DrivePane::empty(),
        close: std::ptr::null_mut(),
        status: std::ptr::null_mut(),
        bar: std::ptr::null_mut(),
        go: std::ptr::null_mut(),
        cancel: std::ptr::null_mut(),
        shared: JobShared::new(),
    });
    create_window(owner, CLASS, "Copy disc", 664, 420, Box::into_raw(state).cast());
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
                label(hwnd, 16, 14, 200, "Source drive:");
                s.src.create(hwnd, ID_SRC, 16, 34, 500, 60);
                button(hwnd, ID_SRC_REFRESH, 524, 34, 120, "Refresh");
                label(hwnd, 16, 104, 200, "Destination drive:");
                s.dst.create(hwnd, ID_DST, 16, 124, 500, 60);
                button(hwnd, ID_DST_REFRESH, 524, 124, 120, "Refresh");
                s.close = checkbox(hwnd, ID_CLOSE, 16, 196, 400, "Close the destination disc");
                set_checked(s.close, true);
                s.bar = progress_bar(hwnd, 16, 230, 628);
                s.status = label(
                    hwnd,
                    16,
                    254,
                    628,
                    "Ready. With the test drive on both sides this copies the simulated data disc.",
                );
                s.go = button(hwnd, ID_GO, 404, 300, 120, "Copy");
                s.cancel = button(hwnd, ID_CANCEL, 532, 300, 112, "Cancel");
                set_enabled(s.cancel, false);
                0
            }
            WM_COMMAND => {
                let s = st(hwnd);
                match ((wp & 0xFFFF) as u16, (wp >> 16) as u16) {
                    (ID_SRC_REFRESH, _) => {
                        s.src.refresh();
                        0
                    }
                    (ID_DST_REFRESH, _) => {
                        s.dst.refresh();
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
                        set_text(s.status, &text);
                        crate::win::gui::note_log(&text, false);
                    }
                    Some(Err(e)) => {
                        set_text(s.status, &format!("Failed: {e}"));
                        crate::win::gui::note_log(&format!("Copy disc failed: {e}"), true);
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
    let Some(src_info) = s.src.selected() else {
        set_text(s.status, "No source drive selected.");
        return;
    };
    let Some(dst_info) = s.dst.selected() else {
        set_text(s.status, "No destination drive selected.");
        return;
    };
    let close = is_checked(s.close);
    set_enabled(s.go, false);
    set_enabled(s.cancel, true);
    set_text(s.status, "Copying…");
    let shared = s.shared.clone();
    run_job(hwnd, &shared, move |sh| {
        let mut src: Box<dyn if_drive::OpticalDrive> = if src_info.id == "fake" {
            Box::new(crate::cli_rip::fake_data_drive(&[], &[])?)
        } else {
            if_drive::open(&src_info).map_err(|e| e.to_string())?
        };
        let mut dst: Box<dyn if_drive::OpticalDrive> = if dst_info.id == "fake" {
            Box::new(if_drive::fake_drive())
        } else {
            if_drive::open(&dst_info).map_err(|e| e.to_string())?
        };
        let tmp = std::env::temp_dir().join(format!("isofoundry-copy-{}.iso", std::process::id()));
        let rip_opts = if_drive::rip::RipImageOptions::default();
        let stage = if_drive::rip::rip_image(&mut *src, &tmp, &rip_opts, &sh.progress(), &sh.cancel);
        let result = match stage {
            Ok(rep) if rep.bad_sectors.is_empty() => {
                let burn_opts = if_drive::BurnOptions { verify: true, close_disc: close, ..Default::default() };
                dst.burn(&if_drive::BurnSource::Image(tmp.clone()), &burn_opts, &sh.progress(), &sh.cancel)
                    .map_err(|e| e.to_string())
            }
            Ok(rep) => Err(format!(
                "the source has {} unreadable sectors — use Recover files, or copy anyway from the CLI",
                rep.bad_sectors.len()
            )),
            Err(e) => Err(e.to_string()),
        };
        std::fs::remove_file(&tmp).ok();
        let rep = result?;
        Ok(format!(
            "Copied to {}: {} bytes{}.",
            dst_info.display_name(),
            rep.bytes_written,
            if rep.verified { ", verified" } else { "" }
        ))
    });
}
