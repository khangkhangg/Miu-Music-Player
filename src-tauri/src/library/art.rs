//! Album art for the UI (redesign): the cover embedded in a song, cut to a square thumbnail,
//! JPEG-encoded and cached on disk, so grids of covers stay fast and light on memory.

use std::path::{Path, PathBuf};

use image::imageops::FilterType;

/// Largest thumbnail the UI may ask for (the now-playing cover).
pub const MAX_SIZE: u32 = 640;

/// The song's cover as a `size`×`size` JPEG (centre-cropped; never enlarged), or `None` if it has none.
pub fn thumbnail(song: &Path, size: u32) -> Result<Option<Vec<u8>>, String> {
    let Some(art) = crate::library::tags::read_artwork(song)? else { return Ok(None) };
    square_jpeg(&art.data, size).map(Some)
}

/// Centre-cropped square JPEG of an encoded image, never enlarged.
fn square_jpeg(bytes: &[u8], size: u32) -> Result<Vec<u8>, String> {
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let side = size.min(img.width()).min(img.height()).max(1);
    let square = img.resize_to_fill(side, side, FilterType::Lanczos3).to_rgb8();
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85).encode_image(&square).map_err(|e| e.to_string())?;
    Ok(out)
}

/// A cover found online (`library::artfetch`) as a thumbnail, cached by file, its time and size.
pub fn cached_file_thumbnail(cache: &Path, file: &Path, size: u32) -> Result<Option<Vec<u8>>, String> {
    let Ok(meta) = std::fs::metadata(file) else { return Ok(None) };
    let stamp = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    let name = crate::library::fingerprint::fnv64(file.to_string_lossy().as_bytes());
    let cached = cache.join(format!("f{name}-{stamp}-{size}.jpg"));
    if let Ok(bytes) = std::fs::read(&cached) {
        return Ok(Some(bytes));
    }
    let jpg = square_jpeg(&std::fs::read(file).map_err(|e| e.to_string())?, size)?;
    std::fs::create_dir_all(cache).map_err(|e| e.to_string())?;
    std::fs::write(&cached, &jpg).map_err(|e| e.to_string())?;
    Ok(Some(jpg))
}

fn cache_file(cache: &Path, song_id: i64, mtime: i64, size: u32) -> PathBuf {
    cache.join(format!("{song_id}-{mtime}-{size}.jpg"))
}

/// `thumbnail`, kept in `cache` per song, file time and size. A cached "no cover" is an empty file.
pub fn cached_thumbnail(cache: &Path, song_id: i64, mtime: i64, song: &Path, size: u32) -> Result<Option<Vec<u8>>, String> {
    let file = cache_file(cache, song_id, mtime, size);
    if let Ok(bytes) = std::fs::read(&file) {
        return Ok((!bytes.is_empty()).then_some(bytes));
    }
    let made = thumbnail(song, size)?;
    std::fs::create_dir_all(cache).map_err(|e| e.to_string())?;
    std::fs::write(&file, made.as_deref().unwrap_or_default()).map_err(|e| e.to_string())?;
    Ok(made)
}

/// `/<song id>-<size>` from a `miuart` request path.
pub fn parse_request(path: &str) -> Option<(i64, u32)> {
    let (id, size) = path.trim_start_matches('/').split_once('-')?;
    Some((id.parse().ok()?, size.parse::<u32>().ok()?.min(MAX_SIZE)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tags::fixtures::silent_mp3;
    use lofty::config::WriteOptions;
    use lofty::picture::{MimeType, Picture, PictureType};
    use lofty::tag::{Tag, TagExt, TagType};

    /// An MP3 whose tag holds a real `w`×`h` PNG cover.
    fn song_with_cover(dir: &std::path::Path, w: u32, h: u32) -> std::path::PathBuf {
        let p = silent_mp3(dir, "cover.mp3");
        let img = image::RgbImage::from_fn(w, h, |x, _| image::Rgb([(x % 256) as u8, 80, 160]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let mut tag = Tag::new(TagType::Id3v2);
        tag.push_picture(Picture::new_unchecked(PictureType::CoverFront, Some(MimeType::Png), None, png));
        tag.save_to_path(&p, WriteOptions::default()).unwrap();
        p
    }

    #[test]
    fn a_fetched_cover_file_becomes_a_cached_square_thumbnail() {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("cover.jpg");
        image::RgbImage::from_pixel(500, 300, image::Rgb([200, 80, 40])).save(&file).unwrap();
        let cache = d.path().join("cache");
        let jpg = cached_file_thumbnail(&cache, &file, 120).unwrap().expect("a thumbnail");
        let img = image::load_from_memory(&jpg).unwrap();
        assert_eq!((img.width(), img.height()), (120, 120));
        assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 1, "cached");
        std::fs::remove_file(&file).unwrap();
        assert_eq!(cached_file_thumbnail(&cache, &file, 120).unwrap(), None, "a removed cover is not served from cache");
    }

    #[test]
    fn a_cover_becomes_a_square_jpeg_thumbnail() {
        let d = tempfile::tempdir().unwrap();
        let p = song_with_cover(d.path(), 600, 400);
        let jpg = thumbnail(&p, 160).unwrap().expect("has a cover");
        assert_eq!(&jpg[..3], &[0xFF, 0xD8, 0xFF], "JPEG");
        let img = image::load_from_memory(&jpg).unwrap();
        assert_eq!((img.width(), img.height()), (160, 160), "cropped to a square");
    }

    #[test]
    fn small_covers_are_not_blown_up() {
        let d = tempfile::tempdir().unwrap();
        let p = song_with_cover(d.path(), 100, 100);
        let img = image::load_from_memory(&thumbnail(&p, 480).unwrap().unwrap()).unwrap();
        assert_eq!((img.width(), img.height()), (100, 100));
    }

    #[test]
    fn a_song_without_a_cover_has_no_thumbnail() {
        let d = tempfile::tempdir().unwrap();
        assert!(thumbnail(&silent_mp3(d.path(), "plain.mp3"), 160).unwrap().is_none());
    }

    #[test]
    fn thumbnails_are_cached_until_the_file_changes() {
        let d = tempfile::tempdir().unwrap();
        let cache = d.path().join("art");
        let p = song_with_cover(d.path(), 300, 300);
        let a = cached_thumbnail(&cache, 7, 1_000, &p, 160).unwrap().unwrap();
        std::fs::remove_file(&p).unwrap(); // gone from disk: only the cache can answer now
        let b = cached_thumbnail(&cache, 7, 1_000, &p, 160).unwrap().unwrap();
        assert_eq!(a, b, "served from the cache");
        assert!(cached_thumbnail(&cache, 7, 2_000, &p, 160).is_err(), "new mtime: not taken from the old cache");
    }

    #[test]
    fn art_urls_parse_into_song_and_size() {
        assert_eq!(parse_request("/42-160"), Some((42, 160)));
        assert_eq!(parse_request("/42-9999"), Some((42, MAX_SIZE)), "sizes are capped");
        assert_eq!(parse_request("/../etc"), None);
        assert_eq!(parse_request("/x-160"), None);
    }
}
