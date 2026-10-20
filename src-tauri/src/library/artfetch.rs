//! Find covers for albums that have none (no song of the album has embedded art): the iTunes
//! Search API first (good for Vietnamese and Asian catalogues), then MusicBrainz + the Cover Art
//! Archive. Both are free and need no key. Covers are saved under `%APPDATA%\MiuMusic\covers`;
//! the music files are not changed. Misses are remembered and asked again after 30 days.

use std::path::{Path, PathBuf};

use unicode_normalization::UnicodeNormalization;

use crate::library::db::Db;

/// A cover a service offers for an album.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    pub artist: String,
    pub album: String,
    pub image_url: String,
    pub source: &'static str,
}

/// Fetching over HTTP; the app uses ureq, tests a fake.
pub trait Http {
    fn get(&self, url: &str) -> Result<Vec<u8>, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Found { source: &'static str, file: PathBuf },
    NotFound,
}

/// An album as the Albums view groups it: album artist (or artist) and album title.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AlbumKey {
    pub artist: String,
    pub album: String,
}

pub const RETRY_AFTER_SECS: u64 = 30 * 24 * 3600;

pub fn itunes_search_url(key: &AlbumKey) -> String {
    format!("https://itunes.apple.com/search?term={}&entity=album&limit=10", encode(&format!("{} {}", key.artist, key.album)))
}

pub fn musicbrainz_search_url(key: &AlbumKey) -> String {
    let clean = |s: &str| s.replace('"', " ");
    let query = format!("release:\"{}\" AND artist:\"{}\"", clean(&key.album), clean(&key.artist));
    format!("https://musicbrainz.org/ws/2/release/?query={}&fmt=json&limit=5", encode(&query))
}

/// Percent-encode everything but unreserved characters.
fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub fn itunes_candidates(json: &str) -> Vec<Candidate> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    v["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let art = r["artworkUrl100"].as_str()?;
            Some(Candidate {
                artist: r["artistName"].as_str()?.to_string(),
                album: r["collectionName"].as_str()?.to_string(),
                // The URL names its size; Apple serves larger renditions of the same image.
                image_url: art.replace("100x100bb", "600x600bb"),
                source: "itunes",
            })
        })
        .collect()
}

const MIN_MB_SCORE: i64 = 90;

pub fn musicbrainz_candidates(json: &str) -> Vec<Candidate> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    v["releases"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["score"].as_i64().unwrap_or(0) >= MIN_MB_SCORE)
        .filter_map(|r| {
            let id = r["id"].as_str()?;
            let artist = r["artist-credit"].as_array()?.iter().filter_map(|c| c["name"].as_str()).collect::<Vec<_>>().join(" ");
            Some(Candidate {
                artist,
                album: r["title"].as_str()?.to_string(),
                image_url: format!("https://coverartarchive.org/release/{id}/front-500"),
                source: "musicbrainz",
            })
        })
        .collect()
}

/// Lower case, no accents (đ → d), no bracketed or " - Single"-style suffixes, single spaces.
pub fn fold(s: &str) -> String {
    let lower = s.to_lowercase().replace('đ', "d");
    let mut kept = String::new();
    let mut depth = 0;
    for c in lower.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => kept.push(c),
            _ => {}
        }
    }
    let trimmed = kept.trim_end();
    let base = [" - single", " - ep", " - album"].iter().find_map(|suffix| trimmed.strip_suffix(suffix)).unwrap_or(trimmed);
    let plain: String = base.nfd().filter(|c| !('\u{300}'..='\u{36f}').contains(c)).collect();
    plain.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// The candidate that is this album, if any: same album title and the same (or a credited) artist.
pub fn best<'a>(candidates: &'a [Candidate], key: &AlbumKey) -> Option<&'a Candidate> {
    let (album, artist) = (fold(&key.album), fold(&key.artist));
    if album.is_empty() || artist.is_empty() {
        return None;
    }
    candidates.iter().find(|c| {
        let (ca, cr) = (fold(&c.album), fold(&c.artist));
        ca == album && (cr == artist || cr.contains(&artist) || artist.contains(&cr) && !cr.is_empty())
    })
}

