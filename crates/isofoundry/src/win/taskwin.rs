//! Shared scaffolding for the task windows: one window class per task, a
//! state pointer in `GWLP_USERDATA`, controls made in code (no resources),
//! file/folder pickers and a worker-thread job runner with the shared
//! [`Progress`]/[`CancelToken`] pair from `if-core`. A job reports through
//! messages only; the worker never touches a window.
//!
//! Layout numbers in the task modules are 96-DPI units, scaled with
//! [`crate::layout::px`] like the main window.

use super::gui;
use super::util::{from_wide, wide};
use crate::layout::px;
use crate::theme::{self, colorref};
use if_core::job::{CancelToken, Progress};
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows_sys::Win32::Graphics::Gdi::{CreateSolidBrush, SetBkColor, SetTextColor, HDC, HFONT};
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, GetSaveFileNameW, OFN_ALLOWMULTISELECT, OFN_EXPLORER, OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR,
    OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// Posted to the task window when the worker has stored its result.
pub(crate) const WM_JOB_DONE: u32 = WM_APP + 2;
/// Timer that polls the shared progress while a job runs.
pub(crate) const TIMER_JOB: usize = 7;

/// Window and fonts for one task window, snapshotted at creation.
#[derive(Clone, Copy)]
pub struct Ctx {
    pub dpi: u32,
    pub label: HFONT,
    pub mono: HFONT,
}

pub(crate) fn ctx() -> Ctx {
    let (dpi, label, mono) = gui::ui_metrics();
    Ctx { dpi, label, mono }
}

pub(crate) fn instance() -> windows_sys::Win32::Foundation::HINSTANCE {
    gui::instance()
}

// ---------------------------------------------------------------- jobs

/// State shared between a task window and its worker thread.
pub struct JobShared {
    progress: Progress,
    pub cancel: CancelToken,
    result: Mutex<Option<Result<String, String>>>,
    last_done: Mutex<u64>,
}

impl JobShared {
    /// The shared progress counters (clone; the atomics are shared).
    pub fn progress(&self) -> Progress {
        self.progress.clone()
    }

    pub(crate) fn new() -> Arc<JobShared> {
        Arc::new(JobShared {
            progress: Progress::default(),
            cancel: CancelToken::new(),
            result: Mutex::new(None),
            last_done: Mutex::new(0),
        })
    }

    fn note(&self, done: u64, total: u64) -> bool {
        if total > 0 {
            self.progress.set_total(total);
        }
        let mut last = self.last_done.lock().unwrap();
        if done > *last {
            self.progress.add(done - *last);
            *last = done;
        }
        !self.cancel.is_cancelled()
    }

    /// `(done, total)` adapter for the `if-image` operations. Owns a clone,
    /// so it satisfies the `+ 'static` bounds on the writers.
    pub(crate) fn cb(self: &Arc<JobShared>) -> impl FnMut(u64, u64) -> bool {
        let s = self.clone();
        move |done, total| s.note(done, total)
    }

    /// Cumulative-bytes adapter for `if-hash` (its callback returns false
    /// to cancel, like the others).
    pub(crate) fn cb_bytes(self: &Arc<JobShared>, total: u64) -> impl FnMut(u64) -> bool {
        let s = self.clone();
        move |done| s.note(done, total)
    }

    pub(crate) fn snapshot(&self) -> (u64, u64) {
        self.progress.snapshot()
    }

    pub(crate) fn fraction(&self) -> Option<f32> {
        self.progress.fraction()
    }

    fn finish(&self, r: Result<String, String>) {
        *self.result.lock().unwrap() = Some(r);
    }

    pub(crate) fn take_result(&self) -> Option<Result<String, String>> {
        self.result.lock().unwrap().take()
    }
}

/// Runs `work` on a worker thread; posts [`WM_JOB_DONE`] when it returns.
pub(crate) fn run_job(
    hwnd: HWND,
    shared: &Arc<JobShared>,
    work: impl FnOnce(Arc<JobShared>) -> Result<String, String> + Send + 'static,
) {
    let shared = shared.clone();
    let target = hwnd as usize;
    std::thread::spawn(move || {
        let r = work(shared.clone());
        shared.finish(r);
        // SAFETY: the window outlives the job (closing cancels and waits
        // for this message first); `target` is the window we were given.
        unsafe { PostMessageW(target as HWND, WM_JOB_DONE, 0, 0) };
    });
    // SAFETY: plain timer on our own window.
    unsafe { SetTimer(hwnd, TIMER_JOB, 100, None) };
}

