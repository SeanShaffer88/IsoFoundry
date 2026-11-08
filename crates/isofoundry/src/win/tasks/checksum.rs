//! Checksum task window: hash a file (CRC32, MD5, SHA-1, SHA-256), write a
//! redump-style `.dat` for it, or verify a file against an existing `.dat`
//! list. Same calls as the `checksum`/`verify` CLI commands.

use super::super::gui;
use super::super::taskwin::*;
use if_hash::dat::{Dat, Game, Rom};
use if_hash::Algorithms;
use std::path::PathBuf;
use std::sync::Arc;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "IsoFoundryTaskChecksum";

const ID_FILE: u16 = 1;
const ID_BROWSE: u16 = 2;
const ID_VERIFY: u16 = 3;
const ID_DAT: u16 = 4;
const ID_DAT_BROWSE: u16 = 5;
const ID_WRITE_DAT: u16 = 6;
const ID_START: u16 = 10;
const ID_CANCEL: u16 = 11;
const ID_OUT: u16 = 20;

struct State {
    file: HWND,
    verify: HWND,
    dat: HWND,
    dat_browse: HWND,
    write_dat: HWND,
    start: HWND,
    cancel: HWND,
    bar: HWND,
    status: HWND,
    out: HWND,
    shared: Option<Arc<JobShared>>,
    running: bool,
    close_when_done: bool,
}

pub fn open(owner: HWND) {
    register_class(CLASS, Some(wndproc));
    let state = Box::new(State {
        file: std::ptr::null_mut(),
        verify: std::ptr::null_mut(),
        dat: std::ptr::null_mut(),
        dat_browse: std::ptr::null_mut(),
        write_dat: std::ptr::null_mut(),
        start: std::ptr::null_mut(),
        cancel: std::ptr::null_mut(),
        bar: std::ptr::null_mut(),
        status: std::ptr::null_mut(),
        out: std::ptr::null_mut(),
        shared: None,
        running: false,
        close_when_done: false,
    });
    let title = if_core::strings::task_title("checksum");
    create_window(owner, CLASS, title, 664, 478, Box::into_raw(state).cast());
}

