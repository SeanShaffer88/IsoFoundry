//! Save a USB drive to an image file (M2). The source is read-only;
//! physical disks are read through the elevated helper, while disk-image
//! files use the same in-process route as the CLI.

use super::super::gui;
use super::super::taskwin::*;
use super::usbtools::*;
use if_core::ipc::UsbJobKind;
use std::sync::Arc;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "IsoFoundryTaskUsbRead";

const ID_OUTPUT: u16 = 1;
const ID_BROWSE: u16 = 2;
const ID_LIMIT: u16 = 3;
const ID_LIST: u16 = 4;
const ID_REFRESH: u16 = 5;
const ID_FILE: u16 = 6;
const ID_READ: u16 = 7;
const ID_CANCEL: u16 = 8;
const ID_OUTLOG: u16 = 9;

const EN_CHANGE: u32 = 0x0300;
const LBN_SELCHANGE: u32 = 1;

struct State {
    output: HWND,
    limit: HWND,
    list: HWND,
    read: HWND,
    cancel: HWND,
    bar: HWND,
    status: HWND,
    outlog: HWND,
    pane: UsbPane,
    shared: Option<Arc<JobShared>>,
    running: bool,
    cancellable: bool,
    close_when_done: bool,
}

pub fn open(owner: HWND) {
    register_class(CLASS, Some(wndproc));
    let state = Box::new(State {
        output: std::ptr::null_mut(),
        limit: std::ptr::null_mut(),
        list: std::ptr::null_mut(),
        read: std::ptr::null_mut(),
        cancel: std::ptr::null_mut(),
        bar: std::ptr::null_mut(),
        status: std::ptr::null_mut(),
        outlog: std::ptr::null_mut(),
        pane: UsbPane::default(),
        shared: None,
        running: false,
        cancellable: false,
        close_when_done: false,
    });
    create_window(owner, CLASS, "Save USB drive to image file", 664, 500, Box::into_raw(state).cast());
}

fn st(hwnd: HWND) -> &'static mut State {
    // SAFETY: our pointer from WM_NCCREATE; window thread only.
    unsafe { &mut *state_ptr::<State>(hwnd) }
}

fn parsed_limit(text: &str) -> Result<Option<u64>, &'static str> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    let bytes = text.parse::<u64>().map_err(|_| "Read limit must be a positive number of bytes.")?;
    if bytes == 0 {
        return Err("Read limit must be greater than zero.");
    }
    Ok(Some(bytes))
}

fn refresh_enabled(s: &mut State) {
    if s.running {
        return;
    }
    let has_output = !get_text(s.output).trim().is_empty();
    let has_source = s.pane.selection(s.list).is_some();
    let valid_limit = parsed_limit(&get_text(s.limit)).is_ok();
    set_enabled(s.read, has_output && has_source && valid_limit);
}

