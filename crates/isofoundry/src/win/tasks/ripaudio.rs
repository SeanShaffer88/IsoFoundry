//! Rip audio CD window (M5): read the track list, rip securely
//! (every sector read twice and compared) to WAV / FLAC / MP3,
//! with AccurateRip checksums in the rip log and an optional CUE.

use super::super::taskwin::*;
use super::drivepick::DrivePane;
use std::path::PathBuf;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "IsoFoundryTaskRipAudio";
const ID_DRIVES: u16 = 1;
const ID_REFRESH: u16 = 2;
const ID_TRACKS: u16 = 3;
const ID_TOC: u16 = 4;
const ID_OUT: u16 = 5;
const ID_BROWSE: u16 = 6;
const ID_FMT_WAV: u16 = 7;
const ID_FMT_FLAC: u16 = 8;
const ID_FMT_MP3: u16 = 9;
const ID_SECURE: u16 = 10;
const ID_CUE: u16 = 11;
const ID_GO: u16 = 12;
const ID_CANCEL: u16 = 13;

struct State {
    drives: DrivePane,
    tracks: HWND,
    out: HWND,
    fmt_wav: HWND,
    fmt_flac: HWND,
    fmt_mp3: HWND,
    secure: HWND,
    cue: HWND,
    status: HWND,
    bar: HWND,
    go: HWND,
    cancel: HWND,
    shared: std::sync::Arc<JobShared>,
}

pub fn open(owner: HWND) {
    register_class(CLASS, Some(wndproc));
    let state = Box::new(State {
        drives: DrivePane::empty(),
        tracks: std::ptr::null_mut(),
        out: std::ptr::null_mut(),
        fmt_wav: std::ptr::null_mut(),
        fmt_flac: std::ptr::null_mut(),
        fmt_mp3: std::ptr::null_mut(),
        secure: std::ptr::null_mut(),
        cue: std::ptr::null_mut(),
        status: std::ptr::null_mut(),
        bar: std::ptr::null_mut(),
        go: std::ptr::null_mut(),
        cancel: std::ptr::null_mut(),
        shared: JobShared::new(),
    });
    create_window(owner, CLASS, "Rip audio CD", 664, 520, Box::into_raw(state).cast());
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
                label(hwnd, 16, 14, 200, "Drive:");
                s.drives.create(hwnd, ID_DRIVES, 16, 34, 500, 60);
                button(hwnd, ID_REFRESH, 524, 34, 120, "Refresh");
                button(hwnd, ID_TOC, 524, 66, 120, "Read tracks");
                label(hwnd, 16, 102, 300, "Tracks:");
                s.tracks = listbox(hwnd, ID_TRACKS, 16, 122, 628, 110);
                label(hwnd, 16, 240, 200, "Output folder:");
                s.out = edit(hwnd, ID_OUT, 16, 260, 500, "", false);
                button(hwnd, ID_BROWSE, 524, 258, 120, "Browse…");
                s.fmt_wav = radio(hwnd, ID_FMT_WAV, 16, 292, 90, "WAV", true);
                s.fmt_flac = radio(hwnd, ID_FMT_FLAC, 116, 292, 90, "FLAC", false);
                s.fmt_mp3 = radio(hwnd, ID_FMT_MP3, 216, 292, 90, "MP3", false);
                s.secure = checkbox(hwnd, ID_SECURE, 332, 292, 200, "Secure rip (read twice)");
                set_checked(s.secure, true);
                s.cue = checkbox(hwnd, ID_CUE, 332, 316, 200, "Write CUE sheet (WAV)");
                s.bar = progress_bar(hwnd, 16, 346, 628);
                s.status = label(hwnd, 16, 370, 628, "Ready. The test drive holds a small generated audio CD.");
                s.go = button(hwnd, ID_GO, 404, 402, 120, "Rip");
                s.cancel = button(hwnd, ID_CANCEL, 532, 402, 112, "Cancel");
                set_enabled(s.cancel, false);
                0
            }
            WM_COMMAND => {
                let s = st(hwnd);
                match ((wp & 0xFFFF) as u16, (wp >> 16) as u16) {
                    (ID_BROWSE, _) => {
                        if let Some(p) = pick_folder(hwnd, "Choose the output folder") {
                            set_text(s.out, &p.to_string_lossy());
                        }
                        0
                    }
                    (ID_REFRESH, _) => {
                        s.drives.refresh();
                        0
                    }
                    (ID_TOC, _) => {
                        read_tracks(s);
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
                        crate::win::gui::note_log(&format!("Rip audio failed: {e}"), true);
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

fn open_drive(info: &if_drive::DriveInfo) -> Result<Box<dyn if_drive::OpticalDrive>, String> {
    if info.id == "fake" {
        Ok(Box::new(if_drive::fake_source_audio()))
    } else {
        if_drive::open(info).map_err(|e| e.to_string())
    }
}

fn read_tracks(s: &mut State) {
    let Some(info) = s.drives.selected() else {
        set_text(s.status, "No drive selected.");
        return;
    };
    match open_drive(&info).and_then(|d| d.toc().map_err(|e| e.to_string())) {
        Ok(toc) => {
            list_clear(s.tracks);
            for t in &toc.tracks {
                let (m, sec, _) = if_drive::Toc::msf(t.start_lba);
                let len = t.sectors / 75;
                list_add(
                    s.tracks,
                    &format!(
                        "Track {:02} — {}:{:02} ({} sectors){}",
                        t.number,
                        len / 60,
                        len % 60,
                        t.sectors,
                        t.title.as_deref().map(|x| format!(" — {x}")).unwrap_or_default()
                    ),
                );
                let _ = (m, sec);
            }
            set_text(s.status, &format!("{} tracks found.", toc.tracks.len()));
        }
        Err(e) => set_text(s.status, &format!("Could not read the track list: {e}")),
    }
}

fn start(s: &mut State, hwnd: HWND) {
    let out = get_text(s.out);
    if out.is_empty() {
        set_text(s.status, "Choose an output folder first.");
        return;
    }
    let Some(info) = s.drives.selected() else {
        set_text(s.status, "No drive selected.");
        return;
    };
    let format = if is_checked(s.fmt_flac) {
        "flac"
    } else if is_checked(s.fmt_mp3) {
        "mp3"
    } else {
        "wav"
    }
    .to_string();
    let secure = is_checked(s.secure);
    let cue = is_checked(s.cue);
    let out_dir = PathBuf::from(out);
    set_enabled(s.go, false);
    set_enabled(s.cancel, true);
    set_text(s.status, "Ripping…");
    let shared = s.shared.clone();
    run_job(hwnd, &shared, move |sh| {
        let mut drive = open_drive(&info)?;
        let desc = info.display_name();
        let opts = if_drive::rip::RipAudioOptions { secure, retries: 3 };
        let tracks =
            if_drive::rip::rip_audio(&mut *drive, &opts, &sh.progress(), &sh.cancel).map_err(|e| e.to_string())?;
        let bad: usize = tracks.iter().map(|t| t.bad_sectors.len()).sum();
        let written = crate::cli_rip::write_ripped_tracks(&tracks, &out_dir, &format, cue, false, &desc)?;
        Ok(format!(
            "Ripped {} tracks to {}.{}",
            written.len(),
            out_dir.display(),
            if bad > 0 {
                format!(" {bad} sectors were unreadable (silenced, listed in rip.log).")
            } else {
                String::new()
            }
        ))
    });
}
