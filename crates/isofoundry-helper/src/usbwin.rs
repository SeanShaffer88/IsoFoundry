//! Windows physical-disk backend for the USB jobs (M2). Runs only in
//! the elevated helper. Everything destructive is gated by
//! `usbwriter::device::assess_target` before the first write, and the
//! disk's volumes are locked and dismounted before partitioning.
//!
//! Runtime status: **Unverified** — built and reviewed here, first
//! exercised on the owner's machine (VHDX or a real stick).

use if_core::ipc::{DiskDto, UsbJob, UsbJobKind};
use if_core::job::{CancelToken, Progress};
use std::ffi::c_void;
use std::path::Path;
use usbwriter::device::{assess_target, BlockDevice, Bus, DiskInfo};
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::{
    BusTypeFileBackedVirtual, BusTypeNvme, BusTypeSata, BusTypeUsb, BusTypeVirtual, CreateFileW, FindFirstVolumeW,
    FindNextVolumeW, FindVolumeClose, GetVolumeInformationW, FILE_SHARE_READ, FILE_SHARE_WRITE,
    IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, OPEN_EXISTING,
};
use windows_sys::Win32::System::Ioctl::{
    PropertyStandardQuery, StorageDeviceProperty, DISK_GEOMETRY_EX, FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME,
    IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, IOCTL_DISK_UPDATE_PROPERTIES, IOCTL_STORAGE_QUERY_PROPERTY,
    STORAGE_DEVICE_DESCRIPTOR, STORAGE_PROPERTY_QUERY, VOLUME_DISK_EXTENTS,
};
use windows_sys::Win32::System::IO::DeviceIoControl;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn win_err(what: &str) -> String {
    format!("{what} (Windows error {})", unsafe { GetLastError() })
}

struct Owned(HANDLE);
impl Drop for Owned {
    fn drop(&mut self) {
        if self.0 != INVALID_HANDLE_VALUE && !self.0.is_null() {
            unsafe { CloseHandle(self.0) };
        }
    }
}

fn open_handle(path: &str, access: u32, share: u32) -> Option<Owned> {
    let p = wide(path);
    // SAFETY: path is NUL-terminated; the handle is owned by the caller.
    let h = unsafe { CreateFileW(p.as_ptr(), access, share, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut()) };
    if h == INVALID_HANDLE_VALUE {
        None
    } else {
        Some(Owned(h))
    }
}

