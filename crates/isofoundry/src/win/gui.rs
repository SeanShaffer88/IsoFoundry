//! The main window (D-006, D-043, D-045): a Win32 window with owner-drawn
//! task buttons laid out by [`crate::layout`] and painted from the design
//! tokens in [`crate::theme`].
//!
//! State lives in a thread-local and is only borrowed briefly; anything
//! that runs a nested message loop (message boxes, menus, file dialogs)
//! happens after the borrow is released.

use super::util::{from_wide, wide};
use crate::layout::{self, px, Layout, Rect};
use crate::theme::{self, colorref, Palette};
use if_core::log::Logger;
use if_core::paths::DataDirs;
use if_core::settings::{AutoAction, LogLevel, Settings, WindowPlacement};
use if_core::{strings, tasks};
use std::cell::RefCell;
use std::process::ExitCode;
use windows_sys::Win32::Foundation::{
    GetLastError, COLORREF, ERROR_ALREADY_EXISTS, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows_sys::Win32::UI::Controls::Dialogs::{
    GetSaveFileNameW, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForWindow};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Shell::{
    DefSubclassProc, SHGetStockIconInfo, SetWindowSubclass, ShellExecuteW, Shell_NotifyIconW, NIF_ICON, NIF_INFO,
    NIF_TIP, NIIF_ERROR, NIIF_INFO, NIM_ADD, NIM_DELETE, NIM_MODIFY, NOTIFYICONDATAW, SHGSI_ICON, SHGSI_SMALLICON,
    SHSTOCKICONINFO, SIID_SHIELD,
};
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const CLASS: &str = "IsoFoundryMain";
const TEXT_CLASS: &str = "IsoFoundryText";
/// Single-instance mutex (Directive V2.0 §12), GUID-prefixed, per session.
const MUTEX: &str = r"Local\{6B6E2A0C-3F7D-4C1E-9E5B-2F8A1D0C7E41}-IsoFoundry";

// Control and command ids.
const ID_MAIN: u16 = 1000; // + task index
const ID_ARROW: u16 = 1100; // + task index
const ID_MENU_TASK: u16 = 2000; // + task index
const ID_VARIANT: u16 = 3000; // + flat variant index
const IDM_EXIT: u16 = 100;
const IDM_OPEN_DATA: u16 = 101;
const IDM_LOG_BASE: u16 = 110; // + LogLevel index
const IDM_SOUND: u16 = 115;
const IDM_TOAST: u16 = 116;
const IDM_SHUTDOWN: u16 = 117;
const IDM_MB: u16 = 118;
const IDM_UPDATE: u16 = 119;
const IDM_AUTO_BASE: u16 = 130; // + AutoAction index (Off, Image, Rip, Both)
const TIMER_DISC: usize = 91;
const TIMER_TOAST: usize = 92;
/// Posted by the update-check thread with a heap `Box<String>` tag.
const WM_UPDATE_AVAILABLE: u32 = WM_APP + 22;
const IDM_VIEW_LOG: u16 = 120;
const IDM_ROADMAP: u16 = 121;
const IDM_REPORT: u16 = 122;
const IDM_CLI_HELP: u16 = 123;
const IDM_ABOUT: u16 = 124;

const LEVELS: [LogLevel; 4] = [LogLevel::Error, LogLevel::Warn, LogLevel::Info, LogLevel::Debug];

struct Fonts {
    button: HFONT,
    label: HFONT,
    logo: HFONT,
    mono: HFONT,
}

impl Fonts {
    fn new(dpi: u32) -> Fonts {
        Fonts {
            button: font(px(13, dpi), 600, "Segoe UI"),
            label: font(px(13, dpi), 400, "Segoe UI"),
            logo: font(px(36, dpi), 700, "Segoe UI"),
            mono: font(px(13, dpi), 400, "Consolas"),
        }
    }
}

impl Drop for Fonts {
    fn drop(&mut self) {
        for f in [self.button, self.label, self.logo, self.mono] {
            // SAFETY: fonts created by us, deselected from every DC before drop.
            unsafe { DeleteObject(f) };
        }
    }
}

fn font(height: i32, weight: i32, face: &str) -> HFONT {
    let face = wide(face);
    // SAFETY: plain GDI call with a NUL-terminated face name.
    unsafe {
        CreateFontW(
            -height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            CLEARTYPE_QUALITY as u32,
            (DEFAULT_PITCH as u32) | (FF_DONTCARE as u32),
            face.as_ptr(),
        )
    }
}

/// One owner-drawn control: the main part of a task or its split arrow.
#[derive(Clone, Copy)]
struct Control {
    hwnd: HWND,
    task: usize,
    arrow: bool,
}

struct App {
    hwnd: HWND,
    dirs: DataDirs,
    settings: Settings,
    log: Logger,
    dpi: u32,
    palette: Palette,
    fonts: Fonts,
    layout: Layout,
    controls: Vec<Control>,
    hot: Option<HWND>,
    focus_task: Option<usize>,
    shield: HICON,
    logo: HICON,
    logo_size: i32,
    /// Fingerprints of the discs seen at the last insert scan (M6).
    disc_seen: Vec<String>,
    /// A finish-notification balloon icon is currently added (M6).
    toast_icon: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.try_borrow_mut().ok().and_then(|mut g| g.as_mut().map(f)))
}

fn task_list() -> Vec<tasks::Task> {
    tasks::all().collect()
}

fn variant_list() -> Vec<(tasks::Task, tasks::Variant)> {
    tasks::all().flat_map(|t| t.variants.iter().map(move |v| (t, *v))).collect()
}

fn hinstance() -> HINSTANCE {
    // SAFETY: NULL asks for this executable's module handle.
    unsafe { GetModuleHandleW(std::ptr::null()) }
}

/// Accessors for the task windows (see `super::taskwin`): the module handle,
/// the main window's DPI and fonts, the current palette and the app log.
/// Fonts and the palette are read as raw values; the `Fonts` themselves stay
/// owned by `App`.
pub(crate) fn instance() -> HINSTANCE {
    hinstance()
}

pub(crate) fn ui_metrics() -> (u32, HFONT, HFONT) {
    with_app(|a| (a.dpi, a.fonts.label, a.fonts.mono)).unwrap_or((96, std::ptr::null_mut(), std::ptr::null_mut()))
}

pub(crate) fn palette() -> Palette {
    with_app(|a| a.palette).unwrap_or(Palette::DARK)
}

pub(crate) fn note_log(line: &str, error: bool) {
    with_app(|a| {
        if error {
            a.log.error(line.to_string());
        } else {
            a.log.info(line.to_string());
        }
    });
}

fn loword(v: usize) -> u16 {
    (v & 0xFFFF) as u16
}

fn hiword(v: usize) -> u16 {
    ((v >> 16) & 0xFFFF) as u16
}

