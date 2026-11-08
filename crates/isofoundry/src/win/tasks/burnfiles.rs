//! Burn files / Burn MP3 disc / Burn video windows (M4): one
//! window implementation, three modes. Files mode burns a folder
//! as a data disc; MP3 mode stages the folder's audio with a
//! generated playlist first (the same staging the CLI uses); Video
//! mode checks for a VIDEO_TS structure before burning.

use super::super::taskwin::*;
use super::drivepick::DrivePane;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    Files,
    Mp3,
    Video,
}

const ID_FOLDER: u16 = 1;
const ID_BROWSE: u16 = 2;
const ID_LABEL: u16 = 3;
const ID_DRIVES: u16 = 4;
const ID_REFRESH: u16 = 5;
const ID_VERIFY: u16 = 6;
const ID_APPEND: u16 = 7;
const ID_CLOSE: u16 = 8;
const ID_EJECT: u16 = 9;
const ID_BURN: u16 = 13;
const ID_CANCEL: u16 = 14;

struct State {
    mode: Mode,
    folder: HWND,
    label_edit: HWND,
    drives: DrivePane,
    media: HWND,
    verify: HWND,
    append: HWND,
    close: HWND,
    eject: HWND,
    status: HWND,
    bar: HWND,
    burn: HWND,
    cancel: HWND,
    shared: std::sync::Arc<JobShared>,
}

fn class_of(mode: Mode) -> &'static str {
    match mode {
        Mode::Files => "IsoFoundryTaskBurnFiles",
        Mode::Mp3 => "IsoFoundryTaskBurnMp3",
        Mode::Video => "IsoFoundryTaskBurnVideo",
    }
}

fn title_of(mode: Mode) -> &'static str {
    match mode {
        Mode::Files => "Burn files to disc",
        Mode::Mp3 => "Burn MP3 CD / DVD",
        Mode::Video => "Burn DVD-video folder",
    }
}

pub fn open_files(owner: HWND) {
    open_mode(owner, Mode::Files)
}

pub fn open_mp3(owner: HWND) {
    open_mode(owner, Mode::Mp3)
}

pub fn open_video(owner: HWND) {
    open_mode(owner, Mode::Video)
}

