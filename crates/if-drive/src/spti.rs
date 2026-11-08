//! MMC over SPTI (SCSI Pass Through Direct) — the raw read side
//! (M5, D-003). Windows-only, like the rest of the real hardware
//! layer. Hand-declared: windows-sys carries the IOCTL number but
//! not the pass-through structures or MMC CDB layouts.
//!
//! Runtime behaviour is **Unverified** — no optical drive exists on
//! the development machines; the fake drive is the test gate, and
//! this backend follows the MMC-6 command layouts directly
//! (READ TOC 0x43, READ CD 0xBE, READ(10) 0x28 fallback).

use crate::{err, Result, Toc, TrackInfo};
use std::ffi::c_void;
use std::sync::Mutex;
use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows_sys::Win32::System::IO::DeviceIoControl;

const IOCTL_SCSI_PASS_THROUGH_DIRECT: u32 = 0x0004_d014;
const SCSI_IOCTL_DATA_IN: u8 = 1;

/// SCSI_PASS_THROUGH_DIRECT (ntddscsi.h) followed by 32 sense
/// bytes in the same allocation.
#[repr(C)]
struct Sptd {
    length: u16,
    scsi_status: u8,
    path_id: u8,
    target_id: u8,
    lun: u8,
    cdb_length: u8,
    sense_info_length: u8,
    data_in: u8,
    data_transfer_length: u32,
    time_out_value: u32,
    data_buffer: *mut c_void,
    sense_info_offset: u32,
    cdb: [u8; 16],
}

const SENSE_LEN: usize = 32;

/// A raw-reading session on one drive letter.
pub struct SptiDrive {
    handle: HANDLE,
    pub letter: char,
    toc_cache: Mutex<Option<Toc>>,
}

// The handle is used from one thread at a time by the job layer;
// SPTI commands themselves are synchronous.
unsafe impl Send for SptiDrive {}

impl SptiDrive {
    pub fn open(letter: char) -> Result<SptiDrive> {
        let path: Vec<u16> = format!("\\\\.\\{}:", letter.to_ascii_uppercase()).encode_utf16().chain([0]).collect();
        let handle = unsafe {
            CreateFileW(
                path.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                0,
                std::ptr::null_mut(),
            )
        };
        if handle as isize == -1 {
            // Read/write may need elevation on some systems; try
            // read-only before giving up.
            let handle = unsafe {
                CreateFileW(
                    path.as_ptr(),
                    GENERIC_READ,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    std::ptr::null(),
                    OPEN_EXISTING,
                    0,
                    std::ptr::null_mut(),
                )
            };
            if handle as isize == -1 {
                return err(format!("cannot open drive {letter}: for raw access (try running elevated)"));
            }
            return Ok(SptiDrive { handle, letter, toc_cache: Mutex::new(None) });
        }
        Ok(SptiDrive { handle, letter, toc_cache: Mutex::new(None) })
    }

    /// Runs one CDB with a data-in transfer; returns the bytes the
    /// device produced (exactly `out_len` on success).
    fn exec(&self, cdb: &[u8], out_len: usize, timeout_secs: u32) -> Result<Vec<u8>> {
        let mut data = vec![0u8; out_len];
        let total = std::mem::size_of::<Sptd>() + SENSE_LEN;
        let mut packet = vec![0u8; total];
        unsafe {
            let sptd = &mut *(packet.as_mut_ptr() as *mut Sptd);
            sptd.length = std::mem::size_of::<Sptd>() as u16;
            sptd.cdb_length = cdb.len() as u8;
            sptd.sense_info_length = SENSE_LEN as u8;
            sptd.data_in = SCSI_IOCTL_DATA_IN;
            sptd.data_transfer_length = out_len as u32;
            sptd.time_out_value = timeout_secs;
            sptd.data_buffer = data.as_mut_ptr() as *mut c_void;
            sptd.sense_info_offset = std::mem::size_of::<Sptd>() as u32;
            sptd.cdb[..cdb.len()].copy_from_slice(cdb);
            let mut returned = 0u32;
            let ok = DeviceIoControl(
                self.handle,
                IOCTL_SCSI_PASS_THROUGH_DIRECT,
                packet.as_mut_ptr() as *const c_void,
                total as u32,
                packet.as_mut_ptr() as *mut c_void,
                total as u32,
                &mut returned,
                std::ptr::null_mut(),
            );
            let sptd = &*(packet.as_ptr() as *const Sptd);
            if ok == 0 || sptd.scsi_status != 0 {
                let sense = &packet[std::mem::size_of::<Sptd>()..];
                let key = if sense.len() > 2 { sense[2] & 0x0f } else { 0 };
                return err(format!(
                    "drive {}: SCSI command 0x{:02x} failed (status {}, sense key {})",
                    self.letter, cdb[0], sptd.scsi_status, key
                ));
            }
        }
        Ok(data)
    }

