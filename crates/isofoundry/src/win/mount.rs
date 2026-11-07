//! Mounting ISO images with Windows' own virtual disk support (D-005):
//! `OpenVirtualDisk` + `AttachVirtualDisk`, read only, until unmounted.
//! No driver is installed.

use super::util::{from_wide, wide};
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, HANDLE};
use windows_sys::Win32::Storage::FileSystem::{GetLogicalDriveStringsW, QueryDosDeviceW};
use windows_sys::Win32::Storage::Vhd::{
    AttachVirtualDisk, DetachVirtualDisk, GetVirtualDiskPhysicalPath, OpenVirtualDisk,
    ATTACH_VIRTUAL_DISK_FLAG_PERMANENT_LIFETIME, ATTACH_VIRTUAL_DISK_FLAG_READ_ONLY, DETACH_VIRTUAL_DISK_FLAG_NONE,
    OPEN_VIRTUAL_DISK_FLAG_NONE, VIRTUAL_DISK_ACCESS_ATTACH_RO, VIRTUAL_DISK_ACCESS_DETACH,
    VIRTUAL_DISK_ACCESS_GET_INFO, VIRTUAL_STORAGE_TYPE, VIRTUAL_STORAGE_TYPE_DEVICE_ISO,
    VIRTUAL_STORAGE_TYPE_VENDOR_MICROSOFT,
};

struct Disk(HANDLE);

impl Drop for Disk {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.0) };
    }
}

fn err(code: u32, what: &str) -> String {
    let m = std::io::Error::from_raw_os_error(code as i32);
    match code {
        5 | 1314 => format!("{what}: {m} (mounting may need administrator rights on this system)"),
        _ => format!("{what}: {m}"),
    }
}

fn open(iso: &std::path::Path, access: i32) -> Result<Disk, String> {
    let ty = VIRTUAL_STORAGE_TYPE {
        DeviceId: VIRTUAL_STORAGE_TYPE_DEVICE_ISO,
        VendorId: VIRTUAL_STORAGE_TYPE_VENDOR_MICROSOFT,
    };
    let path = wide(&iso.display().to_string());
    let mut h: HANDLE = std::ptr::null_mut();
    let r =
        unsafe { OpenVirtualDisk(&ty, path.as_ptr(), access, OPEN_VIRTUAL_DISK_FLAG_NONE, std::ptr::null(), &mut h) };
    if r != ERROR_SUCCESS {
        return Err(err(r, "cannot open the image"));
    }
    Ok(Disk(h))
}

/// `\\.\CDROMn` for an attached image.
fn physical_path(d: &Disk) -> Result<String, String> {
    let mut buf = [0u16; 512];
    let mut size = (buf.len() * 2) as u32;
    let r = unsafe { GetVirtualDiskPhysicalPath(d.0, &mut size, buf.as_mut_ptr()) };
    if r != ERROR_SUCCESS {
        return Err(err(r, "cannot find the mounted drive"));
    }
    Ok(from_wide(&buf))
}

/// Drive letters whose device is `\Device\CdRomN` for `\\.\CDROMn`.
fn letter_for(phys: &str) -> Option<String> {
    let n = phys.to_ascii_uppercase().rsplit("CDROM").next()?.to_string();
    let want = format!("\\DEVICE\\CDROM{n}");
    let mut buf = [0u16; 512];
    let len = unsafe { GetLogicalDriveStringsW(buf.len() as u32, buf.as_mut_ptr()) } as usize;
    for d in buf[..len.min(buf.len())].split(|&c| c == 0).filter(|s| !s.is_empty()) {
        let root = String::from_utf16_lossy(d); // "E:\"
        let name = wide(root.trim_end_matches('\\'));
        let mut target = [0u16; 512];
        let n = unsafe { QueryDosDeviceW(name.as_ptr(), target.as_mut_ptr(), target.len() as u32) };
        if n > 0 && from_wide(&target).to_ascii_uppercase() == want {
            return Some(root);
        }
    }
    None
}

/// Mounts `iso` read only. Returns the drive root (`E:\`) or, if Windows
/// gave no letter within a few seconds, the device path.
pub fn mount(iso: &std::path::Path) -> Result<String, String> {
    let d = open(iso, VIRTUAL_DISK_ACCESS_ATTACH_RO | VIRTUAL_DISK_ACCESS_GET_INFO)?;
    let r = unsafe {
        AttachVirtualDisk(
            d.0,
            std::ptr::null_mut(),
            ATTACH_VIRTUAL_DISK_FLAG_READ_ONLY | ATTACH_VIRTUAL_DISK_FLAG_PERMANENT_LIFETIME,
            0,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if r != ERROR_SUCCESS {
        return Err(err(r, "cannot mount the image"));
    }
    let phys = physical_path(&d)?;
    for _ in 0..50 {
        if let Some(l) = letter_for(&phys) {
            return Ok(l);
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    Ok(phys)
}

/// Unmounts an image mounted from `iso`.
pub fn unmount(iso: &std::path::Path) -> Result<(), String> {
    let d = open(iso, VIRTUAL_DISK_ACCESS_DETACH)?;
    let r = unsafe { DetachVirtualDisk(d.0, DETACH_VIRTUAL_DISK_FLAG_NONE, 0) };
    if r != ERROR_SUCCESS {
        return Err(err(r, "cannot unmount the image"));
    }
    Ok(())
}