/// Stops the progress timer; called from the job-done handler.
pub(crate) fn stop_timer(hwnd: HWND) {
    // SAFETY: our own window and timer id.
    unsafe { KillTimer(hwnd, TIMER_JOB) };
}

/// "12.5 MB of 700 MB (2%)" / "12.5 MB" when the total is unknown.
pub(crate) fn progress_text(done: u64, total: u64) -> String {
    if total > 0 {
        let pct = (done as u128 * 100 / total as u128) as u64;
        format!("{} of {} ({pct}%)", fmt_bytes(done), fmt_bytes(total))
    } else if done > 0 {
        fmt_bytes(done)
    } else {
        String::new()
    }
}

pub(crate) fn fmt_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["bytes", "KB", "MB", "GB", "TB"];
    if n < 1024 {
        return format!("{n} bytes");
    }
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if v >= 100.0 {
        format!("{v:.0} {}", UNITS[u])
    } else {
        format!("{v:.1} {}", UNITS[u])
    }
}

// ---------------------------------------------------------------- window

/// Registers a task-window class (idempotent) with the dark surface brush.
pub(crate) fn register_class(name: &str, proc: WNDPROC) {
    let cls = wide(name);
    // SAFETY: standard class registration; a duplicate name just fails.
    unsafe {
        let wc = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: proc,
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: instance(),
            hIcon: std::ptr::null_mut(),
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hbrBackground: CreateSolidBrush(colorref(theme::SURFACE)),
            lpszMenuName: std::ptr::null(),
            lpszClassName: cls.as_ptr(),
        };
        RegisterClassW(&wc);
    }
}

/// Creates a visible task window centred on `owner`, with `state` as the
/// creation parameter (the wndproc stores it in `GWLP_USERDATA`).
pub(crate) fn create_window(owner: HWND, class: &str, title: &str, w: i32, h: i32, state: *mut c_void) -> HWND {
    let dpi = ctx().dpi;
    let (mut x, mut y) = (CW_USEDEFAULT, CW_USEDEFAULT);
    // SAFETY: reads the owner's rect to centre on it.
    unsafe {
        let mut r: windows_sys::Win32::Foundation::RECT = std::mem::zeroed();
        if GetWindowRect(owner, &mut r) != 0 {
            x = r.left + ((r.right - r.left) - px(w, dpi)) / 2;
            y = r.top + ((r.bottom - r.top) - px(h, dpi)) / 2;
        }
        let cls = wide(class);
        let t = wide(title);
        let hwnd = CreateWindowExW(
            0,
            cls.as_ptr(),
            t.as_ptr(),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX | WS_VISIBLE,
            x,
            y,
            px(w, dpi),
            px(h, dpi),
            owner,
            std::ptr::null_mut(),
            instance(),
            state,
        );
        if !hwnd.is_null() {
            let dark: i32 = 1;
            DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE as u32, (&dark as *const i32).cast(), 4);
            let icc = INITCOMMONCONTROLSEX {
                dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
                dwICC: ICC_PROGRESS_CLASS,
            };
            InitCommonControlsEx(&icc);
        }
        hwnd
    }
}

/// Stores the creation state in `GWLP_USERDATA`; call from `WM_NCCREATE`.
pub(crate) fn attach<T>(hwnd: HWND, lp: LPARAM) -> *mut T {
    // SAFETY: `lp` is the CREATESTRUCTW of WM_NCCREATE; its create params
    // are the pointer the module passed to `create_window`.
    unsafe {
        let cs = &*(lp as *const CREATESTRUCTW);
        let p = cs.lpCreateParams as *mut T;
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, p as isize);
        p
    }
}

/// The state pointer stored by [`attach`]; null before `WM_NCCREATE`.
pub(crate) fn state_ptr<T>(hwnd: HWND) -> *mut T {
    // SAFETY: we only ever store our own state pointer here.
    unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut T }
}

/// Dark-theme colours for child controls (same handling as the main
/// window's text view); call from `WM_CTLCOLOREDIT`/`WM_CTLCOLORSTATIC`/
/// `WM_CTLCOLORLISTBOX` and return the result.
pub(crate) fn ctl_color(wp: WPARAM) -> LRESULT {
    let p = gui::palette();
    let dc = wp as HDC;
    // SAFETY: `wp` is the control's DC during the colour messages.
    unsafe {
        SetTextColor(dc, colorref(p.text));
        SetBkColor(dc, colorref(p.surface));
        static BRUSH: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
        *BRUSH.get_or_init(|| CreateSolidBrush(colorref(theme::SURFACE)) as usize) as LRESULT
    }
}

