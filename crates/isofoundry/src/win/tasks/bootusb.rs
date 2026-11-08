//! Create bootable USB window (M2): pick an ISO, pick files mode
//! (partition + FAT32 + copy the tree) or raw mode (byte-for-byte),
//! pick a target disk, confirm, write. The typed ERASE confirmation
//! appears when the target has mounted volumes or is over 64 GB
//! (ARCHITECTURE.md §7); the safety assessment runs again inside the
//! helper right before the write.

use super::super::gui;
use super::super::taskwin::*;
use super::usbtools::*;
use if_core::ipc::UsbJobKind;
use std::sync::Arc;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "IsoFoundryTaskBootUsb";

const ID_IMAGE: u16 = 1;
const ID_BROWSE: u16 = 2;
const ID_MODE_FILES: u16 = 3;
const ID_MODE_RAW: u16 = 4;
const ID_SCH_MBR: u16 = 5;
const ID_SCH_GPT: u16 = 6;
const ID_VERIFY: u16 = 7;
const ID_LIST: u16 = 8;
const ID_REFRESH: u16 = 9;
const ID_FILE: u16 = 10;
const ID_CONFIRM: u16 = 11;
const ID_WRITE: u16 = 12;
const ID_CANCEL: u16 = 13;
const ID_OUTLOG: u16 = 14;

const EN_CHANGE: u32 = 0x0300;
const LBN_SELCHANGE: u32 = 1;

struct State {
    image: HWND,
    mode_files: HWND,
    sch_mbr: HWND,
    sch_gpt: HWND,
    verify: HWND,
    list: HWND,
    confirm: HWND,
    write: HWND,
    cancel: HWND,
    bar: HWND,
    status: HWND,
    outlog: HWND,
    pane: UsbPane,
    shared: Option<Arc<JobShared>>,
    running: bool,
    close_when_done: bool,
}

pub fn open(owner: HWND) {
    register_class(CLASS, Some(wndproc));
    let state = Box::new(State {
        image: std::ptr::null_mut(),
        mode_files: std::ptr::null_mut(),
        sch_mbr: std::ptr::null_mut(),
        sch_gpt: std::ptr::null_mut(),
        verify: std::ptr::null_mut(),
        list: std::ptr::null_mut(),
        confirm: std::ptr::null_mut(),
        write: std::ptr::null_mut(),
        cancel: std::ptr::null_mut(),
        bar: std::ptr::null_mut(),
        status: std::ptr::null_mut(),
        outlog: std::ptr::null_mut(),
        pane: UsbPane::default(),
        shared: None,
        running: false,
        close_when_done: false,
    });
    let title = if_core::strings::task_title("bootable-usb");
    create_window(owner, CLASS, title, 664, 540, Box::into_raw(state).cast());
}

fn st(hwnd: HWND) -> &'static mut State {
    // SAFETY: our pointer from WM_NCCREATE; window thread only.
    unsafe { &mut *state_ptr::<State>(hwnd) }
}

fn refresh_enabled(s: &mut State) {
    if s.running {
        return;
    }
    let has_image = !get_text(s.image).trim().is_empty();
    let choice = s.pane.selection(s.list);
    let need_confirm = choice.as_ref().map(confirm_required).unwrap_or(false);
    set_enabled(s.confirm, need_confirm);
    let confirmed = !need_confirm || get_text(s.confirm).trim() == "ERASE";
    set_enabled(s.write, has_image && choice.is_some() && confirmed);
    let files_mode = is_checked(s.mode_files);
    set_enabled(s.sch_mbr, files_mode);
    set_enabled(s.sch_gpt, files_mode);
    set_enabled(s.verify, !files_mode);
}

