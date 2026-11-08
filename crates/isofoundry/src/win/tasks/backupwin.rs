//! Spanning backup window (M6): back a folder up to one ISO per
//! span (burn them with Burn image, or let the `backup` command
//! burn directly), and restore a backup from its span folders or
//! ISOs with every checksum verified.

use super::super::taskwin::*;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "IsoFoundryTaskBackup";
const ID_SOURCE: u16 = 1;
const ID_SOURCE_PICK: u16 = 2;
const ID_NAME: u16 = 3;
const ID_SIZES: u16 = 4;
const ID_OUT: u16 = 5;
const ID_OUT_PICK: u16 = 6;
const ID_LIST: u16 = 7;
const ID_ADD_ISO: u16 = 8;
const ID_ADD_DIR: u16 = 9;
const ID_REMOVE: u16 = 10;
const ID_GO: u16 = 11;
const ID_CANCEL: u16 = 12;

const SIZES: [(&str, u64); 4] = [
    ("CD (700 MB)", crate::backup::CAP_CD),
    ("DVD (4.7 GB)", crate::backup::CAP_DVD),
    ("DVD dual layer (8.5 GB)", crate::backup::CAP_DVD_DL),
    ("Blu-ray (25 GB)", crate::backup::CAP_BD),
];

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Backup,
    Restore,
}

struct State {
    mode: Mode,
    source: HWND,
    name: HWND,
    sizes: HWND,
    out: HWND,
    list: HWND,
    status: HWND,
    bar: HWND,
    go: HWND,
    cancel: HWND,
    sources: Vec<PathBuf>,
    shared: std::sync::Arc<JobShared>,
}

pub fn open_backup(owner: HWND) {
    open(owner, Mode::Backup);
}

pub fn open_restore(owner: HWND) {
    open(owner, Mode::Restore);
}

