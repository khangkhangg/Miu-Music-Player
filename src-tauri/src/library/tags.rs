//! Song tags and cover art (F3.3), via `lofty`: ID3v2, MP4, Vorbis comments, APE, RIFF INFO…
//! Text is Unicode end to end. Legacy 8-bit encodings in old ID3 tags are handled in F8.1.

use std::path::Path;

use lofty::file::{AudioFile, TaggedFileExt};
use lofty::picture::PictureType;
use lofty::tag::{Accessor, ItemKey, Tag};
use serde::Serialize;

#[derive(Debug, Default, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackTags {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub year: Option<u32>,
    pub track: Option<u32>,
    pub track_total: Option<u32>,
    pub disc: Option<u32>,
    pub duration_ms: Option<u64>,
    pub has_artwork: bool,
}

impl TrackTags {
    /// Title tag, or the file name without extension when the tag is missing or blank.
    pub fn display_title(&self, path: &Path) -> String {
        match self.title.as_deref().map(str::trim) {
            Some(t) if !t.is_empty() => t.to_string(),
            _ => path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Artwork {
    pub mime: String,
    pub data: Vec<u8>,
}

fn text(s: Option<std::borrow::Cow<'_, str>>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// First four digits of a date or year field ("2021-01-06" → 2021).
fn year_of(tag: &Tag) -> Option<u32> {
    [ItemKey::Year, ItemKey::RecordingDate, ItemKey::OriginalReleaseDate]
        .iter()
        .filter_map(|k| tag.get_string(k))
        .find_map(|s| s.trim().get(..4).and_then(|y| y.parse().ok()))
}

pub fn read_tags(path: &Path) -> Result<TrackTags, String> {
    let file = lofty::read_from_path(path).map_err(|e| e.to_string())?;
    let duration = file.properties().duration().as_millis() as u64;
    let mut t = TrackTags { duration_ms: (duration > 0).then_some(duration), ..Default::default() };
    if let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) {
        t.title = text(tag.title());
        t.artist = text(tag.artist());
        t.album = text(tag.album());
        t.genre = text(tag.genre());
        t.album_artist = text(tag.get_string(&ItemKey::AlbumArtist).map(Into::into));
        t.year = year_of(tag);
        t.track = tag.track();
        t.track_total = tag.track_total();
        t.disc = tag.disk();
        t.has_artwork = !tag.pictures().is_empty();
    }
    Ok(t)
}

/// The front cover, or else the first picture in the tag.
pub fn read_artwork(path: &Path) -> Result<Option<Artwork>, String> {
    let file = lofty::read_from_path(path).map_err(|e| e.to_string())?;
    let Some(tag) = file.primary_tag().or_else(|| file.first_tag()) else { return Ok(None) };
    let pictures = tag.pictures();
    let pic = pictures.iter().find(|p| p.pic_type() == PictureType::CoverFront).or(pictures.first());
    Ok(pic.map(|p| Artwork {
        mime: p.mime_type().map(|m| m.as_str().to_string()).unwrap_or_else(|| "image/jpeg".into()),
        data: p.data().to_vec(),
    }))
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! Tiny generated audio files for tests (no real music in the repo).
    use std::path::{Path, PathBuf};

    use lofty::config::WriteOptions;
    use lofty::picture::{MimeType, Picture, PictureType};
    use lofty::tag::{Accessor, ItemKey, Tag, TagExt, TagType};

    /// 40 frames of MPEG-1 Layer III, 128 kbit/s, 44.1 kHz (~1 s of silence).
    pub fn silent_mp3(dir: &Path, name: &str) -> PathBuf {
        let mut frame = vec![0u8; 417];
        frame[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0x64]);
        let p = dir.join(name);
        std::fs::write(&p, frame.repeat(40)).unwrap();
        p
    }

    /// `fLaC` + a STREAMINFO block: 44.1 kHz, stereo, 16-bit, 88200 samples (2 s).
    pub fn silent_flac(dir: &Path, name: &str) -> PathBuf {
        let mut v = b"fLaC".to_vec();
        v.extend([0x00, 0x00, 0x00, 0x22]); // STREAMINFO, 34 bytes (a PADDING block follows, as in real files)
        v.extend(4096u16.to_be_bytes());
        v.extend(4096u16.to_be_bytes());
        v.extend([0u8; 6]); // min/max frame size unknown
        // 20 bits sample rate | 3 bits channels-1 | 5 bits bps-1 | 36 bits total samples
        let packed: u64 = (44_100u64 << 44) | (1u64 << 41) | (15u64 << 36) | 88_200;
        v.extend(packed.to_be_bytes());
        v.extend([0u8; 16]); // MD5
        v.extend([0x81, 0x00, 0x00, 0x40]); // last block, PADDING, 64 bytes
        v.extend([0u8; 64]);
        // A frame header and some bytes: lofty's FLAC writer expects audio after the metadata.
        v.extend([0xFF, 0xF8, 0x69, 0x08, 0x00, 0x00, 0x00, 0x00]);
        v.extend([0u8; 256]);
        let p = dir.join(name);
        std::fs::write(&p, v).unwrap();
        p
    }

    pub const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x06\x00\x00\x00\x1f\x15\xc4\x89";

    pub struct Fields<'a> {
        pub title: &'a str,
        pub artist: &'a str,
        pub album: &'a str,
        pub album_artist: &'a str,
        pub genre: &'a str,
        pub year: &'a str,
        pub track: u32,
        pub track_total: u32,
        pub disc: u32,
        pub cover: bool,
    }

    pub fn write_tags(path: &Path, kind: TagType, f: &Fields) {
        let mut tag = Tag::new(kind);
        tag.set_title(f.title.into());
        tag.set_artist(f.artist.into());
        tag.set_album(f.album.into());
        tag.set_genre(f.genre.into());
        tag.insert_text(ItemKey::AlbumArtist, f.album_artist.into());
        tag.insert_text(ItemKey::RecordingDate, f.year.into());
        tag.set_track(f.track);
        tag.set_track_total(f.track_total);
        tag.set_disk(f.disc);
        if f.cover {
            tag.push_picture(Picture::new_unchecked(PictureType::CoverFront, Some(MimeType::Png), None, PNG.to_vec()));
        }
        tag.save_to_path(path, WriteOptions::default()).unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;
    use lofty::tag::TagType;

    fn vietnamese() -> Fields<'static> {
        Fields {
            title: "Mùa thu cho em",
            artist: "Ngọc Lan",
            album: "Tình khúc vàng",
            album_artist: "Nhiều ca sĩ",
            genre: "Nhạc vàng",
            year: "1990",
            track: 3,
            track_total: 12,
            disc: 1,
            cover: true,
        }
    }

    #[test]
    fn reads_vietnamese_id3_tags_exactly() {
        let d = tempfile::tempdir().unwrap();
        let p = silent_mp3(d.path(), "a.mp3");
        write_tags(&p, TagType::Id3v2, &vietnamese());
        let t = read_tags(&p).unwrap();
        assert_eq!(t.title.as_deref(), Some("Mùa thu cho em"));
        assert_eq!(t.artist.as_deref(), Some("Ngọc Lan"));
        assert_eq!(t.album.as_deref(), Some("Tình khúc vàng"));
        assert_eq!(t.album_artist.as_deref(), Some("Nhiều ca sĩ"));
        assert_eq!(t.genre.as_deref(), Some("Nhạc vàng"));
        assert_eq!(t.year, Some(1990));
        assert_eq!((t.track, t.track_total, t.disc), (Some(3), Some(12), Some(1)));
        assert!(t.has_artwork);
    }

    #[test]
    fn reads_cjk_and_rtl_vorbis_comments_in_flac() {
        let d = tempfile::tempdir().unwrap();
        let p = silent_flac(d.path(), "b.flac");
        write_tags(&p, TagType::VorbisComments, &Fields {
            title: "夜に駆ける",
            artist: "YOASOBI",
            album: "THE BOOK",
            album_artist: "YOASOBI",
            genre: "J-Pop",
            year: "2021-01-06",
            track: 1,
            track_total: 9,
            disc: 1,
            cover: false,
        });
        let t = read_tags(&p).unwrap();
        assert_eq!(t.title.as_deref(), Some("夜に駆ける"));
        assert_eq!(t.year, Some(2021), "year taken from a full date");
        assert!(!t.has_artwork);

        let p2 = silent_flac(d.path(), "c.flac");
        write_tags(&p2, TagType::VorbisComments, &Fields { title: "أهواك", artist: "عبد الحليم حافظ", ..vietnamese() });
        let t2 = read_tags(&p2).unwrap();
        assert_eq!(t2.title.as_deref(), Some("أهواك"));
        assert_eq!(t2.artist.as_deref(), Some("عبد الحليم حافظ"));
    }

    #[test]
    fn duration_comes_from_the_audio_stream() {
        let d = tempfile::tempdir().unwrap();
        let p = silent_flac(d.path(), "d.flac");
        write_tags(&p, TagType::VorbisComments, &vietnamese());
        assert_eq!(read_tags(&p).unwrap().duration_ms, Some(2_000));
    }

    #[test]
    fn untagged_file_has_empty_tags_not_an_error() {
        let d = tempfile::tempdir().unwrap();
        let p = silent_mp3(d.path(), "Diễm xưa.mp3");
        let t = read_tags(&p).unwrap();
        assert_eq!(t.title, None);
        assert!(!t.has_artwork);
    }

    #[test]
    fn display_title_falls_back_to_the_file_name() {
        let t = TrackTags::default();
        assert_eq!(t.display_title(std::path::Path::new(r"E:\Nhạc\Diễm xưa.mp3")), "Diễm xưa");
        let t = TrackTags { title: Some("  ".into()), ..Default::default() };
        assert_eq!(t.display_title(std::path::Path::new("x.mp3")), "x");
    }

    #[test]
    fn extracts_the_front_cover() {
        let d = tempfile::tempdir().unwrap();
        let p = silent_mp3(d.path(), "e.mp3");
        write_tags(&p, TagType::Id3v2, &vietnamese());
        let art = read_artwork(&p).unwrap().expect("cover");
        assert_eq!(art.mime, "image/png");
        assert_eq!(art.data, PNG);
    }

    #[test]
    fn not_audio_is_an_error() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.mp3");
        std::fs::write(&p, b"hello").unwrap();
        assert!(read_tags(&p).is_err());
    }
}