fn start(hwnd: HWND) {
    let s = st(hwnd);
    if s.running {
        return;
    }
    let output = get_text(s.output);
    if output.trim().is_empty() {
        append_line(s.outlog, "Choose an output image file first.");
        return;
    }
    let limit_bytes = match parsed_limit(&get_text(s.limit)) {
        Ok(limit) => limit,
        Err(message) => {
            append_line(s.outlog, message);
            return;
        }
    };
    let Some(choice) = s.pane.selection(s.list) else {
        append_line(s.outlog, "Pick a source disk (Refresh disks, or a disk-image file).");
        return;
    };

    let cancellable = matches!(&choice, Choice::File(_, _));
    let kind = UsbJobKind::ReadImage { out: output, limit_bytes };
    let shared = JobShared::new();
    s.shared = Some(shared.clone());
    s.running = true;
    s.cancellable = cancellable;
    set_enabled(s.read, false);
    set_enabled(s.cancel, cancellable);
    progress_show(s.bar, None);
    set_text(
        s.status,
        if cancellable {
            "Reading disk-image file…"
        } else {
            "Reading physical disk… cancellation is unavailable while the helper is running."
        },
    );
    append_line(s.outlog, "Reading source; its contents will not be changed.");
    run_job(hwnd, &shared, move |sh| {
        let (_, bytes, notes) = run_choice(&choice, kind, &sh)?;
        let mut msg = format!("Done: {bytes} bytes saved.");
        for note in notes {
            msg.push_str(&format!("\nNote: {note}"));
        }
        Ok(msg)
    });
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_NCCREATE => {
            attach::<State>(hwnd, lp);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_CREATE => {
            let s = st(hwnd);
            label(hwnd, 12, 14, 640, "Source disk (read-only):");
            s.list = listbox(hwnd, ID_LIST, 12, 36, 640, 92);
            button(hwnd, ID_REFRESH, 12, 136, 200, "Refresh disks (admin)…");
            button(hwnd, ID_FILE, 220, 136, 230, "Read from disk-image file…");
            label(hwnd, 12, 172, 90, "Output image:");
            s.output = edit(hwnd, ID_OUTPUT, 108, 170, 420, "", false);
            button(hwnd, ID_BROWSE, 536, 169, 112, "Browse…");
            label(hwnd, 12, 206, 640, "Read limit in bytes (optional; blank reads the whole source):");
            s.limit = edit(hwnd, ID_LIMIT, 12, 228, 220, "", false);
            label(hwnd, 244, 230, 400, "Leave blank to create a full-size image.");
            s.bar = progress_bar(hwnd, 12, 270, 640);
            s.status = label(
                hwnd,
                12,
                292,
                640,
                "The source is read-only. Disk-image reads can be cancelled; physical-disk reads run to completion.",
            );
            s.outlog = multiline(hwnd, ID_OUTLOG, 12, 318, 640, 96);
            s.read = button(hwnd, ID_READ, 428, 426, 104, "Read");
            s.cancel = button(hwnd, ID_CANCEL, 540, 426, 108, "Cancel");
            set_enabled(s.cancel, false);
            set_enabled(s.read, false);
            progress_show(s.bar, Some(0.0));
            0
        }
        WM_COMMAND => {
            let id = (wp & 0xFFFF) as u16;
            let code = (wp >> 16) as u32;
            match id {
                ID_BROWSE => {
                    if let Some(path) = pick_save(
                        hwnd,
                        "Save USB image",
                        "Disk images|*.img;*.bin|All files|*.*",
                        "img",
                        "usb-backup.img",
                    ) {
                        set_text(st(hwnd).output, &path.to_string_lossy());
                    }
                    refresh_enabled(st(hwnd));
                }
                ID_REFRESH => {
                    let outlog = st(hwnd).outlog;
                    append_line(outlog, "Asking the helper for the disk list (approve the admin prompt)…");
                    match refresh_disks() {
                        Ok(disks) => {
                            let s = st(hwnd);
                            s.pane.disks = disks;
                            s.pane.fill(s.list);
                            append_line(s.outlog, "Disk list updated.");
                        }
                        Err(message) => append_line(st(hwnd).outlog, &format!("Could not list disks: {message}")),
                    }
                    refresh_enabled(st(hwnd));
                }
                ID_FILE => {
                    if let Some(path) =
                        pick_open(hwnd, "Source disk image", "Disk images|*.img;*.vhdx;*.vhd|All files|*.*")
                    {
                        let path = path.to_string_lossy().into_owned();
                        match file_info(&path) {
                            Ok(info) => {
                                let s = st(hwnd);
                                let index = s.pane.disks.len();
                                s.pane.file = Some((path, info));
                                s.pane.fill(s.list);
                                // SAFETY: our own list box on the window thread.
                                unsafe { select_row(s.list, index) };
                            }
                            Err(message) => append_line(st(hwnd).outlog, &message),
                        }
                    }
                    refresh_enabled(st(hwnd));
                }
                ID_READ => start(hwnd),
                ID_CANCEL => {
                    if let Some(shared) = &st(hwnd).shared {
                        shared.cancel.cancel();
                    }
                }
                ID_LIST | ID_OUTPUT | ID_LIMIT if code == LBN_SELCHANGE || code == EN_CHANGE => {
                    refresh_enabled(st(hwnd));
                }
                _ => {}
            }
            0
        }
        WM_TIMER if wp == TIMER_JOB => {
            let s = st(hwnd);
            if let Some(shared) = &s.shared {
                let (done, total) = shared.snapshot();
                progress_show(s.bar, shared.fraction());
                set_text(s.status, &progress_text(done, total));
            }
            0
        }
        WM_JOB_DONE => {
            stop_timer(hwnd);
            let s = st(hwnd);
            s.running = false;
            s.cancellable = false;
            set_enabled(s.cancel, false);
            progress_show(s.bar, Some(1.0));
            if let Some(shared) = &s.shared {
                if let Some(result) = shared.take_result() {
                    match result {
                        Ok(text) => {
                            for line in text.lines() {
                                append_line(s.outlog, line);
                            }
                            set_text(s.status, "Done.");
                            gui::note_log("usb-read-image: done", false);
                        }
                        Err(message) => {
                            append_line(s.outlog, &format!("Failed: {message}"));
                            set_text(s.status, "Failed.");
                            gui::note_log(&format!("usb-read-image: {message}"), true);
                        }
                    }
                }
            }
            refresh_enabled(s);
            if s.close_when_done {
                // SAFETY: closing our own window after the job ended.
                unsafe { DestroyWindow(hwnd) };
                return 0;
            }
            0
        }
        WM_CLOSE => {
            let s = st(hwnd);
            if s.running {
                if s.cancellable {
                    if let Some(shared) = &s.shared {
                        shared.cancel.cancel();
                    }
                    set_text(s.status, "Cancelling…");
                } else {
                    set_text(s.status, "Waiting for the physical disk read to finish before closing…");
                }
                s.close_when_done = true;
                0
            } else {
                // SAFETY: closing our own window.
                unsafe { DestroyWindow(hwnd) };
                0
            }
        }
        WM_NCDESTROY => {
            let pointer = state_ptr::<State>(hwnd);
            if !pointer.is_null() {
                // SAFETY: created with Box::into_raw in `open`.
                unsafe { drop(Box::from_raw(pointer)) };
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN => ctl_color(wp),
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
