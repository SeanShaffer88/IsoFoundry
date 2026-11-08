//! The IMAPI2 backend (Windows). COM interfaces are called through
//! their v-tables directly: windows-sys 0.59 ships the IMAPI2
//! CLSIDs and enums but not the interface definitions, so the
//! layouts below are hand-declared from the authoritative generated
//! bindings (the `windows` crate, Storage::Imapi) — every slot index
//! and signature was transcribed from that source. Runtime
//! behaviour is **Unverified**: this milestone's test gate is the
//! fake drive (roadmap M4), and the machine this was developed on
//! has no optical drive.
//!
//! Apartment rule: an [`ImapiSession`] creates its COM objects on
//! the calling thread and must be used from that thread. Enumeration
//! returns plain data, so callers enumerate wherever they like and
//! open a session where the work happens.

use crate::{err, BurnOptions, BurnReport, BurnSource, DriveInfo, EraseKind, MediaInfo, OpticalDrive, Result};
use if_core::job::{CancelToken, Progress};
use std::ffi::c_void;
use std::path::Path;
use windows_sys::core::GUID;
use windows_sys::Win32::Foundation::{SysAllocString, SysFreeString, SysStringLen, VARIANT_BOOL};
use windows_sys::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, SAFEARRAY,
};
use windows_sys::Win32::UI::Shell::SHCreateStreamOnFileEx;

type HResult = i32;
const S_OK: HResult = 0;
const RPC_E_CHANGED_MODE: HResult = -2147417850; // 0x80010106 — COM already initialized with another model

const fn guid(v: u128) -> GUID {
    GUID { data1: (v >> 96) as u32, data2: (v >> 80) as u16, data3: (v >> 64) as u16, data4: (v as u64).to_be_bytes() }
}

// CLSIDs (windows-sys Storage::Imapi) and IIDs (windows crate bindings).
const CLSID_DISC_MASTER2: GUID = guid(0x2735412e_7f64_5b0f_8f00_5d77afbe261e);
const IID_DISC_MASTER2: GUID = guid(0x27354130_7f64_5b0f_8f00_5d77afbe261e);
const CLSID_DISC_RECORDER2: GUID = guid(0x2735412d_7f64_5b0f_8f00_5d77afbe261e);
const IID_DISC_RECORDER2: GUID = guid(0x27354133_7f64_5b0f_8f00_5d77afbe261e);
const CLSID_FORMAT2_DATA: GUID = guid(0x2735412a_7f64_5b0f_8f00_5d77afbe261e);
const IID_FORMAT2_DATA: GUID = guid(0x27354153_9f64_5b0f_8f00_5d77afbe261e);
const CLSID_FORMAT2_TAO: GUID = guid(0x27354129_7f64_5b0f_8f00_5d77afbe261e);
const IID_FORMAT2_TAO: GUID = guid(0x27354154_8f64_5b0f_8f00_5d77afbe261e);
const CLSID_FORMAT2_ERASE: GUID = guid(0x2735412b_7f64_5b0f_8f00_5d77afbe261e);
const IID_FORMAT2_ERASE: GUID = guid(0x27354156_8f64_5b0f_8f00_5d77afbe261e);
const CLSID_FILE_SYSTEM_IMAGE: GUID = guid(0x2c941fc5_975b_59be_a960_9a2a262853a5);
const IID_FILE_SYSTEM_IMAGE: GUID = guid(0x2c941fe1_975b_59be_a960_9a2a262853a5);
const IID_FSI_DIRECTORY_ITEM: GUID = guid(0x2c941fdc_975b_59be_a960_9a2a262853a5);
const IID_FILE_SYSTEM_IMAGE_RESULT: GUID = guid(0x2c941fd8_975b_59be_a960_9a2a262853a5);