fn st(hwnd: HWND) -> &'static mut State {
    // SAFETY: the pointer is ours, set at WM_NCCREATE, dropped at WM_NCDESTROY;
    // all access is on the window's thread.
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
            label(hwnd, 12, 17, 44, "File:");
            s.file = edit(hwnd, ID_FILE, 56, 14, 452, "", false);
            let b = button(hwnd, ID_BROWSE, 516, 13, 112, "Browse…");
            s.verify = checkbox(hwnd, ID_VERIFY, 12, 48, 300, "Verify against a .dat hash list");
            label(hwnd, 12, 77, 44, "List:");
            s.dat = edit(hwnd, ID_DAT, 56, 74, 452, "", false);
            s.dat_browse = button(hwnd, ID_DAT_BROWSE, 516, 73, 112, "Browse…");
            s.write_dat = checkbox(hwnd, ID_WRITE_DAT, 12, 106, 300, "Also write a .dat file");
            s.start = button(hwnd, ID_START, 392, 104, 112, "Start");
            s.cancel = button(hwnd, ID_CANCEL, 516, 104, 112, "Cancel");
            s.bar = progress_bar(hwnd, 12, 142, 616);
            s.status = label(hwnd, 12, 164, 616, "");
            s.out = multiline(hwnd, ID_OUT, 12, 188, 616, 216);
            let _ = b;
            set_enabled(s.dat, false);
            set_enabled(s.dat_browse, false);
            set_enabled(s.cancel, false);
            progress_show(s.bar, Some(0.0));
            0
        }
        WM_COMMAND => {
            let id = (wp & 0xFFFF) as u16;
            let s = st(hwnd);
            match id {
                ID_BROWSE => {
                    if let Some(p) = pick_open(hwnd, "Choose a file to hash", "All files|*.*") {
                        set_text(s.file, &p.to_string_lossy());
                    }
                }
                ID_VERIFY => {
                    let on = is_checked(s.verify);
                    set_enabled(s.dat, on);
                    set_enabled(s.dat_browse, on);
                    set_enabled(s.write_dat, !on);
                }
                ID_DAT_BROWSE => {
                    if let Some(p) =
                        pick_open(hwnd, "Choose a .dat hash list", "Hash lists (*.dat)|*.dat|All files|*.*")
                    {
                        set_text(s.dat, &p.to_string_lossy());
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
                                append_line(s.out, line);
                            }
                            set_text(s.status, "Done.");
                            gui::note_log("checksum: done", false);
                        }
                        Err(m) => {
                            append_line(s.out, &format!("Failed: {m}"));
                            set_text(s.status, "Failed.");
                            gui::note_log(&format!("checksum: {m}"), true);
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
        WM_CTLCOLOREDIT | WM_CTLCOLORSTATIC | WM_CTLCOLORLISTBOX | WM_CTLCOLORBTN => ctl_color(wp),
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn start(hwnd: HWND) {
    let s = st(hwnd);
    if s.running {
        return;
    }
    let file = PathBuf::from(get_text(s.file));
    if !file.is_file() {
        append_line(s.out, "Choose a file first.");
        return;
    }
    let verify = is_checked(s.verify);
    let shared = JobShared::new();
    s.shared = Some(shared.clone());
    s.running = true;
    set_enabled(s.start, false);
    set_enabled(s.cancel, true);
    progress_show(s.bar, None);
    set_text(s.status, "Working…");
    if verify {
        let dat = PathBuf::from(get_text(s.dat));
        if !dat.is_file() {
            append_line(s.out, "Choose the .dat hash list to verify against.");
            s.running = false;
            set_enabled(s.start, true);
            set_enabled(s.cancel, false);
            return;
        }
        run_job(hwnd, &shared, move |sh| {
            let d = crate::cli_image::hash_file(&file, Algorithms::default(), sh.cb_bytes(file_size(&file)))
                .map_err(|(_, m)| m)?;
            let text = std::fs::read_to_string(&dat).map_err(|e| format!("{}: {e}", dat.display()))?;
            let list = Dat::parse(&text).map_err(|m| format!("{}: {m}", dat.display()))?;
            let hits = list.find(&d);
            if hits.is_empty() {
                return Err(format!("No entry in {} matches this file.", list.name));
            }
            let mut lines = vec![format!("File:    {}", file.display())];
            for (g, r) in hits {
                lines.push(format!("Verified: matches \u{201C}{}\u{201D} ({}) in {}", g.name, r.name, list.name));
            }
            Ok(lines.join("\n"))
        });
    } else {
        let dat_out = if is_checked(s.write_dat) {
            let initial = file.with_extension("dat");
            let name = initial.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            match pick_save(hwnd, "Write .dat file", "Hash lists (*.dat)|*.dat", "dat", &name) {
                Some(p) => Some(p),
                None => {
                    s.running = false;
                    set_enabled(s.start, true);
                    set_enabled(s.cancel, false);
                    return;
                }
            }
        } else {
            None
        };
        run_job(hwnd, &shared, move |sh| {
            let d = crate::cli_image::hash_file(&file, Algorithms::default(), sh.cb_bytes(file_size(&file)))
                .map_err(|(_, m)| m)?;
            if let Some(out) = &dat_out {
                let name = file.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let stem = file.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let dat = Dat {
                    name: stem.clone(),
                    description: format!("Hashes written by IsoFoundry {}", if_core::APP_VERSION),
                    games: vec![Game { name: stem, roms: vec![Rom::from_digests(&name, &d)] }],
                };
                if_core::fsutil::atomic_write(out, dat.to_xml().as_bytes())
                    .map_err(|e| format!("{}: {e}", out.display()))?;
            }
            let mut lines = vec![format!("File:    {}", file.display()), format!("Size:    {} bytes", d.size)];
            for (k, v) in [("CRC32:", &d.crc32), ("MD5:", &d.md5), ("SHA-1:", &d.sha1), ("SHA-256:", &d.sha256)] {
                if let Some(v) = v {
                    lines.push(format!("{k:<8} {v}"));
                }
            }
            if let Some(out) = &dat_out {
                lines.push(format!("List:    {}", out.display()));
            }
            Ok(lines.join("\n"))
        });
    }
}

fn file_size(p: &PathBuf) -> u64 {
    std::fs::metadata(p).map(|m| m.len()).unwrap_or(0)
}