fn rect(r: Rect) -> RECT {
    RECT { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
}

pub fn run(dirs: DataDirs, settings: Settings, log: Logger) -> ExitCode {
    // SAFETY: Win32 setup and message loop on this (the UI) thread. Every
    // pointer passed below outlives its call.
    unsafe {
        let mutex_name = wide(MUTEX);
        let _mutex = super::util::OwnedHandle(CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr()));
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let cls = wide(CLASS);
            let other = FindWindowW(cls.as_ptr(), std::ptr::null());
            if !other.is_null() {
                ShowWindow(other, SW_RESTORE);
                SetForegroundWindow(other);
            }
            log.info("another IsoFoundry window is open; activated it");
            return ExitCode::SUCCESS;
        }

        let icc = INITCOMMONCONTROLSEX {
            dwSize: std::mem::size_of::<INITCOMMONCONTROLSEX>() as u32,
            dwICC: ICC_STANDARD_CLASSES | ICC_WIN95_CLASSES,
        };
        InitCommonControlsEx(&icc);

        let hinst = hinstance();
        let icon = LoadIconW(hinst, 1 as _);
        let cls = wide(CLASS);
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            hIcon: icon,
            hIconSm: icon,
            hCursor: LoadCursorW(std::ptr::null_mut(), IDC_ARROW),
            hbrBackground: std::ptr::null_mut(),
            lpszClassName: cls.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassExW(&wc);
        let tcls = wide(TEXT_CLASS);
        let wc2 = WNDCLASSEXW { lpfnWndProc: Some(text_wndproc), lpszClassName: tcls.as_ptr(), ..wc };
        RegisterClassExW(&wc2);

        let title = wide(if_core::APP_NAME);
        let style = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN;
        let hwnd = CreateWindowExW(
            WS_EX_CONTROLPARENT,
            cls.as_ptr(),
            title.as_ptr(),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            std::ptr::null_mut(),
            build_menu(&settings),
            hinst,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            log.error(format!("cannot create the main window (Windows error {})", GetLastError()));
            return ExitCode::from(if_core::error::Exit::Generic.code() as u8);
        }
        let dark: i32 = 1;
        DwmSetWindowAttribute(hwnd, DWMWA_USE_IMMERSIVE_DARK_MODE as u32, (&dark as *const i32).cast(), 4);

        let dpi = GetDpiForWindow(hwnd).max(96);
        let mut shield_info: SHSTOCKICONINFO = std::mem::zeroed();
        shield_info.cbSize = std::mem::size_of::<SHSTOCKICONINFO>() as u32;
        let shield = if SHGetStockIconInfo(SIID_SHIELD, SHGSI_ICON | SHGSI_SMALLICON, &mut shield_info) >= 0 {
            shield_info.hIcon
        } else {
            std::ptr::null_mut()
        };
        let (cw, ch) = layout::default_client(dpi);
        let app = App {
            hwnd,
            dirs,
            settings,
            log,
            dpi,
            palette: current_palette(),
            fonts: Fonts::new(dpi),
            layout: layout::compute(cw, ch, dpi),
            controls: Vec::new(),
            hot: None,
            focus_task: None,
            shield,
            logo: std::ptr::null_mut(),
            logo_size: 0,
            disc_seen: Vec::new(),
            toast_icon: false,
        };
        APP.with(|a| *a.borrow_mut() = Some(app));
        create_controls(hwnd);
        place_window(hwnd);
        // Keyboard users start on the first task, as the focus ring shows.
        if let Some(first) = with_app(|a| a.controls.first().map(|c| c.hwnd)).flatten() {
            SetFocus(first);
        }
        with_app(|a| {
            a.log.info(format!("window ready at {} DPI", a.dpi));
        });
        super::finish::set_main_hwnd(hwnd);
        let (snd, tst, shd, check_updates) = with_app(|a| {
            (
                a.settings.sound_on_finish,
                a.settings.toast_on_finish,
                a.settings.shutdown_when_done,
                a.settings.check_updates,
            )
        })
        .unwrap_or((true, false, false, false));
        super::finish::set_prefs(snd, tst, shd);
        scan_discs(hwnd, false);
        if check_updates {
            let target = hwnd as usize;
            std::thread::spawn(move || {
                if let Ok(tag) = crate::cli_meta::fetch_latest_tag() {
                    if crate::cli_meta::is_newer(&tag, if_core::APP_VERSION) {
                        let boxed = Box::into_raw(Box::new(tag));
                        // SAFETY: the window reclaims the box when it
                        // handles the message; reclaim on failure.
                        let posted = PostMessageW(target as HWND, WM_UPDATE_AVAILABLE, 0, boxed as LPARAM);
                        if posted == 0 {
                            drop(Box::from_raw(boxed));
                        }
                    }
                }
            });
        }

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
            if pre_translate(hwnd, &msg) {
                continue;
            }
            // Dialog navigation (Tab, arrows, mnemonics) must run
            // against the window the message belongs to — the main
            // window alone left every task window without working
            // keyboard navigation (found in the RC accessibility pass).
            let root = if msg.hwnd.is_null() { hwnd } else { GetAncestor(msg.hwnd, GA_ROOT) };
            let dlg = if root.is_null() { hwnd } else { root };
            if IsDialogMessageW(dlg, &msg) != 0 {
                continue;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let app = APP.with(|a| a.borrow_mut().take());
        if let Some(app) = app {
            if !app.shield.is_null() {
                DestroyIcon(app.shield);
            }
            if !app.logo.is_null() {
                DestroyIcon(app.logo);
            }
            app.log.info("exit");
        }
        ExitCode::SUCCESS
    }
}

/// Keyboard handling the dialog manager does not do for owner-drawn
/// buttons: Enter activates, arrow keys move around the grid, Alt+Down
/// opens a split button's menu.
fn pre_translate(main: HWND, msg: &MSG) -> bool {
    if msg.message != WM_KEYDOWN && msg.message != WM_SYSKEYDOWN {
        return false;
    }
    // SAFETY: reads the focused window handle of this thread.
    let focus = unsafe { GetFocus() };
    let Some((idx, ctl)) =
        with_app(|a| a.controls.iter().position(|c| c.hwnd == focus).map(|i| (i, a.controls[i]))).flatten()
    else {
        return false;
    };
    let key = msg.wParam as u16;
    let alt = msg.message == WM_SYSKEYDOWN;
    if key == VK_RETURN && !alt {
        // SAFETY: the focused control is one of our buttons.
        unsafe { SendMessageW(ctl.hwnd, BM_CLICK, 0, 0) };
        return true;
    }
    if alt && key == VK_DOWN {
        // SAFETY: posting a command to our own window.
        unsafe { PostMessageW(main, WM_COMMAND, (ID_ARROW + ctl.task as u16) as usize, 0) };
        return true;
    }
    if alt {
        return false;
    }
    let target = with_app(|a| neighbor(a, idx, key)).flatten();
    if let Some(h) = target {
        // SAFETY: `h` is one of our child controls.
        unsafe { SetFocus(h) };
        return true;
    }
    false
}

/// Next control in the arrow-key direction, by grid position.
fn neighbor(a: &App, idx: usize, key: u16) -> Option<HWND> {
    let c = a.controls[idx];
    let cells = &a.layout.cells;
    let cell = cells[c.task];
    let pick = |row: usize, col: usize, arrow: bool| -> Option<HWND> {
        let t = cells.iter().position(|x| x.row == row && x.col == col)?;
        let want_arrow = arrow && cells[t].arrow.is_some();
        a.controls.iter().find(|k| k.task == t && k.arrow == want_arrow).map(|k| k.hwnd)
    };
    match key {
        VK_UP if cell.row > 0 => pick(cell.row - 1, cell.col, false).or_else(|| pick(cell.row - 1, 0, false)),
        VK_DOWN => pick(cell.row + 1, cell.col, false).or_else(|| pick(cell.row + 1, 0, false)),
        VK_RIGHT if !c.arrow && cell.arrow.is_some() => pick(cell.row, cell.col, true),
        VK_RIGHT if cell.col == 0 => pick(cell.row, 1, false),
        VK_LEFT if c.arrow => pick(cell.row, cell.col, false),
        VK_LEFT if cell.col == 1 => pick(cell.row, 0, true).or_else(|| pick(cell.row, 0, false)),
        _ => None,
    }
}

