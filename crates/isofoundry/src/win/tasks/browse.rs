//! Browse-image task window: open an image, see its files, extract all of
//! it or a selected file/folder. Same `Volume` calls as the `extract` and
//! `image-info` CLI commands.

use super::super::gui;
use super::super::taskwin::*;
use std::path::PathBuf;
use std::sync::Arc;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "IsoFoundryTaskBrowse";

const ID_PATH: u16 = 1;
const ID_OPEN: u16 = 2;
const ID_LIST: u16 = 4;
const ID_EXTRACT_ALL: u16 = 5;
const ID_EXTRACT_SEL: u16 = 6;

struct State {
    path_edit: HWND,
    info: HWND,
    list: HWND,
    extract_all: HWND,
    extract_sel: HWND,
    bar: HWND,
    status: HWND,
    image: Option<PathBuf>,
    /// (path inside the image, is folder, size) in list order.
    entries: Vec<(String, bool, u64)>,
    shared: Option<Arc<JobShared>>,
    running: bool,
    close_when_done: bool,
}

pub fn open(owner: HWND) {
    register_class(CLASS, Some(wndproc));
    let state = Box::new(State {
        path_edit: std::ptr::null_mut(),
        info: std::ptr::null_mut(),
        list: std::ptr::null_mut(),
        extract_all: std::ptr::null_mut(),
        extract_sel: std::ptr::null_mut(),
        bar: std::ptr::null_mut(),
        status: std::ptr::null_mut(),
        image: None,
        entries: Vec::new(),
        shared: None,
        running: false,
        close_when_done: false,
    });
    let title = if_core::strings::task_title("browse-image");
    create_window(owner, CLASS, title, 720, 512, Box::into_raw(state).cast());
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
            label(hwnd, 12, 17, 44, "Image:");
            s.path_edit = edit(hwnd, ID_PATH, 56, 14, 520, "", false);
            button(hwnd, ID_OPEN, 584, 13, 112, "Open…");
            s.info = label(hwnd, 12, 48, 684, "Open an image to browse it.");
            s.list = listbox(hwnd, ID_LIST, 12, 74, 684, 286);
            s.extract_all = button(hwnd, ID_EXTRACT_ALL, 12, 374, 140, "Extract all…");
            s.extract_sel = button(hwnd, ID_EXTRACT_SEL, 164, 374, 150, "Extract selected…");
            s.bar = progress_bar(hwnd, 12, 416, 684);
            s.status = label(hwnd, 12, 438, 684, "");
            set_enabled(s.extract_all, false);
            set_enabled(s.extract_sel, false);
            progress_show(s.bar, Some(0.0));
            0
        }
        WM_COMMAND => {
            let id = (wp & 0xFFFF) as u16;
            match id {
                ID_OPEN => {
                    if let Some(p) = pick_open(
                        hwnd,
                        "Open an image",
                        "Disc images|*.iso;*.cue;*.ccd;*.chd;*.nrg;*.mds;*.bin;*.img|All files|*.*",
                    ) {
                        load(hwnd, p);
                    }
                }
                ID_EXTRACT_ALL => extract(hwnd, None),
                ID_EXTRACT_SEL => {
                    let s = st(hwnd);
                    match list_selected(s.list).first().and_then(|&i| s.entries.get(i)) {
                        Some((path, _, _)) => {
                            let path = path.clone();
                            extract(hwnd, Some(path));
                        }
                        None => set_text(s.status, "Select a file or folder in the list first."),
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
            set_enabled(s.extract_all, true);
            set_enabled(s.extract_sel, true);
            progress_show(s.bar, Some(1.0));
            if let Some(sh) = &s.shared {
                if let Some(r) = sh.take_result() {
                    match r {
                        Ok(text) => {
                            set_text(s.status, &text);
                            gui::note_log(&format!("extract: {text}"), false);
                        }
                        Err(m) => {
                            set_text(s.status, &format!("Failed: {m}"));
                            gui::note_log(&format!("extract: {m}"), true);
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

fn load(hwnd: HWND, path: PathBuf) {
    let s = st(hwnd);
    set_text(s.path_edit, &path.to_string_lossy());
    set_text(s.status, "Loading…");
    let mut v = match crate::cli_image::open_volume(&path, None) {
        Ok(v) => v,
        Err((_, m)) => {
            set_text(s.info, &format!("Could not open: {m}"));
            set_text(s.status, "");
            return;
        }
    };
    let root = v.root();
    match v.walk(&root) {
        Ok(items) => {
            s.entries = items.iter().map(|(p, e)| (p.clone(), e.dir, e.size)).collect();
            list_clear(s.list);
            for (p, e) in &items {
                if e.dir {
                    list_add(s.list, &format!("{p}/"));
                } else {
                    list_add(s.list, &format!("{p}    ({})", fmt_bytes(e.size)));
                }
            }
            let format = if_image::container::open_image(&path).map(|(f, _)| f.name().to_string()).unwrap_or_default();
            let fss: Vec<&str> = v.found.iter().map(|k| k.name()).collect();
            let boot: Vec<String> = v
                .boot
                .as_ref()
                .map(|c| {
                    c.entries
                        .iter()
                        .filter(|e| e.bootable)
                        .map(|e| match e.platform {
                            if_image::eltorito::Platform::Bios => "BIOS".to_string(),
                            if_image::eltorito::Platform::Efi => "UEFI".to_string(),
                            if_image::eltorito::Platform::Other(x) => format!("platform 0x{x:02x}"),
                        })
                        .collect()
                })
                .unwrap_or_default();
            let files = s.entries.iter().filter(|(_, d, _)| !d).count();
            set_text(
                s.info,
                &format!(
                    "{} · label \u{201C}{}\u{201D} · {} · {} files · bootable: {}",
                    format,
                    v.fs.label(),
                    fss.join(", "),
                    files,
                    if boot.is_empty() { "no".to_string() } else { boot.join(", ") }
                ),
            );
            set_text(s.status, "");
            s.image = Some(path);
            set_enabled(s.extract_all, true);
            set_enabled(s.extract_sel, true);
        }
        Err(e) => {
            set_text(s.info, &format!("Could not list the image: {e}"));
            set_text(s.status, "");
        }
    }
}

/// `what`: `None` extracts everything; `Some(path)` one file or folder.
fn extract(hwnd: HWND, what: Option<String>) {
    let s = st(hwnd);
    if s.running {
        return;
    }
    let Some(image) = s.image.clone() else {
        return;
    };
    let Some(dest) = pick_folder(hwnd, "Extract to") else {
        return;
    };
    let shared = JobShared::new();
    s.shared = Some(shared.clone());
    s.running = true;
    set_enabled(s.extract_all, false);
    set_enabled(s.extract_sel, false);
    progress_show(s.bar, None);
    set_text(s.status, "Working…");
    run_job(hwnd, &shared, move |sh| {
        let mut v = crate::cli_image::open_volume(&image, None).map_err(|(_, m)| m)?;
        let entry = match &what {
            Some(p) => v.lookup(p).map_err(|e| e.to_string())?,
            None => v.root(),
        };
        let rep = v.extract(&entry, &dest, sh.cb()).map_err(|e| e.to_string())?;
        let mut text = format!(
            "Extracted {} files and {} folders ({}) to {}",
            rep.files,
            rep.folders,
            fmt_bytes(rep.bytes),
            dest.display()
        );
        for (from, to) in &rep.renamed {
            text.push_str(&format!(" · renamed for Windows: {from} -> {to}"));
        }
        Ok(text)
    });
}