fn ioctl(handle: HANDLE, code: u32, input: &[u8], output: &mut [u8]) -> Result<u32, String> {
    let mut returned = 0u32;
    // SAFETY: buffers are sized by the caller and outlive the call.
    let ok = unsafe {
        DeviceIoControl(
            handle,
            code,
            if input.is_empty() { std::ptr::null() } else { input.as_ptr().cast::<c_void>() },
            input.len() as u32,
            if output.is_empty() { std::ptr::null_mut() } else { output.as_mut_ptr().cast::<c_void>() },
            output.len() as u32,
            &mut returned,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        Err(win_err("device control failed"))
    } else {
        Ok(returned)
    }
}

/// Disk numbers hosting the volume at `path` (a drive-letter path like
/// `\\.\C:` or a volume GUID path without trailing backslash).
fn volume_disks(path: &str) -> Vec<u32> {
    let Some(h) = open_handle(path, 0, FILE_SHARE_READ | FILE_SHARE_WRITE) else {
        return Vec::new();
    };
    let mut buf = vec![0u8; 4096];
    if ioctl(h.0, IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, &[], &mut buf).is_err() {
        return Vec::new();
    }
    // SAFETY: the buffer holds a VOLUME_DISK_EXTENTS from the driver.
    let header = unsafe { &*(buf.as_ptr() as *const VOLUME_DISK_EXTENTS) };
    let n = header.NumberOfDiskExtents as usize;
    let extents = unsafe {
        std::slice::from_raw_parts(
            buf.as_ptr().add(std::mem::size_of::<u32>() * 2) as *const windows_sys::Win32::System::Ioctl::DISK_EXTENT,
            n.min(64),
        )
    };
    extents.iter().map(|e| e.DiskNumber).collect()
}

fn disk_size(handle: HANDLE) -> u64 {
    let mut buf = [0u8; 512];
    if ioctl(handle, IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, &[], &mut buf).is_err() {
        return 0;
    }
    let g = unsafe { &*(buf.as_ptr() as *const DISK_GEOMETRY_EX) };
    g.DiskSize.max(0) as u64
}

fn c_string_at(buf: &[u8], offset: u32) -> String {
    if offset == 0 || offset as usize >= buf.len() {
        return String::new();
    }
    let bytes = &buf[offset as usize..];
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}

/// The set of disk numbers Windows itself lives on (the volume holding
/// the Windows folder) — treated as both system and boot disks.
fn os_disks() -> Vec<u32> {
    use windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW;
    let mut buf = [0u16; 260];
    // SAFETY: fixed-size buffer, documented API.
    let n = unsafe { GetWindowsDirectoryW(buf.as_mut_ptr(), buf.len() as u32) };
    if n < 2 {
        return Vec::new();
    }
    let letter = String::from_utf16_lossy(&buf[..2.min(n as usize)]);
    volume_disks(&format!("\\\\.\\{letter}"))
}

fn pagefile_disks() -> Vec<u32> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_MULTI_SZ};
    let sub = wide("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Memory Management");
    let name = wide("PagingFiles");
    let mut buf = vec![0u16; 2048];
    let mut size = (buf.len() * 2) as u32;
    // SAFETY: buffer sized for the value; size in/out is per the docs.
    let rc = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            sub.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_MULTI_SZ,
            std::ptr::null_mut(),
            buf.as_mut_ptr().cast::<c_void>(),
            &mut size,
        )
    };
    if rc != 0 {
        return Vec::new();
    }
    let text = String::from_utf16_lossy(&buf[..(size as usize / 2).min(buf.len())]);
    let mut out = Vec::new();
    for part in text.split('\0').filter(|p| p.len() >= 2) {
        let letter = &part[..2];
        if letter.ends_with(':') {
            out.extend(volume_disks(&format!("\\\\.\\{letter}")));
        }
    }
    out
}

/// Volume labels per disk number, from every drive letter.
fn labels_by_disk() -> std::collections::HashMap<u32, Vec<String>> {
    let mut map: std::collections::HashMap<u32, Vec<String>> = std::collections::HashMap::new();
    for c in b'A'..=b'Z' {
        let letter = format!("{}:\\", c as char);
        let root = wide(&letter);
        let mut label = [0u16; 261];
        // SAFETY: buffers are valid for the call.
        let ok = unsafe {
            GetVolumeInformationW(
                root.as_ptr(),
                label.as_mut_ptr(),
                label.len() as u32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                0,
            )
        };
        if ok == 0 {
            continue;
        }
        let text: String =
            label.iter().take_while(|u| **u != 0).map(|u| char::from_u32(*u as u32).unwrap_or(' ')).collect();
        let text = text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        for disk in volume_disks(&format!("\\\\.\\{}:", c as char)) {
            map.entry(disk).or_default().push(text.clone());
        }
    }
    map
}

