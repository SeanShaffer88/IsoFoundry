//! Convert-image task window: pick a source image and an output format
//! (ISO, BIN/CUE, CloneCD, CHD); converts with the same
//! `if_image::convert::convert` call as the `convert-image` CLI command.

use super::super::gui;
use super::super::taskwin::*;
use if_image::convert::{convert, Output};
use std::path::PathBuf;
use std::sync::Arc;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "IsoFoundryTaskConvert";

const ID_SRC: u16 = 1;
const ID_SRC_BROWSE: u16 = 2;
const ID_OUT: u16 = 3;
const ID_OUT_BROWSE: u16 = 4;
const ID_FMT_BASE: u16 = 10; // + format index
const ID_START: u16 = 20;
const ID_CANCEL: u16 = 21;
const ID_OUTLOG: u16 = 30;

const FORMATS: [(&str, Output, &str); 5] = [
    ("ISO", Output::Iso, "iso"),
    ("BIN/CUE", Output::BinCue, "cue"),
    ("CloneCD", Output::CloneCd, "ccd"),
    ("CHD", Output::Chd, "chd"),
    ("CHD (CD layout)", Output::ChdCd, "chd"),
];

struct State {
    src: HWND,
    out: HWND,
    fmts: Vec<HWND>,
    start: HWND,
    cancel: HWND,
    bar: HWND,
    status: HWND,
    outlog: HWND,
    shared: Option<Arc<JobShared>>,
    running: bool,
    close_when_done: bool,
}

pub fn open(owner: HWND) {
    register_class(CLASS, Some(wndproc));
    let state = Box::new(State {
        src: std::ptr::null_mut(),
        out: std::ptr::null_mut(),
        fmts: Vec::new(),
        start: std::ptr::null_mut(),
        cancel: std::ptr::null_mut(),
        bar: std::ptr::null_mut(),
        status: std::ptr::null_mut(),
        outlog: std::ptr::null_mut(),
        shared: None,
        running: false,
        close_when_done: false,
    });
    let title = if_core::strings::task_title("convert-image");
    create_window(owner, CLASS, title, 664, 446, Box::into_raw(state).cast());
}

fn st(hwnd: HWND) -> &'static mut State {
    // SAFETY: our pointer from WM_NCCREATE; window thread only.
    unsafe { &mut *state_ptr::<State>(hwnd) }
}

fn chosen(s: &State) -> usize {
    s.fmts.iter().position(|&h| is_checked(h)).unwrap_or(0)
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
            s.src = edit(hwnd, ID_SRC, 56, 14, 452, "", false);
            button(hwnd, ID_SRC_BROWSE, 516, 13, 112, "Browse…");
            label(hwnd, 12, 52, 60, "Format:");
            let mut x = 76;
            for (i, (name, _, _)) in FORMATS.iter().enumerate() {
                let w = 20 + name.len() as i32 * 8;
                let h = radio(hwnd, ID_FMT_BASE + i as u16, x, 50, w, name, i == 0);
                s.fmts.push(h);
                x += w + 10;
            }
            set_checked(s.fmts[0], true);
            label(hwnd, 12, 85, 44, "Output:");
            s.out = edit(hwnd, ID_OUT, 56, 82, 452, "", false);
            button(hwnd, ID_OUT_BROWSE, 516, 81, 112, "Save as…");
            s.start = button(hwnd, ID_START, 392, 118, 112, "Convert");
            s.cancel = button(hwnd, ID_CANCEL, 516, 118, 112, "Cancel");
            s.bar = progress_bar(hwnd, 12, 156, 616);
            s.status = label(hwnd, 12, 178, 616, "");
            s.outlog = multiline(hwnd, ID_OUTLOG, 12, 202, 616, 150);
            set_enabled(s.cancel, false);
            progress_show(s.bar, Some(0.0));
            0
        }
        WM_COMMAND => {
            let id = (wp & 0xFFFF) as u16;
            match id {
                ID_SRC_BROWSE => {
                    let s = st(hwnd);
                    if let Some(p) = pick_open(
                        hwnd,
                        "Choose an image to convert",
                        "Disc images|*.iso;*.cue;*.ccd;*.chd;*.nrg;*.mds;*.bin;*.img|All files|*.*",
                    ) {
                        set_text(s.src, &p.to_string_lossy());
                        suggest_out(s);
                    }
                }
                x if (ID_FMT_BASE..ID_FMT_BASE + FORMATS.len() as u16).contains(&x) => {
                    suggest_out(st(hwnd));
                }
                ID_OUT_BROWSE => {
                    let s = st(hwnd);
                    let (_, _, ext) = FORMATS[chosen(s)];
                    if let Some(p) = pick_save(hwnd, "Convert to", "Image files|*.*", ext, &format!("image.{ext}")) {
                        set_text(s.out, &p.to_string_lossy());
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
                            gui::note_log("convert-image: done", false);
                        }
                        Err(m) => {
                            append_line(s.outlog, &format!("Failed: {m}"));
                            set_text(s.status, "Failed.");
                            gui::note_log(&format!("convert-image: {m}"), true);
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

fn suggest_out(s: &mut State) {
    let src = PathBuf::from(get_text(s.src));
    if src.as_os_str().is_empty() {
        return;
    }
    let (_, _, ext) = FORMATS[chosen(s)];
    set_text(s.out, &src.with_extension(ext).to_string_lossy());
}

fn start(hwnd: HWND) {
    let s = st(hwnd);
    if s.running {
        return;
    }
    let src = PathBuf::from(get_text(s.src));
    if !src.is_file() {
        append_line(s.outlog, "Choose an image to convert first.");
        return;
    }
    let dest = PathBuf::from(get_text(s.out));
    if dest.as_os_str().is_empty() {
        append_line(s.outlog, "Choose where to save the converted image.");
        return;
    }
    let out_fmt = FORMATS[chosen(s)].1;
    let shared = JobShared::new();
    s.shared = Some(shared.clone());
    s.running = true;
    set_enabled(s.start, false);
    set_enabled(s.cancel, true);
    progress_show(s.bar, None);
    set_text(s.status, "Working…");
    run_job(hwnd, &shared, move |sh| {
        let mut cb = sh.cb();
        let sum = convert(&src, &dest, out_fmt, &mut cb).map_err(|e| e.to_string())?;
        let mut lines = vec![format!("Converted {} ({}) to:", src.display(), sum.source.name())];
        for f in &sum.files {
            lines.push(format!("  {}", f.display()));
        }
        for w in &sum.warnings {
            lines.push(format!("  note: {w}"));
        }
        Ok(lines.join("\n"))
    });
}