fn start(hwnd: HWND) {
    let s = st(hwnd);
    if s.running {
        return;
    }
    let image = get_text(s.image);
    if image.trim().is_empty() {
        append_line(s.outlog, "Choose an image first.");
        return;
    }
    let Some(choice) = s.pane.selection(s.list) else {
        append_line(s.outlog, "Pick a target disk (Refresh disks, or a disk-image file).");
        return;
    };
    let kind = if is_checked(s.mode_files) {
        let scheme = if is_checked(s.sch_gpt) { "gpt" } else { "mbr" }.to_string();
        UsbJobKind::Files { image, scheme, windows_installer: false }
    } else {
        UsbJobKind::RawWrite { image, verify: is_checked(s.verify) }
    };
    let shared = JobShared::new();
    s.shared = Some(shared.clone());
    s.running = true;
    set_enabled(s.write, false);
    set_enabled(s.cancel, true);
    progress_show(s.bar, None);
    set_text(s.status, "Working…");
    append_line(s.outlog, "Writing… this erases the target.");
    run_job(hwnd, &shared, move |sh| {
        let (files, bytes, notes) = run_choice(&choice, kind, &sh)?;
        let mut msg = format!("Done: {files} files, {bytes} bytes written.");
        for n in notes {
            msg.push_str(&format!("\nNote: {n}"));
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
            label(hwnd, 12, 14, 90, "Image:");
            s.image = edit(hwnd, ID_IMAGE, 108, 12, 420, "", false);
            button(hwnd, ID_BROWSE, 536, 11, 112, "Browse…");
            label(hwnd, 12, 48, 90, "Mode:");
            s.mode_files = radio(hwnd, ID_MODE_FILES, 108, 47, 170, "Copy files", true);
            radio(hwnd, ID_MODE_RAW, 290, 47, 170, "Raw copy", false);
            label(hwnd, 12, 76, 90, "Scheme:");
            s.sch_mbr = radio(hwnd, ID_SCH_MBR, 108, 75, 90, "MBR", true);
            s.sch_gpt = radio(hwnd, ID_SCH_GPT, 210, 75, 90, "GPT", false);
            s.verify = checkbox(hwnd, ID_VERIFY, 330, 75, 300, "Verify after write (raw mode)");
            label(hwnd, 12, 106, 400, "Target (everything on it is erased):");
            s.list = listbox(hwnd, ID_LIST, 12, 128, 640, 92);
            button(hwnd, ID_REFRESH, 12, 228, 200, "Refresh disks (admin)…");
            button(hwnd, ID_FILE, 220, 228, 210, "Use disk-image file…");
            label(hwnd, 12, 262, 640, "This target has volumes or is over 64 GB. Type ERASE to allow writing:");
            s.confirm = edit(hwnd, ID_CONFIRM, 12, 284, 200, "", false);
            s.write = button(hwnd, ID_WRITE, 428, 282, 104, "Write");
            s.cancel = button(hwnd, ID_CANCEL, 540, 282, 104, "Cancel");
            s.bar = progress_bar(hwnd, 12, 318, 640);
            s.status = label(hwnd, 12, 340, 640, "");
            s.outlog = multiline(hwnd, ID_OUTLOG, 12, 364, 640, 120);
            set_enabled(s.cancel, false);
            set_enabled(s.confirm, false);
            set_enabled(s.write, false);
            progress_show(s.bar, Some(0.0));
            0
        }
        WM_COMMAND => {
            let id = (wp & 0xFFFF) as u16;
            let code = (wp >> 16) as u32;
            match id {
                ID_BROWSE => {
                    let s = st(hwnd);
                    if let Some(p) = pick_open(hwnd, "Disc image", "Images|*.iso;*.img;*.bin;*.chd|All files|*.*") {
                        set_text(s.image, &p.to_string_lossy());
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
                        Err(m) => append_line(st(hwnd).outlog, &format!("Could not list disks: {m}")),
                    }
                    refresh_enabled(st(hwnd));
                }
                ID_FILE => {
                    if let Some(p) = pick_open(hwnd, "Disk image file", "Disk images|*.img;*.vhdx;*.vhd|All files|*.*")
                    {
                        let path = p.to_string_lossy().into_owned();
                        match file_info(&path) {
                            Ok(info) => {
                                let s = st(hwnd);
                                let idx = s.pane.disks.len();
                                s.pane.file = Some((path, info));
                                s.pane.fill(s.list);
                                unsafe { select_row(s.list, idx) };
                            }
                            Err(m) => append_line(st(hwnd).outlog, &m),
                        }
                    }
                    refresh_enabled(st(hwnd));
                }
                ID_WRITE => start(hwnd),
                ID_CANCEL => {
                    if let Some(sh) = &st(hwnd).shared {
                        sh.cancel.cancel();
                    }
                }
                ID_LIST | ID_CONFIRM | ID_IMAGE | ID_MODE_FILES | ID_MODE_RAW
                    if code == LBN_SELCHANGE || code == EN_CHANGE || id == ID_MODE_FILES || id == ID_MODE_RAW =>
                {
                    refresh_enabled(st(hwnd));
                }
                _ => {}
            }
            0
        }
        WM_TIMER if wp == TIMER_JOB => {
            let s = st(hwnd);
            if let Some(sh) = &s.shared {
                let (done, total) = sh.snapshot();
                progress_show(s.bar, sh.fraction());
                set_text(s.status, &progress_text(done, total));
            }
            0
        }
        WM_JOB_DONE => {
            stop_timer(hwnd);
            let s = st(hwnd);
            s.running = false;
            set_enabled(s.cancel, false);
            progress_show(s.bar, Some(1.0));
            if let Some(sh) = &s.shared {
                if let Some(r) = sh.take_result() {
                    match r {
                        Ok(text) => {
                            for line in text.lines() {
                                append_line(s.outlog, line);
                            }
                            set_text(s.status, "Done.");
                            gui::note_log("bootable-usb: done", false);
                        }
                        Err(m) => {
                            append_line(s.outlog, &format!("Failed: {m}"));
                            set_text(s.status, "Failed.");
                            gui::note_log(&format!("bootable-usb: {m}"), true);
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
                if let Some(sh) = &s.shared {
                    sh.cancel.cancel();
                }
                s.close_when_done = true;
                set_text(s.status, "Cancelling…");
                0
            } else {
                // SAFETY: closing our own window.
                unsafe { DestroyWindow(hwnd) };
                0
            }
        }
        WM_NCDESTROY => {
            let p = state_ptr::<State>(hwnd);
            if !p.is_null() {
                // SAFETY: created with Box::into_raw in `open`.
                unsafe { drop(Box::from_raw(p)) };
            }
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN => ctl_color(wp),
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
