//! Burn audio CD window (M5): add audio files, set the gap, and
//! burn. The simulated drive writes disc-at-once with CD-Text;
//! IMAPI2 drives write track-at-once (see the note in the window).

use super::super::taskwin::*;
use super::drivepick::DrivePane;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "IsoFoundryTaskBurnAudio";
const ID_FILES: u16 = 1;
const ID_ADD: u16 = 2;
const ID_REMOVE: u16 = 3;
const ID_DRIVES: u16 = 4;
const ID_REFRESH: u16 = 5;
const ID_GAP: u16 = 6;
const ID_SIMULATE: u16 = 7;
const ID_GO: u16 = 8;
const ID_CANCEL: u16 = 9;

struct State {
    files: HWND,
    paths: Vec<PathBuf>,
    drives: DrivePane,
    media: HWND,
    gap: HWND,
    simulate: HWND,
    status: HWND,
    bar: HWND,
    go: HWND,
    cancel: HWND,
    shared: std::sync::Arc<JobShared>,
}

pub fn open(owner: HWND) {
    register_class(CLASS, Some(wndproc));
    let state = Box::new(State {
        files: std::ptr::null_mut(),
        paths: Vec::new(),
        drives: DrivePane::empty(),
        media: std::ptr::null_mut(),
        gap: std::ptr::null_mut(),
        simulate: std::ptr::null_mut(),
        status: std::ptr::null_mut(),
        bar: std::ptr::null_mut(),
        go: std::ptr::null_mut(),
        cancel: std::ptr::null_mut(),
        shared: JobShared::new(),
    });
    create_window(owner, CLASS, "Burn audio CD", 664, 500, Box::into_raw(state).cast());
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
                label(hwnd, 16, 14, 300, "Audio files (in track order):");
                s.files = listbox(hwnd, ID_FILES, 16, 34, 500, 120);
                button(hwnd, ID_ADD, 524, 34, 120, "Add files…");
                button(hwnd, ID_REMOVE, 524, 66, 120, "Remove");
                label(hwnd, 16, 162, 200, "Recorder:");
                s.drives.create(hwnd, ID_DRIVES, 16, 182, 500, 60);
                button(hwnd, ID_REFRESH, 524, 182, 120, "Refresh");
                s.media = label(hwnd, 16, 248, 628, "Media: —");
                set_text(s.media, &format!("Media: {}", s.drives.media_summary()));
                label(hwnd, 16, 276, 140, "Gap (seconds):");
                s.gap = edit(hwnd, ID_GAP, 160, 274, 60, "2", true);
                s.simulate = checkbox(hwnd, ID_SIMULATE, 332, 274, 300, "Simulate only (write nothing)");
                s.bar = progress_bar(hwnd, 16, 310, 628);
                s.status = label(hwnd, 16, 334, 628, "Ready. The test drive writes disc-at-once with CD-Text titles; real drives use track-at-once (2 s gaps, no CD-Text).");
                s.go = button(hwnd, ID_GO, 404, 380, 120, "Burn");
                s.cancel = button(hwnd, ID_CANCEL, 532, 380, 112, "Cancel");
                set_enabled(s.cancel, false);
                0
            }
            WM_COMMAND => {
                let s = st(hwnd);
                match ((wp & 0xFFFF) as u16, (wp >> 16) as u16) {
                    (ID_ADD, _) => {
                        let picked = pick_open_multi(
                            hwnd,
                            "Add audio files",
                            "Audio|*.wav;*.flac;*.mp3;*.ogg;*.m4a;*.aac|All files|*.*",
                        );
                        if !picked.is_empty() {
                            for p in picked {
                                list_add(
                                    s.files,
                                    &p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                                );
                                s.paths.push(p);
                            }
                        }
                        0
                    }
                    (ID_REMOVE, _) => {
                        if let Some(i) = list_selected(s.files).first().copied() {
                            if i < s.paths.len() {
                                s.paths.remove(i);
                            }
                            list_clear(s.files);
                            for p in &s.paths {
                                list_add(
                                    s.files,
                                    &p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                                );
                            }
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
                        crate::win::gui::note_log(&format!("Burn audio failed: {e}"), true);
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
    if s.paths.is_empty() {
        set_text(s.status, "Add at least one audio file first.");
        return;
    }
    let Some(info) = s.drives.selected() else {
        set_text(s.status, "No drive selected.");
        return;
    };
    let gap_sectors = get_text(s.gap).trim().parse::<u32>().unwrap_or(2) * 75;
    let simulate = is_checked(s.simulate);
    let paths = s.paths.clone();
    set_enabled(s.go, false);
    set_enabled(s.cancel, true);
    set_text(s.status, "Decoding and burning…");
    let shared = s.shared.clone();
    run_job(hwnd, &shared, move |sh| {
        let mut tracks = Vec::new();
        for p in &paths {
            let (pcm, title) = crate::cli_rip::decode_for_cd(p)?;
            tracks.push(if_drive::AudioTrackData { title, performer: String::new(), pcm, gap_sectors });
        }
        let mut drive = if_drive::open(&info).map_err(|e| e.to_string())?;
        let opts = if_drive::AudioBurnOptions { simulate, ..Default::default() };
        let rep = drive.burn_audio(&tracks, &opts, &sh.progress(), &sh.cancel).map_err(|e| e.to_string())?;
        let mut msg = format!("Burned {} tracks ({} bytes) via {}.", tracks.len(), rep.bytes_written, rep.backend);
        for n in &rep.notes {
            msg.push_str(&format!(" {n}"));
        }
        Ok(msg)
    });
}
