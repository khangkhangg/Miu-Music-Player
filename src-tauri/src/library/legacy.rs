//! Legacy tag encodings (F8.1). Old ID3 tags often hold Vietnamese (Windows-1258), Chinese (GBK),
//! Japanese (Shift-JIS) or Cyrillic (Windows-1251) bytes while claiming Latin-1, so they arrive
//! here as Latin-1 mojibake. Every Latin-1 character is one byte, so the bytes are recovered
//! exactly, the real encoding is detected across all of a song's fields together (or taken from
//! the folder's override), and the text is decoded again.

use encoding_rs::{Encoding, WINDOWS_1252};
use unicode_normalization::UnicodeNormalization;

/// The bytes behind text a reader decoded as Latin-1, if it looks like that happened: every
/// character fits in a byte and at least one is beyond ASCII.
fn latin1_bytes(s: &str) -> Option<Vec<u8>> {
    let mut high = false;
    let mut out = Vec::with_capacity(s.len());
    for c in s.chars() {
        let v = c as u32;
        if v > 0xFF {
            return None;
        }
        high |= v >= 0x80;
        out.push(v as u8);
    }
    high.then_some(out)
}

/// An encoding by its WHATWG label ("windows-1258", "gbk", "shift_jis", …).
pub fn encoding_named(label: &str) -> Option<&'static Encoding> {
    Encoding::for_label(label.trim().as_bytes())
}

