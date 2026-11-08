//! AAC (M4A) encoding through the Windows Media Foundation sink
//! writer. windows-sys 0.59 ships no Media Foundation bindings, so
//! the small surface used here is declared directly; the GUIDs and
//! vtbl orders match the SDK headers (mfplat.h / mfreadwrite.h) —
//! IUnknown is slots 0–2 everywhere, IMFAttributes methods start at
//! slot 3 (SetUINT32 = 21, SetGUID = 24), IMFSample continues after
//! the 29 attribute methods, and IMFSinkWriter follows IUnknown.
//!
//! Runtime behaviour is **Unverified** until exercised on a real
//! Windows machine (see docs/LIMITATIONS.md).

use crate::decode::Decoded;
use crate::{err, Result};
use if_core::job::CancelToken;
use std::ffi::c_void;
use std::path::Path;

#[repr(C)]
#[derive(Clone, Copy)]
struct Guid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

const fn guid(v: u128) -> Guid {
    Guid { data1: (v >> 96) as u32, data2: (v >> 80) as u16, data3: (v >> 64) as u16, data4: (v as u64).to_be_bytes() }
}

const MF_MT_MAJOR_TYPE: Guid = guid(0x48eba18e_f8c9_4687_bf11_0a74c9f96a8f);
const MF_MT_SUBTYPE: Guid = guid(0xf7e34c9a_42e8_4714_b74b_cb29d72c35e5);
const MF_MT_AUDIO_NUM_CHANNELS: Guid = guid(0x37e48bf5_645e_4c5b_89de_ada9e29b696a);
const MF_MT_AUDIO_SAMPLES_PER_SECOND: Guid = guid(0x5faeeae7_0290_4c31_9e8a_c534f68d9dba);
const MF_MT_AUDIO_BITS_PER_SAMPLE: Guid = guid(0xf2deb57f_40fa_4764_aa33_ed4f2d1ff669);
const MF_MT_AUDIO_AVG_BYTES_PER_SECOND: Guid = guid(0x1aab75c8_cfef_451c_ab95_ac034b8e1731);
const MF_MT_AUDIO_BLOCK_ALIGNMENT: Guid = guid(0x322de230_9eeb_43bd_ab7a_ff412251541d);
const MF_MT_AAC_PAYLOAD_TYPE: Guid = guid(0xbfbabe79_7434_4d1c_94f0_72a3b9e17188);
const MF_MEDIA_TYPE_AUDIO: Guid = guid(0x73647561_0000_0010_8000_00aa00389b71);
const MF_AUDIO_FORMAT_AAC: Guid = guid(0x00001610_0000_0010_8000_00aa00389b71);
const MF_AUDIO_FORMAT_PCM: Guid = guid(0x00000001_0000_0010_8000_00aa00389b71);

const MF_VERSION: u32 = 0x0002_0070;

extern "system" {
    fn MFStartup(version: u32, flags: u32) -> i32;
    fn MFShutdown() -> i32;
    fn MFCreateMediaType(pp: *mut *mut c_void) -> i32;
    fn MFCreateSinkWriterFromURL(
        url: *const u16,
        byte_stream: *mut c_void,
        attributes: *mut c_void,
        pp: *mut *mut c_void,
    ) -> i32;
    fn MFCreateSample(pp: *mut *mut c_void) -> i32;
    fn MFCreateMemoryBuffer(max_length: u32, pp: *mut *mut c_void) -> i32;
}

type FnRelease = unsafe extern "system" fn(*mut c_void) -> u32;
type FnSetGuid = unsafe extern "system" fn(*mut c_void, *const Guid, *const Guid) -> i32;
type FnSetU32 = unsafe extern "system" fn(*mut c_void, *const Guid, u32) -> i32;
type FnAddStream = unsafe extern "system" fn(*mut c_void, *mut c_void, *mut u32) -> i32;
type FnSetInput = unsafe extern "system" fn(*mut c_void, u32, *mut c_void, *mut c_void) -> i32;
type FnNoArgs = unsafe extern "system" fn(*mut c_void) -> i32;
type FnWriteSample = unsafe extern "system" fn(*mut c_void, u32, *mut c_void) -> i32;
type FnLock = unsafe extern "system" fn(*mut c_void, *mut *mut u8, *mut u32, *mut u32) -> i32;
type FnSetLen = unsafe extern "system" fn(*mut c_void, u32) -> i32;
type FnAddBuffer = unsafe extern "system" fn(*mut c_void, *mut c_void) -> i32;
type FnSetTime = unsafe extern "system" fn(*mut c_void, i64) -> i32;

