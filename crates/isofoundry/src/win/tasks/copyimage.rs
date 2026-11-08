//! Copy disc to image window (M5) — the preservation dumping
//! surface: pick a source drive, choose raw / subchannel, rip to
//! an image with its CUE, dump report and hash file sidecars.

use super::super::taskwin::*;
use super::drivepick::DrivePane;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "IsoFoundryTaskCopyImage";
const ID_DEST: u16 = 1;
const ID_BROWSE: u16 = 2;
const ID_DRIVES: u16 = 3;
const ID_REFRESH: u16 = 4;
const ID_RAW: u16 = 5;
const ID_SUB: u16 = 6;
const ID_GO: u16 = 7;
const ID_CANCEL: u16 = 8;

struct State {
    dest: HWND,
    drives: DrivePane,
    media: HWND,
    raw: HWND,
    sub: HWND,
    status: HWND,
    bar: HWND,
    go: HWND,
    cancel: HWND,
    shared: std::sync::Arc<JobShared>,
}

pub fn open(owner: HWND) {
    register_class(CLASS, Some(wndproc));
    let state = Box::new(State {
        dest: std::ptr::null_mut(),
        drives: DrivePane::empty(),
        media: std::ptr::null_mut(),
        raw: std::ptr::null_mut(),
        sub: std::ptr::null_mut(),
        status: std::ptr::null_mut(),
        bar: std::ptr::null_mut(),
        go: std::ptr::null_mut(),
        cancel: std::ptr::null_mut(),
        shared: JobShared::new(),
    });
    create_window(owner, CLASS, "Copy disc to image", 664, 400, Box::into_raw(state).cast());
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
                s.drives.create(hwnd, ID_DRIVES, 16, 34, 500, 66);
                button(hwnd, ID_REFRESH, 524, 34, 120, "Refresh");
                s.media = label(hwnd, 16, 106, 628, "Media: —");
                set_text(s.media, &format!("Media: {}", s.drives.media_summary()));
                label(hwnd, 16, 134, 200, "Image file:");
                s.dest = edit(hwnd, ID_DEST, 16, 154, 500, "", false);
                button(hwnd, ID_BROWSE, 524, 152, 120, "Browse…");
                s.raw = checkbox(hwnd, ID_RAW, 16, 186, 300, "Raw sectors (BIN + CUE)");
                s.sub = checkbox(hwnd, ID_SUB, 332, 186, 300, "Include subchannel data");
                s.bar = progress_bar(hwnd, 16, 222, 628);
                s.status = label(hwnd, 16, 246, 628, "Ready. The test drive holds a small data disc; rips also write a dump report and a hash file next to the image.");
                s.go = button(hwnd, ID_GO, 404, 292, 120, "Rip");
                s.cancel = button(hwnd, ID_CANCEL, 532, 292, 112, "Cancel");
                set_enabled(s.cancel, false);
                0
            }
            WM_COMMAND => {
                let s = st(hwnd);
                match ((wp & 0xFFFF) as u16, (wp >> 16) as u16) {
                    (ID_BROWSE, _) => {
                        if let Some(p) = pick_save(
                            hwnd,
                            "Save disc image",
                            "ISO image|*.iso|Raw image|*.bin|All files|*.*",
                            "iso",
                            "",
                        ) {
                            set_text(s.dest, &p.to_string_lossy());
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
                        set_text(s.status, &text);
                        crate::win::gui::note_log(&text, false);
                    }
                    Some(Err(e)) => {
                        set_text(s.status, &format!("Failed: {e}"));
                        crate::win::gui::note_log(&format!("Copy to image failed: {e}"), true);
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
    let dest = get_text(s.dest);
    if dest.is_empty() {
        set_text(s.status, "Choose an image file first.");
        return;
    }
    let Some(info) = s.drives.selected() else {
        set_text(s.status, "No drive selected.");
        return;
    };
    let raw = is_checked(s.raw);
    let sub = is_checked(s.sub);
    let dest = PathBuf::from(dest);
    set_enabled(s.go, false);
    set_enabled(s.cancel, true);
    set_text(s.status, "Ripping…");
    let shared = s.shared.clone();
    run_job(hwnd, &shared, move |sh| {
        let mut drive: Box<dyn if_drive::OpticalDrive> = if info.id == "fake" {
            Box::new(crate::cli_rip::fake_data_drive(&[], &[])?)
        } else {
            if_drive::open(&info).map_err(|e| e.to_string())?
        };
        let desc = info.display_name();
        let toc = drive.toc().map_err(|e| e.to_string())?;
        let opts = if_drive::rip::RipImageOptions { raw, subchannel: sub, retries: 3 };
        let report = if_drive::rip::rip_image(&mut *drive, &dest, &opts, &sh.progress(), &sh.cancel)
            .map_err(|e| e.to_string())?;
        let sidecars = crate::cli_rip::write_sidecars(&dest, &toc, &desc, &report, raw);
        let mut msg = format!("Ripped {} sectors to {}.", report.sectors, dest.display());
        if !report.bad_sectors.is_empty() {
            msg.push_str(&format!(
                " {} sectors were unreadable (zero-filled, listed in the dump report).",
                report.bad_sectors.len()
            ));
        }
        if !sidecars.is_empty() {
            msg.push_str(&format!(" Sidecars: {}", sidecars.join(", ")));
        }
        Ok(msg)
    });
}