fn build_menu(s: &Settings) -> HMENU {
    // SAFETY: menu creation with NUL-terminated strings that outlive each call.
    unsafe {
        let bar = CreateMenu();
        let task = CreatePopupMenu();
        for (i, t) in task_list().iter().enumerate() {
            let mut label = strings::task_title(t.id).to_string();
            if t.admin {
                label.push_str(" (administrator)");
            }
            let w = wide(&label);
            AppendMenuW(task, MF_STRING, (ID_MENU_TASK + i as u16) as usize, w.as_ptr());
        }
        AppendMenuW(task, MF_SEPARATOR, 0, std::ptr::null());
        let exit = wide("E&xit\tAlt+F4");
        AppendMenuW(task, MF_STRING, IDM_EXIT as usize, exit.as_ptr());

        let opts = CreatePopupMenu();
        let levels = CreatePopupMenu();
        for (i, l) in LEVELS.iter().enumerate() {
            let name = match l {
                LogLevel::Error => "&Errors only",
                LogLevel::Warn => "&Warnings and errors",
                LogLevel::Info => "&Normal",
                LogLevel::Debug => "&Debug (detailed)",
            };
            let w = wide(name);
            let flags = if *l == s.log_level { MF_STRING | MF_CHECKED } else { MF_STRING };
            AppendMenuW(levels, flags, (IDM_LOG_BASE + i as u16) as usize, w.as_ptr());
        }
        let lv = wide("&Log detail");
        AppendMenuW(opts, MF_POPUP, levels as usize, lv.as_ptr());
        let snd = wide("Play a &sound when a task finishes");
        AppendMenuW(
            opts,
            if s.sound_on_finish { MF_STRING | MF_CHECKED } else { MF_STRING },
            IDM_SOUND as usize,
            snd.as_ptr(),
        );
        for (id, label, on) in [
            (IDM_TOAST, "Show a &notification when a task finishes", s.toast_on_finish),
            (IDM_SHUTDOWN, "Shut down the &PC when a task finishes", s.shutdown_when_done),
            (IDM_MB, "Look up CDs on &MusicBrainz when ripping (online)", s.musicbrainz_lookup),
            (IDM_UPDATE, "Check for &updates on start (online)", s.check_updates),
        ] {
            let w = wide(label);
            AppendMenuW(opts, if on { MF_STRING | MF_CHECKED } else { MF_STRING }, id as usize, w.as_ptr());
        }
        let auto = CreatePopupMenu();
        for (i, label) in ["&Off", "&Data discs: copy to image", "&Audio CDs: rip", "&Both"].iter().enumerate() {
            let on = (i == 0 && s.auto_action == AutoAction::Off)
                || (i == 1 && s.auto_action == AutoAction::Image)
                || (i == 2 && s.auto_action == AutoAction::Rip)
                || (i == 3 && s.auto_action == AutoAction::Both);
            let w = wide(label);
            AppendMenuW(
                auto,
                if on { MF_STRING | MF_CHECKED } else { MF_STRING },
                (IDM_AUTO_BASE + i as u16) as usize,
                w.as_ptr(),
            );
        }
        let av = wide("&Auto-action on disc insert");
        AppendMenuW(opts, MF_POPUP, auto as usize, av.as_ptr());
        AppendMenuW(opts, MF_SEPARATOR, 0, std::ptr::null());
        let data = wide("Open &data folder");
        AppendMenuW(opts, MF_STRING, IDM_OPEN_DATA as usize, data.as_ptr());

        let help = CreatePopupMenu();
        for (id, text) in [
            (IDM_ROADMAP, "&Roadmap"),
            (IDM_CLI_HELP, "&Command line"),
            (IDM_VIEW_LOG, "View &log"),
            (IDM_REPORT, "Export drive &report…"),
        ] {
            let w = wide(text);
            AppendMenuW(help, MF_STRING, id as usize, w.as_ptr());
        }
        AppendMenuW(help, MF_SEPARATOR, 0, std::ptr::null());
        let about = wide("&About IsoFoundry");
        AppendMenuW(help, MF_STRING, IDM_ABOUT as usize, about.as_ptr());

        for (m, text) in [(task, "&Task"), (opts, "&Options"), (help, "&Help")] {
            let w = wide(text);
            AppendMenuW(bar, MF_POPUP, m as usize, w.as_ptr());
        }
        bar
    }
}

fn accessible_name(t: &tasks::Task, arrow: bool) -> String {
    let title = strings::task_title(t.id);
    match (arrow, t.admin) {
        (true, _) => format!("More options: {title}"),
        (false, true) => format!("{title} (requires administrator)"),
        (false, false) => title.to_string(),
    }
}

fn create_controls(parent: HWND) {
    let hinst = hinstance();
    let button = wide("BUTTON");
    // SAFETY: child creation with valid parent and NUL-terminated strings.
    let tooltip = unsafe {
        let tt = CreateWindowExW(
            WS_EX_TOPMOST,
            TOOLTIPS_CLASSW,
            std::ptr::null(),
            WS_POPUP | TTS_ALWAYSTIP | TTS_NOPREFIX,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            parent,
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        );
        SendMessageW(tt, TTM_SETMAXTIPWIDTH, 0, 360);
        SendMessageW(tt, TTM_SETDELAYTIME, TTDT_AUTOPOP as usize, 20_000);
        tt
    };
    let mut controls = Vec::new();
    for (i, t) in task_list().iter().enumerate() {
        let mut parts = vec![false];
        if !t.variants.is_empty() {
            parts.push(true);
        }
        for arrow in parts {
            let name = wide(&accessible_name(t, arrow));
            let id = if arrow { ID_ARROW } else { ID_MAIN } + i as u16;
            // SAFETY: as above; the tooltip text buffer outlives TTM_ADDTOOLW,
            // which copies it.
            unsafe {
                let h = CreateWindowExW(
                    0,
                    button.as_ptr(),
                    name.as_ptr(),
                    WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_OWNERDRAW as u32,
                    0,
                    0,
                    10,
                    10,
                    parent,
                    id as usize as HMENU,
                    hinst,
                    std::ptr::null(),
                );
                SetWindowSubclass(h, Some(button_subclass), 1, 0);
                let tip = if arrow {
                    let names: Vec<&str> = t.variants.iter().map(|v| strings::variant_label(v.id)).collect();
                    format!("More ways to run this task (Alt+Down): {}", names.join(" · "))
                } else {
                    strings::task_tip(t.id).to_string()
                };
                let mut tipw = wide(&tip);
                let mut ti: TTTOOLINFOW = std::mem::zeroed();
                ti.cbSize = std::mem::size_of::<TTTOOLINFOW>() as u32;
                ti.uFlags = TTF_IDISHWND | TTF_SUBCLASS;
                ti.hwnd = parent;
                ti.uId = h as usize;
                ti.lpszText = tipw.as_mut_ptr();
                SendMessageW(tooltip, TTM_ADDTOOLW, 0, &ti as *const _ as LPARAM);
                controls.push(Control { hwnd: h, task: i, arrow });
            }
        }
    }
    with_app(|a| {
        a.controls = controls;
        // SAFETY: valid window handle.
        let mut rc: RECT = unsafe { std::mem::zeroed() };
        unsafe { GetClientRect(a.hwnd, &mut rc) };
        relayout(a, rc.right, rc.bottom);
    });
}

