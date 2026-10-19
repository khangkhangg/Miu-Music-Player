//! Finding songs in a watched folder and keeping the `songs` table in step with the disk (F3.1).
//! Candidates are picked by extension, then each file content is sniffed (F3.2); files whose
//! content is not recognisable audio are left out.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::library::db::Db;
use crate::library::format::{detect_file, Format};
use crate::library::tags::{read_tags_with, TrackTags};
use rusqlite::OptionalExtension;

const AUDIO_EXTENSIONS: &[&str] = &["mp3", "m4a", "aac", "alac", "flac", "wav", "aif", "aiff", "ogg", "oga", "opus", "wma", "ape", "wv"];

#[derive(Debug, Clone, PartialEq)]
pub struct FileEntry {
    pub path: PathBuf,
    pub size: u64,
    /// Seconds since 1970.
    pub mtime: i64,
    /// From the file content (F3.2), not the extension.
    pub format: Format,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ScanDiff {
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
}

pub fn is_audio_path(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTENSIONS.iter().any(|a| a.eq_ignore_ascii_case(e)))
}

/// Every audio file under `root`, sorted by path. Folders starting with `.` are skipped;
/// unreadable folders are skipped silently (a watched drive may be partly inaccessible).
pub fn scan_folder(root: &Path) -> Vec<FileEntry> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                if !entry.file_name().to_string_lossy().starts_with('.') {
                    stack.push(path);
                }
            } else if meta.is_file() && is_audio_path(&path) {
                let format = detect_file(&path).unwrap_or(Format::Unknown);
                if format == Format::Unknown {
                    continue;
                }
                let mtime = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs() as i64);
                out.push(FileEntry { path, size: meta.len(), mtime, format });
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// Make the folder's rows in `songs` match `found`: insert new files, update changed ones
/// (keeping rating), delete files that are gone. Tags are read only for new or changed files
/// (and rows that never had them read). One transaction.
pub fn reconcile(db: &Db, folder_id: i64, found: &[FileEntry], now_unix: u64) -> rusqlite::Result<ScanDiff> {
    let forced: Option<String> = db
        .conn()
        .query_row("SELECT tag_encoding FROM watched_folders WHERE id = ?1", [folder_id], |r| r.get(0))
        .optional()?
        .flatten();
    let forced = forced.as_deref().and_then(crate::library::legacy::encoding_named);
    reconcile_with(db, folder_id, found, now_unix, |p| read_tags_with(p, forced).unwrap_or_default())
}

struct Known {
    id: i64,
    size: i64,
    mtime: i64,
    format: String,
    tags_read: bool,
}

pub fn reconcile_with(
    db: &Db,
    folder_id: i64,
    found: &[FileEntry],
    now_unix: u64,
    read: impl Fn(&Path) -> TrackTags,
) -> rusqlite::Result<ScanDiff> {
    let conn = db.conn();
    let tx = conn.unchecked_transaction()?;
    let mut known: HashMap<String, Known> = HashMap::new();
    {
        let mut stmt = tx.prepare("SELECT id, path, size, mtime, format, tags_read FROM songs WHERE folder_id = ?1")?;
        let rows = stmt.query_map([folder_id], |r| {
            let k = Known { id: r.get(0)?, size: r.get(2)?, mtime: r.get(3)?, format: r.get(4)?, tags_read: r.get(5)? };
            Ok((r.get::<_, String>(1)?.to_lowercase(), k))
        })?;
        for row in rows {
            let (path, k) = row?;
            known.insert(path, k);
        }
    }
    let mut diff = ScanDiff::default();
    for f in found {
        let path = f.path.to_string_lossy();
        let id = match known.remove(&path.to_lowercase()) {
            None => {
                tx.execute(
                    "INSERT INTO songs (folder_id, path, size, mtime, added_at, format) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![folder_id, path, f.size as i64, f.mtime, now_unix as i64, f.format.name()],
                )?;
                diff.added += 1;
                tx.last_insert_rowid()
            }
            Some(k) if k.size != f.size as i64 || k.mtime != f.mtime || k.format != f.format.name() || !k.tags_read => {
                tx.execute(
                    "UPDATE songs SET size = ?1, mtime = ?2, format = ?3,
                         audio_hash = CASE WHEN size != ?1 OR mtime != ?2 OR format != ?3 THEN NULL ELSE audio_hash END
                     WHERE id = ?4",
                    rusqlite::params![f.size as i64, f.mtime, f.format.name(), k.id],
                )?;
                diff.updated += 1;
                k.id
            }
            Some(_) => continue,
        };
        store_tags(&tx, id, &read(&f.path))?;
    }
    for Known { id, .. } in known.into_values() {
        tx.execute("DELETE FROM songs WHERE id = ?1", [id])?;
        diff.removed += 1;
    }
    tx.commit()?;
    Ok(diff)
}

