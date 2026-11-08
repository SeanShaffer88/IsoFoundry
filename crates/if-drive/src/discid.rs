//! Disc identification (M6): the MusicBrainz and freedb/CDDB disc
//! IDs for an audio CD's table of contents.
//!
//! Both algorithms are implemented from their published specs and
//! pinned by libdiscid's own published test vector (see the tests):
//! - MusicBrainz: SHA-1 over a hex string of first/last track plus
//!   100 offsets (lead-out first, then tracks), each offset an
//!   absolute frame position including the 150-frame lead-in
//!   (offset = LBA + 150); the digest is base64 with '+' → '.',
//!   '/' → '_', '=' → '-'.
//! - freedb: digit-sum checksum of the track starts in whole
//!   seconds, total playing time, track count (freedb DISCID howto,
//!   Appendix A).

use crate::Toc;

/// The audio tracks of the TOC, in order. Disc IDs are only defined
/// for audio CDs; a mixed disc is identified by its audio session.
fn audio_tracks(toc: &Toc) -> Vec<&crate::TrackInfo> {
    toc.tracks.iter().filter(|t| t.audio).collect()
}

/// MusicBrainz disc ID (28 characters), or `None` when the disc has
/// no audio tracks.
pub fn musicbrainz_id(toc: &Toc) -> Option<String> {
    let tracks = audio_tracks(toc);
    if tracks.is_empty() {
        return None;
    }
    let first = tracks.first().unwrap().number;
    let last = tracks.last().unwrap().number;
    let mut hex = format!("{first:02X}{last:02X}");
    hex.push_str(&format!("{:08X}", toc.lead_out_lba + 150));
    for n in 1..=99u32 {
        let off = tracks.iter().find(|t| t.number == n).map(|t| t.start_lba + 150).unwrap_or(0);
        hex.push_str(&format!("{off:08X}"));
    }
    let digest = sha1(hex.as_bytes());
    Some(mb_base64(&digest))
}

/// freedb/CDDB disc ID (8 lowercase hex digits), or `None` when the
/// disc has no audio tracks.
pub fn freedb_id(toc: &Toc) -> Option<String> {
    let tracks = audio_tracks(toc);
    if tracks.is_empty() {
        return None;
    }
    fn digit_sum(mut n: u32) -> u32 {
        let mut s = 0;
        while n > 0 {
            s += n % 10;
            n /= 10;
        }
        s
    }
    // (minutes*60)+seconds of an MSF position == frames/75 whole seconds.
    let n: u32 = tracks.iter().map(|t| digit_sum((t.start_lba + 150) / 75)).sum();
    let t = (toc.lead_out_lba + 150) / 75 - (tracks[0].start_lba + 150) / 75;
    let id = ((n % 0xff) << 24) | (t << 8) | tracks.len() as u32;
    Some(format!("{id:08x}"))
}

/// The `toc` parameter MusicBrainz expects in lookup/attach URLs:
/// first, last, lead-out and track offsets (all including the
/// 150-frame lead-in), space-separated.
pub fn toc_param(toc: &Toc) -> Option<String> {
    let tracks = audio_tracks(toc);
    if tracks.is_empty() {
        return None;
    }
    let mut parts = vec![
        tracks.first().unwrap().number.to_string(),
        tracks.last().unwrap().number.to_string(),
        (toc.lead_out_lba + 150).to_string(),
    ];
    parts.extend(tracks.iter().map(|t| (t.start_lba + 150).to_string()));
    Some(parts.join(" "))
}

/// The web page where this disc can be looked up on / attached to a
/// MusicBrainz release.
pub fn lookup_url(toc: &Toc) -> Option<String> {
    let id = musicbrainz_id(toc)?;
    let toc = toc_param(toc)?.replace(' ', "+");
    Some(format!("https://musicbrainz.org/cdtoc/attach?toc={toc}&discid={id}"))
}

fn sha1(data: &[u8]) -> [u8; 20] {
    use if_hash::{Algorithms, Hasher};
    let mut h = Hasher::new(Algorithms::parse("sha1").expect("sha1 is a known algorithm"));
    h.update(data);
    let hex = h.finish().sha1.expect("sha1 was requested");
    let mut out = [0u8; 20];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).expect("hasher returns hex");
    }
    out
}

/// Standard base64 with MusicBrainz's character substitutions.
fn mb_base64(data: &[u8; 20]) -> String {
    const ALPHA: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(28);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        s.push(ALPHA[(n >> 18) as usize & 63] as char);
        s.push(ALPHA[(n >> 12) as usize & 63] as char);
        s.push(if chunk.len() > 1 { ALPHA[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if chunk.len() > 2 { ALPHA[n as usize & 63] as char } else { '=' });
    }
    s.replace('+', ".").replace('/', "_").replace('=', "-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{fake_source_audio, OpticalDrive, TrackInfo};

    fn toc_from_mb_offsets(offsets: &[u32], lead_out: u32) -> Toc {
        // `offsets` are MusicBrainz-style (LBA + 150); store as LBA.
        Toc {
            tracks: offsets
                .iter()
                .enumerate()
                .map(|(i, o)| TrackInfo {
                    number: i as u32 + 1,
                    start_lba: o - 150,
                    sectors: 0,
                    audio: true,
                    title: None,
                    performer: None,
                })
                .collect(),
            lead_out_lba: lead_out - 150,
            sector_size: 2352,
        }
    }

    #[test]
    fn libdiscid_published_vector() {
        // libdiscid's own test_discid.c: first 1, last 22, these
        // offsets -> MB "xUp1F2NkfP8s8jaeFn_Av3jNEI4-"; the freedb
        // value was computed independently from the freedb spec.
        let offsets = [
            150, 9700, 25887, 39297, 53795, 63735, 77517, 94877, 107270, 123552, 135522, 148422, 161197, 174790,
            192022, 205545, 218010, 228700, 239590, 255470, 266932, 288750,
        ];
        let toc = toc_from_mb_offsets(&offsets, 303602);
        assert_eq!(musicbrainz_id(&toc).unwrap(), "xUp1F2NkfP8s8jaeFn_Av3jNEI4-");
        assert_eq!(freedb_id(&toc).unwrap(), "370fce16");
        assert!(toc_param(&toc).unwrap().starts_with("1 22 303602 150 9700"));
        assert!(lookup_url(&toc).unwrap().contains("discid=xUp1F2NkfP8s8jaeFn_Av3jNEI4-"));
    }

    #[test]
    fn fake_disc_ids() {
        let d = fake_source_audio();
        let toc = d.toc().unwrap();
        assert_eq!(musicbrainz_id(&toc).unwrap(), "QVQ3cY_KZqC3cV3YPPZPXOS7ciU-");
        assert_eq!(freedb_id(&toc).unwrap(), "08000d03");
    }

    #[test]
    fn data_disc_has_no_ids() {
        let toc = Toc {
            tracks: vec![TrackInfo {
                number: 1,
                start_lba: 0,
                sectors: 100,
                audio: false,
                title: None,
                performer: None,
            }],
            lead_out_lba: 100,
            sector_size: 2048,
        };
        assert!(musicbrainz_id(&toc).is_none());
        assert!(freedb_id(&toc).is_none());
    }
}