// V-table slots (IDispatch base = 0..6: QI, AddRef, Release, type info x4).
mod slot {
    pub const RELEASE: usize = 2;
    pub mod master {
        pub const GET_ITEM: usize = 8;
        pub const COUNT: usize = 9;
    }
    pub mod recorder {
        pub const EJECT_MEDIA: usize = 7;
        pub const INITIALIZE: usize = 13;
        pub const VENDOR_ID: usize = 15;
        pub const PRODUCT_ID: usize = 16;
        pub const PRODUCT_REVISION: usize = 17;
        pub const VOLUME_PATH_NAMES: usize = 19;
    }
    pub mod format2 {
        pub const IS_RECORDER_SUPPORTED: usize = 7;
        pub const IS_CURRENT_MEDIA_SUPPORTED: usize = 8;
        pub const MEDIA_PHYSICALLY_BLANK: usize = 9;
    }
    pub mod format_data {
        pub const SET_RECORDER: usize = 12;
        pub const CURRENT_MEDIA_STATUS: usize = 18;
        pub const TOTAL_SECTORS: usize = 20;
        pub const FREE_SECTORS: usize = 21;
        pub const SET_FORCE_CLOSED: usize = 25;
        pub const CURRENT_MEDIA_TYPE: usize = 29;
        pub const SET_CLIENT_NAME: usize = 30;
        pub const MULTISESSION_INTERFACES: usize = 40;
        pub const WRITE: usize = 41;
        pub const SET_WRITE_SPEED: usize = 43;
    }
    pub mod format_tao {
        pub const PREPARE_MEDIA: usize = 12;
        pub const ADD_AUDIO_TRACK: usize = 13;
        pub const RELEASE_MEDIA: usize = 15;
        pub const SET_RECORDER: usize = 17;
        pub const SET_DO_NOT_FINALIZE: usize = 25;
        pub const SET_CLIENT_NAME: usize = 29;
    }
    pub mod format_erase {
        pub const SET_RECORDER: usize = 12;
        pub const SET_FULL_ERASE: usize = 14;
        pub const SET_CLIENT_NAME: usize = 17;
        pub const ERASE_MEDIA: usize = 19;
    }
    pub mod fsi {
        pub const ROOT: usize = 7;
        pub const SET_VOLUME_NAME: usize = 15;
        pub const SET_FILE_SYSTEMS_TO_CREATE: usize = 29;
        pub const CHOOSE_IMAGE_DEFAULTS: usize = 34;
        pub const CREATE_RESULT_IMAGE: usize = 39;
        pub const IMPORT_FILE_SYSTEM: usize = 44;
        pub const SET_MULTISESSION_INTERFACES: usize = 56;
    }
    pub mod fsi_dir {
        pub const ADD_TREE: usize = 25;
    }
    pub mod fsi_result {
        pub const IMAGE_STREAM: usize = 7;
        pub const TOTAL_BLOCKS: usize = 9;
        pub const BLOCK_SIZE: usize = 10;
    }
}

unsafe fn vtbl(p: *mut c_void) -> *const usize {
    *(p as *const *const usize)
}

unsafe fn call<T>(p: *mut c_void, idx: usize) -> T {
    let f: unsafe extern "system" fn() -> T = std::mem::transmute_copy(&*vtbl(p).add(idx));
    f()
}

fn hr(what: &str, code: HResult) -> Result<()> {
    if code >= 0 {
        Ok(())
    } else {
        err(format!("{what} failed (HRESULT 0x{:08X})", code as u32))
    }
}

unsafe fn release(p: *mut c_void) {
    if !p.is_null() {
        let _: HResult = call(p, slot::RELEASE);
    }
}

/// A COM pointer released on drop.
struct ComPtr(*mut c_void);

impl ComPtr {
    fn get(&self) -> *mut c_void {
        self.0
    }
}

impl Drop for ComPtr {
    fn drop(&mut self) {
        unsafe { release(self.0) }
    }
}

unsafe fn create(clsid: &GUID, iid: &GUID) -> Result<ComPtr> {
    let mut p: *mut c_void = std::ptr::null_mut();
    let code = CoCreateInstance(clsid, std::ptr::null_mut(), CLSCTX_INPROC_SERVER, iid, &mut p);
    hr("creating the IMAPI object", code)?;
    if p.is_null() {
        return err("creating the IMAPI object returned no object");
    }
    Ok(ComPtr(p))
}

/// QueryInterface helper (e.g. a returned object re-queried for its
/// documented interface).
unsafe fn query(p: *mut c_void, iid: &GUID) -> Result<ComPtr> {
    type Qi = unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HResult;
    let f: Qi = std::mem::transmute_copy(&*vtbl(p).add(0));
    let mut out: *mut c_void = std::ptr::null_mut();
    hr("QueryInterface", f(p, iid, &mut out))?;
    Ok(ComPtr(out))
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Owned BSTR, freed on drop.
struct BStr(*mut u16);

impl BStr {
    fn new(s: &str) -> BStr {
        let w = wide(s);
        BStr(unsafe { SysAllocString(w.as_ptr()) } as *mut u16)
    }
    fn get(&self) -> *mut c_void {
        self.0 as *mut c_void
    }
}

impl Drop for BStr {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { SysFreeString(self.0) }
        }
    }
}

