//! Symphonia decoding: any supported file to interleaved `f32`
//! samples in memory. Sources with more than two channels are
//! reduced to the first two (front left/right); mono stays mono.

use crate::{err, AudioInfo, Result};
use if_core::job::{CancelToken, Progress};
use std::fs::File;
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// A fully decoded track.
pub struct Decoded {
    pub sample_rate: u32,
    pub channels: u32,
    /// Bits per sample of the source, when known (drives the WAV /
    /// FLAC output depth: 24 when the source is deeper than 16).
    pub bits_per_sample: Option<u32>,
    /// Interleaved samples, `channels` per frame.
    pub samples: Vec<f32>,
}

fn open_format(path: &Path) -> Result<Box<dyn symphonia::core::formats::FormatReader>> {
    let file = File::open(path).map_err(|e| crate::Error(format!("{}: {e}", path.display())))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| crate::Error(format!("{}: not a readable audio file ({e})", path.display())))?;
    Ok(probed.format)
}

fn best_track(format: &dyn symphonia::core::formats::FormatReader) -> Result<symphonia::core::formats::Track> {
    format
        .tracks()
        .iter()
        .find(|t| {
            t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL && t.codec_params.sample_rate.is_some()
        })
        .or_else(|| format.default_track())
        .cloned()
        .ok_or_else(|| crate::Error("the file has no audio track".into()))
}

pub fn probe_info(path: &Path) -> Result<AudioInfo> {
    let format = open_format(path)?;
    let track = best_track(format.as_ref())?;
    let cp = &track.codec_params;
    Ok(AudioInfo {
        codec: format!("{:?}", cp.codec),
        sample_rate: cp.sample_rate.unwrap_or(0),
        channels: cp.channels.map(|c| c.count() as u32).unwrap_or(0),
        frames: cp.n_frames,
        bits_per_sample: cp.bits_per_sample,
    })
}

/// Decodes the whole file. Progress counts decoded frames against
/// the container's frame count when it is known.
pub fn decode_all(path: &Path, progress: &Progress, cancel: &CancelToken) -> Result<Decoded> {
    let mut format = open_format(path)?;
    let track = best_track(format.as_ref())?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| crate::Error(format!("no decoder for this file ({e})")))?;

    let total = track.codec_params.n_frames.unwrap_or(0);
    if total > 0 {
        progress.set_total(total);
    }
    let mut sample_rate = track.codec_params.sample_rate.unwrap_or(44_100);
    let mut channels = track.codec_params.channels.map(|c| c.count() as u32).unwrap_or(2);
    let bits_per_sample = track.codec_params.bits_per_sample;
    let mut out: Vec<f32> = Vec::new();
    let mut sample_buf: Option<SampleBuffer<f32>> = None;
    let mut decoded_frames: u64 = 0;

    loop {
        if cancel.is_cancelled() {
            return err("cancelled");
        }
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(ref e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymError::ResetRequired) => break,
            Err(e) => return err(format!("read failed: {e}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymError::DecodeError(_)) => continue, // skip a bad packet, keep going
            Err(e) => return err(format!("decode failed: {e}")),
        };
        let spec = *decoded.spec();
        sample_rate = spec.rate;
        let src_channels = spec.channels.count();
        channels = src_channels.min(2) as u32;
        let frames = decoded.frames();
        if sample_buf.is_none() {
            sample_buf = Some(SampleBuffer::<f32>::new(decoded.capacity() as u64, spec));
        }
        let buf = sample_buf.as_mut().expect("set above");
        buf.copy_interleaved_ref(decoded);
        let samples = buf.samples();
        if src_channels <= 2 {
            out.extend_from_slice(&samples[..frames * src_channels]);
        } else {
            // Keep front left/right, drop the rest.
            for f in 0..frames {
                out.push(samples[f * src_channels]);
                out.push(samples[f * src_channels + 1]);
            }
        }
        decoded_frames += frames as u64;
        progress.add(frames as u64);
        let _ = decoded_frames;
    }

    if out.is_empty() {
        return err("the file decoded to no samples");
    }
    Ok(Decoded { sample_rate, channels: channels.max(1), bits_per_sample, samples: out })
}
