//! Duplicate detection (F3.6): songs with the same audio fingerprint and (nearly) the same length,
//! whatever their names or tags. Hashes are computed after scanning and cleared when a file changes.

use std::path::PathBuf;

use crate::library::db::Db;
use crate::library::fingerprint::audio_hash;

/// Durations of the same audio can differ slightly between containers and encoders.
const SAME_LENGTH_MS: i64 = 1_000;

/// Up to `limit` songs that still need a fingerprint.
pub fn pending_hashes(db: &Db, limit: usize) -> rusqlite::Result<Vec<(i64, PathBuf)>> {
    let mut stmt = db.conn().prepare("SELECT id, path FROM songs WHERE audio_hash IS NULL ORDER BY id LIMIT ?1")?;
    let rows = stmt.query_map([limit as i64], |r| Ok((r.get(0)?, PathBuf::from(r.get::<_, String>(1)?))))?;
    rows.collect()
}

/// Store a fingerprint; `None` marks the file unreadable so it is not tried again until it changes.
pub fn store_hash(db: &Db, id: i64, hash: Option<&str>) -> rusqlite::Result<()> {
    db.conn().execute("UPDATE songs SET audio_hash = ?1 WHERE id = ?2", rusqlite::params![hash.unwrap_or(""), id])?;
    Ok(())
}

/// Fingerprint up to `limit` songs; returns how many were done. (The app hashes outside the
/// database lock using `pending_hashes` / `store_hash`.)
pub fn fill_audio_hashes(db: &Db, limit: usize) -> rusqlite::Result<usize> {
    let todo = pending_hashes(db, limit)?;
    for (id, path) in &todo {
        store_hash(db, *id, audio_hash(path).ok().as_deref())?;
    }
    Ok(todo.len())
}