/// Enumerates physical drives 0..=31 that answer.
pub fn enumerate() -> Vec<DiskInfo> {
    let os = os_disks();
    let pf = pagefile_disks();
    let labels = labels_by_disk();
    let mut out = Vec::new();
    for n in 0..32u32 {
        let Some(h) =
            open_handle(&format!("\\\\.\\PhysicalDrive{n}"), GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE)
        else {
            continue;
        };
        let size = disk_size(h.0);
        if size == 0 {
            continue;
        }
        // Storage descriptor: model + bus + removable.
        let query = STORAGE_PROPERTY_QUERY {
            PropertyId: StorageDeviceProperty,
            QueryType: PropertyStandardQuery,
            AdditionalParameters: [0],
        };
        let qbytes = unsafe {
            std::slice::from_raw_parts(
                &query as *const STORAGE_PROPERTY_QUERY as *const u8,
                std::mem::size_of::<STORAGE_PROPERTY_QUERY>(),
            )
        };
        let mut dbuf = vec![0u8; 1024];
        let (model, bus, removable) = if ioctl(h.0, IOCTL_STORAGE_QUERY_PROPERTY, qbytes, &mut dbuf).is_ok() {
            let d = unsafe { &*(dbuf.as_ptr() as *const STORAGE_DEVICE_DESCRIPTOR) };
            let vendor = c_string_at(&dbuf, d.VendorIdOffset);
            let product = c_string_at(&dbuf, d.ProductIdOffset);
            let model = [vendor, product].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join(" ");
            let bus = if d.BusType == BusTypeUsb {
                Bus::Usb
            } else if d.BusType == BusTypeNvme {
                Bus::Nvme
            } else if d.BusType == BusTypeSata {
                Bus::Sata
            } else if d.BusType == BusTypeVirtual || d.BusType == BusTypeFileBackedVirtual {
                Bus::Virtual
            } else {
                Bus::Other
            };
            (model, bus, d.RemovableMedia != 0)
        } else {
            (String::new(), Bus::Other, false)
        };
        out.push(DiskInfo {
            number: n,
            model: if model.is_empty() { format!("Disk {n}") } else { model },
            size_bytes: size,
            bus,
            removable,
            is_system: os.contains(&n),
            holds_boot: os.contains(&n),
            holds_pagefile: pf.contains(&n),
            volume_labels: labels.get(&n).cloned().unwrap_or_default(),
            is_file_target: false,
        });
    }
    out
}

/// A BlockDevice over an open physical-drive handle, with
/// read-modify-write for ranges that are not sector aligned (FAT
/// directory entries write 32-byte pieces).
pub struct PhysicalDevice {
    handle: Owned,
    size: u64,
}

impl PhysicalDevice {
    fn read_sectors(&self, off: u64, buf: &mut [u8]) -> Result<(), String> {
        use windows_sys::Win32::Storage::FileSystem::{ReadFile, SetFilePointerEx, FILE_BEGIN};
        // SAFETY: handle is open; pointer set immediately before the read.
        unsafe {
            let mut pos = 0i64;
            if SetFilePointerEx(self.handle.0, off as i64, &mut pos, FILE_BEGIN) == 0 {
                return Err(win_err("seek failed"));
            }
            let mut read = 0u32;
            if ReadFile(self.handle.0, buf.as_mut_ptr(), buf.len() as u32, &mut read, std::ptr::null_mut()) == 0 {
                return Err(win_err("read failed"));
            }
            if read as usize != buf.len() {
                return Err("short read from the disk".into());
            }
        }
        Ok(())
    }

    fn write_sectors(&mut self, off: u64, buf: &[u8]) -> Result<(), String> {
        use windows_sys::Win32::Storage::FileSystem::{SetFilePointerEx, WriteFile, FILE_BEGIN};
        // SAFETY: handle is open; pointer set immediately before the write.
        unsafe {
            let mut pos = 0i64;
            if SetFilePointerEx(self.handle.0, off as i64, &mut pos, FILE_BEGIN) == 0 {
                return Err(win_err("seek failed"));
            }
            let mut written = 0u32;
            if WriteFile(self.handle.0, buf.as_ptr(), buf.len() as u32, &mut written, std::ptr::null_mut()) == 0 {
                return Err(win_err("write failed"));
            }
            if written as usize != buf.len() {
                return Err("short write to the disk".into());
            }
        }
        Ok(())
    }
}

const SECTOR: u64 = 512;

impl BlockDevice for PhysicalDevice {
    fn size(&self) -> u64 {
        self.size
    }

