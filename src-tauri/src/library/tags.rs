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
    /// From the audio stream, not the tag: shown as "FLAC · 24-bit / 96 kHz" or "MP3 · 320 kbps".
    pub sample_rate: Option<u32>,
    /// Lossless files only.
    pub bit_depth: Option<u32>,
    pub bitrate_kbps: Option<u32>,
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
    read_tags_with(path, None)
}

/// Like `read_tags`, repairing legacy-encoded text (F8.1) with `forced` or a detected encoding.
pub fn read_tags_with(path: &Path, forced: Option<&'static encoding_rs::Encoding>) -> Result<TrackTags, String> {
    let file = lofty::read_from_path(path).map_err(|e| e.to_string())?;
    let props = file.properties();
    let duration = props.duration().as_millis() as u64;
    let mut t = TrackTags {
        duration_ms: (duration > 0).then_some(duration),
        sample_rate: props.sample_rate().filter(|&r| r > 0),
        bit_depth: props.bit_depth().map(u32::from).filter(|&b| b > 0),
        bitrate_kbps: props.audio_bitrate().filter(|&b| b > 0),
        ..Default::default()
    };
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
    crate::library::legacy::repair(&mut [&mut t.title, &mut t.artist, &mut t.album, &mut t.album_artist, &mut t.genre], forced);
    Ok(t)
}

/// Lyrics written in the file's tags (unsynced lyrics frames; often LRC text), if any.
pub fn read_lyrics(path: &Path) -> Option<String> {
    let file = lofty::read_from_path(path).ok()?;
    file.tags().iter().find_map(|t| t.get_string(&ItemKey::Lyrics).map(str::trim).filter(|s| !s.is_empty()).map(String::from))
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

    fn mp4_box(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
        let mut v = ((payload.len() + 8) as u32).to_be_bytes().to_vec();
        v.extend(kind);
        v.extend(payload);
        v
    }

    fn full_box(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut p = vec![0u8; 4]; // version + flags
        p.extend(body);
        mp4_box(kind, &p)
    }

    /// A minimal AAC M4A (1 s, 44.1 kHz stereo, no samples) that tag readers and writers accept.
    pub fn silent_m4a(dir: &Path, name: &str) -> PathBuf {
        let ftyp = mp4_box(b"ftyp", b"M4A \0\0\0\0M4A mp42isom");
        let mut mvhd = vec![0u8; 96];
        mvhd[8..12].copy_from_slice(&44_100u32.to_be_bytes()); // timescale
        mvhd[12..16].copy_from_slice(&44_100u32.to_be_bytes()); // duration
        let mut tkhd = vec![0u8; 80];
        tkhd[3] = 1;
        let mut mdhd = vec![0u8; 20];
        mdhd[8..12].copy_from_slice(&44_100u32.to_be_bytes());
        mdhd[12..16].copy_from_slice(&44_100u32.to_be_bytes());
        let mut hdlr = vec![0u8; 4];
        hdlr.extend(b"soun");
        hdlr.extend([0u8; 13]);
        let mut entry = vec![0u8; 6];
        entry.extend(1u16.to_be_bytes()); // data reference index
        entry.extend([0u8; 8]);
        entry.extend(2u16.to_be_bytes()); // channels
        entry.extend(16u16.to_be_bytes()); // bits
        entry.extend([0u8; 4]);
        entry.extend((44_100u32 << 16).to_be_bytes());
        let mut stsd = 1u32.to_be_bytes().to_vec();
        stsd.extend(mp4_box(b"mp4a", &entry));
        let stbl = [full_box(b"stsd", &stsd), full_box(b"stts", &[0; 4]), full_box(b"stsc", &[0; 4]), full_box(b"stsz", &[0; 8]), full_box(b"stco", &[0; 4])].concat();
        let minf = [full_box(b"smhd", &[0; 4]), mp4_box(b"stbl", &stbl)].concat();
        let mdia = [full_box(b"mdhd", &mdhd), full_box(b"hdlr", &hdlr), mp4_box(b"minf", &minf)].concat();
        let trak = [full_box(b"tkhd", &tkhd), mp4_box(b"mdia", &mdia)].concat();
        let moov = [full_box(b"mvhd", &mvhd), mp4_box(b"trak", &trak)].concat();
        let p = dir.join(name);
        std::fs::write(&p, [ftyp, mp4_box(b"moov", &moov), mp4_box(b"mdat", &[0; 16])].concat()).unwrap();
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


#[cfg(test)]
mod quality_tests {
    use super::fixtures::*;
    use super::read_tags;

    #[test]
    fn lossless_files_report_sample_rate_and_bit_depth() {
        let d = tempfile::tempdir().unwrap();
        let t = read_tags(&silent_flac(d.path(), "a.flac")).unwrap();
        assert_eq!((t.sample_rate, t.bit_depth), (Some(44_100), Some(16)));
        let hr = crate::audio::fixtures::wav(d.path(), "hr.wav", 96_000, 2, 24, 9_600);
        let t = read_tags(&hr).unwrap();
        assert_eq!((t.sample_rate, t.bit_depth), (Some(96_000), Some(24)));
    }

    #[test]
    fn lossy_files_report_their_bitrate_and_no_bit_depth() {
        let d = tempfile::tempdir().unwrap();
        let t = read_tags(&silent_mp3(d.path(), "a.mp3")).unwrap();
        assert_eq!(t.bitrate_kbps, Some(128));
        assert_eq!(t.sample_rate, Some(44_100));
        assert_eq!(t.bit_depth, None);
    }
}

#[cfg(test)]
mod lyrics_tests {
    use super::fixtures::*;
    use super::*;
    use lofty::config::WriteOptions;
    use lofty::tag::{TagExt, TagType};

    #[test]
    fn reads_lyrics_written_in_the_tags() {
        let d = tempfile::tempdir().unwrap();
        let p = silent_mp3(d.path(), "a.mp3");
        assert_eq!(read_lyrics(&p), None, "none yet");
        let mut tag = Tag::new(TagType::Id3v2);
        tag.insert_text(ItemKey::Lyrics, "[00:01.00] Mưa vẫn mưa bay".into());
        tag.save_to_path(&p, WriteOptions::default()).unwrap();
        assert_eq!(read_lyrics(&p).as_deref(), Some("[00:01.00] Mưa vẫn mưa bay"));
    }
}
