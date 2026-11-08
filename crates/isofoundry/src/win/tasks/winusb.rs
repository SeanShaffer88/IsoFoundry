//! Install Windows to USB window (M2): pick a Windows ISO, pick a
//! layout (MBR+FAT32 default, GPT+FAT32, GPT+NTFS for install images
//! over the FAT32 file limit), pick a target, confirm, write. All
//! boot files are Microsoft's own, copied unchanged from the ISO, so
//! Secure Boot keeps working.

use super::super::gui;
use super::super::taskwin::*;
use super::usbtools::*;
use if_core::ipc::UsbJobKind;
use std::sync::Arc;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "IsoFoundryTaskWinUsb";

const ID_IMAGE: u16 = 1;
const ID_BROWSE: u16 = 2;
const ID_LAY_MBR: u16 = 3;
const ID_LAY_GPT: u16 = 4;
const ID_LAY_NTFS: u16 = 5;
const ID_LIST: u16 = 6;
const ID_REFRESH: u16 = 7;
const ID_FILE: u16 = 8;
const ID_CONFIRM: u16 = 9;
const ID_WRITE: u16 = 10;
const ID_CANCEL: u16 = 11;
const ID_OUTLOG: u16 = 12;

const EN_CHANGE: u32 = 0x0300;
const LBN_SELCHANGE: u32 = 1;

struct State {
    image: HWND,
    lay_mbr: HWND,
    lay_gpt: HWND,
    lay_ntfs: HWND,
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
        lay_mbr: std::ptr::null_mut(),
        lay_gpt: std::ptr::null_mut(),
        lay_ntfs: std::ptr::null_mut(),
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
    let title = if_core::strings::task_title("install-windows-usb");
    create_window(owner, CLASS, title, 664, 510, Box::into_raw(state).cast());
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
    // The NTFS layout is formatted by the helper, so it needs a
    // physical disk; on a file target it is disabled (and unpicked).
    let file_target = matches!(choice, Some(Choice::File(_, _)));
    set_enabled(s.lay_ntfs, !file_target);
    if file_target && is_checked(s.lay_ntfs) {
        set_checked(s.lay_ntfs, false);
        set_checked(s.lay_mbr, true);
    }
}

fn start(hwnd: HWND) {
    let s = st(hwnd);
    if s.running {
        return;
    }
    let image = get_text(s.image);
    if image.trim().is_empty() {
        append_line(s.outlog, "Choose a Windows ISO first.");
        return;
    }
    let Some(choice) = s.pane.selection(s.list) else {
        append_line(s.outlog, "Pick a target disk (Refresh disks, or a disk-image file).");
        return;
    };
    let scheme = if is_checked(s.lay_ntfs) {
        "gpt-ntfs"
    } else if is_checked(s.lay_gpt) {
        "gpt"
    } else {
        "mbr"
    }
    .to_string();
    let kind = UsbJobKind::Files { image, scheme, windows_installer: true };
    let shared = JobShared::new();
    s.shared = Some(shared.clone());
    s.running = true;
    set_enabled(s.write, false);
    set_enabled(s.cancel, true);
    progress_show(s.bar, None);
    set_text(s.status, "Working…");
    append_line(s.outlog, "Writing the Windows installer… this erases the target.");
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
            label(hwnd, 12, 14, 90, "ISO:");
            s.image = edit(hwnd, ID_IMAGE, 108, 12, 420, "", false);
            button(hwnd, ID_BROWSE, 536, 11, 112, "Browse…");
            label(hwnd, 12, 48, 90, "Layout:");
            s.lay_mbr = radio(hwnd, ID_LAY_MBR, 108, 47, 150, "MBR + FAT32", true);
            s.lay_gpt = radio(hwnd, ID_LAY_GPT, 268, 47, 150, "GPT + FAT32", false);
            s.lay_ntfs = radio(hwnd, ID_LAY_NTFS, 428, 47, 224, "GPT + NTFS (big install.wim)", false);
            label(hwnd, 12, 78, 400, "Target (everything on it is erased):");
            s.list = listbox(hwnd, ID_LIST, 12, 100, 640, 92);
            button(hwnd, ID_REFRESH, 12, 200, 200, "Refresh disks (admin)…");
            button(hwnd, ID_FILE, 220, 200, 210, "Use disk-image file…");
            label(hwnd, 12, 234, 640, "This target has volumes or is over 64 GB. Type ERASE to allow writing:");
            s.confirm = edit(hwnd, ID_CONFIRM, 12, 256, 200, "", false);
            s.write = button(hwnd, ID_WRITE, 428, 254, 104, "Write");
            s.cancel = button(hwnd, ID_CANCEL, 540, 254, 104, "Cancel");
            s.bar = progress_bar(hwnd, 12, 290, 640);
            s.status = label(hwnd, 12, 312, 640, "");
            s.outlog = multiline(hwnd, ID_OUTLOG, 12, 336, 640, 120);
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
                    if let Some(p) = pick_open(hwnd, "Windows ISO", "ISO images|*.iso|All files|*.*") {
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
                ID_LIST | ID_CONFIRM | ID_IMAGE if code == LBN_SELCHANGE || code == EN_CHANGE => {
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
                            gui::note_log("install-windows-usb: done", false);
                        }
                        Err(m) => {
                            append_line(s.outlog, &format!("Failed: {m}"));
                            set_text(s.status, "Failed.");
                            gui::note_log(&format!("install-windows-usb: {m}"), true);
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