/// Puts a themed control on the dark Explorer theme (dark checkboxes,
/// radios, scrollbars and progress track on Windows 10 1903+). Without it
/// the standard controls paint with the light theme, whose label text is a
/// low-contrast gray on our dark background.
pub(crate) fn dark_theme(h: HWND) {
    static NAME: std::sync::OnceLock<Vec<u16>> = std::sync::OnceLock::new();
    let name = NAME.get_or_init(|| wide("DarkMode_Explorer"));
    // SAFETY: a theme name for our own control; failure just keeps the
    // default theme.
    unsafe { SetWindowTheme(h, name.as_ptr(), std::ptr::null()) };
}

// ---------------------------------------------------------------- controls

#[allow(clippy::too_many_arguments)] // mirrors CreateWindowExW's parameter list
fn child(
    parent: HWND,
    class: &str,
    text: &str,
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    id: u16,
    font: HFONT,
) -> HWND {
    let c = ctx();
    let cls = wide(class);
    let t = wide(text);
    // SAFETY: creates a child of our own window; strings outlive the call.
    unsafe {
        let hwnd = CreateWindowExW(
            0,
            cls.as_ptr(),
            t.as_ptr(),
            WS_CHILD | WS_VISIBLE | style,
            px(x, c.dpi),
            px(y, c.dpi),
            px(w, c.dpi),
            px(h, c.dpi),
            parent,
            id as HMENU,
            instance(),
            std::ptr::null(),
        );
        if !font.is_null() {
            SendMessageW(hwnd, WM_SETFONT, font as usize, 1);
        }
        dark_theme(hwnd);
        hwnd
    }
}

pub(crate) fn label(parent: HWND, x: i32, y: i32, w: i32, text: &str) -> HWND {
    let f = ctx().label;
    child(parent, "STATIC", text, 0, x, y, w, 18, 0, f) // style 0 = SS_LEFT
}

pub(crate) fn edit(parent: HWND, id: u16, x: i32, y: i32, w: i32, text: &str, mono: bool) -> HWND {
    let c = ctx();
    child(
        parent,
        "EDIT",
        text,
        WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
        x,
        y,
        w,
        23,
        id,
        if mono { c.mono } else { c.label },
    )
}

pub(crate) fn multiline(parent: HWND, id: u16, x: i32, y: i32, w: i32, h: i32) -> HWND {
    let c = ctx();
    child(
        parent,
        "EDIT",
        "",
        WS_TABSTOP | WS_BORDER | WS_VSCROLL | ES_MULTILINE as u32 | ES_READONLY as u32 | ES_AUTOVSCROLL as u32,
        x,
        y,
        w,
        h,
        id,
        c.mono,
    )
}

pub(crate) fn button(parent: HWND, id: u16, x: i32, y: i32, w: i32, text: &str) -> HWND {
    let f = ctx().label;
    child(parent, "BUTTON", text, WS_TABSTOP | BS_PUSHBUTTON as u32, x, y, w, 26, id, f)
}

pub(crate) fn checkbox(parent: HWND, id: u16, x: i32, y: i32, w: i32, text: &str) -> HWND {
    let f = ctx().label;
    child(parent, "BUTTON", text, WS_TABSTOP | BS_AUTOCHECKBOX as u32, x, y, w, 20, id, f)
}

/// Auto radio button; `first` starts a new radio group.
pub(crate) fn radio(parent: HWND, id: u16, x: i32, y: i32, w: i32, text: &str, first: bool) -> HWND {
    let f = ctx().label;
    let style = WS_TABSTOP | BS_AUTORADIOBUTTON as u32 | if first { WS_GROUP } else { 0 };
    child(parent, "BUTTON", text, style, x, y, w, 20, id, f)
}

pub(crate) fn listbox(parent: HWND, id: u16, x: i32, y: i32, w: i32, h: i32) -> HWND {
    let c = ctx();
    child(
        parent,
        "LISTBOX",
        "",
        WS_TABSTOP | WS_BORDER | WS_VSCROLL | WS_HSCROLL | LBS_NOTIFY as u32 | LBS_EXTENDEDSEL as u32,
        x,
        y,
        w,
        h,
        id,
        c.mono,
    )
}

pub(crate) fn progress_bar(parent: HWND, x: i32, y: i32, w: i32) -> HWND {
    let c = ctx();
    let cls = wide("msctls_progress32");
    // SAFETY: common-control class registered by `create_window`.
    unsafe {
        let hwnd = CreateWindowExW(
            0,
            cls.as_ptr(),
            std::ptr::null(),
            WS_CHILD | WS_VISIBLE | 0x08, // PBS_MARQUEE
            px(x, c.dpi),
            px(y, c.dpi),
            px(w, c.dpi),
            px(14, c.dpi),
            parent,
            std::ptr::null_mut(),
            instance(),
            std::ptr::null(),
        );
        SendMessageW(hwnd, PBM_SETRANGE32, 0, 1000);
        dark_theme(hwnd);
        hwnd
    }
}