/// Restores the saved placement if it is still on a connected monitor,
/// otherwise centers the default size on the primary work area.
fn place_window(hwnd: HWND) {
    let (saved, dpi) = with_app(|a| (a.settings.window, a.dpi)).unwrap_or((None, 96));
    // SAFETY: valid window handle; structs are fully initialized.
    unsafe {
        if let Some(w) = saved.filter(|w| w.is_plausible()) {
            let r = RECT { left: w.x, top: w.y, right: w.x + w.width, bottom: w.y + w.height };
            if !MonitorFromRect(&r, MONITOR_DEFAULTTONULL).is_null() {
                let mut wp: WINDOWPLACEMENT = std::mem::zeroed();
                wp.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
                wp.showCmd = if w.maximized { SW_SHOWMAXIMIZED as u32 } else { SW_SHOWNORMAL as u32 };
                wp.rcNormalPosition = r;
                SetWindowPlacement(hwnd, &wp);
                return;
            }
        }
        let (cw, ch) = layout::default_client(dpi);
        let mut r = RECT { left: 0, top: 0, right: cw, bottom: ch };
        AdjustWindowRectExForDpi(&mut r, WS_OVERLAPPEDWINDOW, 1, WS_EX_CONTROLPARENT, dpi);
        let (w, h) = (r.right - r.left, r.bottom - r.top);
        let mut work: RECT = std::mem::zeroed();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, (&mut work as *mut RECT).cast(), 0);
        let x = work.left + ((work.right - work.left) - w).max(0) / 2;
        let y = work.top + ((work.bottom - work.top) - h).max(0) / 2;
        SetWindowPos(hwnd, std::ptr::null_mut(), x, y, w, h, SWP_NOZORDER);
        ShowWindow(hwnd, SW_SHOWNORMAL);
    }
}

fn save_placement(a: &mut App) {
    // SAFETY: valid window handle; WINDOWPLACEMENT sized as required.
    unsafe {
        let mut wp: WINDOWPLACEMENT = std::mem::zeroed();
        wp.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
        if GetWindowPlacement(a.hwnd, &mut wp) != 0 {
            let r = wp.rcNormalPosition;
            a.settings.window = Some(WindowPlacement {
                x: r.left,
                y: r.top,
                width: r.right - r.left,
                height: r.bottom - r.top,
                maximized: wp.showCmd == SW_SHOWMAXIMIZED as u32,
            });
        }
    }
    save_settings(a);
}

fn save_settings(a: &App) {
    if let Err(e) = if_core::settings::save(&a.dirs.settings_file(), &a.settings) {
        a.log.warn(format!("settings not saved: {e}"));
    }
    super::finish::set_prefs(a.settings.sound_on_finish, a.settings.toast_on_finish, a.settings.shutdown_when_done);
}

fn current_palette() -> Palette {
    // SAFETY: SPI_GETHIGHCONTRAST fills a sized HIGHCONTRASTW.
    unsafe {
        let mut hc: HIGHCONTRASTW = std::mem::zeroed();
        hc.cbSize = std::mem::size_of::<HIGHCONTRASTW>() as u32;
        let on = SystemParametersInfoW(SPI_GETHIGHCONTRAST, hc.cbSize, (&mut hc as *mut HIGHCONTRASTW).cast(), 0) != 0
            && hc.dwFlags & HCF_HIGHCONTRASTON != 0;
        if !on {
            return Palette::DARK;
        }
        let sys = |i| {
            let c = GetSysColor(i);
            ((c & 0xFF) << 16) | (c & 0xFF00) | ((c >> 16) & 0xFF)
        };
        Palette::high_contrast(
            sys(COLOR_WINDOW),
            sys(COLOR_WINDOWTEXT),
            sys(COLOR_BTNFACE),
            sys(COLOR_BTNTEXT),
            sys(COLOR_HIGHLIGHT),
        )
    }
}

fn relayout(a: &mut App, w: i32, h: i32) {
    a.layout = layout::compute(w, h, a.dpi);
    let size = px(56, a.dpi);
    if size != a.logo_size {
        // SAFETY: icon resource 1 is embedded by build.rs; old icon destroyed once.
        unsafe {
            if !a.logo.is_null() {
                DestroyIcon(a.logo);
            }
            a.logo = LoadImageW(hinstance(), 1 as _, IMAGE_ICON, size, size, LR_DEFAULTCOLOR) as HICON;
        }
        a.logo_size = size;
    }
    // SAFETY: repositions our own child windows.
    unsafe {
        let mut dwp = BeginDeferWindowPos(a.controls.len() as i32);
        for c in &a.controls {
            let cell = a.layout.cells[c.task];
            let r = if c.arrow { cell.arrow.unwrap_or(cell.main) } else { cell.main };
            dwp = DeferWindowPos(
                dwp,
                c.hwnd,
                std::ptr::null_mut(),
                r.left,
                r.top,
                r.width(),
                r.height(),
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        EndDeferWindowPos(dwp);
        InvalidateRect(a.hwnd, std::ptr::null(), 0);
    }
}

/// Rectangle the focus ring is drawn in: the whole cell (both halves of a
/// split button), 4 px outside so it sits on the group surface (A-1).
fn ring_rect(a: &App, task: usize) -> Rect {
    let cell = a.layout.cells[task];
    let outer = cell.arrow.map(|r| r.right).unwrap_or(cell.main.right);
    let g = px(4, a.dpi);
    Rect { left: cell.main.left - g, top: cell.main.top - g, right: outer + g, bottom: cell.main.bottom + g }
}

unsafe fn fill(dc: HDC, r: &RECT, rgb: theme::Rgb) {
    let b = CreateSolidBrush(colorref(rgb) as COLORREF);
    FillRect(dc, r, b);
    DeleteObject(b);
}

unsafe fn frame(dc: HDC, r: &RECT, rgb: theme::Rgb, width: i32) {
    for i in 0..width {
        let rr = RECT { left: r.left + i, top: r.top + i, right: r.right - i, bottom: r.bottom - i };
        let b = CreateSolidBrush(colorref(rgb) as COLORREF);
        FrameRect(dc, &rr, b);
        DeleteObject(b);
    }
}

unsafe fn text(dc: HDC, s: &str, r: &mut RECT, rgb: theme::Rgb, font: HFONT, flags: DRAW_TEXT_FORMAT) {
    let w: Vec<u16> = s.encode_utf16().collect();
    let old = SelectObject(dc, font);
    SetTextColor(dc, colorref(rgb) as COLORREF);
    SetBkMode(dc, TRANSPARENT as i32);
    DrawTextW(dc, w.as_ptr(), w.len() as i32, r, flags);
    SelectObject(dc, old);
}

fn paint(hwnd: HWND) {
    // SAFETY: standard BeginPaint/EndPaint with an off-screen buffer; every
    // GDI object created here is selected out and deleted before returning.
    unsafe {
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);
        let mut rc: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut rc);
        let mem = CreateCompatibleDC(hdc);
        let bmp = CreateCompatibleBitmap(hdc, rc.right.max(1), rc.bottom.max(1));
        let old_bmp = SelectObject(mem, bmp);
        with_app(|a| {
            let p = a.palette;
            fill(mem, &rc, p.bg);
            let g = rect(a.layout.group);
            fill(mem, &g, p.surface);
            frame(mem, &g, p.border, 1);
            let mut lr = rect(a.layout.label);
            text(mem, strings::SELECT_TASK, &mut lr, p.text_muted, a.fonts.label, DT_LEFT | DT_VCENTER | DT_SINGLELINE);
            if let Some(t) = a.focus_task {
                let r = rect(ring_rect(a, t));
                frame(mem, &r, p.accent, px(2, a.dpi));
            }
            // Footer: logo + wordmark, centered.
            let f = a.layout.footer;
            let word = "IsoFoundry";
            let mut tr = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            let ww: Vec<u16> = word.encode_utf16().collect();
            let old = SelectObject(mem, a.fonts.logo);
            DrawTextW(mem, ww.as_ptr(), ww.len() as i32, &mut tr, DT_CALCRECT | DT_SINGLELINE);
            SelectObject(mem, old);
            let gap = px(14, a.dpi);
            let total = a.logo_size + gap + tr.right;
            let x = f.left + (f.width() - total) / 2;
            let cy = f.top + f.height() / 2;
            if !a.logo.is_null() {
                DrawIconEx(
                    mem,
                    x,
                    cy - a.logo_size / 2,
                    a.logo,
                    a.logo_size,
                    a.logo_size,
                    0,
                    std::ptr::null_mut(),
                    DI_NORMAL,
                );
            }
            let mut wr =
                RECT { left: x + a.logo_size + gap, top: cy - tr.bottom / 2, right: x + total, bottom: cy + tr.bottom };
            text(mem, word, &mut wr, p.text, a.fonts.logo, DT_LEFT | DT_SINGLELINE);
        });
        BitBlt(hdc, 0, 0, rc.right, rc.bottom, mem, 0, 0, SRCCOPY);
        SelectObject(mem, old_bmp);
        DeleteObject(bmp);
        DeleteDC(mem);
        EndPaint(hwnd, &ps);
    }
}

