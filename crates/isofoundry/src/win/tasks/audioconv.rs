//! Convert audio window (M3): a batch list, an output format
//! (FLAC / WAV / MP3 / AAC), a bit rate for the lossy formats, and
//! ReplayGain options. Every file is converted in turn; tags are
//! carried across by the engine. MP3 needs `libmp3lame.dll` at
//! runtime — the window says so up front when it is missing.

use super::super::gui;
use super::super::taskwin::*;
use if_audio::{ConvertOptions, Format};
use std::path::PathBuf;
use std::sync::Arc;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "IsoFoundryTaskAudioConv";

const ID_LIST: u16 = 1;
const ID_ADD_FILES: u16 = 2;
const ID_ADD_FOLDER: u16 = 3;
const ID_REMOVE: u16 = 4;
const ID_CLEAR: u16 = 5;
const ID_FMT_FLAC: u16 = 6;
const ID_FMT_WAV: u16 = 7;
const ID_FMT_MP3: u16 = 8;
const ID_FMT_AAC: u16 = 9;
const ID_BITRATE: u16 = 10;
const ID_RG: u16 = 11;
const ID_APPLY: u16 = 12;
const ID_OUT: u16 = 13;
const ID_OUT_BROWSE: u16 = 14;
const ID_START: u16 = 15;
const ID_CANCEL: u16 = 16;
const ID_OUTLOG: u16 = 17;

const AUDIO_EXTS: [&str; 8] = ["wav", "flac", "mp3", "ogg", "opus", "m4a", "aac", "wma"];

struct State {
    list: HWND,
    fmt_flac: HWND,
    fmt_mp3: HWND,
    fmt_aac: HWND,
    bitrate: HWND,
    rg: HWND,
    apply: HWND,
    out: HWND,
    start: HWND,
    cancel: HWND,
    bar: HWND,
    status: HWND,
    outlog: HWND,
    files: Vec<PathBuf>,
    shared: Option<Arc<JobShared>>,
    running: bool,
    close_when_done: bool,
}

pub fn open(owner: HWND) {
    register_class(CLASS, Some(wndproc));
    let state = Box::new(State {
        list: std::ptr::null_mut(),
        fmt_flac: std::ptr::null_mut(),
        fmt_mp3: std::ptr::null_mut(),
        fmt_aac: std::ptr::null_mut(),
        bitrate: std::ptr::null_mut(),
        rg: std::ptr::null_mut(),
        apply: std::ptr::null_mut(),
        out: std::ptr::null_mut(),
        start: std::ptr::null_mut(),
        cancel: std::ptr::null_mut(),
        bar: std::ptr::null_mut(),
        status: std::ptr::null_mut(),
        outlog: std::ptr::null_mut(),
        files: Vec::new(),
        shared: None,
        running: false,
        close_when_done: false,
    });
    let title = if_core::strings::task_title("convert-audio");
    create_window(owner, CLASS, title, 664, 520, Box::into_raw(state).cast());
}

fn st(hwnd: HWND) -> &'static mut State {
    // SAFETY: our pointer from WM_NCCREATE; window thread only.
    unsafe { &mut *state_ptr::<State>(hwnd) }
}

fn refresh_list(s: &mut State) {
    list_clear(s.list);
    for p in &s.files {
        list_add(s.list, &p.to_string_lossy());
    }
}

fn push_file(s: &mut State, p: PathBuf) {
    if !s.files.contains(&p) {
        s.files.push(p);
        refresh_list(s);
    }
}

/// Collects audio files under a folder (recursive, name-sorted).
fn collect_folder(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for p in paths {
        if p.is_dir() {
            collect_folder(&p, out);
        } else if p
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| AUDIO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
            .unwrap_or(false)
        {
            out.push(p);
        }
    }
}

fn chosen_format(s: &State) -> Format {
    if is_checked(s.fmt_mp3) {
        Format::Mp3
    } else if is_checked(s.fmt_aac) {
        Format::Aac
    } else if is_checked(s.fmt_flac) {
        Format::Flac
    } else {
        Format::Wav
    }
}