/// Shows determinate progress (`Some`) or the marquee (`None`).
pub(crate) fn progress_show(bar: HWND, frac: Option<f32>) {
    // SAFETY: progress-bar messages on our own control.
    unsafe {
        match frac {
            Some(f) => {
                SendMessageW(bar, PBM_SETMARQUEE, 0, 0);
                SendMessageW(bar, PBM_SETPOS, (f.clamp(0.0, 1.0) * 1000.0) as usize, 0);
            }
            None => {
                SendMessageW(bar, PBM_SETMARQUEE, 1, 0);
            }
        }
    }
}

pub(crate) fn set_text(hwnd: HWND, text: &str) {
    let t = wide(text);
    // SAFETY: our own control; the string outlives the call.
    unsafe { SetWindowTextW(hwnd, t.as_ptr()) };
}

pub(crate) fn get_text(hwnd: HWND) -> String {
    // SAFETY: reads our own control's text into a bounded buffer.
    unsafe {
        let n = GetWindowTextLengthW(hwnd);
        let mut buf = vec![0u16; (n + 1) as usize];
        let got = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        from_wide(&buf[..got.max(0) as usize])
    }
}

/// Appends one line to a read-only multiline edit (the job output area).
pub(crate) fn append_line(edit: HWND, line: &str) {
    // SAFETY: standard append idiom on our own control.
    unsafe {
        let n = SendMessageW(edit, WM_GETTEXTLENGTH, 0, 0);
        SendMessageW(edit, EM_SETSEL, n as usize, n as isize);
        let t = wide(&format!("{line}\r\n"));
        SendMessageW(edit, EM_REPLACESEL, 0, t.as_ptr() as isize);
    }
}

pub(crate) fn set_enabled(hwnd: HWND, on: bool) {
    // SAFETY: our own control.
    unsafe { EnableWindow(hwnd, on as i32) };
}

pub(crate) fn is_checked(hwnd: HWND) -> bool {
    // SAFETY: our own checkbox.
    unsafe { SendMessageW(hwnd, BM_GETCHECK, 0, 0) == BST_CHECKED as LRESULT }
}

pub(crate) fn set_checked(hwnd: HWND, on: bool) {
    // SAFETY: our own checkbox.
    unsafe { SendMessageW(hwnd, BM_SETCHECK, if on { BST_CHECKED } else { BST_UNCHECKED } as usize, 0) };
}

pub(crate) fn list_add(list: HWND, text: &str) {
    let t = wide(text);
    // SAFETY: our own list box; the string is copied by the control.
    unsafe { SendMessageW(list, LB_ADDSTRING, 0, t.as_ptr() as isize) };
}

pub(crate) fn list_clear(list: HWND) {
    // SAFETY: our own list box.
    unsafe { SendMessageW(list, LB_RESETCONTENT, 0, 0) };
}

/// Selected indices of a (possibly extended-selection) list box.
pub(crate) fn list_selected(list: HWND) -> Vec<usize> {
    // SAFETY: standard list-box queries on our own control.
    unsafe {
        let n = SendMessageW(list, LB_GETSELCOUNT, 0, 0);
        if n <= 0 {
            let one = SendMessageW(list, LB_GETCURSEL, 0, 0);
            return if one >= 0 { vec![one as usize] } else { Vec::new() };
        }
        let mut buf = vec![0i32; n as usize];
        let got = SendMessageW(list, LB_GETSELITEMS, n as usize, buf.as_mut_ptr() as isize);
        buf[..got.max(0) as usize].iter().map(|&i| i as usize).collect()
    }
}

// ---------------------------------------------------------------- pickers

fn filter_wide(filter: &str) -> Vec<u16> {
    // "Label|*.iso;*.cue|All files|*.*" → NUL-separated pairs + final NUL.
    let mut v: Vec<u16> = filter.encode_utf16().map(|c| if c == '|' as u16 { 0 } else { c }).collect();
    v.push(0);
    v.push(0);
    v
}