fn draw_item(dis: &DRAWITEMSTRUCT) {
    with_app(|a| {
        let Some(c) = a.controls.iter().find(|c| c.hwnd == dis.hwndItem).copied() else { return };
        let t = task_list()[c.task];
        let p = a.palette;
        let pressed = dis.itemState & ODS_SELECTED != 0;
        let hot = a.hot == Some(c.hwnd);
        let bg = if pressed {
            p.btn_pressed
        } else if hot {
            p.btn_hover
        } else {
            p.btn
        };
        let r = dis.rcItem;
        // SAFETY: the DC and rect come from the system for this paint.
        unsafe {
            let dc = dis.hDC;
            fill(dc, &r, p.surface);
            let b = CreateSolidBrush(colorref(bg) as COLORREF);
            let pen = CreatePen(PS_SOLID, 1, colorref(if p == Palette::DARK { bg } else { p.border }) as COLORREF);
            let ob = SelectObject(dc, b);
            let op = SelectObject(dc, pen);
            let rad = px(8, a.dpi);
            RoundRect(dc, r.left, r.top, r.right, r.bottom, rad, rad);
            SelectObject(dc, ob);
            SelectObject(dc, op);
            DeleteObject(b);
            DeleteObject(pen);

            let fg = if p == Palette::DARK {
                p.btn_text
            } else if hot || pressed {
                p.bg
            } else {
                p.btn_text
            };
            if c.arrow {
                let cx = (r.left + r.right) / 2;
                let cy = (r.top + r.bottom) / 2;
                let s = px(4, a.dpi);
                let pts = [
                    POINT { x: cx - s, y: cy - s / 2 },
                    POINT { x: cx + s, y: cy - s / 2 },
                    POINT { x: cx, y: cy + s / 2 + 1 },
                ];
                let b = CreateSolidBrush(colorref(fg) as COLORREF);
                let pen = CreatePen(PS_SOLID, 1, colorref(fg) as COLORREF);
                let ob = SelectObject(dc, b);
                let op = SelectObject(dc, pen);
                Polygon(dc, pts.as_ptr(), 3);
                SelectObject(dc, ob);
                SelectObject(dc, op);
                DeleteObject(b);
                DeleteObject(pen);
                return;
            }
            let title = strings::task_title(t.id);
            let ww: Vec<u16> = title.encode_utf16().collect();
            let mut m = RECT { left: 0, top: 0, right: 0, bottom: 0 };
            let old = SelectObject(dc, a.fonts.button);
            DrawTextW(dc, ww.as_ptr(), ww.len() as i32, &mut m, DT_CALCRECT | DT_SINGLELINE);
            SelectObject(dc, old);
            let icon = if t.admin && !a.shield.is_null() { px(16, a.dpi) } else { 0 };
            let gap = if icon > 0 { px(6, a.dpi) } else { 0 };
            let pad = px(8, a.dpi);
            let avail = r.right - r.left - 2 * pad;
            let total = (icon + gap + m.right).min(avail);
            let x = r.left + pad + (avail - total).max(0) / 2;
            if icon > 0 {
                let y = (r.top + r.bottom - icon) / 2;
                DrawIconEx(dc, x, y, a.shield, icon, icon, 0, std::ptr::null_mut(), DI_NORMAL);
            }
            let mut tr = RECT { left: x + icon + gap, top: r.top, right: r.right - pad, bottom: r.bottom };
            text(
                dc,
                title,
                &mut tr,
                fg,
                a.fonts.button,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX,
            );
        }
    });
}

unsafe extern "system" fn button_subclass(
    h: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _id: usize,
    _ref: usize,
) -> LRESULT {
    match msg {
        WM_MOUSEMOVE => {
            let changed = with_app(|a| {
                if a.hot != Some(h) {
                    a.hot = Some(h);
                    true
                } else {
                    false
                }
            })
            .unwrap_or(false);
            if changed {
                let mut tme = TRACKMOUSEEVENT {
                    cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: h,
                    dwHoverTime: 0,
                };
                TrackMouseEvent(&mut tme);
                InvalidateRect(h, std::ptr::null(), 0);
            }
        }
        WM_MOUSELEAVE => {
            with_app(|a| {
                if a.hot == Some(h) {
                    a.hot = None;
                }
            });
            InvalidateRect(h, std::ptr::null(), 0);
        }
        WM_SETFOCUS | WM_KILLFOCUS => {
            let ring = with_app(|a| {
                let task = a.controls.iter().find(|c| c.hwnd == h).map(|c| c.task);
                let before = a.focus_task.map(|t| ring_rect(a, t));
                a.focus_task = if msg == WM_SETFOCUS { task } else { None };
                let after = a.focus_task.map(|t| ring_rect(a, t));
                (a.hwnd, before, after)
            });
            if let Some((parent, before, after)) = ring {
                for r in [before, after].into_iter().flatten() {
                    let rr = rect(r);
                    InvalidateRect(parent, &rr, 0);
                }
            }
        }
        WM_ERASEBKGND => return 1,
        _ => {}
    }
    DefSubclassProc(h, msg, wp, lp)
}