/// Reads a caller-owned BSTR (does not free it).
unsafe fn bstr_to_string(b: *mut u16) -> String {
    if b.is_null() {
        return String::new();
    }
    let len = SysStringLen(b) as usize;
    let slice = std::slice::from_raw_parts(b, len);
    String::from_utf16_lossy(slice)
}

/// Reads and frees a BSTR returned by a getter.
unsafe fn take_bstr(b: *mut u16) -> String {
    let s = bstr_to_string(b);
    if !b.is_null() {
        SysFreeString(b);
    }
    s
}

fn init_com() -> Result<()> {
    let code = unsafe { CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED as u32) };
    if code == S_OK || code == 1 || code == RPC_E_CHANGED_MODE {
        Ok(())
    } else {
        hr("initializing COM", code)
    }
}

type GetBstr = unsafe extern "system" fn(*mut c_void, *mut *mut u16) -> HResult;
type SetRecorder = unsafe extern "system" fn(*mut c_void, *mut c_void) -> HResult;

unsafe fn get_bstr(p: *mut c_void, idx: usize) -> Result<String> {
    let f: GetBstr = std::mem::transmute_copy(&*vtbl(p).add(idx));
    let mut b: *mut u16 = std::ptr::null_mut();
    hr("reading a drive property", f(p, &mut b))?;
    Ok(take_bstr(b))
}

unsafe fn get_bool2(p: *mut c_void, idx: usize, arg: *mut c_void) -> Result<bool> {
    type F = unsafe extern "system" fn(*mut c_void, *mut c_void, *mut VARIANT_BOOL) -> HResult;
    let f: F = std::mem::transmute_copy(&*vtbl(p).add(idx));
    let mut v: VARIANT_BOOL = 0;
    hr("querying the drive", f(p, arg, &mut v))?;
    Ok(v != 0)
}

unsafe fn get_i32(p: *mut c_void, idx: usize) -> Result<i32> {
    type F = unsafe extern "system" fn(*mut c_void, *mut i32) -> HResult;
    let f: F = std::mem::transmute_copy(&*vtbl(p).add(idx));
    let mut v: i32 = 0;
    hr("querying the media", f(p, &mut v))?;
    Ok(v)
}

/// Lists recorders as plain data (all COM objects are released
/// before returning, so this is callable from any thread).
pub fn enumerate() -> Result<Vec<DriveInfo>> {
    init_com()?;
    unsafe {
        let master = create(&CLSID_DISC_MASTER2, &IID_DISC_MASTER2)?;
        type Count = unsafe extern "system" fn(*mut c_void, *mut i32) -> HResult;
        let count_f: Count = std::mem::transmute_copy(&*vtbl(master.get()).add(slot::master::COUNT));
        let mut count: i32 = 0;
        hr("counting disc recorders", count_f(master.get(), &mut count))?;
        let mut out = Vec::new();
        for i in 0..count {
            type Item = unsafe extern "system" fn(*mut c_void, i32, *mut *mut u16) -> HResult;
            let item_f: Item = std::mem::transmute_copy(&*vtbl(master.get()).add(slot::master::GET_ITEM));
            let mut id_b: *mut u16 = std::ptr::null_mut();
            if item_f(master.get(), i, &mut id_b) < 0 {
                continue;
            }
            let id = take_bstr(id_b);
            if let Ok(info) = recorder_info(&id) {
                out.push(info);
            }
        }
        Ok(out)
    }
}

/// Reads one recorder's descriptive properties (transient object).
unsafe fn recorder_info(id: &str) -> Result<DriveInfo> {
    let recorder = create_recorder(id)?;
    let vendor = get_bstr(recorder.get(), slot::recorder::VENDOR_ID).unwrap_or_default();
    let product = get_bstr(recorder.get(), slot::recorder::PRODUCT_ID).unwrap_or_default();
    let revision = get_bstr(recorder.get(), slot::recorder::PRODUCT_REVISION).unwrap_or_default();
    // VolumePathNames: SAFEARRAY of BSTR like "E:\".
    type Paths = unsafe extern "system" fn(*mut c_void, *mut *mut SAFEARRAY) -> HResult;
    let f: Paths = std::mem::transmute_copy(&*vtbl(recorder.get()).add(slot::recorder::VOLUME_PATH_NAMES));
    let mut psa: *mut SAFEARRAY = std::ptr::null_mut();
    let mut letter = None;
    if f(recorder.get(), &mut psa) >= 0 && !psa.is_null() {
        let arr = &*psa;
        if arr.cDims >= 1 {
            let n = arr.rgsabound[0].cElements as usize;
            let data = arr.pvData as *const *mut u16;
            if n > 0 && !data.is_null() {
                let first = bstr_to_string(*data);
                let c = first.chars().next().unwrap_or(' ');
                if c.is_ascii_alphabetic() {
                    letter = Some(c.to_ascii_uppercase());
                }
            }
        }
        windows_sys::Win32::System::Ole::SafeArrayDestroy(psa);
    }
    Ok(DriveInfo { id: id.to_string(), letter, vendor, product, revision, backend: "IMAPI2".into() })
}