fn store_tags(tx: &rusqlite::Transaction<'_>, id: i64, t: &TrackTags) -> rusqlite::Result<()> {
    tx.execute(
        "UPDATE songs SET title = ?1, artist = ?2, album = ?3, album_artist = ?4, genre = ?5, year = ?6,
             track_no = ?7, track_total = ?8, disc_no = ?9, duration_ms = ?10, has_artwork = ?11,
             sample_rate = ?12, bit_depth = ?13, bitrate_kbps = ?14, tags_read = 1
         WHERE id = ?15",
        rusqlite::params![
            t.title, t.artist, t.album, t.album_artist, t.genre, t.year, t.track, t.track_total, t.disc,
            t.duration_ms.map(|d| d as i64), t.has_artwork, t.sample_rate, t.bit_depth, t.bitrate_kbps, id
        ],
    )?;
    Ok(())
}

pub fn rescan_folder(db: &Db, folder_id: i64, root: &str, now_unix: u64) -> rusqlite::Result<ScanDiff> {
    reconcile(db, folder_id, &scan_folder(Path::new(root)), now_unix)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::folders::add_folder;
    use std::fs;

    fn touch(path: &Path, bytes: usize) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        // An MPEG Layer III frame header, so content sniffing (F3.2) sees an MP3.
        let mut data = vec![0xFF, 0xFB, 0x90, 0x00];
        data.resize(bytes.max(4), 0);
        fs::write(path, data).unwrap();
    }

    fn rating_of(db: &Db, path: &Path) -> i64 {
        db.conn()
            .query_row("SELECT rating FROM songs WHERE path = ?1", [path.to_string_lossy()], |r| r.get(0))
            .unwrap()
    }

    #[test]
    fn recognises_audio_extensions_in_any_case() {
        for p in ["a.mp3", "b.M4A", "c.flac", "d.wav", "e.aiff", "f.aif", "g.ogg", "h.opus", "i.wma", "j.ape", "k.wv", "l.aac"] {
            assert!(is_audio_path(Path::new(p)), "{p}");
        }
        for p in ["cover.jpg", "notes.txt", "noext", "playlist.m3u"] {
            assert!(!is_audio_path(Path::new(p)), "{p}");
        }
    }

    #[test]
    fn finds_audio_files_recursively_with_unicode_names_and_skips_hidden_folders() {
        let d = tempfile::tempdir().unwrap();
        touch(&d.path().join("Mùa thu cho em.mp3"), 10);
        touch(&d.path().join("Trịnh").join("Diễm xưa.flac"), 20);
        touch(&d.path().join("cover.jpg"), 5);
        touch(&d.path().join(".trash").join("old.mp3"), 5);
        let found = scan_folder(d.path());
        let names: Vec<String> = found.iter().map(|e| e.path.strip_prefix(d.path()).unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["Mùa thu cho em.mp3".to_string(), format!("Trịnh{}Diễm xưa.flac", std::path::MAIN_SEPARATOR)]);
        assert_eq!(found[1].size, 20);
        assert!(found[0].mtime > 0);
    }

    #[test]
    fn first_reconcile_adds_every_file_and_a_rerun_changes_nothing() {
        let d = tempfile::tempdir().unwrap();
        touch(&d.path().join("a.mp3"), 1);
        touch(&d.path().join("b.mp3"), 2);
        let db = Db::open_in_memory().unwrap();
        let f = add_folder(&db, &d.path().to_string_lossy(), 1).unwrap();
        assert_eq!(reconcile(&db, f.id, &scan_folder(d.path()), 100).unwrap(), ScanDiff { added: 2, updated: 0, removed: 0 });
        assert_eq!(reconcile(&db, f.id, &scan_folder(d.path()), 200).unwrap(), ScanDiff::default());
    }

    #[test]
    fn changed_files_are_updated_and_keep_their_rating() {
        let d = tempfile::tempdir().unwrap();
        let song = d.path().join("a.mp3");
        touch(&song, 1);
        let db = Db::open_in_memory().unwrap();
        let f = add_folder(&db, &d.path().to_string_lossy(), 1).unwrap();
        reconcile(&db, f.id, &scan_folder(d.path()), 100).unwrap();
        db.conn().execute("UPDATE songs SET rating = 1", []).unwrap();
        touch(&song, 999);
        assert_eq!(reconcile(&db, f.id, &scan_folder(d.path()), 200).unwrap(), ScanDiff { added: 0, updated: 1, removed: 0 });
        assert_eq!(rating_of(&db, &song), 1);
    }

    #[test]
    fn files_gone_from_disk_are_removed_from_the_library() {
        let d = tempfile::tempdir().unwrap();
        touch(&d.path().join("a.mp3"), 1);
        touch(&d.path().join("b.mp3"), 1);
        let db = Db::open_in_memory().unwrap();
        let f = add_folder(&db, &d.path().to_string_lossy(), 1).unwrap();
        reconcile(&db, f.id, &scan_folder(d.path()), 100).unwrap();
        fs::remove_file(d.path().join("b.mp3")).unwrap();
        assert_eq!(reconcile(&db, f.id, &scan_folder(d.path()), 200).unwrap(), ScanDiff { added: 0, updated: 0, removed: 1 });
    }

    #[test]
    fn reconcile_only_touches_its_own_folder() {
        let (d1, d2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        touch(&d1.path().join("a.mp3"), 1);
        touch(&d2.path().join("b.mp3"), 1);
        let db = Db::open_in_memory().unwrap();
        let f1 = add_folder(&db, &d1.path().to_string_lossy(), 1).unwrap();
        let f2 = add_folder(&db, &d2.path().to_string_lossy(), 1).unwrap();
        reconcile(&db, f1.id, &scan_folder(d1.path()), 100).unwrap();
        reconcile(&db, f2.id, &scan_folder(d2.path()), 100).unwrap();
        assert_eq!(reconcile(&db, f1.id, &[], 200).unwrap().removed, 1);
        let left: i64 = db.conn().query_row("SELECT count(*) FROM songs", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 1);
    }
}