enum Action {
    None,
    Info(String),
    NotYet(String, &'static str, &'static str),
    OpenTask(&'static str),
    VariantMenu(usize),
    Exit,
    OpenData,
    ViewLog,
    Roadmap,
    CliHelp,
    Report,
    About,
}

fn on_command(hwnd: HWND, id: u16) {
    let tasks = task_list();
    let action = match id {
        IDM_EXIT => Action::Exit,
        IDM_OPEN_DATA => Action::OpenData,
        IDM_VIEW_LOG => Action::ViewLog,
        IDM_ROADMAP => Action::Roadmap,
        IDM_CLI_HELP => Action::CliHelp,
        IDM_REPORT => Action::Report,
        IDM_ABOUT => Action::About,
        IDM_SOUND => {
            with_app(|a| {
                a.settings.sound_on_finish = !a.settings.sound_on_finish;
                save_settings(a);
                // SAFETY: menu handle owned by the window.
                unsafe {
                    let m = GetMenu(a.hwnd);
                    CheckMenuItem(
                        m,
                        IDM_SOUND as u32,
                        if a.settings.sound_on_finish { MF_CHECKED } else { MF_UNCHECKED },
                    );
                }
            });
            Action::None
        }
        IDM_TOAST | IDM_SHUTDOWN | IDM_MB | IDM_UPDATE => {
            with_app(|a| {
                let on = match id {
                    IDM_TOAST => {
                        a.settings.toast_on_finish = !a.settings.toast_on_finish;
                        a.settings.toast_on_finish
                    }
                    IDM_SHUTDOWN => {
                        a.settings.shutdown_when_done = !a.settings.shutdown_when_done;
                        a.settings.shutdown_when_done
                    }
                    IDM_MB => {
                        a.settings.musicbrainz_lookup = !a.settings.musicbrainz_lookup;
                        a.settings.musicbrainz_lookup
                    }
                    _ => {
                        a.settings.check_updates = !a.settings.check_updates;
                        a.settings.check_updates
                    }
                };
                save_settings(a);
                // SAFETY: menu handle owned by the window.
                unsafe {
                    let m = GetMenu(a.hwnd);
                    CheckMenuItem(m, id as u32, if on { MF_CHECKED } else { MF_UNCHECKED });
                }
            });
            Action::None
        }
        x if (IDM_AUTO_BASE..IDM_AUTO_BASE + 4).contains(&x) => {
            let mode =
                [AutoAction::Off, AutoAction::Image, AutoAction::Rip, AutoAction::Both][(x - IDM_AUTO_BASE) as usize];
            with_app(|a| {
                a.settings.auto_action = mode;
                save_settings(a);
                a.log.info(format!("auto-action on disc insert: {mode:?}"));
                // SAFETY: menu handle owned by the window.
                unsafe {
                    let m = GetMenu(a.hwnd);
                    CheckMenuRadioItem(m, IDM_AUTO_BASE as u32, (IDM_AUTO_BASE + 3) as u32, x as u32, MF_BYCOMMAND);
                }
            });
            Action::None
        }
        x if (IDM_LOG_BASE..IDM_LOG_BASE + LEVELS.len() as u16).contains(&x) => {
            let level = LEVELS[(x - IDM_LOG_BASE) as usize];
            with_app(|a| {
                a.settings.log_level = level;
                a.log.set_level(level);
                save_settings(a);
                // SAFETY: menu handle owned by the window.
                unsafe {
                    let m = GetMenu(a.hwnd);
                    CheckMenuRadioItem(m, IDM_LOG_BASE as u32, (IDM_LOG_BASE + 3) as u32, x as u32, MF_BYCOMMAND);
                }
                a.log.info(format!("log detail set to {level:?}"));
            });
            Action::None
        }
        x if (ID_MAIN..ID_MAIN + tasks.len() as u16).contains(&x)
            || (ID_MENU_TASK..ID_MENU_TASK + tasks.len() as u16).contains(&x) =>
        {
            let i = if x >= ID_MENU_TASK { x - ID_MENU_TASK } else { x - ID_MAIN } as usize;
            let t = tasks[i];
            if t.is_available() {
                Action::OpenTask(t.id)
            } else {
                Action::NotYet(strings::task_title(t.id).to_string(), t.milestone.label(), t.milestone.release())
            }
        }
        x if (ID_ARROW..ID_ARROW + tasks.len() as u16).contains(&x) => Action::VariantMenu((x - ID_ARROW) as usize),
        x if x >= ID_VARIANT => match variant_list().get((x - ID_VARIANT) as usize) {
            Some((_, v)) if v.id == "backup-restore" => Action::OpenTask("backup-restore"),
            Some((_, v)) if v.id == "usb-image-to-usb" => Action::OpenTask("bootable-usb"),
            Some((_, v)) if v.id == "usb-usb-to-image" => Action::Info(
                "Copy a USB stick to an image file from the command line:\n\nisofoundry usb read --disk N --out FILE\n\nA window for it is planned after 1.0; the copy itself works today.".to_string(),
            ),
            Some((_, v)) => Action::NotYet(
                strings::variant_label(v.id).trim_end_matches('…').to_string(),
                v.milestone.label(),
                v.milestone.release(),
            ),
            None => Action::None,
        },
        _ => Action::None,
    };
    run_action(hwnd, action);
}

fn run_action(hwnd: HWND, action: Action) {
    match action {
        Action::None => {}
        Action::Info(text) => {
            message(hwnd, &text, MB_ICONINFORMATION);
        }
        Action::Exit => {
            // SAFETY: closing our own window.
            unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
        }
        Action::NotYet(title, m, rel) => {
            with_app(|a| a.log.info(format!("task requested: {title} (ships in {m})")));
            message(hwnd, &strings::not_yet(&title, m, rel), MB_ICONINFORMATION);
        }
        Action::OpenTask(id) => {
            with_app(|a| a.log.info(format!("task opened: {}", strings::task_title(id))));
            match id {
                "browse-image" => super::tasks::browse::open(hwnd),
                "checksum" => super::tasks::checksum::open(hwnd),
                "convert-image" => super::tasks::convert::open(hwnd),
                "create-image" => super::tasks::create::open(hwnd),
                "edit-image" => super::tasks::edit::open(hwnd),
                "bootable-usb" => super::tasks::bootusb::open(hwnd),
                "install-windows-usb" => super::tasks::winusb::open(hwnd),
                "convert-audio" => super::tasks::audioconv::open(hwnd),
                "burn-image" => super::tasks::burnimage::open(hwnd),
                "burn-files" => super::tasks::burnfiles::open_files(hwnd),
                "burn-mp3" => super::tasks::burnfiles::open_mp3(hwnd),
                "burn-video" => super::tasks::burnfiles::open_video(hwnd),
                "erase" => super::tasks::erasewin::open(hwnd),
                "drive-info" => super::tasks::driveinfo::open(hwnd),
                "copy-to-image" => super::tasks::copyimage::open(hwnd),
                "copy-disc" => super::tasks::copydisc::open(hwnd),
                "rip-audio" => super::tasks::ripaudio::open(hwnd),
                "burn-audio" => super::tasks::burnaudio::open(hwnd),
                "surface-test" => super::tasks::surfacetest::open(hwnd),
                "recover-files" => super::tasks::recoverwin::open(hwnd),
                "backup" => super::tasks::backupwin::open_backup(hwnd),
                "backup-restore" => super::tasks::backupwin::open_restore(hwnd),
                _ => {}
            }
        }
        Action::VariantMenu(i) => variant_menu(hwnd, i),
        Action::OpenData => {
            let root = with_app(|a| a.dirs.root.clone());
            if let Some(root) = root {
                let _ = std::fs::create_dir_all(&root);
                let p = wide(&root.to_string_lossy());
                let verb = wide("explore");
                // SAFETY: NUL-terminated strings outlive the call.
                unsafe {
                    ShellExecuteW(hwnd, verb.as_ptr(), p.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOWNORMAL)
                };
            }
        }
        Action::ViewLog => {
            let lines = with_app(|a| a.log.entries().iter().map(|e| e.line()).collect::<Vec<_>>()).unwrap_or_default();
            let body = if lines.is_empty() { "The log is empty.".to_string() } else { lines.join("\r\n") };
            text_window(hwnd, "IsoFoundry log", &body);
        }
        Action::Roadmap => text_window(hwnd, "IsoFoundry roadmap", &crate::cli::roadmap_text().replace('\n', "\r\n")),
        Action::CliHelp => text_window(hwnd, "IsoFoundry command line", &crate::cli::usage().replace('\n', "\r\n")),
        Action::Report => export_report(hwnd),
        Action::About => {
            let data =
                with_app(|a| format!("{}{}", a.dirs.root.display(), if a.dirs.portable { " (portable)" } else { "" }))
                    .unwrap_or_default();
            let body = format!(
                "IsoFoundry {}\nOpen source disc burning, imaging and bootable USB suite.\n\nCopyright (c) 2026 Sean Shaffer. MIT License.\nhttps://github.com/SeanShaffer88/IsoFoundry\n\nData folder: {data}\n\nWorks offline. Collects no telemetry.",
                if_core::APP_VERSION
            );
            message(hwnd, &body, MB_ICONINFORMATION);
        }
    }
}

fn message(hwnd: HWND, body: &str, icon: MESSAGEBOX_STYLE) {
    let b = wide(body);
    let t = wide(if_core::APP_NAME);
    // SAFETY: NUL-terminated strings outlive the modal call; no state is borrowed.
    unsafe { MessageBoxW(hwnd, b.as_ptr(), t.as_ptr(), MB_OK | icon) };
}

fn variant_menu(hwnd: HWND, task: usize) {
    let t = task_list()[task];
    let variants = variant_list();
    let anchor = with_app(|a| a.controls.iter().find(|c| c.task == task && c.arrow).map(|c| c.hwnd)).flatten();
    // SAFETY: popup menu created, tracked and destroyed here; strings outlive
    // AppendMenuW, which copies them.
    unsafe {
        let m = CreatePopupMenu();
        for (k, (vt, v)) in variants.iter().enumerate() {
            if vt.id != t.id {
                continue;
            }
            let mut label = strings::variant_label(v.id).to_string();
            if v.admin {
                label.push_str(" (administrator)");
            }
            let w = wide(&label);
            AppendMenuW(m, MF_STRING, (ID_VARIANT + k as u16) as usize, w.as_ptr());
        }
        let mut r: RECT = std::mem::zeroed();
        if let Some(h) = anchor {
            GetWindowRect(h, &mut r);
        }
        let cmd = TrackPopupMenu(
            m,
            TPM_RETURNCMD | TPM_RIGHTALIGN | TPM_TOPALIGN,
            r.right,
            r.bottom,
            0,
            hwnd,
            std::ptr::null(),
        );
        DestroyMenu(m);
        if cmd != 0 {
            on_command(hwnd, cmd as u16);
        }
    }
}

fn export_report(hwnd: HWND) {
    let Some((name, json)) = with_app(|a| {
        let report = crate::cli::build_report(&super::WinPlatform, crate::cli::log_tail(&a.dirs, &a.log));
        (report.file_name(), report.to_json())
    }) else {
        return;
    };
    let mut buf = [0u16; 1024];
    for (i, c) in name.encode_utf16().take(buf.len() - 1).enumerate() {
        buf[i] = c;
    }
    let filter: Vec<u16> = "Drive report (*.json)\0*.json\0\0".encode_utf16().collect();
    let title = wide("Export drive report");
    let ext = wide("json");
    // SAFETY: OPENFILENAMEW zeroed and sized; buffers outlive the modal call.
    let ok = unsafe {
        let mut ofn: OPENFILENAMEW = std::mem::zeroed();
        ofn.lStructSize = std::mem::size_of::<OPENFILENAMEW>() as u32;
        ofn.hwndOwner = hwnd;
        ofn.lpstrFilter = filter.as_ptr();
        ofn.lpstrFile = buf.as_mut_ptr();
        ofn.nMaxFile = buf.len() as u32;
        ofn.lpstrTitle = title.as_ptr();
        ofn.lpstrDefExt = ext.as_ptr();
        ofn.Flags = OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST;
        GetSaveFileNameW(&mut ofn) != 0
    };
    if !ok {
        return;
    }
    let path = std::path::PathBuf::from(from_wide(&buf));
    match if_core::fsutil::atomic_write(&path, json.as_bytes()) {
        Ok(()) => {
            with_app(|a| a.log.info("drive report exported"));
            message(
                hwnd,
                &format!(
                    "Drive report saved to\n{}\n\nSerial numbers and personal folder names are removed. Attach it to a \u{201C}Drive report\u{201D} issue on GitHub.",
                    path.display()
                ),
                MB_ICONINFORMATION,
            );
        }
        Err(e) => {
            with_app(|a| a.log.error(format!("drive report not saved: {e}")));
            message(hwnd, &format!("The report could not be saved:\n{e}"), MB_ICONWARNING);
        }
    }
}

/// A read-only text window (log, roadmap, command-line help).
fn text_window(owner: HWND, title: &str, body: &str) {
    let (dpi, mono) = with_app(|a| (a.dpi, a.fonts.mono)).unwrap_or((96, std::ptr::null_mut()));
    // SAFETY: creates an owned popup and a child EDIT; strings outlive the calls.
    unsafe {
        let cls = wide(TEXT_CLASS);
        let t = wide(title);
        let w = CreateWindowExW(
            0,
            cls.as_ptr(),
            t.as_ptr(),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            px(760, dpi),
            px(520, dpi),
            owner,
            std::ptr::null_mut(),
            hinstance(),
            std::ptr::null(),
        );
        if w.is_null() {
            return;
        }
        let dark: i32 = 1;
        DwmSetWindowAttribute(w, DWMWA_USE_IMMERSIVE_DARK_MODE as u32, (&dark as *const i32).cast(), 4);
        let edit_cls = wide("EDIT");
        let b = wide(body);
        let e = CreateWindowExW(
            0,
            edit_cls.as_ptr(),
            b.as_ptr(),
            WS_CHILD
                | WS_VISIBLE
                | WS_VSCROLL
                | WS_HSCROLL
                | WS_TABSTOP
                | (ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL | ES_AUTOHSCROLL) as u32,
            0,
            0,
            10,
            10,
            w,
            1 as HMENU,
            hinstance(),
            std::ptr::null(),
        );
        SendMessageW(e, WM_SETFONT, mono as usize, 1);
        SetWindowSubclass(e, Some(edit_subclass), 1, 0);
        let mut rc: RECT = std::mem::zeroed();
        GetClientRect(w, &mut rc);
        MoveWindow(e, 0, 0, rc.right, rc.bottom, 1);
        SetFocus(e);
    }
}

unsafe extern "system" fn edit_subclass(h: HWND, msg: u32, wp: WPARAM, lp: LPARAM, _id: usize, _ref: usize) -> LRESULT {
    if msg == WM_KEYDOWN && wp as u16 == VK_ESCAPE {
        PostMessageW(GetParent(h), WM_CLOSE, 0, 0);
        return 0;
    }
    if msg == WM_GETDLGCODE {
        // Keep Esc for us; stop the edit from selecting all text on focus.
        return (DefSubclassProc(h, msg, wp, lp) & !(DLGC_HASSETSEL as LRESULT)) | DLGC_WANTALLKEYS as LRESULT;
    }
    DefSubclassProc(h, msg, wp, lp)
}

unsafe extern "system" fn text_wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_SIZE => {
            let e = GetDlgItem(hwnd, 1);
            MoveWindow(e, 0, 0, loword(lp as usize) as i32, hiword(lp as usize) as i32, 1);
            0
        }
        WM_CTLCOLORSTATIC | WM_CTLCOLOREDIT => {
            let p = with_app(|a| a.palette).unwrap_or(Palette::DARK);
            let dc = wp as HDC;
            SetTextColor(dc, colorref(p.text) as COLORREF);
            SetBkColor(dc, colorref(p.surface) as COLORREF);
            // A stock-less brush cached per process: one brush, never freed (≤ 1 object).
            static BRUSH: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
            *BRUSH.get_or_init(|| CreateSolidBrush(colorref(theme::SURFACE) as COLORREF) as usize) as LRESULT
        }
        WM_SETFOCUS => {
            SetFocus(GetDlgItem(hwnd, 1));
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            paint(hwnd);
            0
        }
        WM_DRAWITEM => {
            draw_item(&*(lp as *const DRAWITEMSTRUCT));
            1
        }
        WM_SIZE => {
            let (w, h) = (loword(lp as usize) as i32, hiword(lp as usize) as i32);
            with_app(|a| relayout(a, w, h));
            0
        }
        WM_GETMINMAXINFO => {
            let mmi = &mut *(lp as *mut MINMAXINFO);
            let dpi = GetDpiForWindow(hwnd).max(96);
            let (w, h) = layout::min_client(dpi);
            let mut r = RECT { left: 0, top: 0, right: w, bottom: h };
            AdjustWindowRectExForDpi(&mut r, WS_OVERLAPPEDWINDOW, 1, WS_EX_CONTROLPARENT, dpi);
            mmi.ptMinTrackSize = POINT { x: r.right - r.left, y: r.bottom - r.top };
            0
        }
        WM_DPICHANGED => {
            let dpi = hiword(wp) as u32;
            with_app(|a| {
                a.dpi = dpi;
                a.fonts = Fonts::new(dpi);
                a.logo_size = 0;
            });
            let r = &*(lp as *const RECT);
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                r.left,
                r.top,
                r.right - r.left,
                r.bottom - r.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            0
        }
        WM_SETTINGCHANGE | WM_SYSCOLORCHANGE | WM_THEMECHANGED => {
            let p = current_palette();
            with_app(|a| a.palette = p);
            RedrawWindow(hwnd, std::ptr::null(), std::ptr::null_mut(), RDW_INVALIDATE | RDW_ALLCHILDREN);
            DefWindowProcW(hwnd, msg, wp, lp)
        }
        WM_COMMAND => {
            on_command(hwnd, loword(wp));
            0
        }
        m if m == super::finish::WM_FINISH_TOAST => {
            // SAFETY: the box was created by the finish hook (or the
            // update thread's sibling message below) and is ours now.
            let text = unsafe { Box::from_raw(lp as *mut String) };
            let ok = wp != 0;
            with_app(|a| a.log.info(format!("task notification: {text}")));
            show_balloon(hwnd, if ok { "Task finished" } else { "Task failed" }, &text, !ok);
            0
        }
        WM_UPDATE_AVAILABLE => {
            // SAFETY: heap tag string posted by the update thread.
            let tag = unsafe { Box::from_raw(lp as *mut String) };
            with_app(|a| a.log.info(format!("update available: IsoFoundry {tag}")));
            show_balloon(
                hwnd,
                "Update available",
                &format!(
                    "IsoFoundry {} is available. Get it from the GitHub releases page.",
                    tag.trim_start_matches('v')
                ),
                false,
            );
            0
        }
        WM_DEVICECHANGE if wp == 0x8000 => {
            // DBT_DEVICEARRIVAL: give the disc a moment to spin up,
            // then see what arrived.
            // SAFETY: plain timer on our own window.
            unsafe { SetTimer(hwnd, TIMER_DISC, 2000, None) };
            0
        }
        WM_TIMER if wp == TIMER_DISC => {
            // SAFETY: our own timer.
            unsafe { KillTimer(hwnd, TIMER_DISC) };
            scan_discs(hwnd, true);
            0
        }
        WM_TIMER if wp == TIMER_TOAST => {
            // SAFETY: our own timer.
            unsafe { KillTimer(hwnd, TIMER_TOAST) };
            hide_balloon(hwnd);
            0
        }
        WM_CLOSE => {
            with_app(save_placement);
            DestroyWindow(hwnd);
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

/// Shows (or refreshes) the finish-notification balloon (M6). The
/// icon is temporary: it exists only while a balloon is up.
fn show_balloon(hwnd: HWND, title: &str, text: &str, error: bool) {
    let already = with_app(|a| a.toast_icon).unwrap_or(false);
    // SAFETY: a stack NOTIFYICONDATAW with fixed-size buffers; the
    // shell copies what it needs during the call.
    unsafe {
        let mut data: NOTIFYICONDATAW = std::mem::zeroed();
        data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = hwnd;
        data.uID = 1;
        data.uFlags = NIF_ICON | NIF_TIP | NIF_INFO;
        data.hIcon = LoadIconW(hinstance(), 1 as _);
        let put = |dst: &mut [u16], s: &str| {
            for (i, u) in s.encode_utf16().take(dst.len() - 1).enumerate() {
                dst[i] = u;
            }
        };
        put(&mut data.szTip, "IsoFoundry");
        put(&mut data.szInfoTitle, title);
        put(&mut data.szInfo, text);
        data.dwInfoFlags = if error { NIIF_ERROR } else { NIIF_INFO };
        Shell_NotifyIconW(if already { NIM_MODIFY } else { NIM_ADD }, &data);
        SetTimer(hwnd, TIMER_TOAST, 6000, None);
    }
    with_app(|a| a.toast_icon = true);
}

fn hide_balloon(hwnd: HWND) {
    let added = with_app(|a| a.toast_icon).unwrap_or(false);
    if !added {
        return;
    }
    // SAFETY: removing our own temporary icon.
    unsafe {
        let mut data: NOTIFYICONDATAW = std::mem::zeroed();
        data.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        data.hWnd = hwnd;
        data.uID = 1;
        Shell_NotifyIconW(NIM_DELETE, &data);
    }
    with_app(|a| a.toast_icon = false);
}

/// Disc-insert scan (M6 auto-action): compares the discs now in the
/// drives with the last scan; a genuinely new disc is logged and,
/// when the user turned auto-action on, routed to its task window.
/// With `announce = false` (startup) it only seeds the snapshot, so
/// a disc that was already in the drive never triggers anything.
fn scan_discs(hwnd: HWND, announce: bool) {
    let (mode, seen) =
        with_app(|a| (a.settings.auto_action, a.disc_seen.clone())).unwrap_or((AutoAction::Off, Vec::new()));
    let mut current: Vec<String> = Vec::new();
    let mut fresh: Vec<(bool, String)> = Vec::new();
    for info in if_drive::enumerate() {
        let Ok(d) = if_drive::open(&info) else { continue };
        let Ok(m) = d.media() else { continue };
        if !m.present {
            continue;
        }
        let Ok(toc) = d.toc() else { continue };
        let letter = info.letter.map(|c| c.to_string()).unwrap_or_default();
        let fp = format!("{letter}:{}:{}", toc.lead_out_lba, toc.tracks.len());
        if !seen.contains(&fp) {
            fresh.push((toc.is_audio(), letter));
        }
        current.push(fp);
    }
    with_app(|a| a.disc_seen = current);
    if !announce {
        return;
    }
    for (audio, letter) in fresh {
        note_log(&format!("disc inserted in {letter}: {}", if audio { "audio CD" } else { "data disc" }), false);
        let task = match mode {
            AutoAction::Image if !audio => Some("copy-to-image"),
            AutoAction::Rip if audio => Some("rip-audio"),
            AutoAction::Both => Some(if audio { "rip-audio" } else { "copy-to-image" }),
            _ => None,
        };
        if let Some(id) = task {
            run_action(hwnd, Action::OpenTask(id));
        }
    }
}