    fn read_at(&self, off: u64, buf: &mut [u8]) -> Result<(), usbwriter::Error> {
        let start = off / SECTOR * SECTOR;
        let end = (off + buf.len() as u64).div_ceil(SECTOR) * SECTOR;
        let mut tmp = vec![0u8; (end - start) as usize];
        self.read_sectors(start, &mut tmp).map_err(usbwriter::Error)?;
        let skip = (off - start) as usize;
        buf.copy_from_slice(&tmp[skip..skip + buf.len()]);
        Ok(())
    }

    fn write_at(&mut self, off: u64, buf: &[u8]) -> Result<(), usbwriter::Error> {
        if off % SECTOR == 0 && buf.len() as u64 % SECTOR == 0 {
            return self.write_sectors(off, buf).map_err(usbwriter::Error);
        }
        // Read-modify-write the covering sectors.
        let start = off / SECTOR * SECTOR;
        let end = (off + buf.len() as u64).div_ceil(SECTOR) * SECTOR;
        let mut tmp = vec![0u8; (end - start) as usize];
        self.read_sectors(start, &mut tmp).map_err(usbwriter::Error)?;
        let skip = (off - start) as usize;
        tmp[skip..skip + buf.len()].copy_from_slice(buf);
        self.write_sectors(start, &tmp).map_err(usbwriter::Error)
    }

    fn flush(&mut self) -> Result<(), usbwriter::Error> {
        use windows_sys::Win32::Storage::FileSystem::FlushFileBuffers;
        // SAFETY: open handle.
        if unsafe { FlushFileBuffers(self.handle.0) } == 0 {
            return Err(usbwriter::Error(win_err("flush failed")));
        }
        Ok(())
    }
}

/// Locks and dismounts every volume living on `disk`. Returns the open
/// volume handles — dropping them releases the locks. Failures to lock
/// are fatal: writing to a mounted disk is how data gets lost.
fn lock_volumes(disk: u32) -> Result<Vec<Owned>, String> {
    let mut held = Vec::new();
    let mut name_buf = [0u16; 1024];
    // SAFETY: buffer sized for volume names; handles closed per the API.
    let find = unsafe { FindFirstVolumeW(name_buf.as_mut_ptr(), name_buf.len() as u32) };
    if find == INVALID_HANDLE_VALUE {
        return Ok(held);
    }
    loop {
        let name = String::from_utf16_lossy(&name_buf[..name_buf.iter().position(|u| *u == 0).unwrap_or(0)]);
        let open_path = name.trim_end_matches('\\').to_string();
        if volume_disks(&open_path).contains(&disk) {
            if let Some(h) = open_handle(&open_path, GENERIC_READ | GENERIC_WRITE, FILE_SHARE_READ | FILE_SHARE_WRITE) {
                ioctl(h.0, FSCTL_LOCK_VOLUME, &[], &mut [])
                    .map_err(|e| format!("cannot lock a volume on disk {disk}: {e}"))?;
                ioctl(h.0, FSCTL_DISMOUNT_VOLUME, &[], &mut [])
                    .map_err(|e| format!("cannot dismount a volume on disk {disk}: {e}"))?;
                held.push(h);
            }
        }
        name_buf = [0u16; 1024];
        // SAFETY: find handle is valid until FindVolumeClose.
        if unsafe { FindNextVolumeW(find, name_buf.as_mut_ptr(), name_buf.len() as u32) } == 0 {
            break;
        }
    }
    // SAFETY: find handle from FindFirstVolumeW.
    unsafe { FindVolumeClose(find) };
    Ok(held)
}

