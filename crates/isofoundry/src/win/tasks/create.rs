//! Create-image task window: pick files and folders, an output ISO, a
//! label and optional El Torito boot images; builds with the same
//! `if_image::write::build` call as the `create-image` CLI command.

use super::super::gui;
use super::super::taskwin::*;
use if_image::write::{build, BootImage, Options};
use std::path::PathBuf;
use std::sync::Arc;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "IsoFoundryTaskCreate";

const ID_LIST: u16 = 1;
const ID_ADD_FILES: u16 = 2;
const ID_ADD_FOLDER: u16 = 3;
const ID_REMOVE: u16 = 4;
const ID_OUT: u16 = 5;
const ID_OUT_BROWSE: u16 = 6;
const ID_LABEL: u16 = 7;
const ID_BIOS: u16 = 8;
const ID_BIOS_BROWSE: u16 = 9;
const ID_EFI: u16 = 10;
const ID_EFI_BROWSE: u16 = 11;
const ID_INFO_TABLE: u16 = 12;
const ID_JOLIET: u16 = 13;
const ID_UDF: u16 = 14;
const ID_START: u16 = 20;
const ID_CANCEL: u16 = 21;
const ID_OUTLOG: u16 = 30;

struct State {
    list: HWND,
    out: HWND,
    label_edit: HWND,
    bios: HWND,
    efi: HWND,
    info_table: HWND,
    joliet: HWND,
    udf: HWND,
    start: HWND,
    cancel: HWND,
    bar: HWND,
    status: HWND,
    outlog: HWND,
    sources: Vec<PathBuf>,
    shared: Option<Arc<JobShared>>,
    running: bool,
    close_when_done: bool,
}

pub fn open(owner: HWND) {
    register_class(CLASS, Some(wndproc));
    let state = Box::new(State {
        list: std::ptr::null_mut(),
        out: std::ptr::null_mut(),
        label_edit: std::ptr::null_mut(),
        bios: std::ptr::null_mut(),
        efi: std::ptr::null_mut(),
        info_table: std::ptr::null_mut(),
        joliet: std::ptr::null_mut(),
        udf: std::ptr::null_mut(),
        start: std::ptr::null_mut(),
        cancel: std::ptr::null_mut(),
        bar: std::ptr::null_mut(),
        status: std::ptr::null_mut(),
        outlog: std::ptr::null_mut(),
        sources: Vec::new(),
        shared: None,
        running: false,
        close_when_done: false,
    });
    let title = if_core::strings::task_title("create-image");
    create_window(owner, CLASS, title, 664, 566, Box::into_raw(state).cast());
}