/// Builds the dialog struct; `title`/`filter`/`def_ext` are caller-owned
/// buffers that must outlive the modal call (they do: same function).
fn ofn(owner: HWND, title: &[u16], filter: &[u16], buf: &mut [u16], def_ext: Option<&[u16]>) -> OPENFILENAMEW {
    // SAFETY: zeroed struct, every used field set before the call.
    let mut o: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    o.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
    o.hwndOwner = owner;
    o.lpstrFilter = filter.as_ptr();
    o.lpstrFile = buf.as_mut_ptr();
    o.nMaxFile = buf.len() as u32;
    o.lpstrTitle = title.as_ptr();
    o.lpstrDefExt = def_ext.map(|e| e.as_ptr()).unwrap_or(std::ptr::null());
    o.Flags = OFN_NOCHANGEDIR | OFN_PATHMUSTEXIST;
    o
}

fn put_initial(buf: &mut [u16], name: &str) {
    for (i, c) in name.encode_utf16().take(buf.len() - 1).enumerate() {
        buf[i] = c;
    }
}

/// Single existing file, or `None` when the user cancels.
pub(crate) fn pick_open(owner: HWND, title: &str, filter: &str) -> Option<PathBuf> {
    let f = filter_wide(filter);
    let t = wide(title);
    let mut buf = vec![0u16; 4096];
    let mut o = ofn(owner, &t, &f, &mut buf, None);
    o.Flags |= OFN_FILEMUSTEXIST;
    // SAFETY: buffers outlive the modal call.
    if unsafe { GetOpenFileNameW(&mut o) } == 0 {
        return None;
    }
    Some(PathBuf::from(from_wide(&buf)))
}

/// Several existing files (multi-select), or an empty vec on cancel.
pub(crate) fn pick_open_multi(owner: HWND, title: &str, filter: &str) -> Vec<PathBuf> {
    let f = filter_wide(filter);
    let t = wide(title);
    let mut buf = vec![0u16; 32768];
    let mut o = ofn(owner, &t, &f, &mut buf, None);
    o.Flags |= OFN_FILEMUSTEXIST | OFN_ALLOWMULTISELECT | OFN_EXPLORER;
    // SAFETY: buffers outlive the modal call.
    if unsafe { GetOpenFileNameW(&mut o) } == 0 {
        return Vec::new();
    }
    // Multi-select returns "dir\0file1\0file2\0\0"; a single pick is one path.
    let parts: Vec<String> = {
        let mut out = Vec::new();
        let mut start = 0;
        for i in 0..buf.len() {
            if buf[i] == 0 {
                if i == start {
                    break;
                }
                out.push(from_wide(&buf[start..i]));
                start = i + 1;
            }
        }
        out
    };
    match parts.as_slice() {
        [] => Vec::new(),
        [one] => vec![PathBuf::from(one)],
        [dir, names @ ..] => names.iter().map(|n| PathBuf::from(dir).join(n)).collect(),
    }
}

/// Save path (asks before overwriting), prefilled with `initial`.
pub(crate) fn pick_save(owner: HWND, title: &str, filter: &str, def_ext: &str, initial: &str) -> Option<PathBuf> {
    let f = filter_wide(filter);
    let t = wide(title);
    let e = wide(def_ext);
    let mut buf = vec![0u16; 4096];
    put_initial(&mut buf, initial);
    let mut o = ofn(owner, &t, &f, &mut buf, Some(&e));
    o.Flags |= OFN_OVERWRITEPROMPT;
    // SAFETY: buffers outlive the modal call.
    if unsafe { GetSaveFileNameW(&mut o) } == 0 {
        return None;
    }
    Some(PathBuf::from(from_wide(&buf)))
}

/// Folder via the shell browse dialog, or `None` on cancel.
pub(crate) fn pick_folder(owner: HWND, title: &str) -> Option<PathBuf> {
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{
        SHBrowseForFolderW, SHGetPathFromIDListW, BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW,
    };
    let t = wide(title);
    let mut display = vec![0u16; 260];
    // SAFETY: zeroed browse info pointing at our buffers; the returned
    // item-id list is freed with CoTaskMemFree.
    unsafe {
        let mut bi: BROWSEINFOW = std::mem::zeroed();
        bi.hwndOwner = owner;
        bi.pszDisplayName = display.as_mut_ptr();
        bi.lpszTitle = t.as_ptr();
        bi.ulFlags = BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE;
        let pidl = SHBrowseForFolderW(&bi);
        if pidl.is_null() {
            return None;
        }
        let mut buf = vec![0u16; 4096];
        let ok = SHGetPathFromIDListW(pidl, buf.as_mut_ptr());
        CoTaskMemFree(pidl as *const c_void);
        if ok == 0 {
            return None;
        }
        Some(PathBuf::from(from_wide(&buf)))
    }
}