unsafe fn create_recorder(id: &str) -> Result<ComPtr> {
    let recorder = create(&CLSID_DISC_RECORDER2, &IID_DISC_RECORDER2)?;
    type Init = unsafe extern "system" fn(*mut c_void, *mut c_void) -> HResult;
    let f: Init = std::mem::transmute_copy(&*vtbl(recorder.get()).add(slot::recorder::INITIALIZE));
    let b = BStr::new(id);
    hr("opening the recorder", f(recorder.get(), b.get()))?;
    Ok(recorder)
}

/// A live IMAPI session: create it on the thread that will burn.
pub struct ImapiSession {
    info: DriveInfo,
    recorder: ComPtr,
    format: Option<ComPtr>,
    /// MMC/SPTI raw reader for the M5 read side; None when the
    /// drive has no letter or raw access was refused.
    reader: Option<crate::spti::SptiDrive>,
}

// The session owns raw COM pointers bound to its creating thread;
// it must not move across threads. Rust cannot express the
// apartment rule — callers uphold it (the app opens sessions on the
// job thread). Marking it Send would be a lie; it stays !Send by
// virtue of the raw pointers' documentation above. (ComPtr holds a
// raw pointer, which is Send+Sync by default in Rust — so be
// careful: never share an ImapiSession across threads.)

impl ImapiSession {
    pub fn open(info: DriveInfo) -> Result<ImapiSession> {
        init_com()?;
        let recorder = unsafe { create_recorder(&info.id)? };
        let reader = info.letter.and_then(|l| crate::spti::SptiDrive::open(l).ok());
        Ok(ImapiSession { info, recorder, format: None, reader })
    }

    /// Lazily creates (and caches) the IDiscFormat2Data object with
    /// this session's recorder set on it.
    unsafe fn format(&mut self) -> Result<*mut c_void> {
        if self.format.is_none() {
            let format = create(&CLSID_FORMAT2_DATA, &IID_FORMAT2_DATA)?;
            let f: SetRecorder = std::mem::transmute_copy(&*vtbl(format.get()).add(slot::format_data::SET_RECORDER));
            hr("binding the recorder", f(format.get(), self.recorder.get()))?;
            let name = BStr::new("IsoFoundry");
            type SetName = unsafe extern "system" fn(*mut c_void, *mut c_void) -> HResult;
            let nf: SetName = std::mem::transmute_copy(&*vtbl(format.get()).add(slot::format_data::SET_CLIENT_NAME));
            let _ = nf(format.get(), name.get());
            self.format = Some(format);
        }
        Ok(self.format.as_ref().unwrap().get())
    }
}

const MEDIA_TYPE_NAMES: &[(i32, &str)] = &[
    (1, "CD-ROM"),
    (2, "CD-R"),
    (3, "CD-RW"),
    (4, "DVD-ROM"),
    (5, "DVD-RAM"),
    (6, "DVD+R"),
    (7, "DVD+RW"),
    (8, "DVD+R DL"),
    (9, "DVD-R"),
    (10, "DVD-RW"),
    (11, "DVD-R DL"),
    (17, "BD-ROM"),
    (18, "BD-R"),
    (19, "BD-RE"),
];

fn media_type_name(t: i32) -> String {
    MEDIA_TYPE_NAMES
        .iter()
        .find(|(n, _)| *n == t)
        .map(|(_, s)| s.to_string())
        .unwrap_or_else(|| format!("media type {t}"))
}

impl OpticalDrive for ImapiSession {
    fn info(&self) -> Result<DriveInfo> {
        Ok(self.info.clone())
    }