fn st(hwnd: HWND) -> &'static mut State {
    // SAFETY: our pointer from WM_NCCREATE; window thread only.
    unsafe { &mut *state_ptr::<State>(hwnd) }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_NCCREATE => {
            attach::<State>(hwnd, lp);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_CREATE => {
            let s = st(hwnd);
            label(hwnd, 12, 14, 300, "Files and folders to put in the image:");
            s.list = listbox(hwnd, ID_LIST, 12, 36, 480, 148);
            button(hwnd, ID_ADD_FILES, 504, 36, 124, "Add files…");
            button(hwnd, ID_ADD_FOLDER, 504, 68, 124, "Add folder…");
            button(hwnd, ID_REMOVE, 504, 100, 124, "Remove");
            label(hwnd, 12, 197, 44, "Image:");
            s.out = edit(hwnd, ID_OUT, 56, 194, 452, "", false);
            button(hwnd, ID_OUT_BROWSE, 516, 193, 112, "Save as…");
            label(hwnd, 12, 229, 44, "Label:");
            s.label_edit = edit(hwnd, ID_LABEL, 56, 226, 240, "", false);
            label(hwnd, 12, 261, 190, "BIOS boot image (optional):");
            s.bios = edit(hwnd, ID_BIOS, 208, 258, 300, "", false);
            button(hwnd, ID_BIOS_BROWSE, 516, 257, 112, "Browse…");
            label(hwnd, 12, 293, 190, "UEFI boot image (optional):");
            s.efi = edit(hwnd, ID_EFI, 208, 290, 300, "", false);
            button(hwnd, ID_EFI_BROWSE, 516, 289, 112, "Browse…");
            s.info_table = checkbox(hwnd, ID_INFO_TABLE, 208, 322, 130, "Boot info table");
            s.joliet = checkbox(hwnd, ID_JOLIET, 356, 322, 80, "Joliet");
            s.udf = checkbox(hwnd, ID_UDF, 444, 322, 64, "UDF");
            set_checked(s.joliet, true);
            set_checked(s.udf, true);
            s.start = button(hwnd, ID_START, 392, 352, 112, "Create");
            s.cancel = button(hwnd, ID_CANCEL, 516, 352, 112, "Cancel");
            s.bar = progress_bar(hwnd, 12, 390, 616);
            s.status = label(hwnd, 12, 412, 616, "");
            s.outlog = multiline(hwnd, ID_OUTLOG, 12, 436, 616, 84);
            set_enabled(s.cancel, false);
            progress_show(s.bar, Some(0.0));
            0
        }
        WM_COMMAND => {
            let id = (wp & 0xFFFF) as u16;
            match id {
                ID_ADD_FILES => {
                    let s = st(hwnd);
                    for p in pick_open_multi(hwnd, "Add files", "All files|*.*") {
                        push_source(s, p);
                    }
                }
                ID_ADD_FOLDER => {
                    let s = st(hwnd);
                    if let Some(p) = pick_folder(hwnd, "Add folder") {
                        push_source(s, p);
                    }
                }
                ID_REMOVE => {
                    let s = st(hwnd);
                    let mut sel = list_selected(s.list);
                    sel.sort_unstable_by(|a, b| b.cmp(a));
                    for i in sel {
                        if i < s.sources.len() {
                            s.sources.remove(i);
                        }
                    }
                    refresh_list(s);
                }
                ID_OUT_BROWSE => {
                    let s = st(hwnd);
                    if let Some(p) = pick_save(hwnd, "Create image", "ISO images (*.iso)|*.iso", "iso", "image.iso") {
                        set_text(s.out, &p.to_string_lossy());
                    }
                }
                ID_BIOS_BROWSE => {
                    let s = st(hwnd);
                    if let Some(p) = pick_open(hwnd, "BIOS boot image", "Boot images|*.img;*.bin;*.iso|All files|*.*") {
                        set_text(s.bios, &p.to_string_lossy());
                    }
                }
                ID_EFI_BROWSE => {
                    let s = st(hwnd);
                    if let Some(p) = pick_open(hwnd, "UEFI boot image", "Boot images|*.img;*.bin;*.efi|All files|*.*") {
                        set_text(s.efi, &p.to_string_lossy());
                    }
                }
                ID_START => start(hwnd),
                ID_CANCEL => {
                    if let Some(sh) = &st(hwnd).shared {
                        sh.cancel.cancel();
                    }
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
            set_enabled(s.start, true);
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
                            gui::note_log("create-image: done", false);
                        }
                        Err(m) => {
                            append_line(s.outlog, &format!("Failed: {m}"));
                            set_text(s.status, "Failed.");
                            gui::note_log(&format!("create-image: {m}"), true);
                        }
                    }
                }
            }
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
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC | WM_CTLCOLORLISTBOX => ctl_color(wp),
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn push_source(s: &mut State, p: PathBuf) {
    if !s.sources.contains(&p) {
        s.sources.push(p);
        refresh_list(s);
    }
}

fn refresh_list(s: &mut State) {
    list_clear(s.list);
    for p in &s.sources {
        list_add(s.list, &p.to_string_lossy());
    }
}

fn start(hwnd: HWND) {
    let s = st(hwnd);
    if s.running {
        return;
    }
    if s.sources.is_empty() {
        append_line(s.outlog, "Add at least one file or folder first.");
        return;
    }
    let dest = PathBuf::from(get_text(s.out));
    if dest.as_os_str().is_empty() {
        append_line(s.outlog, "Choose where to save the image.");
        return;
    }
    let sources = s.sources.clone();
    // Options (and its boot images) hold a non-Send source type, so the
    // worker builds them from these plain parts on its own thread.
    let label_text = get_text(s.label_edit);
    let label = if !label_text.trim().is_empty() {
        label_text
    } else {
        match sources.as_slice() {
            [one] => one.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "CDROM".into()),
            _ => "CDROM".into(),
        }
    };
    let joliet = is_checked(s.joliet);
    let udf = is_checked(s.udf);
    let bios = get_text(s.bios);
    let efi = get_text(s.efi);
    let info_table = is_checked(s.info_table);
    let shared = JobShared::new();
    s.shared = Some(shared.clone());
    s.running = true;
    set_enabled(s.start, false);
    set_enabled(s.cancel, true);
    progress_show(s.bar, None);
    set_text(s.status, "Working…");
    run_job(hwnd, &shared, move |sh| {
        let mut opts = Options { label, joliet, udf, ..Options::default() };
        if !bios.trim().is_empty() {
            let mut img = BootImage::from_path(std::path::Path::new(bios.trim())).map_err(|e| e.to_string())?;
            img.info_table = info_table;
            opts.bios = Some(img);
        }
        if !efi.trim().is_empty() {
            opts.efi = Some(BootImage::from_path(std::path::Path::new(efi.trim())).map_err(|e| e.to_string())?);
        }
        let root = crate::cli_image::source_tree(&sources).map_err(|(_, m)| m)?;
        let sum = build(&root, opts, &dest, sh.cb()).map_err(|e| e.to_string())?;
        Ok(format!(
            "Built {} ({} bytes, {} files, {} folders)",
            dest.display(),
            sum.sectors * 2048,
            sum.files,
            sum.dirs
        ))
    });
}