// vtbl slot indices (see the module note).
const SLOT_RELEASE: usize = 2;
const SLOT_SET_U32: usize = 21;
const SLOT_SET_GUID: usize = 24;
const SW_ADD_STREAM: usize = 3;
const SW_SET_INPUT: usize = 4;
const SW_BEGIN: usize = 5;
const SW_WRITE: usize = 6;
const SW_FINALIZE: usize = 11;
const BUF_LOCK: usize = 3;
const BUF_UNLOCK: usize = 4;
const BUF_SET_LEN: usize = 6;
const SMP_SET_TIME: usize = 35;
const SMP_SET_DURATION: usize = 37;
const SMP_ADD_BUFFER: usize = 41;

/// # Safety
/// `p` must be a live COM interface pointer of the expected type.
unsafe fn slot<T>(p: *mut c_void, index: usize) -> T {
    let vtbl = *(p as *const *const usize);
    std::mem::transmute_copy(&*vtbl.add(index))
}

unsafe fn release(p: *mut c_void) {
    if !p.is_null() {
        let f: FnRelease = slot(p, SLOT_RELEASE);
        f(p);
    }
}

fn hr(code: i32, what: &str) -> Result<()> {
    if code < 0 {
        err(format!("Media Foundation failed at {what} (error 0x{:08X})", code as u32))
    } else {
        Ok(())
    }
}

unsafe fn set_guid(media_type: *mut c_void, key: &Guid, value: &Guid) -> Result<()> {
    let f: FnSetGuid = slot(media_type, SLOT_SET_GUID);
    hr(f(media_type, key, value), "setting a media type GUID")
}

unsafe fn set_u32(media_type: *mut c_void, key: &Guid, value: u32) -> Result<()> {
    let f: FnSetU32 = slot(media_type, SLOT_SET_U32);
    hr(f(media_type, key, value), "setting a media type value")
}

unsafe fn make_media_type() -> Result<*mut c_void> {
    let mut p: *mut c_void = std::ptr::null_mut();
    hr(MFCreateMediaType(&mut p), "creating a media type")?;
    Ok(p)
}

pub fn encode_aac(decoded: &Decoded, dst: &Path, bitrate_kbps: u32, cancel: &CancelToken) -> Result<()> {
    let url: Vec<u16> = dst.as_os_str().to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: the whole sequence runs on this thread; every
    // interface pointer is released exactly once, and HRESULTs are
    // checked at each step.
    unsafe {
        hr(MFStartup(MF_VERSION, 0), "startup")?;
        let result = run(decoded, &url, bitrate_kbps, cancel);
        MFShutdown();
        result
    }
}