#[cfg(test)]
mod rescan_tests {
    use super::*;
    use crate::library::folders::add_folder;

    #[test]
    fn rescan_folder_scans_and_reconciles_in_one_step() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.mp3"), [0xFF, 0xFB, 0x90, 0x00]).unwrap();
        let db = Db::open_in_memory().unwrap();
        let f = add_folder(&db, &d.path().to_string_lossy(), 1).unwrap();
        assert_eq!(rescan_folder(&db, f.id, &f.path, 10).unwrap().added, 1);
        assert_eq!(rescan_folder(&db, f.id, &f.path, 20).unwrap(), ScanDiff::default());
    }
}

#[cfg(test)]
mod format_tests {
    use super::*;
    use crate::library::folders::add_folder;
    use crate::library::format::Format;

    #[test]
    fn scan_records_the_real_format_and_skips_non_audio_content() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("really flac.mp3"), b"fLaC\x00\x00\x00\x22").unwrap();
        std::fs::write(d.path().join("song.mp3"), [0xFF, 0xFB, 0x90, 0x00]).unwrap();
        std::fs::write(d.path().join("readme.mp3"), b"this is text").unwrap();
        let found = scan_folder(d.path());
        let got: Vec<(String, Format)> = found.iter().map(|e| (e.path.file_name().unwrap().to_string_lossy().into_owned(), e.format)).collect();
        assert_eq!(got, vec![("really flac.mp3".into(), Format::Flac), ("song.mp3".into(), Format::Mp3)]);
    }

    #[test]
    fn reconcile_stores_the_format() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("x.mp3"), b"fLaC\x00\x00\x00\x22").unwrap();
        let db = Db::open_in_memory().unwrap();
        let f = add_folder(&db, &d.path().to_string_lossy(), 1).unwrap();
        rescan_folder(&db, f.id, &f.path, 1).unwrap();
        let fmt: String = db.conn().query_row("SELECT format FROM songs", [], |r| r.get(0)).unwrap();
        assert_eq!(fmt, "flac");
    }
}

