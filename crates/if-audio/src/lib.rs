//! if-audio — the audio conversion engine (M3).
//!
//! Decode is Symphonia (WAV, FLAC, MP3, Ogg Vorbis, AAC in M4A and
//! more). Encode: WAV and FLAC natively, MP3 through `libmp3lame.dll`
//! loaded at runtime (D-012 — LAME is never linked in), AAC/M4A
//! through Windows Media Foundation. Opus and Ogg Vorbis *encoding*
//! is not offered yet: there is no permissive pure-Rust encoder for
//! either, and the C libraries cannot be built by this project's
//! toolchain — decoding both works fine.
//!
//! Tags are carried across a conversion (lofty), and ReplayGain
//! track gain/peak can be measured (BS.1770-style K-weighted
//! loudness) and written, or the gain baked into the samples.

pub mod decode;
pub mod encode;
pub mod gain;
#[cfg(windows)]
mod lame;
#[cfg(windows)]
mod mf;
pub mod tags;

use if_core::job::{CancelToken, Progress};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct Error(pub String);

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error(msg.into()))
}

/// An output format for a conversion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Wav,
    Flac,
    Mp3,
    Aac,
    Opus,
    Vorbis,
}

impl Format {
    pub fn parse(s: &str) -> Result<Format> {
        Ok(match s.to_ascii_lowercase().as_str() {
            "wav" | "wave" => Format::Wav,
            "flac" => Format::Flac,
            "mp3" => Format::Mp3,
            "aac" | "m4a" => Format::Aac,
            "opus" => Format::Opus,
            "vorbis" | "ogg" => Format::Vorbis,
            other => return err(format!("unknown audio format '{other}' (use wav, flac, mp3, aac, opus, vorbis)")),
        })
    }

    pub fn extension(&self) -> &'static str {
        match self {
            Format::Wav => "wav",
            Format::Flac => "flac",
            Format::Mp3 => "mp3",
            Format::Aac => "m4a",
            Format::Opus => "opus",
            Format::Vorbis => "ogg",
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Format::Wav => "WAV",
            Format::Flac => "FLAC",
            Format::Mp3 => "MP3",
            Format::Aac => "AAC (M4A)",
            Format::Opus => "Opus",
            Format::Vorbis => "Ogg Vorbis",
        }
    }

    /// Whether this build can encode the format at all.
    pub fn encode_supported(&self) -> bool {
        matches!(self, Format::Wav | Format::Flac | Format::Mp3 | Format::Aac)
    }
}

/// Facts about an audio file, from the decoder's probe.
#[derive(Debug, Clone)]
pub struct AudioInfo {
    pub codec: String,
    pub sample_rate: u32,
    pub channels: u32,
    /// Total frames when the container reports it.
    pub frames: Option<u64>,
    pub bits_per_sample: Option<u32>,
}

impl AudioInfo {
    pub fn seconds(&self) -> Option<f64> {
        self.frames.map(|f| f as f64 / self.sample_rate.max(1) as f64)
    }
}

/// Options for one conversion.
#[derive(Debug, Clone)]
pub struct ConvertOptions {
    pub format: Format,
    /// MP3/AAC bit rate in kbit/s.
    pub bitrate_kbps: u32,
    /// Measure ReplayGain and store it as tags on the output.
    pub replaygain: bool,
    /// Bake the ReplayGain adjustment into the samples (peak-safe:
    /// never boosts past full scale).
    pub apply_gain: bool,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        ConvertOptions { format: Format::Flac, bitrate_kbps: 192, replaygain: false, apply_gain: false }
    }
}

/// What a conversion produced.
#[derive(Debug, Clone)]
pub struct ConvertReport {
    pub frames: u64,
    pub sample_rate: u32,
    pub channels: u32,
    /// Measured ReplayGain in dB, when requested.
    pub gain_db: Option<f32>,
    /// Sample peak (0..=1+), when measured.
    pub peak: Option<f32>,
}

/// Probes a file without decoding it.
pub fn probe_info(path: &Path) -> Result<AudioInfo> {
    decode::probe_info(path)
}

/// Whether MP3 encoding can work right now: Windows with
/// `libmp3lame.dll` findable (it is loaded at runtime, D-012).
pub fn mp3_available() -> bool {
    #[cfg(windows)]
    {
        lame::lame_available()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Converts one file. Tags are carried across; ReplayGain handling
/// follows `opts`. Progress is reported in input frames when the
/// total is known, otherwise in decoded frames against an estimate.
pub fn convert_file(
    src: &Path,
    dst: &Path,
    opts: &ConvertOptions,
    progress: &Progress,
    cancel: &CancelToken,
) -> Result<ConvertReport> {
    if !opts.format.encode_supported() {
        return err(format!(
            "encoding to {} is not supported yet — no permissive pure-Rust encoder exists for it and the C library cannot be built by this project's toolchain. Decoding {} files works fine.",
            opts.format.name(),
            opts.format.name()
        ));
    }
    let src_tags = tags::read_tags(src);
    let mut decoded = decode::decode_all(src, progress, cancel)?;
    let channels = decoded.channels;
    let rate = decoded.sample_rate;

    let mut gain_db = None;
    let mut peak = None;
    if opts.replaygain || opts.apply_gain {
        let m = gain::measure(&decoded.samples, channels as usize, rate);
        gain_db = Some(m.gain_db);
        peak = Some(m.peak);
        if opts.apply_gain {
            let linear = gain::gain_linear_peak_safe(m.gain_db, m.peak);
            for s in &mut decoded.samples {
                *s *= linear;
            }
        }
    }

    // Encode to a .part sibling, then rename: a cancelled or failed
    // conversion never leaves a half-written file at the destination.
    let part = part_path(dst);
    let enc_result = encode::encode(&decoded, &part, opts, cancel);
    if let Err(e) = enc_result {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    if let Err(e) = std::fs::rename(&part, dst) {
        let _ = std::fs::remove_file(&part);
        return err(format!("could not move the finished file into place: {e}"));
    }

    // Tags last (they rewrite the file header in place).
    let mut out_tags = src_tags;
    if let (Some(g), Some(p)) = (gain_db, peak) {
        if opts.replaygain {
            out_tags.replaygain_track_gain = Some(g);
            out_tags.replaygain_track_peak = Some(p);
        }
    }
    let _ = tags::write_tags(dst, &out_tags);

    Ok(ConvertReport {
        frames: (decoded.samples.len() / channels.max(1) as usize) as u64,
        sample_rate: rate,
        channels,
        gain_db,
        peak,
    })
}

fn part_path(dst: &Path) -> PathBuf {
    // Keeps the real extension last ("song.part.m4a"): Media
    // Foundation picks its container from the extension, and a
    // trailing ".part" would break that.
    let stem = dst.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "audio".into());
    let ext = dst.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    let name = if ext.is_empty() { format!("{stem}.part") } else { format!("{stem}.part.{ext}") };
    dst.with_file_name(name)
}
