//! MP3 encoding through LAME, loaded at runtime (D-012): the DLL is
//! never linked in and never bundled — the user drops
//! `libmp3lame.dll` next to the program or on the PATH. Every entry
//! point is resolved with `GetProcAddress` and checked.

use crate::decode::Decoded;
use crate::{err, Result};
use if_core::job::CancelToken;
use std::ffi::{c_int, c_void};
use std::io::Write;
use std::path::Path;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

type LameInit = unsafe extern "C" fn() -> *mut c_void;
type LameSetInt = unsafe extern "C" fn(*mut c_void, c_int) -> c_int;
type LameInitParams = unsafe extern "C" fn(*mut c_void) -> c_int;
type LameEncode = unsafe extern "C" fn(*mut c_void, *mut i16, c_int, *mut u8, c_int) -> c_int;
type LameFlush = unsafe extern "C" fn(*mut c_void, *mut u8, c_int) -> c_int;
type LameClose = unsafe extern "C" fn(*mut c_void) -> c_int;

struct Lame {
    init: LameInit,
    set_samplerate: LameSetInt,
    set_channels: LameSetInt,
    set_brate: LameSetInt,
    set_quality: LameSetInt,
    init_params: LameInitParams,
    encode: LameEncode,
    flush: LameFlush,
    close: LameClose,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn load() -> Result<Lame> {
    // SAFETY: standard dynamic loading; every pointer is checked
    // before it is ever called.
    unsafe {
        let dll = LoadLibraryW(wide("libmp3lame.dll").as_ptr());
        if dll.is_null() {
            return err(
                "MP3 encoding needs libmp3lame.dll, which was not found. It is loaded at runtime and never bundled: \
                 put a copy next to isofoundry.exe or on the PATH, then try again.",
            );
        }
        macro_rules! symt {
            ($name:literal, $ty:ty) => {{
                let p = GetProcAddress(dll, $name.as_ptr());
                match p {
                    Some(f) => std::mem::transmute::<unsafe extern "system" fn() -> isize, $ty>(f),
                    None => return err(concat!("libmp3lame.dll is missing the export ", $name)),
                }
            }};
        }
        Ok(Lame {
            init: symt!("lame_init\0", LameInit),
            set_samplerate: symt!("lame_set_in_samplerate\0", LameSetInt),
            set_channels: symt!("lame_set_num_channels\0", LameSetInt),
            set_brate: symt!("lame_set_brate\0", LameSetInt),
            set_quality: symt!("lame_set_quality\0", LameSetInt),
            init_params: symt!("lame_init_params\0", LameInitParams),
            encode: symt!("lame_encode_buffer_interleaved\0", LameEncode),
            flush: symt!("lame_encode_flush\0", LameFlush),
            close: symt!("lame_close\0", LameClose),
        })
    }
}

/// Whether the DLL can be loaded right now (for UI hints).
pub fn lame_available() -> bool {
    // SAFETY: load + immediate free; no entry points are called.
    unsafe {
        let dll = LoadLibraryW(wide("libmp3lame.dll").as_ptr());
        if dll.is_null() {
            false
        } else {
            windows_sys::Win32::Foundation::FreeLibrary(dll);
            true
        }
    }
}

const LAME_RATES: [u32; 9] = [8000, 11025, 12000, 16000, 22050, 24000, 32000, 44100, 48000];

pub fn encode_mp3(decoded: &Decoded, dst: &Path, bitrate_kbps: u32, cancel: &CancelToken) -> Result<()> {
    if !LAME_RATES.contains(&decoded.sample_rate) {
        return err(format!(
            "MP3 cannot use a {} Hz sample rate (LAME accepts 8000–48000 Hz in fixed steps); convert via WAV/FLAC instead",
            decoded.sample_rate
        ));
    }
    let lame = load()?;
    let channels = decoded.channels.max(1);
    let mut out = std::fs::File::create(dst).map_err(|e| crate::Error(format!("{}: {e}", dst.display())))?;
    // SAFETY: the encoder state is used on this thread only, every
    // return code is checked, and the buffers outlive each call.
    unsafe {
        let st = (lame.init)();
        if st.is_null() {
            return err("LAME failed to initialize");
        }
        let check = |rc: c_int, what: &str| -> Result<()> {
            if rc < 0 {
                (lame.close)(st);
                err(format!("LAME rejected the {what} setting"))
            } else {
                Ok(())
            }
        };
        check((lame.set_samplerate)(st, decoded.sample_rate as c_int), "sample rate")?;
        check((lame.set_channels)(st, channels as c_int), "channel count")?;
        check((lame.set_brate)(st, bitrate_kbps as c_int), "bit rate")?;
        check((lame.set_quality)(st, 2), "quality")?;
        check((lame.init_params)(st), "parameters")?;

        let frames_total = decoded.samples.len() / channels as usize;
        let chunk = 8192usize;
        let mut mp3buf = vec![0u8; (chunk as f64 * 1.25) as usize + 7200];
        let mut pcm: Vec<i16> = Vec::with_capacity(chunk * channels as usize);
        let mut frame = 0usize;
        while frame < frames_total {
            if cancel.is_cancelled() {
                (lame.close)(st);
                return err("cancelled");
            }
            let n = chunk.min(frames_total - frame);
            pcm.clear();
            for &s in &decoded.samples[frame * channels as usize..(frame + n) * channels as usize] {
                pcm.push((s.clamp(-1.0, 1.0) * 32767.0).round() as i16);
            }
            let written = (lame.encode)(st, pcm.as_mut_ptr(), n as c_int, mp3buf.as_mut_ptr(), mp3buf.len() as c_int);
            if written < 0 {
                (lame.close)(st);
                return err(format!("LAME encode failed (code {written})"));
            }
            out.write_all(&mp3buf[..written as usize]).map_err(|e| crate::Error(format!("write failed: {e}")))?;
            frame += n;
        }
        let written = (lame.flush)(st, mp3buf.as_mut_ptr(), mp3buf.len() as c_int);
        if written < 0 {
            out.write_all(&mp3buf[..written as usize]).map_err(|e| crate::Error(format!("write failed: {e}")))?;
        }
        (lame.close)(st);
    }
    Ok(())
}