fn open_mode(owner: HWND, mode: Mode) {
    register_class(class_of(mode), Some(wndproc));
    let state = Box::new(State {
        mode,
        folder: std::ptr::null_mut(),
        label_edit: std::ptr::null_mut(),
        drives: DrivePane::empty(),
        media: std::ptr::null_mut(),
        verify: std::ptr::null_mut(),
        append: std::ptr::null_mut(),
        close: std::ptr::null_mut(),
        eject: std::ptr::null_mut(),
        status: std::ptr::null_mut(),
        bar: std::ptr::null_mut(),
        burn: std::ptr::null_mut(),
        cancel: std::ptr::null_mut(),
        shared: JobShared::new(),
    });
    create_window(owner, class_of(mode), title_of(mode), 664, 470, Box::into_raw(state).cast());
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
                let prompt = match s.mode {
                    Mode::Files => "Folder to burn (its contents become the disc):",
                    Mode::Mp3 => "Folder with audio files (MP3 disc, playlist generated):",
                    Mode::Video => "DVD-video folder (must contain VIDEO_TS):",
                };
                label(hwnd, 16, 14, 500, prompt);
                s.folder = edit(hwnd, ID_FOLDER, 16, 34, 500, "", false);
                button(hwnd, ID_BROWSE, 524, 32, 120, "Browse…");
                label(hwnd, 16, 66, 120, "Disc label:");
                let default_label = match s.mode {
                    Mode::Files => "",
                    Mode::Mp3 => "MP3 DISC",
                    Mode::Video => "DVD VIDEO",
                };
                s.label_edit = edit(hwnd, ID_LABEL, 16, 86, 240, default_label, false);
                label(hwnd, 276, 66, 200, "Recorder:");
                s.drives.create(hwnd, ID_DRIVES, 276, 86, 368, 66);
                button(hwnd, ID_REFRESH, 140, 120, 120, "Refresh drives");
                s.media = label(hwnd, 16, 158, 628, "Media: —");
                set_text(s.media, &format!("Media: {}", s.drives.media_summary()));
                s.verify = checkbox(hwnd, ID_VERIFY, 16, 186, 300, "Verify after burn");
                set_checked(s.verify, true);
                s.append = checkbox(hwnd, ID_APPEND, 16, 210, 300, "Append to the disc (new session)");
                s.close = checkbox(hwnd, ID_CLOSE, 332, 186, 300, "Close the disc (no more sessions)");
                s.eject = checkbox(hwnd, ID_EJECT, 332, 210, 300, "Eject when done");
                s.bar = progress_bar(hwnd, 16, 244, 628);
                s.status = label(hwnd, 16, 268, 628, "Ready.");
                s.burn = button(hwnd, ID_BURN, 404, 298, 120, "Burn");
                s.cancel = button(hwnd, ID_CANCEL, 532, 298, 112, "Cancel");
                set_enabled(s.cancel, false);
                0
            }
            WM_COMMAND => {
                let s = st(hwnd);
                match ((wp & 0xFFFF) as u16, (wp >> 16) as u16) {
                    (ID_BROWSE, _) => {
                        if let Some(p) = pick_folder(hwnd, "Choose a folder") {
                            set_text(s.folder, &p.to_string_lossy());
                        }
                        0
                    }
                    (ID_REFRESH, _) => {
                        s.drives.refresh();
                        set_text(s.media, &format!("Media: {}", s.drives.media_summary()));
                        0
                    }
                    (ID_DRIVES, 1) => {
                        set_text(s.media, &format!("Media: {}", s.drives.media_summary()));
                        0
                    }
                    (ID_BURN, _) => {
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
                set_enabled(s.burn, true);
                set_enabled(s.cancel, false);
                match s.shared.take_result() {
                    Some(Ok(text)) => {
                        progress_show(s.bar, Some(1.0));
                        set_text(s.status, &text);
                        crate::win::gui::note_log(&text, false);
                    }
                    Some(Err(e)) => {
                        set_text(s.status, &format!("Failed: {e}"));
                        crate::win::gui::note_log(&format!("Burn failed: {e}"), true);
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
                    // SAFETY: created with Box::into_raw in `open`.
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
    let folder = get_text(s.folder);
    let root = PathBuf::from(&folder);
    if folder.is_empty() || !root.is_dir() {
        set_text(s.status, "Choose a folder first.");
        return;
    }
    if s.mode == Mode::Video && !root.join("VIDEO_TS").join("VIDEO_TS.IFO").is_file() {
        set_text(s.status, "Not a DVD-video folder: VIDEO_TS/VIDEO_TS.IFO is missing.");
        return;
    }
    let Some(info) = s.drives.selected() else {
        set_text(s.status, "No drive selected.");
        return;
    };
    let mut label = get_text(s.label_edit);
    if label.trim().is_empty() {
        label =
            root.file_name().map(|n| n.to_string_lossy().to_uppercase()).unwrap_or_else(|| "ISOFOUNDRY".to_string());
    }
    let opts = if_drive::BurnOptions {
        simulate: false,
        verify: is_checked(s.verify),
        close_disc: is_checked(s.close),
        append: is_checked(s.append),
        speed_kbps: None,
        eject_when_done: is_checked(s.eject),
    };
    let mode = s.mode;
    set_enabled(s.burn, false);
    set_enabled(s.cancel, true);
    set_text(s.status, "Burning…");
    let shared = s.shared.clone();
    run_job(hwnd, &shared, move |sh| {
        // MP3 mode stages a playlist folder first (CLI parity).
        let (source_root, staged) = if mode == Mode::Mp3 {
            let stage = crate::cli_burn::stage_mp3(&root)?;
            (stage.clone(), Some(stage))
        } else {
            (root.clone(), None)
        };
        let source = if_drive::BurnSource::Files { root: source_root, label };
        let mut session = if_drive::open(&info).map_err(|e| e.to_string())?;
        let result = session.burn(&source, &opts, &sh.progress(), &sh.cancel).map_err(|e| e.to_string());
        if let Some(stage) = staged {
            let _ = std::fs::remove_dir_all(stage);
        }
        let rep = result?;
        let mut msg = format!("Done: {} bytes written via {}.", rep.bytes_written, rep.backend);
        if rep.verified {
            msg.push_str(" Verified.");
        }
        for n in &rep.notes {
            msg.push_str(&format!(" {n}"));
        }
        Ok(msg)
    });
}
