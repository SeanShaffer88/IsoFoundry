//! Edit-image task window: open an image, stage changes (add, remove,
//! rename, new folder, new label) and save the result as a new image. The
//! save runs the same `if_image::edit::edit` call as the `edit-image` CLI
//! command; the original is never modified.

use super::super::gui;
use super::super::taskwin::*;
use if_image::edit::Edit;
use std::path::PathBuf;
use std::sync::Arc;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "IsoFoundryTaskEdit";

const ID_PATH: u16 = 1;
const ID_OPEN: u16 = 2;
const ID_LIST: u16 = 3;
const ID_ADD_FILE: u16 = 5;
const ID_ADD_FOLDER: u16 = 6;
const ID_REMOVE: u16 = 7;
const ID_NEWNAME: u16 = 8;
const ID_MKDIR: u16 = 9;
const ID_RENAME: u16 = 10;
const ID_RENAME_BTN: u16 = 11;
const ID_LABEL: u16 = 12;
const ID_LABEL_BTN: u16 = 13;
const ID_OPS: u16 = 14;
const ID_SAVE: u16 = 20;
const ID_CANCEL: u16 = 21;

struct State {
    path_edit: HWND,
    list: HWND,
    newname: HWND,
    rename: HWND,
    label_edit: HWND,
    ops_list: HWND,
    save: HWND,
    cancel: HWND,
    bar: HWND,
    status: HWND,
    image: Option<PathBuf>,
    /// (path inside the image, is folder) in list order.
    entries: Vec<(String, bool)>,
    ops: Vec<Edit>,
    shared: Option<Arc<JobShared>>,
    running: bool,
    close_when_done: bool,
}

pub fn open(owner: HWND) {
    register_class(CLASS, Some(wndproc));
    let state = Box::new(State {
        path_edit: std::ptr::null_mut(),
        list: std::ptr::null_mut(),
        newname: std::ptr::null_mut(),
        rename: std::ptr::null_mut(),
        label_edit: std::ptr::null_mut(),
        ops_list: std::ptr::null_mut(),
        save: std::ptr::null_mut(),
        cancel: std::ptr::null_mut(),
        bar: std::ptr::null_mut(),
        status: std::ptr::null_mut(),
        image: None,
        entries: Vec::new(),
        ops: Vec::new(),
        shared: None,
        running: false,
        close_when_done: false,
    });
    let title = if_core::strings::task_title("edit-image");
    create_window(owner, CLASS, title, 720, 566, Box::into_raw(state).cast());
}

fn st(hwnd: HWND) -> &'static mut State {
    // SAFETY: our pointer from WM_NCCREATE; window thread only.
    unsafe { &mut *state_ptr::<State>(hwnd) }
}

/// The folder changes apply to: the selected folder, else the folder of
/// the selected file, else the root ("").
fn target_dir(s: &State) -> String {
    match list_selected(s.list).first().and_then(|&i| s.entries.get(i)) {
        Some((p, true)) => p.clone(),
        Some((p, false)) => p.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default(),
        None => String::new(),
    }
}