    fn media(&self) -> Result<MediaInfo> {
        unsafe {
            // A throwaway session copy is not possible (the trait
            // takes &self), so build the format object locally.
            let format = create(&CLSID_FORMAT2_DATA, &IID_FORMAT2_DATA)?;
            let f: SetRecorder = std::mem::transmute_copy(&*vtbl(format.get()).add(slot::format_data::SET_RECORDER));
            hr("binding the recorder", f(format.get(), self.recorder.get()))?;
            let supported = get_bool2(format.get(), slot::format2::IS_CURRENT_MEDIA_SUPPORTED, self.recorder.get())
                .unwrap_or(false);
            if !supported {
                return Ok(MediaInfo::absent());
            }
            let media_type = get_i32(format.get(), slot::format_data::CURRENT_MEDIA_TYPE).unwrap_or(0);
            let blank = {
                type F = unsafe extern "system" fn(*mut c_void, *mut VARIANT_BOOL) -> HResult;
                let bf: F = std::mem::transmute_copy(&*vtbl(format.get()).add(slot::format2::MEDIA_PHYSICALLY_BLANK));
                let mut v: VARIANT_BOOL = 0;
                if bf(format.get(), &mut v) >= 0 {
                    v != 0
                } else {
                    false
                }
            };
            let total = get_i32(format.get(), slot::format_data::TOTAL_SECTORS).unwrap_or(0).max(0) as u64;
            let free = get_i32(format.get(), slot::format_data::FREE_SECTORS).unwrap_or(0).max(0) as u64;
            let status = get_i32(format.get(), slot::format_data::CURRENT_MEDIA_STATUS).unwrap_or(0);
            const APPENDABLE: i32 = 4;
            Ok(MediaInfo {
                present: true,
                kind: media_type_name(media_type),
                blank,
                appendable: status & APPENDABLE != 0,
                capacity_bytes: total * 2048,
                free_bytes: free * 2048,
            })
        }
    }

    fn burn(
        &mut self,
        source: &BurnSource,
        opts: &BurnOptions,
        progress: &Progress,
        cancel: &CancelToken,
    ) -> Result<BurnReport> {
        let _ = cancel; // IDiscFormat2Data::Write is blocking; CancelWrite is wired to the same token's thread in a fuller event-driven build (see LIMITATIONS).
        unsafe {
            let format_ptr = self.format()?;
            let supported = get_bool2(format_ptr, slot::format2::IS_RECORDER_SUPPORTED, self.recorder.get())?;
            if !supported {
                return err("this recorder is not supported by the IMAPI data format");
            }
            let media_supported =
                get_bool2(format_ptr, slot::format2::IS_CURRENT_MEDIA_SUPPORTED, self.recorder.get())?;
            if !media_supported {
                return err("no supported disc in the drive (or the media cannot be written by this format)");
            }
            if let Some(kbps) = opts.speed_kbps {
                // SetWriteSpeed takes sectors per second (2048-byte sectors).
                let sectors = ((kbps as i64 * 1024) / 8 / 2048).max(1) as i32;
                type F = unsafe extern "system" fn(*mut c_void, i32, VARIANT_BOOL) -> HResult;
                let f: F = std::mem::transmute_copy(&*vtbl(format_ptr).add(slot::format_data::SET_WRITE_SPEED));
                let _ = f(format_ptr, sectors, 0);
            }
            {
                type F = unsafe extern "system" fn(*mut c_void, VARIANT_BOOL) -> HResult;
                let f: F = std::mem::transmute_copy(&*vtbl(format_ptr).add(slot::format_data::SET_FORCE_CLOSED));
                let _ = f(format_ptr, if opts.close_disc { -1 } else { 0 });
            }

            // Build the stream to write.
            let (stream, total_bytes) = match source {
                BurnSource::Image(path) => {
                    let stream = stream_on_file(path)?;
                    let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
                    (stream, size)
                }
                BurnSource::Files { root, label } => self.build_files_stream(format_ptr, root, label, opts)?,
            };
            progress.set_total(total_bytes.max(1));

            let mut notes = Vec::new();
            if opts.simulate {
                notes.push("Simulated only — the IMAPI data format has no test-write mode, so checks ran and no burn was started.".to_string());
                return Ok(BurnReport { bytes_written: 0, verified: false, notes, backend: "IMAPI2".into() });
            }
            type Write = unsafe extern "system" fn(*mut c_void, *mut c_void) -> HResult;
            let wf: Write = std::mem::transmute_copy(&*vtbl(format_ptr).add(slot::format_data::WRITE));
            hr("burning the disc", wf(format_ptr, stream.get()))?;
            progress.add(total_bytes);
            if opts.verify {
                notes.push("Verify after burn is not available on the IMAPI path yet — reading a disc back arrives with the rip work (M5). The fake drive performs a real digest verify.".to_string());
            }
            if opts.eject_when_done {
                type F = unsafe extern "system" fn(*mut c_void) -> HResult;
                let f: F = std::mem::transmute_copy(&*vtbl(self.recorder.get()).add(slot::recorder::EJECT_MEDIA));
                let _ = f(self.recorder.get());
            }
            Ok(BurnReport { bytes_written: total_bytes, verified: false, notes, backend: "IMAPI2".into() })
        }
    }