fn open(owner: HWND, mode: Mode) {
    register_class(CLASS, Some(wndproc));
    let null = std::ptr::null_mut();
    let state = Box::new(State {
        mode,
        source: null,
        name: null,
        sizes: null,
        out: null,
        list: null,
        status: null,
        bar: null,
        go: null,
        cancel: null,
        sources: Vec::new(),
        shared: JobShared::new(),
    });
    let title = match mode {
        Mode::Backup => "Spanning backup",
        Mode::Restore => "Restore backup",
    };
    create_window(owner, CLASS, title, 664, 470, Box::into_raw(state).cast());
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
                match s.mode {
                    Mode::Backup => {
                        label(hwnd, 16, 14, 300, "Folder to back up:");
                        s.source = edit(hwnd, ID_SOURCE, 16, 34, 500, "", false);
                        button(hwnd, ID_SOURCE_PICK, 524, 34, 120, "Browse…");
                        label(hwnd, 16, 66, 200, "Backup name:");
                        s.name = edit(hwnd, ID_NAME, 16, 86, 300, "backup", false);
                        label(hwnd, 16, 118, 200, "Disc size:");
                        s.sizes = listbox(hwnd, ID_SIZES, 16, 138, 300, 92);
                        for (label_text, _) in SIZES {
                            list_add(s.sizes, label_text);
                        }
                        list_select(s.sizes, 1);
                        label(hwnd, 16, 242, 300, "Write span ISOs to:");
                        s.out = edit(hwnd, ID_OUT, 16, 262, 500, "", false);
                        button(hwnd, ID_OUT_PICK, 524, 262, 120, "Browse…");
                        label(
                            hwnd,
                            16,
                            296,
                            628,
                            "Each span ISO carries the full catalog. Burn them with Burn image, or use the backup command to burn directly.",
                        );
                    }
                    Mode::Restore => {
                        label(hwnd, 16, 14, 400, "Span sources (folders or span ISOs, any order):");
                        s.list = listbox(hwnd, ID_LIST, 16, 34, 420, 150);
                        button(hwnd, ID_ADD_ISO, 444, 34, 200, "Add span ISO…");
                        button(hwnd, ID_ADD_DIR, 444, 66, 200, "Add span folder…");
                        button(hwnd, ID_REMOVE, 444, 98, 200, "Remove selected");
                        label(hwnd, 16, 196, 300, "Restore into:");
                        s.out = edit(hwnd, ID_OUT, 16, 216, 500, "", false);
                        button(hwnd, ID_OUT_PICK, 524, 216, 120, "Browse…");
                        label(
                            hwnd,
                            16,
                            250,
                            628,
                            "Every file's checksum is verified as it is restored; a missing span is named before anything is written.",
                        );
                    }
                }
                s.bar = progress_bar(hwnd, 16, 336, 628);
                s.status = label(hwnd, 16, 360, 628, "Ready.");
                s.go = button(hwnd, ID_GO, 404, 392, 120, if s.mode == Mode::Backup { "Back up" } else { "Restore" });
                s.cancel = button(hwnd, ID_CANCEL, 532, 392, 112, "Cancel");
                set_enabled(s.cancel, false);
                0
            }
            WM_COMMAND => {
                let s = st(hwnd);
                match ((wp & 0xFFFF) as u16, (wp >> 16) as u16) {
                    (ID_SOURCE_PICK, _) => {
                        if let Some(p) = pick_folder(hwnd, "Folder to back up") {
                            set_text(s.source, &p.to_string_lossy());
                            if let Some(n) = p.file_name() {
                                set_text(s.name, &n.to_string_lossy());
                            }
                        }
                        0
                    }
                    (ID_OUT_PICK, _) => {
                        if let Some(p) = pick_folder(hwnd, "Destination folder") {
                            set_text(s.out, &p.to_string_lossy());
                        }
                        0
                    }
                    (ID_ADD_ISO, _) => {
                        for p in pick_open_multi(hwnd, "Span ISO", "Disc images|*.iso|All files|*.*") {
                            s.sources.push(p.clone());
                            list_add(s.list, &p.to_string_lossy());
                        }
                        0
                    }
                    (ID_ADD_DIR, _) => {
                        if let Some(p) = pick_folder(hwnd, "Span folder") {
                            list_add(s.list, &p.to_string_lossy());
                            s.sources.push(p);
                        }
                        0
                    }
                    (ID_REMOVE, _) => {
                        for i in list_selected(s.list).into_iter().rev() {
                            if i < s.sources.len() {
                                s.sources.remove(i);
                            }
                        }
                        list_clear(s.list);
                        for p in &s.sources {
                            list_add(s.list, &p.to_string_lossy());
                        }
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
                        crate::win::gui::note_log(&format!("Backup window job failed: {e}"), true);
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
    match s.mode {
        Mode::Backup => {
            let source = get_text(s.source);
            if source.trim().is_empty() {
                set_text(s.status, "Pick a folder to back up.");
                return;
            }
            let out_dir = get_text(s.out);
            if out_dir.trim().is_empty() {
                set_text(s.status, "Pick where the span ISOs go.");
                return;
            }
            let name = {
                let n = get_text(s.name);
                if n.trim().is_empty() {
                    "backup".to_string()
                } else {
                    n.trim().to_string()
                }
            };
            let idx = list_selected(s.sizes).first().copied().unwrap_or(1);
            let capacity = SIZES[idx.min(SIZES.len() - 1)].1;
            set_enabled(s.go, false);
            set_enabled(s.cancel, true);
            set_text(s.status, "Backing up…");
            let shared = s.shared.clone();
            run_job(hwnd, &shared, move |sh| {
                let work = std::env::temp_dir().join(format!("isofoundry-backup-{}", std::process::id()));
                let staged = crate::backup::create_backup(
                    &PathBuf::from(&source),
                    &name,
                    capacity,
                    &work,
                    &sh.progress(),
                    &sh.cancel,
                )?;
                let out = PathBuf::from(&out_dir);
                std::fs::create_dir_all(&out).map_err(|e| format!("cannot create {}: {e}", out.display()))?;
                let slug: String = name
                    .chars()
                    .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
                    .collect();
                for (i, dir) in staged.span_dirs.iter().enumerate() {
                    let iso = out.join(format!("{}-{:03}.iso", slug.trim_matches('-'), i + 1));
                    crate::backup::write_span_iso(dir, &iso, &name, &sh.progress(), &sh.cancel)?;
                }
                let summary = format!(
                    "Backup '{name}' complete: {} file(s), {} across {} span ISO(s) in {}.",
                    staged.catalog.files.len(),
                    crate::backup::human_bytes(staged.catalog.total_bytes()),
                    staged.catalog.spans,
                    out.display()
                );
                let _ = std::fs::remove_dir_all(&work);
                Ok(summary)
            });
        }
        Mode::Restore => {
            if s.sources.is_empty() {
                set_text(s.status, "Add at least one span source.");
                return;
            }
            let dest = get_text(s.out);
            if dest.trim().is_empty() {
                set_text(s.status, "Pick where to restore into.");
                return;
            }
            let sources = s.sources.clone();
            set_enabled(s.go, false);
            set_enabled(s.cancel, true);
            set_text(s.status, "Restoring…");
            let shared = s.shared.clone();
            run_job(hwnd, &shared, move |sh| {
                let work = std::env::temp_dir().join(format!("isofoundry-restore-{}", std::process::id()));
                let r = crate::backup::restore(&sources, &PathBuf::from(&dest), &work, &sh.progress(), &sh.cancel);
                let _ = std::fs::remove_dir_all(&work);
                let r = r?;
                Ok(format!(
                    "Restored '{}': {} file(s), {}, every checksum verified.",
                    r.backup_name,
                    r.files,
                    crate::backup::human_bytes(r.bytes)
                ))
            });
        }
    }
}