    /// READ TOC (format 0, LBA form), parsed into the shared Toc.
    pub fn toc(&self) -> Result<Toc> {
        if let Some(t) = self.toc_cache.lock().unwrap().as_ref() {
            return Ok(t.clone());
        }
        // Header (4) + up to 100 track descriptors (8 each).
        let raw = self.exec(&[0x43, 0, 0, 0, 0, 0, 0, 0x03, 0x20, 0], 4 + 100 * 8, 30)?;
        if raw.len() < 4 {
            return err("the drive returned a truncated table of contents");
        }
        let first = raw[2] as u32;
        let last = raw[3] as u32;
        let mut starts: Vec<(u32, u32, bool)> = Vec::new(); // (number, lba, audio)
        let mut lead_out = 0u32;
        for chunk in raw[4..].as_chunks::<8>().0 {
            let control = chunk[1];
            let number = chunk[2] as u32;
            let lba = u32::from_be_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
            if number == 0xaa {
                lead_out = lba;
            } else if number >= first && number <= last {
                starts.push((number, lba, control & 0x04 == 0));
            }
        }
        if starts.is_empty() || lead_out == 0 {
            return err("the drive returned an empty table of contents");
        }
        starts.sort_by_key(|t| t.1);
        let mut tracks = Vec::new();
        for (i, (number, lba, audio)) in starts.iter().enumerate() {
            let end = if i + 1 < starts.len() { starts[i + 1].1 } else { lead_out };
            tracks.push(TrackInfo {
                number: *number,
                start_lba: *lba,
                sectors: end.saturating_sub(*lba),
                audio: *audio,
                title: None,
                performer: None,
            });
        }
        let audio_disc = tracks.iter().all(|t| t.audio);
        let toc = Toc { tracks, lead_out_lba: lead_out, sector_size: if audio_disc { 2352 } else { 2048 } };
        *self.toc_cache.lock().unwrap() = Some(toc.clone());
        Ok(toc)
    }

    /// READ CD for a run of sectors (fallback READ(10) for cooked
    /// data when the drive refuses READ CD).
    pub fn read_chunk(&self, lba: u32, count: u32, subchannel: bool) -> Result<Vec<u8>> {
        let toc = self.toc()?;
        let payload = toc.sector_size as usize;
        let stride = if subchannel { payload + 96 } else { payload };
        let mut cdb = [0u8; 12];
        cdb[0] = 0xbe; // READ CD
        cdb[2..6].copy_from_slice(&lba.to_be_bytes());
        cdb[6] = (count >> 16) as u8;
        cdb[7] = (count >> 8) as u8;
        cdb[8] = count as u8;
        cdb[9] = 0x10; // user data only (no sync/headers/ECC)
        cdb[10] = if subchannel { 1 } else { 0 }; // raw P-W subchannel
        match self.exec(&cdb, stride * count as usize, 60) {
            Ok(v) => Ok(v),
            Err(e) => {
                if toc.sector_size == 2048 && !subchannel {
                    // READ(10) fallback for cooked data sectors.
                    let mut c = [0u8; 10];
                    c[0] = 0x28;
                    c[2..6].copy_from_slice(&lba.to_be_bytes());
                    c[7] = (count >> 8) as u8;
                    c[8] = count as u8;
                    self.exec(&c, 2048 * count as usize, 60)
                } else {
                    Err(e)
                }
            }
        }
    }
}

impl Drop for SptiDrive {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.handle);
        }
    }
}