#[cfg(test)]
mod backfill_tests {
    use super::*;
    use crate::library::folders::add_folder;

    #[test]
    fn songs_stored_before_format_detection_get_their_format_on_the_next_rescan() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.mp3"), b"fLaC\x00\x00\x00\x22").unwrap();
        let db = Db::open_in_memory().unwrap();
        let f = add_folder(&db, &d.path().to_string_lossy(), 1).unwrap();
        rescan_folder(&db, f.id, &f.path, 1).unwrap();
        db.conn().execute("UPDATE songs SET format = 'unknown'", []).unwrap(); // as stored by F3.1
        assert_eq!(rescan_folder(&db, f.id, &f.path, 2).unwrap().updated, 1);
        let fmt: String = db.conn().query_row("SELECT format FROM songs", [], |r| r.get(0)).unwrap();
        assert_eq!(fmt, "flac");
    }
}

#[cfg(test)]
mod tag_tests {
    use super::*;
    use crate::library::folders::add_folder;
    use crate::library::tags::fixtures::{silent_mp3, write_tags, Fields};
    use crate::library::tags::TrackTags;
    use lofty::tag::TagType;
    use std::cell::Cell;

    fn fields() -> Fields<'static> {
        Fields { title: "Diễm xưa", artist: "Khánh Ly", album: "Trịnh Công Sơn", album_artist: "Khánh Ly", genre: "Nhạc Trịnh",
                 year: "1970", track: 2, track_total: 10, disc: 1, cover: true }
    }

    fn row(db: &Db) -> (Option<String>, Option<String>, Option<i64>, Option<i64>, i64) {
        db.conn()
            .query_row("SELECT title, artist, year, track_no, has_artwork FROM songs", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
            .unwrap()
    }

    #[test]
    fn new_songs_get_their_tags_stored() {
        let d = tempfile::tempdir().unwrap();
        write_tags(&silent_mp3(d.path(), "a.mp3"), TagType::Id3v2, &fields());
        let db = Db::open_in_memory().unwrap();
        let f = add_folder(&db, &d.path().to_string_lossy(), 1).unwrap();
        rescan_folder(&db, f.id, &f.path, 1).unwrap();
        assert_eq!(row(&db), (Some("Diễm xưa".into()), Some("Khánh Ly".into()), Some(1970), Some(2), 1));
    }

    #[test]
    fn unchanged_files_are_not_read_again() {
        let d = tempfile::tempdir().unwrap();
        silent_mp3(d.path(), "a.mp3");
        let db = Db::open_in_memory().unwrap();
        let f = add_folder(&db, &d.path().to_string_lossy(), 1).unwrap();
        let reads = Cell::new(0);
        let reader = |_: &Path| { reads.set(reads.get() + 1); TrackTags::default() };
        reconcile_with(&db, f.id, &scan_folder(d.path()), 1, reader).unwrap();
        reconcile_with(&db, f.id, &scan_folder(d.path()), 2, reader).unwrap();
        assert_eq!(reads.get(), 1);
    }

    #[test]
    fn songs_scanned_before_tag_reading_get_tags_on_the_next_rescan() {
        let d = tempfile::tempdir().unwrap();
        write_tags(&silent_mp3(d.path(), "a.mp3"), TagType::Id3v2, &fields());
        let db = Db::open_in_memory().unwrap();
        let f = add_folder(&db, &d.path().to_string_lossy(), 1).unwrap();
        rescan_folder(&db, f.id, &f.path, 1).unwrap();
        db.conn().execute("UPDATE songs SET title = NULL, artist = NULL, tags_read = 0", []).unwrap(); // as stored by F3.1/F3.2
        assert_eq!(rescan_folder(&db, f.id, &f.path, 2).unwrap().updated, 1);
        assert_eq!(row(&db).0.as_deref(), Some("Diễm xưa"));
    }
}