fn selected_path(s: &State) -> Option<String> {
    list_selected(s.list).first().and_then(|&i| s.entries.get(i)).map(|(p, _)| p.clone())
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
            label(hwnd, 12, 46, 420, "Contents (as opened):");
            s.list = listbox(hwnd, ID_LIST, 12, 68, 420, 218);
            button(hwnd, ID_ADD_FILE, 444, 68, 252, "Add file…");
            button(hwnd, ID_ADD_FOLDER, 444, 100, 252, "Add folder…");
            button(hwnd, ID_REMOVE, 444, 132, 252, "Remove selected");
            label(hwnd, 444, 172, 252, "New folder name:");
            s.newname = edit(hwnd, ID_NEWNAME, 444, 194, 180, "", false);
            button(hwnd, ID_MKDIR, 632, 193, 64, "Add");
            label(hwnd, 444, 228, 252, "Rename selected to:");
            s.rename = edit(hwnd, ID_RENAME, 444, 250, 180, "", false);
            button(hwnd, ID_RENAME_BTN, 632, 249, 64, "Apply");
            label(hwnd, 12, 302, 44, "Label:");
            s.label_edit = edit(hwnd, ID_LABEL, 56, 299, 300, "", false);
            button(hwnd, ID_LABEL_BTN, 364, 298, 68, "Set");
            label(hwnd, 12, 338, 420, "Changes to apply:");
            s.ops_list = listbox(hwnd, ID_OPS, 12, 360, 420, 84);
            s.save = button(hwnd, ID_SAVE, 444, 360, 120, "Save as…");
            s.cancel = button(hwnd, ID_CANCEL, 576, 360, 120, "Cancel job");
            s.bar = progress_bar(hwnd, 12, 458, 684);
            s.status = label(hwnd, 12, 480, 684, "Open an image to edit it. The original is never changed.");
            set_enabled(s.cancel, false);
            progress_show(s.bar, Some(0.0));
            0
        }
        WM_COMMAND => {
            let id = (wp & 0xFFFF) as u16;
            match id {
                ID_OPEN => {
                    if let Some(p) = pick_open(
                        hwnd,
                        "Open an image to edit",
                        "Disc images|*.iso;*.cue;*.ccd;*.chd;*.nrg;*.mds;*.bin;*.img|All files|*.*",
                    ) {
                        load(hwnd, p);
                    }
                }
                ID_ADD_FILE => {
                    let s = st(hwnd);
                    if s.image.is_none() {
                        return 0;
                    }
                    let dir = target_dir(s);
                    for from in pick_open_multi(hwnd, "Add files", "All files|*.*") {
                        stage(
                            s,
                            Edit::Add { dir: dir.clone(), from: from.clone() },
                            &format!("Add {} to /{dir}", from.display()),
                        );
                    }
                }
                ID_ADD_FOLDER => {
                    let s = st(hwnd);
                    if s.image.is_none() {
                        return 0;
                    }
                    let dir = target_dir(s);
                    if let Some(from) = pick_folder(hwnd, "Add folder") {
                        stage(
                            s,
                            Edit::Add { dir: dir.clone(), from: from.clone() },
                            &format!("Add folder {} to /{dir}", from.display()),
                        );
                    }
                }
                ID_REMOVE => {
                    let s = st(hwnd);
                    if let Some(p) = selected_path(s) {
                        stage(s, Edit::Remove(p.clone()), &format!("Remove /{p}"));
                    } else {
                        set_text(s.status, "Select a file or folder in the contents first.");
                    }
                }
                ID_MKDIR => {
                    let s = st(hwnd);
                    let name = get_text(s.newname);
                    let name = name.trim().trim_matches('/');
                    if name.is_empty() {
                        set_text(s.status, "Type the new folder's name first.");
                    } else {
                        let dir = target_dir(s);
                        let path = if dir.is_empty() { name.to_string() } else { format!("{dir}/{name}") };
                        stage(s, Edit::MkDir(path.clone()), &format!("New folder /{path}"));
                    }
                }
                ID_RENAME_BTN => {
                    let s = st(hwnd);
                    let to = get_text(s.rename);
                    let to = to.trim().to_string();
                    match (selected_path(s), to.is_empty()) {
                        (Some(p), false) => {
                            stage(s, Edit::Rename { path: p.clone(), to: to.clone() }, &format!("Rename /{p} to {to}"))
                        }
                        (None, _) => set_text(s.status, "Select a file or folder in the contents first."),
                        (_, true) => set_text(s.status, "Type the new name first."),
                    }
                }
                ID_LABEL_BTN => {
                    let s = st(hwnd);
                    let l = get_text(s.label_edit);
                    if !l.trim().is_empty() {
                        stage(s, Edit::Label(l.clone()), &format!("Set label to \u{201C}{l}\u{201D}"));
                    }
                }
                ID_SAVE => save(hwnd),
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
            set_enabled(s.save, true);
            set_enabled(s.cancel, false);
            progress_show(s.bar, Some(1.0));
            if let Some(sh) = &s.shared {
                if let Some(r) = sh.take_result() {
                    match r {
                        Ok(text) => {
                            set_text(s.status, &text);
                            gui::note_log(&format!("edit-image: {text}"), false);
                        }
                        Err(m) => {
                            set_text(s.status, &format!("Failed: {m}"));
                            gui::note_log(&format!("edit-image: {m}"), true);
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

fn stage(s: &mut State, e: Edit, desc: &str) {
    s.ops.push(e);
    list_add(s.ops_list, desc);
    set_text(s.status, desc);
}

fn load(hwnd: HWND, path: PathBuf) {
    let s = st(hwnd);
    set_text(s.path_edit, &path.to_string_lossy());
    let mut v = match crate::cli_image::open_volume(&path, None) {
        Ok(v) => v,
        Err((_, m)) => {
            set_text(s.status, &format!("Could not open: {m}"));
            return;
        }
    };
    let root = v.root();
    match v.walk(&root) {
        Ok(items) => {
            s.entries = items.iter().map(|(p, e)| (p.clone(), e.dir)).collect();
            list_clear(s.list);
            for (p, e) in &items {
                if e.dir {
                    list_add(s.list, &format!("{p}/"));
                } else {
                    list_add(s.list, &format!("{p}    ({})", fmt_bytes(e.size)));
                }
            }
            let lbl = v.fs.label();
            set_text(s.label_edit, &lbl);
            s.image = Some(path);
            s.ops.clear();
            list_clear(s.ops_list);
            set_text(s.status, "Stage changes, then Save as… to write a new image.");
        }
        Err(e) => set_text(s.status, &format!("Could not list the image: {e}")),
    }
}

fn save(hwnd: HWND) {
    let s = st(hwnd);
    if s.running {
        return;
    }
    let Some(image) = s.image.clone() else {
        set_text(s.status, "Open an image first.");
        return;
    };
    if s.ops.is_empty() {
        set_text(s.status, "No changes staged yet.");
        return;
    }
    let initial = image.with_file_name("edited.iso");
    let name = initial.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let Some(dest) = pick_save(hwnd, "Save edited image as", "ISO images (*.iso)|*.iso", "iso", &name) else {
        return;
    };
    let ops = s.ops.clone();
    let shared = JobShared::new();
    s.shared = Some(shared.clone());
    s.running = true;
    set_enabled(s.save, false);
    set_enabled(s.cancel, true);
    progress_show(s.bar, None);
    set_text(s.status, "Working…");
    run_job(hwnd, &shared, move |sh| {
        let (sum, warnings) = if_image::edit::edit(&image, &dest, &ops, sh.cb()).map_err(|e| e.to_string())?;
        let mut text = format!("Saved {} ({} bytes, {} files)", dest.display(), sum.sectors * 2048, sum.files);
        for w in &warnings {
            text.push_str(&format!(" · note: {w}"));
        }
        Ok(text)
    });
}