/// Groups of song ids with the same audio (each sorted, at least two songs), by lowest id.
pub fn find_duplicates(db: &Db) -> rusqlite::Result<Vec<Vec<i64>>> {
    let mut stmt = db.conn().prepare(
        "SELECT id, audio_hash, COALESCE(duration_ms, -1) FROM songs
         WHERE audio_hash IS NOT NULL AND audio_hash != '' ORDER BY audio_hash, 3, id",
    )?;
    let rows: Vec<(i64, String, i64)> = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
    let mut groups: Vec<Vec<i64>> = Vec::new();
    let mut current: Vec<i64> = Vec::new();
    let mut key: Option<(&str, i64)> = None; // hash and the first duration of the current group
    for (id, hash, ms) in &rows {
        match key {
            Some((h, first)) if h == hash && ms - first <= SAME_LENGTH_MS => current.push(*id),
            _ => {
                if current.len() > 1 {
                    groups.push(std::mem::take(&mut current));
                }
                current = vec![*id];
                key = Some((hash, *ms));
            }
        }
    }
    if current.len() > 1 {
        groups.push(current);
    }
    for g in &mut groups {
        g.sort();
    }
    groups.sort();
    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::fixtures::{wav, wav_with};
    use crate::library::db::Db;
    use crate::library::folders::add_folder;
    use crate::library::scan::rescan_folder;
    use crate::library::tags::fixtures::{write_tags, Fields};
    use lofty::tag::TagType;

    fn tags(title: &str) -> Fields<'_> {
        Fields { title, artist: "Khánh Ly", album: "", album_artist: "", genre: "", year: "1970", track: 1, track_total: 1, disc: 1, cover: false }
    }

    fn id_of(db: &Db, name: &str) -> i64 {
        db.conn().query_row("SELECT id FROM songs WHERE path LIKE ?1", [format!("%{name}")], |r| r.get(0)).unwrap()
    }

    fn library(d: &std::path::Path) -> Db {
        let db = Db::open_in_memory().unwrap();
        let f = add_folder(&db, &d.to_string_lossy(), 1).unwrap();
        rescan_folder(&db, f.id, &f.path, 1).unwrap();
        db
    }

    #[test]
    fn same_audio_under_different_names_is_a_duplicate_and_same_tags_alone_is_not() {
        let d = tempfile::tempdir().unwrap();
        let a = wav(d.path(), "Diễm xưa.wav", 44_100, 2, 16, 44_100);
        std::fs::copy(&a, d.path().join("Track 01.wav")).unwrap();
        let other = wav_with(d.path(), "Other.wav", 44_100, 2, 16, 44_100, |i, _| ((i as f32) * 0.05).sin() * 0.3);
        write_tags(&a, TagType::RiffInfo, &tags("Diễm xưa"));
        write_tags(&other, TagType::RiffInfo, &tags("Diễm xưa"));
        let db = library(d.path());
        assert_eq!(fill_audio_hashes(&db, 100).unwrap(), 3);
        let mut want = vec![id_of(&db, "Diễm xưa.wav"), id_of(&db, "Track 01.wav")];
        want.sort();
        assert_eq!(find_duplicates(&db).unwrap(), vec![want]);
    }

    #[test]
    fn same_opening_but_a_different_length_is_not_a_duplicate() {
        let d = tempfile::tempdir().unwrap();
        wav(d.path(), "short.wav", 8_000, 1, 16, 8_000 * 2);
        wav(d.path(), "long.wav", 8_000, 1, 16, 8_000 * 5);
        let db = library(d.path());
        fill_audio_hashes(&db, 100).unwrap();
        assert!(find_duplicates(&db).unwrap().is_empty());
    }

    #[test]
    fn hashes_are_computed_once_and_redone_when_the_file_changes() {
        let d = tempfile::tempdir().unwrap();
        let a = wav(d.path(), "a.wav", 8_000, 1, 16, 800);
        let db = library(d.path());
        assert_eq!(fill_audio_hashes(&db, 100).unwrap(), 1);
        assert_eq!(fill_audio_hashes(&db, 100).unwrap(), 0, "nothing left to hash");
        wav(d.path(), "a.wav", 8_000, 1, 16, 1_600); // rewritten: new size
        let f = crate::library::folders::list_folders(&db).unwrap().remove(0);
        rescan_folder(&db, f.id, &f.path, 2).unwrap();
        assert_eq!(fill_audio_hashes(&db, 100).unwrap(), 1, "changed file hashed again");
        drop(a);
    }

    #[test]
    fn unreadable_files_are_not_retried_forever() {
        let d = tempfile::tempdir().unwrap();
        let p = wav(d.path(), "a.wav", 8_000, 1, 16, 800);
        let db = library(d.path());
        std::fs::write(&p, b"RIFF....WAVE broken").unwrap(); // broken after the scan
        assert_eq!(fill_audio_hashes(&db, 100).unwrap(), 1, "tried once");
        assert_eq!(fill_audio_hashes(&db, 100).unwrap(), 0);
        assert!(find_duplicates(&db).unwrap().is_empty());
    }

    #[test]
    fn work_is_done_in_batches() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..5 {
            wav(d.path(), &format!("{i}.wav"), 8_000, 1, 16, 80 + i);
        }
        let db = library(d.path());
        assert_eq!(fill_audio_hashes(&db, 2).unwrap(), 2);
        assert_eq!(fill_audio_hashes(&db, 100).unwrap(), 3);
    }

    #[test]
    fn the_song_list_marks_each_duplicate_with_its_group() {
        let d = tempfile::tempdir().unwrap();
        let a = wav(d.path(), "a.wav", 8_000, 1, 16, 800);
        std::fs::copy(&a, d.path().join("b.wav")).unwrap();
        wav_with(d.path(), "c.wav", 8_000, 1, 16, 800, |i, _| (i as f32 * 0.1).sin() * 0.2);
        let db = library(d.path());
        fill_audio_hashes(&db, 100).unwrap();
        let songs = crate::library::songs::list_songs(&db).unwrap();
        let group = |name: &str| songs.iter().find(|s| s.path.ends_with(name)).unwrap().duplicate_group;
        let first = id_of(&db, "a.wav").min(id_of(&db, "b.wav"));
        assert_eq!((group("a.wav"), group("b.wav"), group("c.wav")), (Some(first), Some(first), None));
    }
}