    fn erase(&mut self, kind: EraseKind, progress: &Progress, _cancel: &CancelToken) -> Result<()> {
        unsafe {
            let erase = create(&CLSID_FORMAT2_ERASE, &IID_FORMAT2_ERASE)?;
            let f: SetRecorder = std::mem::transmute_copy(&*vtbl(erase.get()).add(slot::format_erase::SET_RECORDER));
            hr("binding the recorder", f(erase.get(), self.recorder.get()))?;
            type SetBool = unsafe extern "system" fn(*mut c_void, VARIANT_BOOL) -> HResult;
            let bf: SetBool = std::mem::transmute_copy(&*vtbl(erase.get()).add(slot::format_erase::SET_FULL_ERASE));
            hr("setting the erase mode", bf(erase.get(), if kind == EraseKind::Full { -1 } else { 0 }))?;
            let name = BStr::new("IsoFoundry");
            type SetName = unsafe extern "system" fn(*mut c_void, *mut c_void) -> HResult;
            let nf: SetName = std::mem::transmute_copy(&*vtbl(erase.get()).add(slot::format_erase::SET_CLIENT_NAME));
            let _ = nf(erase.get(), name.get());
            progress.set_total(1);
            type F = unsafe extern "system" fn(*mut c_void) -> HResult;
            let ef: F = std::mem::transmute_copy(&*vtbl(erase.get()).add(slot::format_erase::ERASE_MEDIA));
            hr("erasing the disc", ef(erase.get()))?;
            progress.add(1);
            Ok(())
        }
    }

    fn caps(&self) -> crate::Caps {
        crate::Caps {
            read: self.reader.is_some(),
            subchannel: self.reader.is_some(),
            audio_burn: true,
            dao_cd_text: false,
        }
    }

    fn toc(&self) -> Result<crate::Toc> {
        match &self.reader {
            Some(r) => r.toc(),
            None => err("raw disc reading is not available for this drive (no drive letter or access refused)"),
        }
    }

    fn read_chunk(&mut self, lba: u32, count: u32, subchannel: bool) -> Result<Vec<u8>> {
        match &self.reader {
            Some(r) => r.read_chunk(lba, count, subchannel),
            None => err("raw disc reading is not available for this drive (no drive letter or access refused)"),
        }
    }