#[cfg(test)]
mod bench {
    use super::*;
    use crate::library::folders::add_folder;
    use crate::library::tags::fixtures::{silent_mp3, write_tags, Fields};
    use lofty::tag::TagType;

    /// Plan target: a 500-file folder scanned (with tags) in under 10 s. Run with --include-ignored.
    #[test]
    #[ignore]
    fn scans_500_tagged_files_quickly() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..500 {
            let p = silent_mp3(d.path(), &format!("Bài hát {i:03}.mp3"));
            write_tags(&p, TagType::Id3v2, &Fields { title: "Mùa thu", artist: "Ngọc Lan", album: "Album", album_artist: "Ngọc Lan",
                genre: "Pop", year: "1990", track: 1, track_total: 1, disc: 1, cover: false });
        }
        let db = Db::open_in_memory().unwrap();
        let f = add_folder(&db, &d.path().to_string_lossy(), 1).unwrap();
        let t = std::time::Instant::now();
        let diff = rescan_folder(&db, f.id, &f.path, 1).unwrap();
        let first = t.elapsed();
        let t = std::time::Instant::now();
        rescan_folder(&db, f.id, &f.path, 2).unwrap();
        println!("first scan of {} files: {first:?}; unchanged rescan: {:?}", diff.added, t.elapsed());
        assert_eq!(diff.added, 500);
        assert!(first.as_secs_f64() < 10.0);
    }
}

#[cfg(test)]
mod quality_tests {
    use super::*;
    use crate::library::folders::add_folder;
    use crate::library::songs::list_songs;
    use crate::library::tags::fixtures::{silent_flac, silent_mp3};

    #[test]
    fn songs_carry_sample_rate_bit_depth_and_bitrate() {
        let d = tempfile::tempdir().unwrap();
        silent_mp3(d.path(), "a.mp3");
        silent_flac(d.path(), "b.flac");
        let db = Db::open_in_memory().unwrap();
        let f = add_folder(&db, &d.path().to_string_lossy(), 1).unwrap();
        rescan_folder(&db, f.id, &f.path, 1).unwrap();
        let songs = list_songs(&db).unwrap();
        let mp3 = songs.iter().find(|s| s.format == "mp3").unwrap();
        let flac = songs.iter().find(|s| s.format == "flac").unwrap();
        assert_eq!((mp3.sample_rate, mp3.bit_depth, mp3.bitrate_kbps), (Some(44_100), None, Some(128)));
        assert_eq!((flac.sample_rate, flac.bit_depth), (Some(44_100), Some(16)));
    }
}
