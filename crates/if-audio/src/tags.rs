//! Tag reading and writing through lofty: the fields a conversion
//! carries across, plus ReplayGain values.

use crate::Result;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::tag::{ItemKey, Tag};
use std::path::Path;

/// The tag fields IsoFoundry reads and carries across a conversion.
#[derive(Debug, Clone, Default)]
pub struct Tags {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub album_artist: String,
    pub track: Option<u32>,
    pub track_total: Option<u32>,
    pub disc: Option<u32>,
    /// Free-form date string as stored (usually a year or full date).
    pub date: String,
    pub genre: String,
    pub comment: String,
    pub replaygain_track_gain: Option<f32>,
    pub replaygain_track_peak: Option<f32>,
    pub replaygain_album_gain: Option<f32>,
    pub replaygain_album_peak: Option<f32>,
}

impl Tags {
    pub fn is_empty(&self) -> bool {
        self.title.is_empty()
            && self.artist.is_empty()
            && self.album.is_empty()
            && self.album_artist.is_empty()
            && self.track.is_none()
            && self.disc.is_none()
            && self.date.is_empty()
            && self.genre.is_empty()
            && self.comment.is_empty()
            && self.replaygain_track_gain.is_none()
            && self.replaygain_track_peak.is_none()
            && self.replaygain_album_gain.is_none()
            && self.replaygain_album_peak.is_none()
    }
}

fn get(tag: &Tag, key: ItemKey) -> String {
    tag.get_string(key).unwrap_or("").to_string()
}

fn get_gain(tag: &Tag, key: ItemKey) -> Option<f32> {
    // Stored as e.g. "-7.32 dB" or "-7.32"; peaks as "0.987654".
    let s = tag.get_string(key)?;
    s.trim().trim_end_matches("dB").trim().parse::<f32>().ok()
}

/// Reads the tags of a file. Untagged or unreadable files give an
/// empty set — a conversion should never fail over metadata.
pub fn read_tags(path: &Path) -> Tags {
    let Ok(file) = lofty::read_from_path(path) else {
        return Tags::default();
    };
    let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) else {
        return Tags::default();
    };
    use lofty::tag::Accessor;
    Tags {
        title: get(tag, ItemKey::TrackTitle),
        artist: get(tag, ItemKey::TrackArtist),
        album: get(tag, ItemKey::AlbumTitle),
        album_artist: get(tag, ItemKey::AlbumArtist),
        track: tag.track(),
        track_total: tag.track_total(),
        disc: tag.disk(),
        date: get(tag, ItemKey::RecordingDate),
        genre: get(tag, ItemKey::Genre),
        comment: get(tag, ItemKey::Comment),
        replaygain_track_gain: get_gain(tag, ItemKey::ReplayGainTrackGain),
        replaygain_track_peak: get_gain(tag, ItemKey::ReplayGainTrackPeak),
        replaygain_album_gain: get_gain(tag, ItemKey::ReplayGainAlbumGain),
        replaygain_album_peak: get_gain(tag, ItemKey::ReplayGainAlbumPeak),
    }
}

fn set(tag: &mut Tag, key: ItemKey, value: &str) {
    if !value.is_empty() {
        tag.insert_text(key, value.to_string());
    }
}

/// Writes tags to an existing audio file, creating its primary tag
/// when it has none. Only non-empty fields are written.
pub fn write_tags(path: &Path, tags: &Tags) -> Result<()> {
    if tags.is_empty() {
        return Ok(());
    }
    let mut file = lofty::read_from_path(path)
        .map_err(|e| crate::Error(format!("could not open {} to write tags: {e}", path.display())))?;
    if file.primary_tag().is_none() {
        let ty = file.primary_tag_type();
        file.insert_tag(Tag::new(ty));
    }
    let tag = file.primary_tag_mut().ok_or_else(|| crate::Error("the file does not accept tags".into()))?;
    use lofty::tag::Accessor;
    set(tag, ItemKey::TrackTitle, &tags.title);
    set(tag, ItemKey::TrackArtist, &tags.artist);
    set(tag, ItemKey::AlbumTitle, &tags.album);
    set(tag, ItemKey::AlbumArtist, &tags.album_artist);
    set(tag, ItemKey::RecordingDate, &tags.date);
    set(tag, ItemKey::Genre, &tags.genre);
    set(tag, ItemKey::Comment, &tags.comment);
    if let Some(t) = tags.track {
        tag.set_track(t);
    }
    if let Some(t) = tags.track_total {
        tag.set_track_total(t);
    }
    if let Some(d) = tags.disc {
        tag.set_disk(d);
    }
    if let Some(g) = tags.replaygain_track_gain {
        tag.insert_text(ItemKey::ReplayGainTrackGain, format!("{g:+.2} dB"));
    }
    if let Some(p) = tags.replaygain_track_peak {
        tag.insert_text(ItemKey::ReplayGainTrackPeak, format!("{p:.6}"));
    }
    if let Some(g) = tags.replaygain_album_gain {
        tag.insert_text(ItemKey::ReplayGainAlbumGain, format!("{g:+.2} dB"));
    }
    if let Some(p) = tags.replaygain_album_peak {
        tag.insert_text(ItemKey::ReplayGainAlbumPeak, format!("{p:.6}"));
    }
    file.save_to_path(path, lofty::config::WriteOptions::default())
        .map_err(|e| crate::Error(format!("could not save tags to {}: {e}", path.display())))?;
    Ok(())
}