    fn burn_audio(
        &mut self,
        tracks: &[crate::AudioTrackData],
        opts: &crate::AudioBurnOptions,
        progress: &Progress,
        cancel: &CancelToken,
    ) -> Result<BurnReport> {
        if tracks.is_empty() {
            return err("no audio tracks to burn");
        }
        for t in tracks {
            if t.pcm.len() % 2352 != 0 {
                return err(format!("track '{}' is not a whole number of CD frames", t.title));
            }
        }
        let total: u64 = tracks.iter().map(|t| t.pcm.len() as u64).sum();
        let mut notes =
            vec!["Written track-at-once through IMAPI2: fixed 2 s gaps, no CD-Text (the format cannot carry it)."
                .to_string()];
        if opts.simulate {
            let media = self.media()?;
            if !media.present {
                return err("no disc in the drive");
            }
            notes.push("Simulated only — nothing was written.".to_string());
            return Ok(BurnReport { bytes_written: 0, verified: false, notes, backend: "IMAPI2".into() });
        }
        progress.set_total(total);
        unsafe {
            let tao = create(&CLSID_FORMAT2_TAO, &IID_FORMAT2_TAO)?;
            type SetRec = unsafe extern "system" fn(*mut c_void, *mut c_void) -> HResult;
            let sr: SetRec = std::mem::transmute_copy(&*vtbl(tao.get()).add(slot::format_tao::SET_RECORDER));
            hr("binding the recorder", sr(tao.get(), self.recorder.get()))?;
            let name = BStr::new("IsoFoundry");
            type SetName = unsafe extern "system" fn(*mut c_void, *mut c_void) -> HResult;
            let nf: SetName = std::mem::transmute_copy(&*vtbl(tao.get()).add(slot::format_tao::SET_CLIENT_NAME));
            let _ = nf(tao.get(), name.get());
            if !opts.close_disc {
                type SetBool = unsafe extern "system" fn(*mut c_void, VARIANT_BOOL) -> HResult;
                let bf: SetBool =
                    std::mem::transmute_copy(&*vtbl(tao.get()).add(slot::format_tao::SET_DO_NOT_FINALIZE));
                let _ = bf(tao.get(), -1);
            }
            type F = unsafe extern "system" fn(*mut c_void) -> HResult;
            let prep: F = std::mem::transmute_copy(&*vtbl(tao.get()).add(slot::format_tao::PREPARE_MEDIA));
            hr("preparing the disc for audio tracks", prep(tao.get()))?;
            let dir = std::env::temp_dir().join(format!("isofoundry-tao-{}", std::process::id()));
            std::fs::create_dir_all(&dir).map_err(|e| crate::Error(e.to_string()))?;
            let mut result: Result<()> = (|| {
                for (i, t) in tracks.iter().enumerate() {
                    cancel.check().map_err(|e| crate::Error(e.to_string()))?;
                    let path = dir.join(format!("track{:02}.raw", i + 1));
                    std::fs::write(&path, &t.pcm).map_err(|e| crate::Error(e.to_string()))?;
                    let stream = stream_on_file(&path)?;
                    type Add = unsafe extern "system" fn(*mut c_void, *mut c_void) -> HResult;
                    let af: Add = std::mem::transmute_copy(&*vtbl(tao.get()).add(slot::format_tao::ADD_AUDIO_TRACK));
                    hr(&format!("adding track {}", i + 1), af(tao.get(), stream.get()))?;
                    progress.add(t.pcm.len() as u64);
                }
                Ok(())
            })();
            if result.is_ok() {
                let rel: F = std::mem::transmute_copy(&*vtbl(tao.get()).add(slot::format_tao::RELEASE_MEDIA));
                result = hr("finalizing the audio disc", rel(tao.get()));
            }
            std::fs::remove_dir_all(&dir).ok();
            result?;
        }
        if opts.eject_when_done {
            unsafe {
                type F = unsafe extern "system" fn(*mut c_void) -> HResult;
                let f: F = std::mem::transmute_copy(&*vtbl(self.recorder.get()).add(slot::recorder::EJECT_MEDIA));
                let _ = f(self.recorder.get());
            }
        }
        Ok(BurnReport { bytes_written: total, verified: false, notes, backend: "IMAPI2".into() })
    }
}

