//! Encoders. WAV and FLAC are native; MP3 goes through LAME loaded
//! at runtime and AAC through Media Foundation (both Windows-only,
//! in their own modules). The samples arrive fully decoded.

use crate::decode::Decoded;
use crate::{err, ConvertOptions, Format, Result};
use if_core::job::CancelToken;
use std::io::Write;
use std::path::Path;

/// Output depth: 24-bit when the source is deeper than 16-bit.
fn depth(decoded: &Decoded) -> u32 {
    match decoded.bits_per_sample {
        Some(b) if b > 16 => 24,
        _ => 16,
    }
}

pub fn encode(decoded: &Decoded, dst: &Path, opts: &ConvertOptions, cancel: &CancelToken) -> Result<()> {
    match opts.format {
        Format::Wav => write_wav(decoded, dst),
        Format::Flac => write_flac(decoded, dst),
        Format::Mp3 => {
            #[cfg(windows)]
            {
                crate::lame::encode_mp3(decoded, dst, opts.bitrate_kbps, cancel)
            }
            #[cfg(not(windows))]
            {
                let _ = (decoded, dst, cancel);
                err("MP3 encoding loads libmp3lame.dll at runtime and is only available on Windows")
            }
        }
        Format::Aac => {
            #[cfg(windows)]
            {
                crate::mf::encode_aac(decoded, dst, opts.bitrate_kbps, cancel)
            }
            #[cfg(not(windows))]
            {
                let _ = (decoded, dst, cancel);
                err("AAC encoding uses Windows Media Foundation and is only available on Windows")
            }
        }
        Format::Opus | Format::Vorbis => err(format!("encoding to {} is not supported yet", opts.format.name())),
    }
}

/// Clamps and scales one sample to a signed integer of `bits` depth.
fn scale(s: f32, bits: u32) -> i32 {
    let max = ((1i64 << (bits - 1)) - 1) as f32;
    (s.clamp(-1.0, 1.0) * max).round() as i32
}

fn write_wav(decoded: &Decoded, dst: &Path) -> Result<()> {
    let bits = depth(decoded);
    let channels = decoded.channels as u16;
    let bytes_per_sample = bits / 8;
    let block_align = channels * bytes_per_sample as u16;
    let byte_rate = decoded.sample_rate * block_align as u32;
    let data_len = (decoded.samples.len() * bytes_per_sample as usize) as u32;
    let mut f = std::fs::File::create(dst).map_err(|e| crate::Error(format!("{}: {e}", dst.display())))?;
    let mut hdr = Vec::with_capacity(44);
    hdr.extend_from_slice(b"RIFF");
    hdr.extend_from_slice(&(36 + data_len).to_le_bytes());
    hdr.extend_from_slice(b"WAVE");
    hdr.extend_from_slice(b"fmt ");
    hdr.extend_from_slice(&16u32.to_le_bytes());
    hdr.extend_from_slice(&1u16.to_le_bytes()); // PCM
    hdr.extend_from_slice(&channels.to_le_bytes());
    hdr.extend_from_slice(&decoded.sample_rate.to_le_bytes());
    hdr.extend_from_slice(&byte_rate.to_le_bytes());
    hdr.extend_from_slice(&block_align.to_le_bytes());
    hdr.extend_from_slice(&(bits as u16).to_le_bytes());
    hdr.extend_from_slice(b"data");
    hdr.extend_from_slice(&data_len.to_le_bytes());
    f.write_all(&hdr).map_err(io_err)?;
    let mut buf = Vec::with_capacity(1 << 16);
    for &s in &decoded.samples {
        let v = scale(s, bits);
        for i in 0..bytes_per_sample {
            buf.push((v >> (8 * i)) as u8);
        }
        if buf.len() >= 1 << 16 {
            f.write_all(&buf).map_err(io_err)?;
            buf.clear();
        }
    }
    f.write_all(&buf).map_err(io_err)?;
    Ok(())
}

fn write_flac(decoded: &Decoded, dst: &Path) -> Result<()> {
    use flacenc::bitsink::MemSink;
    use flacenc::component::BitRepr;
    let bits = depth(decoded);
    let samples: Vec<i32> = decoded.samples.iter().map(|&s| scale(s, bits)).collect();
    use flacenc::error::Verify;
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|(_, e)| crate::Error(format!("FLAC encoder setup failed: {e:?}")))?;
    let source = flacenc::source::MemSource::from_samples(
        &samples,
        decoded.channels as usize,
        bits as usize,
        decoded.sample_rate as usize,
    );
    let stream = flacenc::encode_with_fixed_block_size(&config, source, 4096)
        .map_err(|e| crate::Error(format!("FLAC encode failed: {e:?}")))?;
    let mut sink = MemSink::<u8>::new();
    stream.write(&mut sink).map_err(|e| crate::Error(format!("FLAC write failed: {e:?}")))?;
    let mut bytes = sink.into_inner();
    // flacenc records the true minimum block size (the short final
    // frame), which makes the stream "variable block size" on
    // paper. The reference encoder instead records the fixed block
    // size in both STREAMINFO fields — the final short frame does
    // not count — and strict readers (Symphonia's frame check)
    // rely on that convention. Follow it: min := max.
    if bytes.len() > 12 && &bytes[..4] == b"fLaC" && (bytes[4] & 0x7F) == 0 {
        bytes[8] = bytes[10];
        bytes[9] = bytes[11];
    }
    std::fs::write(dst, bytes).map_err(|e| crate::Error(format!("{}: {e}", dst.display())))?;
    Ok(())
}

fn io_err(e: std::io::Error) -> crate::Error {
    crate::Error(format!("write failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode;
    use if_core::job::{CancelToken, Progress};
    use std::path::PathBuf;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("if-audio-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn test_tone() -> Decoded {
        let rate = 44_100;
        let mut samples = Vec::new();
        for i in 0..rate / 2 {
            let s = 0.5 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / rate as f32).sin();
            samples.push(s);
            samples.push(s);
        }
        Decoded { sample_rate: rate, channels: 2, bits_per_sample: Some(16), samples }
    }

    #[test]
    fn wav_round_trip() {
        let dir = temp_dir("wav");
        let out = dir.join("tone.wav");
        let tone = test_tone();
        write_wav(&tone, &out).unwrap();
        let back = decode::decode_all(&out, &Progress::default(), &CancelToken::new()).unwrap();
        assert_eq!(back.sample_rate, 44_100);
        assert_eq!(back.channels, 2);
        assert_eq!(back.samples.len(), tone.samples.len());
        let diff = (back.samples[100] - tone.samples[100]).abs();
        assert!(diff < 0.001, "sample drift {diff}");
    }

    #[test]
    fn flac_round_trip() {
        let dir = temp_dir("flac");
        let out = dir.join("tone.flac");
        let tone = test_tone();
        write_flac(&tone, &out).unwrap();
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(&bytes[..4], b"fLaC");
        let back = decode::decode_all(&out, &Progress::default(), &CancelToken::new()).unwrap();
        assert_eq!(back.samples.len(), tone.samples.len());
        // FLAC is lossless at 16-bit: allow one quantization step.
        let diff = (back.samples[1000] - tone.samples[1000]).abs();
        assert!(diff < 0.0001, "sample drift {diff}");
    }
}
