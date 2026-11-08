//! Shared drive picking for the M4 windows: a list box with the
//! drives IMAPI2 can see plus the fake drive (always available, so
//! every burn window can be tried with no burner attached).

use super::super::taskwin::*;
use if_drive::DriveInfo;
use windows_sys::Win32::Foundation::HWND;

/// The fake drive as plain info (matches the CLI's `--fake`).
pub fn fake_info() -> DriveInfo {
    DriveInfo {
        id: "fake".into(),
        letter: Some('Z'),
        vendor: "FakeDrive".into(),
        product: "Simulated CD/DVD Writer".into(),
        revision: "1.0".into(),
        backend: "Fake".into(),
    }
}

pub struct DrivePane {
    pub list: HWND,
    drives: Vec<DriveInfo>,
}

impl DrivePane {
    /// An empty pane; call [`DrivePane::create`] from WM_CREATE.
    pub fn empty() -> DrivePane {
        DrivePane { list: std::ptr::null_mut(), drives: Vec::new() }
    }

    /// Creates the list box at the given position and fills it.
    pub fn create(&mut self, parent: HWND, id: u16, x: i32, y: i32, w: i32, h: i32) {
        self.list = listbox(parent, id, x, y, w, h);
        self.refresh();
    }

    pub fn refresh(&mut self) {
        self.drives = if_drive::enumerate();
        list_clear(self.list);
        for d in &self.drives {
            list_add(self.list, &d.display_name());
        }
        list_add(self.list, &format!("{} (test drive)", fake_info().display_name()));
        unsafe {
            let _ = windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
                self.list,
                windows_sys::Win32::UI::WindowsAndMessaging::LB_SETCURSEL,
                0,
                0,
            );
        }
    }

    /// The selected drive, or None when the list is empty.
    pub fn selected(&self) -> Option<DriveInfo> {
        let idx = list_selected(self.list).first().copied()?;
        if idx < self.drives.len() {
            Some(self.drives[idx].clone())
        } else {
            Some(fake_info())
        }
    }

    /// Short media summary for the selected drive (opens a session
    /// on the calling thread; used for status text only).
    pub fn media_summary(&self) -> String {
        let Some(info) = self.selected() else {
            return "No drive selected.".to_string();
        };
        match if_drive::open(&info).and_then(|s| s.media()) {
            Ok(m) if m.present => format!(
                "{} — {} free of {} bytes{}",
                m.kind,
                fmt_bytes(m.free_bytes),
                fmt_bytes(m.capacity_bytes),
                if m.blank {
                    ", blank"
                } else if m.appendable {
                    ", appendable"
                } else {
                    ", closed"
                }
            ),
            Ok(_) => "No disc inserted.".to_string(),
            Err(e) => format!("Media could not be read: {e}"),
        }
    }
}