/// Look the album up and save its cover as `<dir>/<key hash>.jpg`.
/// A failed search is an error (try again later); a failed image download just moves on.
pub fn fetch_one(http: &dyn Http, key: &AlbumKey, dir: &Path) -> Result<Outcome, String> {
    let searches: [(String, fn(&str) -> Vec<Candidate>); 2] =
        [(itunes_search_url(key), itunes_candidates), (musicbrainz_search_url(key), musicbrainz_candidates)];
    for (url, parse) in searches {
        let body = http.get(&url)?;
        let candidates = parse(&String::from_utf8_lossy(&body));
        let Some(c) = best(&candidates, key) else { continue };
        let Ok(bytes) = http.get(&c.image_url) else { continue };
        let Ok(img) = image::load_from_memory(&bytes) else { continue };
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let file = dir.join(format!("{}.jpg", crate::library::fingerprint::fnv64(format!("{}\u{1f}{}", fold(&key.artist), fold(&key.album)).as_bytes())));
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90).encode_image(&img.to_rgb8()).map_err(|e| e.to_string())?;
        std::fs::write(&file, out).map_err(|e| e.to_string())?;
        return Ok(Outcome::Found { source: c.source, file });
    }
    Ok(Outcome::NotFound)
}

/// Albums without any embedded cover and not looked up recently (misses wait RETRY_AFTER_SECS).
pub fn missing_albums(db: &Db, now_unix: u64) -> Result<Vec<AlbumKey>, String> {
    let mut stmt = db
        .conn()
        .prepare(
            "SELECT g.a, g.album FROM (
                 SELECT COALESCE(NULLIF(album_artist, ''), NULLIF(artist, ''), '') AS a, album, MAX(has_artwork) AS art
                 FROM songs WHERE album IS NOT NULL AND album <> '' GROUP BY a, album
             ) g
             LEFT JOIN album_art x ON x.artist = g.a AND x.album = g.album
             WHERE g.art = 0 AND g.a <> '' AND (x.artist IS NULL OR (x.file IS NULL AND x.checked_at < ?1))
             ORDER BY g.a, g.album",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([now_unix.saturating_sub(RETRY_AFTER_SECS) as i64], |r| Ok(AlbumKey { artist: r.get(0)?, album: r.get(1)? }))
        .map_err(|e| e.to_string())?;
    let albums: Vec<AlbumKey> = rows.collect::<rusqlite::Result<_>>().map_err(|e| e.to_string())?;
    Ok(albums.into_iter().filter(|k| !looks_like_website(&k.album)).collect())
}

/// Download sites often put their own name in the album tag ("NhacCuaTui.Com"): not an album.
fn looks_like_website(album: &str) -> bool {
    let a = album.to_lowercase();
    a.contains("www.") || a.contains("http") || [".com", ".vn", ".net", ".org", ".info"].iter().any(|tld| a.contains(tld))
}