fn start(hwnd: HWND) {
    let s = st(hwnd);
    if s.running {
        return;
    }
    if s.files.is_empty() {
        append_line(s.outlog, "Add at least one audio file first.");
        return;
    }
    let format = chosen_format(s);
    let bitrate: u32 = get_text(s.bitrate).trim().parse().unwrap_or(192);
    if !(32..=512).contains(&bitrate) {
        append_line(s.outlog, "Bit rate must be between 32 and 512 kbit/s.");
        return;
    }
    if format == Format::Mp3 && !if_audio::mp3_available() {
        append_line(
            s.outlog,
            "Note: libmp3lame.dll was not found, so MP3 files will fail. It is loaded at runtime: put a copy next to isofoundry.exe or on the PATH.",
        );
    }
    let out_dir = get_text(s.out);
    let out_dir = if out_dir.trim().is_empty() { None } else { Some(PathBuf::from(out_dir.trim())) };
    if let Some(d) = &out_dir {
        if let Err(e) = std::fs::create_dir_all(d) {
            append_line(s.outlog, &format!("Cannot create the output folder: {e}"));
            return;
        }
    }
    let opts =
        ConvertOptions { format, bitrate_kbps: bitrate, replaygain: is_checked(s.rg), apply_gain: is_checked(s.apply) };
    let mut jobs: Vec<(PathBuf, PathBuf)> = Vec::new();
    for src in &s.files {
        let dir = out_dir.clone().unwrap_or_else(|| src.parent().map(|p| p.to_path_buf()).unwrap_or_default());
        let stem = src.file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_else(|| "audio".into());
        let dst = dir.join(format!("{stem}.{}", format.extension()));
        if dst == *src {
            append_line(s.outlog, &format!("Skipped {}: the output would replace the input.", src.display()));
            continue;
        }
        jobs.push((src.clone(), dst));
    }
    if jobs.is_empty() {
        return;
    }
    let shared = JobShared::new();
    s.shared = Some(shared.clone());
    s.running = true;
    set_enabled(s.start, false);
    set_enabled(s.cancel, true);
    progress_show(s.bar, None);
    set_text(s.status, "Working…");
    run_job(hwnd, &shared, move |sh| {
        let progress = sh.progress();
        let cancel = sh.cancel.clone();
        let mut lines = Vec::new();
        let mut done = 0usize;
        for (src, dst) in &jobs {
            match if_audio::convert_file(src, dst, &opts, &progress, &cancel) {
                Ok(rep) => {
                    done += 1;
                    let mut line = format!("{} -> {} ({} frames)", src.display(), dst.display(), rep.frames);
                    if let Some(g) = rep.gain_db {
                        line.push_str(&format!(", ReplayGain {g:+.2} dB"));
                    }
                    lines.push(line);
                }
                Err(e) => lines.push(format!("{} FAILED: {}", src.display(), e.0)),
            }
            if cancel.is_cancelled() {
                lines.push("Cancelled.".to_string());
                break;
            }
        }
        if done == jobs.len() {
            Ok(lines.join("\n"))
        } else if done > 0 {
            Ok(format!("Converted {done} of {} files.\n{}", jobs.len(), lines.join("\n")))
        } else {
            Err(lines.join("\n"))
        }
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
            label(hwnd, 12, 14, 300, "Audio files to convert:");
            s.list = listbox(hwnd, ID_LIST, 12, 36, 480, 148);
            button(hwnd, ID_ADD_FILES, 504, 36, 124, "Add files…");
            button(hwnd, ID_ADD_FOLDER, 504, 68, 124, "Add folder…");
            button(hwnd, ID_REMOVE, 504, 100, 124, "Remove");
            button(hwnd, ID_CLEAR, 504, 132, 124, "Clear");
            label(hwnd, 12, 197, 90, "Convert to:");
            s.fmt_flac = radio(hwnd, ID_FMT_FLAC, 108, 196, 80, "FLAC", true);
            radio(hwnd, ID_FMT_WAV, 196, 196, 80, "WAV", false);
            s.fmt_mp3 = radio(hwnd, ID_FMT_MP3, 284, 196, 80, "MP3", false);
            s.fmt_aac = radio(hwnd, ID_FMT_AAC, 372, 196, 110, "AAC (M4A)", false);
            label(hwnd, 12, 225, 150, "Bit rate (MP3/AAC):");
            s.bitrate = edit(hwnd, ID_BITRATE, 170, 223, 60, "192", false);
            label(hwnd, 238, 225, 60, "kbit/s");
            s.rg = checkbox(hwnd, ID_RG, 330, 223, 160, "ReplayGain tags");
            s.apply = checkbox(hwnd, ID_APPLY, 498, 223, 150, "Apply gain");
            label(hwnd, 12, 254, 90, "Output folder:");
            s.out = edit(hwnd, ID_OUT, 108, 252, 420, "", false);
            button(hwnd, ID_OUT_BROWSE, 536, 251, 112, "Browse…");
            s.start = button(hwnd, ID_START, 392, 284, 112, "Convert");
            s.cancel = button(hwnd, ID_CANCEL, 516, 284, 112, "Cancel");
            s.bar = progress_bar(hwnd, 12, 322, 640);
            s.status = label(hwnd, 12, 344, 640, "");
            s.outlog = multiline(hwnd, ID_OUTLOG, 12, 368, 640, 110);
            set_enabled(s.cancel, false);
            progress_show(s.bar, Some(0.0));
            0
        }
        WM_COMMAND => {
            let id = (wp & 0xFFFF) as u16;
            match id {
                ID_ADD_FILES => {
                    let s = st(hwnd);
                    for p in pick_open_multi(
                        hwnd,
                        "Add audio files",
                        "Audio files|*.wav;*.flac;*.mp3;*.ogg;*.opus;*.m4a;*.aac;*.wma|All files|*.*",
                    ) {
                        push_file(s, p);
                    }
                }
                ID_ADD_FOLDER => {
                    let s = st(hwnd);
                    if let Some(d) = pick_folder(hwnd, "Add folder") {
                        let mut found = Vec::new();
                        collect_folder(&d, &mut found);
                        let n = found.len();
                        for p in found {
                            push_file(s, p);
                        }
                        append_line(s.outlog, &format!("Added {n} audio files from {}.", d.display()));
                    }
                }
                ID_REMOVE => {
                    let s = st(hwnd);
                    let mut sel = list_selected(s.list);
                    sel.sort_unstable_by(|a, b| b.cmp(a));
                    for i in sel {
                        if i < s.files.len() {
                            s.files.remove(i);
                        }
                    }
                    refresh_list(s);
                }
                ID_CLEAR => {
                    let s = st(hwnd);
                    s.files.clear();
                    refresh_list(s);
                }
                ID_OUT_BROWSE => {
                    let s = st(hwnd);
                    if let Some(d) = pick_folder(hwnd, "Output folder") {
                        set_text(s.out, &d.to_string_lossy());
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
                            gui::note_log("convert-audio: done", false);
                        }
                        Err(m) => {
                            for line in m.lines() {
                                append_line(s.outlog, line);
                            }
                            set_text(s.status, "Failed.");
                            gui::note_log("convert-audio: failed", true);
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
