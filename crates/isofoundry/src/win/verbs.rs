//! Explorer verbs (M6): per-user (HKCU) right-click entries for
//! disc image files, registered under
//! `Software\Classes\SystemFileAssociations\<ext>\shell\<verb>`.
//! No administrator rights, nothing machine-wide; uninstall removes
//! exactly the keys install created.

use super::util::wide;
use windows_sys::Win32::System::Registry::*;

fn verb_key(ext: &str, name: &str) -> String {
    format!(r"Software\Classes\SystemFileAssociations\{ext}\shell\{name}")
}

unsafe fn set_value(key_path: &str, value_name: Option<&str>, data: &str) -> Result<(), String> {
    let path = wide(key_path);
    let mut key: HKEY = std::ptr::null_mut();
    let rc = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            0,
            std::ptr::null(),
            0,
            KEY_WRITE,
            std::ptr::null(),
            &mut key,
            std::ptr::null_mut(),
        )
    };
    if rc != 0 {
        return Err(format!("cannot create the registry key for {key_path} (error {rc})"));
    }
    let name_w = value_name.map(wide);
    let data_w = wide(data);
    let rc = unsafe {
        RegSetValueExW(
            key,
            name_w.as_ref().map(|w| w.as_ptr()).unwrap_or(std::ptr::null()),
            0,
            REG_SZ,
            data_w.as_ptr() as *const u8,
            (data_w.len() * 2) as u32,
        )
    };
    unsafe { RegCloseKey(key) };
    if rc != 0 {
        return Err(format!("cannot write the registry value for {key_path} (error {rc})"));
    }
    Ok(())
}

unsafe fn key_exists(key_path: &str) -> bool {
    let path = wide(key_path);
    let mut key: HKEY = std::ptr::null_mut();
    let rc = unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, KEY_READ, &mut key) };
    if rc == 0 {
        unsafe { RegCloseKey(key) };
        true
    } else {
        false
    }
}

unsafe fn delete_tree(key_path: &str) {
    let path = wide(key_path);
    unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, path.as_ptr()) };
}

pub(crate) fn run(sub: &str) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find our own path: {e}"))?;
    let dir = exe.parent().ok_or("cannot find our own folder")?.to_path_buf();
    let verbs = crate::cli_meta::verb_definitions(&dir);
    match sub {
        "install" => {
            for v in &verbs {
                let base = verb_key(&v.ext, &v.name);
                unsafe {
                    set_value(&base, None, &v.label)?;
                    set_value(&format!(r"{base}\command"), None, &v.command)?;
                }
            }
            Ok(format!(
                "Explorer verbs installed for {} image types (Checksum / Image info, right-click a disc image). Remove them with: explorer-verbs uninstall",
                verbs.len() / 2
            ))
        }
        "uninstall" => {
            for v in &verbs {
                unsafe { delete_tree(&verb_key(&v.ext, &v.name)) };
            }
            Ok("Explorer verbs removed.".into())
        }
        "status" => {
            let installed = verbs.iter().filter(|v| unsafe { key_exists(&verb_key(&v.ext, &v.name)) }).count();
            Ok(if installed == 0 {
                "Explorer verbs are not installed. Add them with: explorer-verbs install".into()
            } else {
                format!("Explorer verbs are installed ({installed} of {} entries present).", verbs.len())
            })
        }
        other => Err(format!("unknown explorer-verbs action {other} (install, uninstall, status)")),
    }
}