unsafe fn run(decoded: &Decoded, url: &[u16], bitrate_kbps: u32, cancel: &CancelToken) -> Result<()> {
    let rate = decoded.sample_rate;
    let channels = decoded.channels.max(1);
    let mut writer: *mut c_void = std::ptr::null_mut();
    hr(
        MFCreateSinkWriterFromURL(url.as_ptr(), std::ptr::null_mut(), std::ptr::null_mut(), &mut writer),
        "creating the sink writer",
    )?;
    let guard = Guard(writer);

    // Output: AAC in an MP4 (.m4a) container.
    let out_type = make_media_type()?;
    let out_guard = Guard(out_type);
    set_guid(out_type, &MF_MT_MAJOR_TYPE, &MF_MEDIA_TYPE_AUDIO)?;
    set_guid(out_type, &MF_MT_SUBTYPE, &MF_AUDIO_FORMAT_AAC)?;
    set_u32(out_type, &MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
    set_u32(out_type, &MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
    set_u32(out_type, &MF_MT_AUDIO_NUM_CHANNELS, channels)?;
    set_u32(out_type, &MF_MT_AUDIO_AVG_BYTES_PER_SECOND, bitrate_kbps * 1000 / 8)?;
    set_u32(out_type, &MF_MT_AAC_PAYLOAD_TYPE, 0)?;
    let mut stream: u32 = 0;
    let add_stream: FnAddStream = slot(writer, SW_ADD_STREAM);
    hr(add_stream(writer, out_type, &mut stream), "adding the AAC stream")?;

    // Input: 16-bit PCM at the source rate.
    let in_type = make_media_type()?;
    let in_guard = Guard(in_type);
    set_guid(in_type, &MF_MT_MAJOR_TYPE, &MF_MEDIA_TYPE_AUDIO)?;
    set_guid(in_type, &MF_MT_SUBTYPE, &MF_AUDIO_FORMAT_PCM)?;
    set_u32(in_type, &MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
    set_u32(in_type, &MF_MT_AUDIO_SAMPLES_PER_SECOND, rate)?;
    set_u32(in_type, &MF_MT_AUDIO_NUM_CHANNELS, channels)?;
    set_u32(in_type, &MF_MT_AUDIO_BLOCK_ALIGNMENT, channels * 2)?;
    set_u32(in_type, &MF_MT_AUDIO_AVG_BYTES_PER_SECOND, rate * channels * 2)?;
    let set_input: FnSetInput = slot(writer, SW_SET_INPUT);
    hr(set_input(writer, stream, in_type, std::ptr::null_mut()), "setting the input type")?;
    drop(out_guard);
    drop(in_guard);

    let begin: FnNoArgs = slot(writer, SW_BEGIN);
    hr(begin(writer), "begin writing")?;

    let write_sample: FnWriteSample = slot(writer, SW_WRITE);
    let frames_total = decoded.samples.len() / channels as usize;
    let chunk = 4096usize;
    let mut frame = 0usize;
    while frame < frames_total {
        if cancel.is_cancelled() {
            return err("cancelled");
        }
        let n = chunk.min(frames_total - frame);
        let byte_len = (n * channels as usize * 2) as u32;
        let mut buffer: *mut c_void = std::ptr::null_mut();
        hr(MFCreateMemoryBuffer(byte_len, &mut buffer), "creating a sample buffer")?;
        let buf_guard = Guard(buffer);
        let lock: FnLock = slot(buffer, BUF_LOCK);
        let mut data: *mut u8 = std::ptr::null_mut();
        hr(lock(buffer, &mut data, std::ptr::null_mut(), std::ptr::null_mut()), "locking a sample buffer")?;
        let pcm = std::slice::from_raw_parts_mut(data, byte_len as usize);
        for (i, &s) in decoded.samples[frame * channels as usize..(frame + n) * channels as usize].iter().enumerate() {
            let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
            pcm[i * 2..i * 2 + 2].copy_from_slice(&v.to_le_bytes());
        }
        let set_len: FnSetLen = slot(buffer, BUF_SET_LEN);
        hr(set_len(buffer, byte_len), "sizing a sample buffer")?;
        let unlock: FnNoArgs = slot(buffer, BUF_UNLOCK);
        hr(unlock(buffer), "unlocking a sample buffer")?;

        let mut sample: *mut c_void = std::ptr::null_mut();
        hr(MFCreateSample(&mut sample), "creating a sample")?;
        let smp_guard = Guard(sample);
        let add_buffer: FnAddBuffer = slot(sample, SMP_ADD_BUFFER);
        hr(add_buffer(sample, buffer), "attaching a buffer")?;
        let set_time: FnSetTime = slot(sample, SMP_SET_TIME);
        hr(set_time(sample, (frame as i64 * 10_000_000) / rate as i64), "stamping a sample")?;
        let set_dur: FnSetTime = slot(sample, SMP_SET_DURATION);
        hr(set_dur(sample, (n as i64 * 10_000_000) / rate as i64), "stamping a sample duration")?;
        hr(write_sample(writer, stream, sample), "writing a sample")?;
        drop(smp_guard);
        drop(buf_guard);
        frame += n;
    }

    let finalize: FnNoArgs = slot(writer, SW_FINALIZE);
    hr(finalize(writer), "finalizing the file")?;
    drop(guard);
    Ok(())
}

/// Releases a COM pointer on drop.
struct Guard(*mut c_void);

impl Drop for Guard {
    fn drop(&mut self) {
        // SAFETY: the pointer was created by Media Foundation on
        // this thread and is released at most once (guard consumed
        // or dropped, never both used after).
        unsafe { release(self.0) };
    }
}