/// Finds the volume GUID path (`\\?\Volume{...}\`) for the partition at
/// `first_lba` on `disk`, waiting for Windows to notice the new layout.
fn wait_for_volume(disk: u32, first_lba: u64) -> Option<String> {
    for _ in 0..24 {
        let mut name_buf = [0u16; 1024];
        // SAFETY: as in lock_volumes.
        let find = unsafe { FindFirstVolumeW(name_buf.as_mut_ptr(), name_buf.len() as u32) };
        if find == INVALID_HANDLE_VALUE {
            return None;
        }
        let mut found = None;
        loop {
            let name = String::from_utf16_lossy(&name_buf[..name_buf.iter().position(|u| *u == 0).unwrap_or(0)]);
            let open_path = name.trim_end_matches('\\').to_string();
            if let Some(h) = open_handle(&open_path, 0, FILE_SHARE_READ | FILE_SHARE_WRITE) {
                let mut buf = vec![0u8; 4096];
                if ioctl(h.0, IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, &[], &mut buf).is_ok() {
                    // SAFETY: driver-filled VOLUME_DISK_EXTENTS.
                    let header = unsafe { &*(buf.as_ptr() as *const VOLUME_DISK_EXTENTS) };
                    if header.NumberOfDiskExtents >= 1 {
                        let ext =
                            unsafe { &*(buf.as_ptr().add(8) as *const windows_sys::Win32::System::Ioctl::DISK_EXTENT) };
                        if ext.DiskNumber == disk && ext.StartingOffset == (first_lba * SECTOR) as i64 {
                            found = Some(name.clone());
                        }
                    }
                }
            }
            if found.is_some() {
                break;
            }
            name_buf = [0u16; 1024];
            // SAFETY: valid find handle.
            if unsafe { FindNextVolumeW(find, name_buf.as_mut_ptr(), name_buf.len() as u32) } == 0 {
                break;
            }
        }
        // SAFETY: valid find handle.
        unsafe { FindVolumeClose(find) };
        if found.is_some() {
            return found;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    None
}

/// Formats a volume as NTFS through fmifs.dll's FormatEx (the same API
/// Windows' own format dialog uses). Quick format, default cluster size.
fn format_ntfs(volume_path: &str, label: &str) -> Result<(), String> {
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    type Callback = unsafe extern "system" fn(i32, u32, *mut c_void) -> u8;
    type FormatEx = unsafe extern "system" fn(*const u16, i32, *const u16, *const u16, u8, u32, Option<Callback>) -> u8;
    unsafe extern "system" fn cb(_packet: i32, _param: u32, _data: *mut c_void) -> u8 {
        1 // TRUE: continue
    }
    // SAFETY: fmifs.dll is a Windows system DLL; the entry point's
    // signature is the documented one. Failure modes are a missing DLL
    // or a FALSE return, both surfaced as errors.
    unsafe {
        let dll = LoadLibraryW(wide("fmifs.dll").as_ptr());
        if dll.is_null() {
            return Err("fmifs.dll (the Windows format library) is unavailable".into());
        }
        let proc = GetProcAddress(dll, c"FormatEx".as_ptr().cast());
        let Some(proc) = proc else {
            return Err("fmifs.dll has no FormatEx entry point".into());
        };
        let f: FormatEx = std::mem::transmute(proc);
        let root = wide(volume_path);
        let fs = wide("NTFS");
        let lbl = wide(label);
        // Media flag 0 = FmIfsMediaUnknown.
        let ok = f(root.as_ptr(), 0, fs.as_ptr(), lbl.as_ptr(), 1, 0, Some(cb));
        if ok == 0 {
            return Err("the NTFS format failed (FormatEx returned FALSE)".into());
        }
    }
    Ok(())
}

/// Runs one USB job against a physical disk.
pub fn run_job(job: &UsbJob, progress: &Progress) -> Result<(usize, u64, Vec<String>), String> {
    let disks = enumerate();
    let info = disks
        .iter()
        .find(|d| d.number == job.disk_number)
        .ok_or_else(|| format!("disk {} is no longer present", job.disk_number))?
        .clone();
    assess_target(&info, false, false).map_err(|e| e.0)?;
    // Re-check immediately before the destructive step against a fresh
    // enumeration (the list the user picked from may be stale).
    let _held_locks = lock_volumes(job.disk_number)?;
    let handle = open_handle(&format!("\\\\.\\PhysicalDrive{}", job.disk_number), GENERIC_READ | GENERIC_WRITE, 0)
        .ok_or_else(|| win_err("cannot open the disk for writing"))?;
    let size = disk_size(handle.0);
    let mut dev = PhysicalDevice { handle, size };
    let cancel = CancelToken::new();
    match &job.kind {
        UsbJobKind::Files { image, scheme, windows_installer } => {
            let scheme = usbwriter::install::Scheme::parse(scheme).map_err(|e| e.0)?;
            if *windows_installer {
                usbwriter::install::check_windows_installer(Path::new(image)).map_err(|e| e.0)?;
            }
            if scheme.is_ntfs() {
                let spec = usbwriter::install::partition(&mut dev, scheme).map_err(|e| e.0)?;
                // Tell Windows the layout changed, then find the new volume.
                let _ = ioctl(dev.handle.0, IOCTL_DISK_UPDATE_PROPERTIES, &[], &mut []);
                drop(dev);
                let volume = wait_for_volume(job.disk_number, spec.first_lba)
                    .ok_or("Windows did not surface the new partition as a volume")?;
                format_ntfs(&volume, if *windows_installer { "WINSETUP" } else { "BOOT" })?;
                // Copy the tree with if-image's extractor.
                let (_, src) = if_image::container::open(Path::new(image)).map_err(|e| e.to_string())?;
                let mut vol = if_image::fs::Volume::open(src, None).map_err(|e| e.to_string())?;
                let root = vol.root();
                let dest = Path::new(&volume).to_path_buf();
                let last = std::cell::Cell::new(0u64);
                let rep = vol
                    .extract(&root, &dest, |done, total| {
                        progress.set_total(total);
                        progress.add(done.saturating_sub(last.get()));
                        last.set(done);
                        !cancel.is_cancelled()
                    })
                    .map_err(|e| e.to_string())?;
                return Ok((rep.files as usize, rep.bytes, vec![]));
            }
            let report =
                usbwriter::install::copy_iso_tree(dev, Path::new(image), scheme, *windows_installer, progress, &cancel)
                    .map_err(|e| e.0)?;
            Ok((report.files, report.bytes, report.notes))
        }
        UsbJobKind::RawWrite { image, verify } => {
            let total = std::fs::metadata(image).map(|m| m.len()).unwrap_or(0);
            let n = usbwriter::raw::image_to_device(Path::new(image), &mut dev, progress, &cancel).map_err(|e| e.0)?;
            let mut notes = Vec::new();
            if *verify {
                let a = usbwriter::raw::hash_device_range(&dev, total).map_err(|e| e.0)?;
                let b = usbwriter::raw::hash_file(Path::new(image)).map_err(|e| e.0)?;
                if a != b {
                    return Err("verify failed: the written data does not match the image".into());
                }
                notes.push("write verified (SHA-256 match)".into());
            }
            Ok((0, n, notes))
        }
        UsbJobKind::ReadImage { out, limit_bytes } => {
            let n = usbwriter::raw::device_to_image(&dev, Path::new(out), *limit_bytes, progress, &cancel)
                .map_err(|e| e.0)?;
            Ok((0, n, vec![]))
        }
    }
}

pub fn to_dto(d: &DiskInfo) -> DiskDto {
    DiskDto {
        number: d.number,
        model: d.model.clone(),
        size_bytes: d.size_bytes,
        bus: match d.bus {
            Bus::Usb => "usb",
            Bus::Nvme => "nvme",
            Bus::Sata => "sata",
            Bus::Scsi => "scsi",
            Bus::Virtual => "virtual",
            Bus::Other => "other",
        }
        .to_string(),
        removable: d.removable,
        is_system: d.is_system,
        holds_boot: d.holds_boot,
        holds_pagefile: d.holds_pagefile,
        volume_labels: d.volume_labels.clone(),
    }
}