pub fn record(db: &Db, key: &AlbumKey, outcome: &Outcome, now_unix: u64) -> Result<(), String> {
    let (source, file) = match outcome {
        Outcome::Found { source, file } => (Some(*source), Some(file.to_string_lossy().into_owned())),
        Outcome::NotFound => (None, None),
    };
    db.conn()
        .execute(
            "INSERT INTO album_art (artist, album, source, file, checked_at) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(artist, album) DO UPDATE SET source = excluded.source, file = excluded.file, checked_at = excluded.checked_at",
            rusqlite::params![key.artist, key.album, source, file, now_unix as i64],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// The saved cover for an album, if one was found.
pub fn fetched_file(db: &Db, key: &AlbumKey) -> Option<PathBuf> {
    db.conn()
        .query_row("SELECT file FROM album_art WHERE artist = ?1 AND album = ?2 AND file IS NOT NULL", [&key.artist, &key.album], |r| r.get::<_, String>(0))
        .ok()
        .map(PathBuf::from)
}

/// Where a song's cover comes from: embedded in the file, or found online for its album.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cover {
    Embedded { path: PathBuf, mtime: i64 },
    Fetched(PathBuf),
}

pub fn cover_of(db: &Db, song_id: i64) -> Option<Cover> {
    let (path, mtime, embedded, key): (String, i64, bool, Option<AlbumKey>) = db
        .conn()
        .query_row(
            "SELECT path, mtime, has_artwork, COALESCE(NULLIF(album_artist, ''), NULLIF(artist, ''), ''), album FROM songs WHERE id = ?1",
            [song_id],
            |r| {
                let album: Option<String> = r.get(4)?;
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, album.map(|album| AlbumKey { artist: r.get(3).unwrap_or_default(), album })))
            },
        )
        .ok()?;
    if embedded {
        return Some(Cover::Embedded { path: PathBuf::from(path), mtime });
    }
    key.and_then(|k| fetched_file(db, &k)).map(Cover::Fetched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::folders::add_folder;
    use std::cell::RefCell;
    use std::collections::HashMap;

    const ITUNES: &str = r#"{"resultCount":2,"results":[
      {"wrapperType":"collection","artistName":"Khánh Ly","collectionName":"Diễm Xưa (1984) [Remastered] - Single",
       "artworkUrl100":"https://is1-ssl.mzstatic.com/image/thumb/Music/v4/ab/cd/source/100x100bb.jpg"},
      {"wrapperType":"collection","artistName":"Ngọc Lan","collectionName":"Mùa Thu Cho Em",
       "artworkUrl100":"https://is1-ssl.mzstatic.com/image/thumb/Music/x/100x100bb.jpg"}]}"#;

    const MUSICBRAINZ: &str = r#"{"releases":[
      {"id":"09cc35b8-903d-49ab-94c5-23587a15d220","score":100,"title":"Mùa thu cho em",
       "artist-credit":[{"name":"Ngọc Lan","artist":{"name":"Ngọc Lan"}}]},
      {"id":"11111111-2222-3333-4444-555555555555","score":40,"title":"Something else",
       "artist-credit":[{"name":"Someone"}]}]}"#;

    fn key(artist: &str, album: &str) -> AlbumKey {
        AlbumKey { artist: artist.into(), album: album.into() }
    }

    #[test]
    fn itunes_results_become_candidates_with_large_artwork() {
        let c = itunes_candidates(ITUNES);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].artist, "Khánh Ly");
        assert_eq!(c[0].album, "Diễm Xưa (1984) [Remastered] - Single");
        assert_eq!(c[0].image_url, "https://is1-ssl.mzstatic.com/image/thumb/Music/v4/ab/cd/source/600x600bb.jpg");
        assert_eq!(c[0].source, "itunes");
    }

    #[test]
    fn musicbrainz_results_point_at_the_cover_art_archive_and_drop_weak_matches() {
        let c = musicbrainz_candidates(MUSICBRAINZ);
        assert_eq!(c.len(), 1, "score 40 is dropped");
        assert_eq!(c[0].image_url, "https://coverartarchive.org/release/09cc35b8-903d-49ab-94c5-23587a15d220/front-500");
        assert_eq!((c[0].artist.as_str(), c[0].album.as_str(), c[0].source), ("Ngọc Lan", "Mùa thu cho em", "musicbrainz"));
    }

    #[test]
    fn folding_ignores_accents_case_and_release_suffixes() {
        assert_eq!(fold("Diễm Xưa (1984) [Remastered] - Single"), "diem xua");
        assert_eq!(fold("ĐÊM TÂM SỰ - EP"), "dem tam su");
        assert_eq!(fold("  Mùa  thu, cho em!  "), "mua thu cho em");
        assert_eq!(fold("情是那么笨"), "情是那么笨");
    }

    #[test]
    fn the_best_candidate_needs_both_album_and_artist() {
        let c = itunes_candidates(ITUNES);
        assert_eq!(best(&c, &key("Khanh Ly", "Diễm xưa")).map(|c| c.artist.as_str()), Some("Khánh Ly"));
        assert!(best(&c, &key("Ngọc Lan", "Diễm xưa")).is_none(), "right album, wrong artist");
        assert!(best(&c, &key("Khánh Ly", "Hạ Trắng")).is_none());
    }

    #[test]
    fn search_urls_are_encoded() {
        let k = key("Khánh Ly", "Diễm Xưa & Tình");
        let it = itunes_search_url(&k);
        assert!(it.starts_with("https://itunes.apple.com/search?") && it.contains("entity=album"), "{it}");
        assert!(it.contains("Kh%C3%A1nh") && it.contains("%26") && !it.contains(' '), "{it}");
        let mb = musicbrainz_search_url(&k);
        assert!(mb.starts_with("https://musicbrainz.org/ws/2/release/?") && mb.contains("fmt=json") && !mb.contains(' '), "{mb}");
    }

    struct FakeHttp {
        pages: HashMap<&'static str, Vec<u8>>,
        asked: RefCell<Vec<String>>,
    }

    impl Http for FakeHttp {
        fn get(&self, url: &str) -> Result<Vec<u8>, String> {
            self.asked.borrow_mut().push(url.to_string());
            self.pages.iter().find(|(prefix, _)| url.starts_with(**prefix)).map(|(_, b)| b.clone()).ok_or_else(|| format!("404 {url}"))
        }
    }

    fn jpeg() -> Vec<u8> {
        let img = image::RgbImage::from_pixel(8, 8, image::Rgb([200, 80, 40]));
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut out).encode_image(&img).unwrap();
        out
    }

    #[test]
    fn falls_back_to_musicbrainz_and_saves_the_cover() {
        let dir = tempfile::tempdir().unwrap();
        let http = FakeHttp {
            pages: HashMap::from([
                ("https://itunes.apple.com/", br#"{"resultCount":0,"results":[]}"#.to_vec()),
                ("https://musicbrainz.org/", MUSICBRAINZ.as_bytes().to_vec()),
                ("https://coverartarchive.org/release/09cc35b8", jpeg()),
            ]),
            asked: RefCell::new(Vec::new()),
        };
        let out = fetch_one(&http, &key("Ngọc Lan", "Mùa Thu Cho Em"), dir.path()).unwrap();
        let Outcome::Found { source, file } = out else { panic!("expected found, got {out:?}") };
        assert_eq!(source, "musicbrainz");
        assert!(file.starts_with(dir.path()) && file.extension().unwrap() == "jpg");
        assert!(image::open(&file).is_ok(), "a readable image");
        assert_eq!(http.asked.borrow().len(), 3);
    }

    #[test]
    fn nothing_found_saves_nothing_and_a_non_image_is_not_a_cover() {
        let dir = tempfile::tempdir().unwrap();
        let http = FakeHttp {
            pages: HashMap::from([
                ("https://itunes.apple.com/", ITUNES.as_bytes().to_vec()),
                ("https://is1-ssl.mzstatic.com/", b"<html>not an image</html>".to_vec()),
                ("https://musicbrainz.org/", br#"{"releases":[]}"#.to_vec()),
            ]),
            asked: RefCell::new(Vec::new()),
        };
        assert_eq!(fetch_one(&http, &key("Khánh Ly", "Diễm Xưa"), dir.path()).unwrap(), Outcome::NotFound);
        assert_eq!(fetch_one(&http, &key("Nobody", "Nothing"), dir.path()).unwrap(), Outcome::NotFound);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    fn song(db: &Db, path: &str, artist: &str, album: Option<&str>, album_artist: Option<&str>, has_art: bool) {
        let f = match add_folder(db, r"E:\Nhạc", 1) {
            Ok(f) => f.id,
            Err(_) => db.conn().query_row("SELECT id FROM watched_folders", [], |r| r.get(0)).unwrap(),
        };
        db.conn()
            .execute(
                "INSERT INTO songs (folder_id, path, size, mtime, added_at, format, title, artist, album, album_artist, has_artwork, tags_read)
                 VALUES (?1, ?2, 1, 1, 1, 'mp3', ?2, ?3, ?4, ?5, ?6, 1)",
                rusqlite::params![f, path, artist, album, album_artist, has_art],
            )
            .unwrap();
    }

    #[test]
    fn missing_albums_are_those_without_any_cover_and_not_asked_recently() {
        let db = Db::open_in_memory().unwrap();
        song(&db, "1.mp3", "Khánh Ly", Some("Diễm Xưa"), None, false);
        song(&db, "2.mp3", "Guest", Some("Diễm Xưa"), Some("Khánh Ly"), false);
        song(&db, "3.mp3", "Ngọc Lan", Some("Mùa Thu"), None, false);
        song(&db, "4.mp3", "Ngọc Lan", Some("Mùa Thu"), None, true);
        song(&db, "5.mp3", "Như Quỳnh", None, None, false);
        song(&db, "6.mp3", "Hà Nhi", Some("Ai"), None, false);
        song(&db, "7.mp3", "Lệ Quyên", Some("Old miss"), None, false);
        let now = 10 * RETRY_AFTER_SECS;
        record(&db, &key("Hà Nhi", "Ai"), &Outcome::NotFound, now - 60).unwrap();
        record(&db, &key("Lệ Quyên", "Old miss"), &Outcome::NotFound, now - RETRY_AFTER_SECS - 1).unwrap();
        assert_eq!(missing_albums(&db, now).unwrap(), vec![key("Khánh Ly", "Diễm Xưa"), key("Lệ Quyên", "Old miss")]);
    }

    #[test]
    fn website_names_in_the_album_tag_are_not_looked_up() {
        let db = Db::open_in_memory().unwrap();
        song(&db, "1.mp3", "Bảo Thy", Some("NhacCuaTui.Com"), None, false);
        song(&db, "2.mp3", "Lam Trường", Some("nhaccuatui.com"), None, false);
        song(&db, "3.mp3", "Someone", Some("www.zing.vn"), None, false);
        song(&db, "4.mp3", "Dan Truong", Some("Nhung Tinh Khuc Hoai An"), None, false);
        assert_eq!(missing_albums(&db, 1).unwrap(), vec![key("Dan Truong", "Nhung Tinh Khuc Hoai An")]);
    }

    #[test]
    fn a_found_cover_is_remembered_and_its_album_is_no_longer_missing() {
        let db = Db::open_in_memory().unwrap();
        song(&db, "1.mp3", "Khánh Ly", Some("Diễm Xưa"), None, false);
        let file = PathBuf::from(r"C:\covers\x.jpg");
        record(&db, &key("Khánh Ly", "Diễm Xưa"), &Outcome::Found { source: "itunes", file: file.clone() }, 100).unwrap();
        assert_eq!(fetched_file(&db, &key("Khánh Ly", "Diễm Xưa")), Some(file));
        assert!(missing_albums(&db, 200).unwrap().is_empty());
        assert_eq!(fetched_file(&db, &key("Khánh Ly", "Other")), None);
    }

    #[test]
    fn a_song_shows_its_own_cover_first_then_its_albums_fetched_one() {
        let db = Db::open_in_memory().unwrap();
        song(&db, "own.mp3", "Khánh Ly", Some("Diễm Xưa"), None, true);
        song(&db, "plain.mp3", "Khánh Ly", Some("Diễm Xưa"), None, false);
        song(&db, "none.mp3", "Hà Nhi", Some("Ai"), None, false);
        let id = |p: &str| -> i64 { db.conn().query_row("SELECT id FROM songs WHERE path = ?1", [p], |r| r.get(0)).unwrap() };
        record(&db, &key("Khánh Ly", "Diễm Xưa"), &Outcome::Found { source: "itunes", file: PathBuf::from(r"C:coversd.jpg") }, 1).unwrap();
        assert_eq!(cover_of(&db, id("own.mp3")), Some(Cover::Embedded { path: PathBuf::from("own.mp3"), mtime: 1 }));
        assert_eq!(cover_of(&db, id("plain.mp3")), Some(Cover::Fetched(PathBuf::from(r"C:coversd.jpg"))));
        assert_eq!(cover_of(&db, id("none.mp3")), None);
        let listed = crate::library::songs::list_songs(&db).unwrap();
        let has = |p: &str| listed.iter().find(|s| s.path == p).unwrap().has_artwork;
        assert!(has("own.mp3") && has("plain.mp3") && !has("none.mp3"), "the song list shows fetched covers too");
    }

    /// Hits the real iTunes and MusicBrainz services (network). `cargo test --lib artfetch -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn finds_real_covers_online() {
        struct Web(ureq::Agent);
        impl Http for Web {
            fn get(&self, url: &str) -> Result<Vec<u8>, String> {
                let mut res = self.0.get(url).call().map_err(|e| e.to_string())?;
                if res.status().as_u16() != 200 {
                    return Err(format!("{url} answered {}", res.status()));
                }
                res.body_mut().with_config().limit(10_000_000).read_to_vec().map_err(|e| e.to_string())
            }
        }
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .user_agent("MiuMusic/test ( https://musicplayer.vietnamcos.com )")
            .build()
            .into();
        let web = Web(agent);
        let dir = tempfile::tempdir().unwrap();
        for k in [key("Khánh Ly", "Diễm Xưa"), key("Dan Truong", "Nhung Tinh Khuc Hoai An")] {
            let out = fetch_one(&web, &k, dir.path()).unwrap();
            println!("{} – {}: {out:?}", k.artist, k.album);
            if let Outcome::Found { file, .. } = &out {
                let img = image::open(file).unwrap();
                println!("  {}x{}", img.width(), img.height());
            }
            std::thread::sleep(std::time::Duration::from_secs(3));
        }
        assert!(matches!(fetch_one(&web, &key("Khánh Ly", "Diễm Xưa"), dir.path()).unwrap(), Outcome::Found { source: "itunes", .. }));
    }
}