impl ImapiSession {
    /// Builds an IFileSystemImage over `root` (optionally importing
    /// the previous session for an append) and returns its result
    /// stream. The second return element is a staging directory to
    /// delete afterwards (currently none — the tree is read in
    /// place by IMAPI).
    unsafe fn build_files_stream(
        &self,
        format_ptr: *mut c_void,
        root: &Path,
        label: &str,
        opts: &BurnOptions,
    ) -> Result<(ComPtr, u64)> {
        let fsi = create(&CLSID_FILE_SYSTEM_IMAGE, &IID_FILE_SYSTEM_IMAGE)?;
        type Rec = unsafe extern "system" fn(*mut c_void, *mut c_void) -> HResult;
        let choose: Rec = std::mem::transmute_copy(&*vtbl(fsi.get()).add(slot::fsi::CHOOSE_IMAGE_DEFAULTS));
        hr("choosing image defaults", choose(fsi.get(), self.recorder.get()))?;
        if opts.append {
            // Hand the format's multisession interfaces to the FSI,
            // then import the session already on the disc.
            type GetArr = unsafe extern "system" fn(*mut c_void, *mut *mut SAFEARRAY) -> HResult;
            let gf: GetArr =
                std::mem::transmute_copy(&*vtbl(format_ptr).add(slot::format_data::MULTISESSION_INTERFACES));
            let mut psa: *mut SAFEARRAY = std::ptr::null_mut();
            hr("reading multisession interfaces", gf(format_ptr, &mut psa))?;
            if !psa.is_null() {
                type SetArr = unsafe extern "system" fn(*mut c_void, *const SAFEARRAY) -> HResult;
                let sf: SetArr =
                    std::mem::transmute_copy(&*vtbl(fsi.get()).add(slot::fsi::SET_MULTISESSION_INTERFACES));
                let code = sf(fsi.get(), psa);
                windows_sys::Win32::System::Ole::SafeArrayDestroy(psa);
                hr("setting multisession interfaces", code)?;
            }
            type Import = unsafe extern "system" fn(*mut c_void, *mut i32) -> HResult;
            let imf: Import = std::mem::transmute_copy(&*vtbl(fsi.get()).add(slot::fsi::IMPORT_FILE_SYSTEM));
            let mut imported: i32 = 0;
            hr("importing the previous session", imf(fsi.get(), &mut imported))?;
        }
        {
            type SetFs = unsafe extern "system" fn(*mut c_void, i32) -> HResult;
            let sf: SetFs = std::mem::transmute_copy(&*vtbl(fsi.get()).add(slot::fsi::SET_FILE_SYSTEMS_TO_CREATE));
            const ISO9660: i32 = 1;
            const JOLIET: i32 = 2;
            let _ = sf(fsi.get(), ISO9660 | JOLIET);
        }
        let label_b = BStr::new(if label.is_empty() { "ISOFOUNDRY" } else { label });
        type SetName = unsafe extern "system" fn(*mut c_void, *mut c_void) -> HResult;
        let nf: SetName = std::mem::transmute_copy(&*vtbl(fsi.get()).add(slot::fsi::SET_VOLUME_NAME));
        hr("setting the volume label", nf(fsi.get(), label_b.get()))?;
        // Root directory item → AddTree over the staging folder.
        type GetObj = unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> HResult;
        let rf: GetObj = std::mem::transmute_copy(&*vtbl(fsi.get()).add(slot::fsi::ROOT));
        let mut raw_root: *mut c_void = std::ptr::null_mut();
        hr("reading the image root", rf(fsi.get(), &mut raw_root))?;
        // Re-query for the directory interface, then drop the
        // getter's own reference.
        let root_item = query(raw_root, &IID_FSI_DIRECTORY_ITEM)?;
        release(raw_root);
        type AddTree = unsafe extern "system" fn(*mut c_void, *mut c_void, VARIANT_BOOL) -> HResult;
        let af: AddTree = std::mem::transmute_copy(&*vtbl(root_item.get()).add(slot::fsi_dir::ADD_TREE));
        let path_b = BStr::new(&root.to_string_lossy());
        hr("adding the folder tree", af(root_item.get(), path_b.get(), 0))?;
        // Build the result image and take its stream.
        let cf: GetObj = std::mem::transmute_copy(&*vtbl(fsi.get()).add(slot::fsi::CREATE_RESULT_IMAGE));
        let mut result: *mut c_void = std::ptr::null_mut();
        hr("building the disc image", cf(fsi.get(), &mut result))?;
        let raw_result = result;
        let result = query(raw_result, &IID_FILE_SYSTEM_IMAGE_RESULT)?;
        release(raw_result);
        let blocks = get_i32(result.get(), slot::fsi_result::TOTAL_BLOCKS).unwrap_or(0).max(0) as u64;
        let block_size = get_i32(result.get(), slot::fsi_result::BLOCK_SIZE).unwrap_or(2048).max(1) as u64;
        let sf2: GetObj = std::mem::transmute_copy(&*vtbl(result.get()).add(slot::fsi_result::IMAGE_STREAM));
        let mut stream: *mut c_void = std::ptr::null_mut();
        hr("taking the image stream", sf2(result.get(), &mut stream))?;
        // Keep the result + fsi alive via the returned stream? The
        // stream is independent once created; but the FSI objects
        // are dropped at the end of this function while Write runs
        // after — the stream holds its own reference to the image
        // data in IMAPI's implementation.
        Ok((ComPtr(stream), blocks * block_size))
    }
}

/// Opens a read stream over a file (shlwapi).
unsafe fn stream_on_file(path: &Path) -> Result<ComPtr> {
    let w = wide(&path.to_string_lossy());
    let mut stream: *mut c_void = std::ptr::null_mut();
    const STGM_READ: u32 = 0;
    let code = SHCreateStreamOnFileEx(w.as_ptr(), STGM_READ, 0, false.into(), std::ptr::null_mut(), &mut stream);
    hr(&format!("opening {}", path.display()), code)?;
    if stream.is_null() {
        return err(format!("opening {} returned no stream", path.display()));
    }
    Ok(ComPtr(stream))
}