/// Re-decode a song's fields that are legacy bytes shown as Latin-1. `forced` (a folder's
/// override) wins; otherwise the encoding is detected from all affected fields together, and
/// text that really is Western Latin is left as it is.
pub fn repair(fields: &mut [&mut Option<String>], forced: Option<&'static Encoding>) {
    let raw: Vec<Option<Vec<u8>>> = fields.iter().map(|f| f.as_deref().and_then(latin1_bytes)).collect();
    if raw.iter().all(Option::is_none) {
        return;
    }
    // A field beyond Latin-1 means the tag was written by a Unicode-aware tagger: trust it all.
    let unicode = fields.iter().any(|f| f.as_deref().is_some_and(|s| s.chars().any(|c| c as u32 > 0xFF)));
    if unicode && forced.is_none() {
        return;
    }
    let enc = forced.unwrap_or_else(|| {
        let mut det = chardetng::EncodingDetector::new(chardetng::Iso2022JpDetection::Deny);
        for bytes in raw.iter().flatten() {
            det.feed(bytes, false);
            det.feed(b" ", false);
        }
        det.feed(b"", true);
        det.guess(None, chardetng::Utf8Detection::Deny)
    });
    if forced.is_none() && (enc == WINDOWS_1252 || enc == encoding_rs::UTF_8) {
        return;
    }
    for (field, bytes) in fields.iter_mut().zip(raw) {
        if let Some(bytes) = bytes {
            let (text, _, _) = enc.decode(&bytes);
            // Windows-1258 stores tones as combining marks; compose them as other sources do.
            **field = Some(text.nfc().collect());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use encoding_rs::{Encoding, GBK, SHIFT_JIS, WINDOWS_1251, WINDOWS_1258};

    /// Windows-1258 keeps letters like ê and ư whole but writes tones as separate marks:
    /// split each character into the longest encodable precomposed part plus leftover marks.
    fn as_1258_writes_it(text: &str) -> String {
        let mut out = String::new();
        for c in text.chars() {
            let mut parts: Vec<char> = c.to_string().nfd().collect();
            let mut base = parts.remove(0).to_string();
            while let Some(&m) = parts.first() {
                let joined: String = format!("{base}{m}").nfc().collect();
                if WINDOWS_1258.encode(&joined).2 {
                    break;
                }
                base = joined;
                parts.remove(0);
            }
            out.push_str(&base);
            out.extend(parts);
        }
        out
    }

    /// What a tag reader shows for `text` stored in `enc` but labelled Latin-1.
    fn mojibake(text: &str, enc: &'static Encoding) -> Option<String> {
        let text = if enc == WINDOWS_1258 { as_1258_writes_it(text) } else { text.to_string() };
        let (bytes, _, unmappable) = enc.encode(&text);
        assert!(!unmappable, "{text} fits {}", enc.name());
        Some(bytes.iter().map(|&b| b as char).collect())
    }

    fn fixed(fields: [&str; 3], enc: &'static Encoding, forced: Option<&'static Encoding>) -> [Option<String>; 3] {
        let mut f = fields.map(|s| mojibake(s, enc));
        let [a, b, c] = &mut f;
        repair(&mut [a, b, c], forced);
        f
    }

    fn some(fields: [&str; 3]) -> [Option<String>; 3] {
        fields.map(|s| Some(s.to_string()))
    }

    #[test]
    fn vietnamese_windows_1258_is_recovered_and_composed() {
        let f = ["Diễm xưa", "Khánh Ly", "Trịnh Công Sơn - Những bài ca không năm tháng"];
        assert_eq!(fixed(f, WINDOWS_1258, None), some(f));
    }

    #[test]
    fn chinese_gbk_is_recovered() {
        let f = ["月亮代表我的心", "邓丽君", "邓丽君经典金曲"];
        assert_eq!(fixed(f, GBK, None), some(f));
    }

    #[test]
    fn japanese_shift_jis_is_recovered() {
        let f = ["夜に駆ける", "ＹＯＡＳＯＢＩ", "ハルカ・夜に駆ける"];
        assert_eq!(fixed(f, SHIFT_JIS, None), some(f));
    }

    #[test]
    fn cyrillic_windows_1251_is_recovered() {
        let f = ["Подмосковные вечера", "Владимир Трошин", "Песни советских композиторов"];
        assert_eq!(fixed(f, WINDOWS_1251, None), some(f));
    }

    #[test]
    fn real_latin_1_text_is_left_alone() {
        let mut a = Some("Café del Mar".to_string());
        let mut b = Some("Beyoncé".to_string());
        let mut c = Some("Déjà vu".to_string());
        repair(&mut [&mut a, &mut b, &mut c], None);
        assert_eq!([a, b, c], some(["Café del Mar", "Beyoncé", "Déjà vu"]));
    }

    #[test]
    fn unicode_text_is_never_touched() {
        let mut a = Some("Diễm xưa".to_string());
        let mut b = Some("夜に駆ける".to_string());
        let mut c = None;
        repair(&mut [&mut a, &mut b, &mut c], Some(WINDOWS_1251));
        assert_eq!((a.as_deref(), b.as_deref(), c), (Some("Diễm xưa"), Some("夜に駆ける"), None));
    }

    #[test]
    fn a_song_whose_other_fields_are_unicode_is_not_reinterpreted() {
        // Written by a Unicode-aware tagger: the title happens to fit Latin-1, the others do not.
        let mut a = Some("Mùa thu cho em".to_string());
        let mut b = Some("Ngọc Lan".to_string());
        let mut c = Some("Mùa thu".to_string());
        repair(&mut [&mut a, &mut b, &mut c], None);
        assert_eq!(a.as_deref(), Some("Mùa thu cho em"));
        assert_eq!(c.as_deref(), Some("Mùa thu"));
    }

    #[test]
    fn the_folder_override_wins_over_detection() {
        // Cyrillic bytes, but the folder says they are Windows-1258: the override is obeyed.
        let f = fixed(["Подмосковные вечера", "Владимир Трошин", "Песни"], WINDOWS_1251, Some(WINDOWS_1258));
        let bytes = WINDOWS_1251.encode("Подмосковные вечера").0;
        let expected = WINDOWS_1258.decode(&bytes).0;
        assert_eq!(f[0].as_deref(), Some(expected.nfc().collect::<String>().as_str()));
    }

    #[test]
    fn override_names_are_checked() {
        assert_eq!(encoding_named("windows-1258"), Some(WINDOWS_1258));
        assert_eq!(encoding_named("shift_jis"), Some(SHIFT_JIS));
        assert_eq!(encoding_named("klingon"), None);
    }
}

#[cfg(test)]
mod scan_tests {
    use crate::library::db::Db;
    use crate::library::folders::{add_folder, list_folders, set_tag_encoding};
    use crate::library::scan::rescan_folder;
    use crate::library::songs::list_songs;
    use encoding_rs::WINDOWS_1251;

    /// An MP3 whose ID3v2.3 title and artist frames say Latin-1 but hold Windows-1251 bytes.
    fn cp1251_mp3(dir: &std::path::Path) {
        let frame = |id: &[u8], text: &str| {
            let mut body = vec![0u8]; // encoding byte 0 = "ISO-8859-1"
            body.extend(WINDOWS_1251.encode(text).0.iter());
            let mut f = id.to_vec();
            f.extend((body.len() as u32).to_be_bytes());
            f.extend([0, 0]);
            f.extend(body);
            f
        };
        let mut frames = frame(b"TIT2", "Подмосковные вечера");
        frames.extend(frame(b"TPE1", "Владимир Трошин"));
        let n = frames.len() as u32;
        let syncsafe = [(n >> 21) as u8 & 0x7F, (n >> 14) as u8 & 0x7F, (n >> 7) as u8 & 0x7F, n as u8 & 0x7F];
        let mut v = b"ID3\x03\x00\x00".to_vec();
        v.extend(syncsafe);
        v.extend(frames);
        let mut mp3 = vec![0u8; 417];
        mp3[..4].copy_from_slice(&[0xFF, 0xFB, 0x90, 0x64]);
        v.extend(mp3.repeat(10));
        std::fs::write(dir.join("old.mp3"), v).unwrap();
    }

    fn title(db: &Db) -> (String, Option<String>) {
        let s = list_songs(db).unwrap().remove(0);
        (s.title, s.artist)
    }

    #[test]
    fn scanning_repairs_legacy_tags() {
        let d = tempfile::tempdir().unwrap();
        cp1251_mp3(d.path());
        let db = Db::open_in_memory().unwrap();
        let f = add_folder(&db, &d.path().to_string_lossy(), 1).unwrap();
        rescan_folder(&db, f.id, &f.path, 1).unwrap();
        assert_eq!(title(&db), ("Подмосковные вечера".into(), Some("Владимир Трошин".into())));
    }

    #[test]
    fn a_folder_override_rereads_the_tags_with_that_encoding() {
        let d = tempfile::tempdir().unwrap();
        cp1251_mp3(d.path());
        let db = Db::open_in_memory().unwrap();
        let f = add_folder(&db, &d.path().to_string_lossy(), 1).unwrap();
        rescan_folder(&db, f.id, &f.path, 1).unwrap();
        set_tag_encoding(&db, f.id, Some("windows-1252")).unwrap();
        assert_eq!(list_folders(&db).unwrap()[0].tag_encoding.as_deref(), Some("windows-1252"));
        rescan_folder(&db, f.id, &f.path, 2).unwrap();
        assert_eq!(title(&db).0, "Ïîäìîñêîâíûå âå÷åðà", "shown as the folder says");
        assert!(set_tag_encoding(&db, f.id, Some("klingon")).is_err());
        set_tag_encoding(&db, f.id, None).unwrap();
        rescan_folder(&db, f.id, &f.path, 3).unwrap();
        assert_eq!(title(&db).0, "Подмосковные вечера", "back to detection");
    }
}
