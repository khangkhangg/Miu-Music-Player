//! Metadata editing (F9): changes to a song's tags, written into the file itself. One change type
//! serves the single-song editor and the bulk editor, where fields shown as "Mixed" stay `Keep`.

use std::path::Path;

use lofty::config::WriteOptions;
use lofty::file::TaggedFileExt;
use lofty::tag::{Accessor, ItemKey, Tag, TagExt};
use serde::{Deserialize, Serialize};

/// What to do with one field. JSON: `"keep"`, `"clear"` or `{"set": value}`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Change<T> {
    #[default]
    Keep,
    Set(T),
    Clear,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TagChanges {
    pub title: Change<String>,
    pub artist: Change<String>,
    pub album: Change<String>,
    pub album_artist: Change<String>,
    pub genre: Change<String>,
    pub year: Change<u32>,
    pub track: Change<u32>,
    pub track_total: Change<u32>,
    pub disc: Change<u32>,
}

impl TagChanges {
    pub fn is_empty(&self) -> bool {
        *self == TagChanges::default()
    }
}

const YEAR_KEYS: [ItemKey; 3] = [ItemKey::Year, ItemKey::RecordingDate, ItemKey::OriginalReleaseDate];

/// Apply `changes` to an in-memory tag.
pub fn apply(tag: &mut Tag, c: &TagChanges) {
    fn text(tag: &mut Tag, change: &Change<String>, set: fn(&mut Tag, String), remove: fn(&mut Tag)) {
        match change {
            Change::Keep => {}
            Change::Set(v) if !v.trim().is_empty() => set(tag, v.trim().to_string()),
            Change::Set(_) | Change::Clear => remove(tag),
        }
    }
    fn number(tag: &mut Tag, change: &Change<u32>, set: fn(&mut Tag, u32), remove: fn(&mut Tag)) {
        match change {
            Change::Keep => {}
            Change::Set(v) => set(tag, *v),
            Change::Clear => remove(tag),
        }
    }
    text(tag, &c.title, |t, v| t.set_title(v), |t| t.remove_title());
    text(tag, &c.artist, |t, v| t.set_artist(v), |t| t.remove_artist());
    text(tag, &c.album, |t, v| t.set_album(v), |t| t.remove_album());
    text(tag, &c.genre, |t, v| t.set_genre(v), |t| t.remove_genre());
    text(
        tag,
        &c.album_artist,
        |t, v| {
            t.insert_text(ItemKey::AlbumArtist, v);
        },
        |t| t.remove_key(&ItemKey::AlbumArtist),
    );
    number(
        tag,
        &c.year,
        |t, y| {
            for k in &YEAR_KEYS {
                t.remove_key(k);
            }
            t.insert_text(ItemKey::RecordingDate, y.to_string());
        },
        |t| {
            for k in &YEAR_KEYS {
                t.remove_key(k);
            }
        },
    );
    number(tag, &c.track, |t, v| t.set_track(v), |t| t.remove_track());
    number(tag, &c.track_total, |t, v| t.set_track_total(v), |t| t.remove_track_total());
    number(tag, &c.disc, |t, v| t.set_disk(v), |t| t.remove_disk());
}

/// Write `changes` into the file's main tag (created if the file has none).
pub fn write_tags(path: &Path, changes: &TagChanges) -> Result<(), String> {
    let mut file = lofty::read_from_path(path).map_err(|e| e.to_string())?;
    let kind = file.primary_tag_type();
    if file.tag(kind).is_none() {
        file.insert_tag(Tag::new(kind));
    }
    let tag = file.tag_mut(kind).ok_or("no tag")?;
    apply(tag, changes);
    tag.save_to_path(path, WriteOptions::default()).map_err(|e| e.to_string())
}

pub const TEMP_MARK: &str = ".miu-tmp.";

/// Run `write` on a copy of `path` next to it, then swap the copy in. If anything fails, the
/// original is untouched and the copy removed (F9.4).
pub fn write_atomically(path: &Path, write: impl FnOnce(&Path) -> Result<(), String>) -> Result<(), String> {
    // Same extension, so tag writers recognise the format; scans skip the `.miu-tmp.` marker.
    let stem = path.file_stem().ok_or("not a file")?.to_string_lossy();
    let ext = path.extension().map(|e| e.to_string_lossy()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{stem}{TEMP_MARK}{ext}"));
    let result = std::fs::copy(path, &tmp)
        .map_err(|e| e.to_string())
        .and_then(|_| write(&tmp))
        .and_then(|_| std::fs::rename(&tmp, path).map_err(|e| e.to_string()));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// The changes that would put a file's tags back exactly as they are now (for undo).
pub fn snapshot(path: &Path) -> Result<TagChanges, String> {
    let t = crate::library::tags::read_tags(path)?;
    fn c<T>(v: Option<T>) -> Change<T> {
        v.map_or(Change::Clear, Change::Set)
    }
    Ok(TagChanges {
        title: c(t.title),
        artist: c(t.artist),
        album: c(t.album),
        album_artist: c(t.album_artist),
        genre: c(t.genre),
        year: c(t.year),
        track: c(t.track),
        track_total: c(t.track_total),
        disc: c(t.disc),
    })
}

/// Changes for one song.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Edit {
    pub id: i64,
    pub changes: TagChanges,
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditReport {
    pub saved: usize,
    /// Edits that put back the old tags of every saved song.
    pub undo: Vec<Edit>,
    /// Song id and why it could not be saved.
    pub failed: Vec<(i64, String)>,
}

/// Write each edit into its file (atomically) and update the library row. Failures are collected,
/// not fatal: the other songs are still saved.
pub fn edit_songs(db: &crate::library::db::Db, edits: &[Edit]) -> Result<EditReport, String> {
    let ids: Vec<i64> = edits.iter().map(|e| e.id).collect();
    let paths: std::collections::HashMap<i64, std::path::PathBuf> =
        crate::library::songs::paths_for(db, &ids).map_err(|e| e.to_string())?.into_iter().collect();
    let mut report = EditReport::default();
    for e in edits.iter().filter(|e| !e.changes.is_empty()) {
        let Some(path) = paths.get(&e.id) else {
            report.failed.push((e.id, "not in the library".into()));
            continue;
        };
        let result = snapshot(path).and_then(|old| {
            write_atomically(path, |tmp| write_tags(tmp, &e.changes))?;
            crate::library::scan::refresh_song(db, e.id, path).map_err(|err| err.to_string())?;
            Ok(old)
        });
        match result {
            Ok(old) => {
                report.saved += 1;
                report.undo.push(Edit { id: e.id, changes: old });
            }
            Err(err) => report.failed.push((e.id, err)),
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::tags::fixtures::{silent_flac, silent_m4a, silent_mp3};
    use crate::library::tags::read_tags;
    use std::path::{Path, PathBuf};

    fn everything() -> TagChanges {
        TagChanges {
            title: Change::Set("Diễm xưa".into()),
            artist: Change::Set("Khánh Ly".into()),
            album: Change::Set("Trịnh Công Sơn".into()),
            album_artist: Change::Set("Khánh Ly".into()),
            genre: Change::Set("Nhạc Trịnh".into()),
            year: Change::Set(1970),
            track: Change::Set(2),
            track_total: Change::Set(12),
            disc: Change::Set(1),
        }
    }

    fn files(d: &Path) -> Vec<PathBuf> {
        vec![silent_mp3(d, "a.mp3"), silent_m4a(d, "b.m4a"), silent_flac(d, "c.flac")]
    }

    #[test]
    fn every_field_written_reads_back_the_same_for_mp3_m4a_and_flac() {
        let d = tempfile::tempdir().unwrap();
        for p in files(d.path()) {
            write_tags(&p, &everything()).unwrap();
            let t = read_tags(&p).unwrap();
            let got = (t.title.as_deref(), t.artist.as_deref(), t.album.as_deref(), t.album_artist.as_deref(), t.genre.as_deref());
            assert_eq!(got, (Some("Diễm xưa"), Some("Khánh Ly"), Some("Trịnh Công Sơn"), Some("Khánh Ly"), Some("Nhạc Trịnh")), "{}", p.display());
            assert_eq!((t.year, t.track, t.track_total, t.disc), (Some(1970), Some(2), Some(12), Some(1)), "{}", p.display());
        }
    }

    #[test]
    fn kept_fields_are_untouched_and_cleared_ones_go() {
        let d = tempfile::tempdir().unwrap();
        for p in files(d.path()) {
            write_tags(&p, &everything()).unwrap();
            let change = TagChanges { title: Change::Set("Hạ trắng".into()), genre: Change::Clear, year: Change::Clear, ..TagChanges::default() };
            write_tags(&p, &change).unwrap();
            let t = read_tags(&p).unwrap();
            assert_eq!(t.title.as_deref(), Some("Hạ trắng"), "{}", p.display());
            assert_eq!(t.artist.as_deref(), Some("Khánh Ly"), "kept: {}", p.display());
            assert_eq!(t.track, Some(2), "kept: {}", p.display());
            assert_eq!((t.genre, t.year), (None, None), "cleared: {}", p.display());
        }
    }

    #[test]
    fn writing_never_touches_the_audio() {
        let d = tempfile::tempdir().unwrap();
        let p = silent_mp3(d.path(), "a.mp3");
        let before = crate::library::fingerprint::audio_hash(&p).ok();
        write_tags(&p, &everything()).unwrap();
        assert_eq!(crate::library::fingerprint::audio_hash(&p).ok(), before);
    }

    #[test]
    fn changes_arrive_from_the_ui_as_json() {
        let c: TagChanges = serde_json::from_str(r#"{"title":{"set":"A"},"genre":"clear"}"#).unwrap();
        assert_eq!((c.title, c.genre, c.artist), (Change::Set("A".into()), Change::Clear, Change::Keep));
    }
}

#[cfg(test)]
mod safety_tests {
    use super::*;
    use crate::library::db::Db;
    use crate::library::folders::add_folder;
    use crate::library::scan::rescan_folder;
    use crate::library::songs::list_songs;
    use crate::library::tags::fixtures::silent_mp3;
    use crate::library::tags::read_tags;

    fn set_title(t: &str) -> TagChanges {
        TagChanges { title: Change::Set(t.into()), ..TagChanges::default() }
    }

    #[test]
    fn a_failure_mid_write_leaves_the_original_intact_and_no_temp_file() {
        let d = tempfile::tempdir().unwrap();
        let p = silent_mp3(d.path(), "a.mp3");
        write_tags(&p, &set_title("Original")).unwrap();
        let before = std::fs::read(&p).unwrap();
        let crash = |tmp: &Path| -> Result<(), String> {
            std::fs::write(tmp, b"half written").unwrap();
            Err("power cut".into())
        };
        assert!(write_atomically(&p, crash).is_err());
        assert_eq!(std::fs::read(&p).unwrap(), before, "original untouched");
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1, "temp file cleaned up");
    }

    #[test]
    fn a_successful_write_replaces_the_file() {
        let d = tempfile::tempdir().unwrap();
        let p = silent_mp3(d.path(), "a.mp3");
        write_atomically(&p, |tmp| write_tags(tmp, &set_title("Hạ trắng"))).unwrap();
        assert_eq!(read_tags(&p).unwrap().title.as_deref(), Some("Hạ trắng"));
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 1);
    }

    fn library(d: &Path) -> (Db, i64) {
        silent_mp3(d, "a.mp3");
        let db = Db::open_in_memory().unwrap();
        let f = add_folder(&db, &d.to_string_lossy(), 1).unwrap();
        rescan_folder(&db, f.id, &f.path, 1).unwrap();
        let id = list_songs(&db).unwrap()[0].id;
        (db, id)
    }

    #[test]
    fn editing_updates_the_library_and_undo_restores_the_old_tags() {
        let d = tempfile::tempdir().unwrap();
        let (db, id) = library(d.path());
        edit_songs(&db, &[Edit { id, changes: set_title("Before") }]).unwrap();
        let report = edit_songs(&db, &[Edit { id, changes: TagChanges { title: Change::Set("After".into()), year: Change::Set(1999), ..TagChanges::default() } }]).unwrap();
        assert!(report.failed.is_empty());
        assert_eq!(list_songs(&db).unwrap()[0].title, "After");
        edit_songs(&db, &report.undo).unwrap();
        let t = read_tags(&d.path().join("a.mp3")).unwrap();
        assert_eq!((t.title.as_deref(), t.year), (Some("Before"), None), "undo restores and clears what was added");
        assert_eq!(list_songs(&db).unwrap()[0].title, "Before");
    }

    #[test]
    fn our_own_writes_do_not_look_like_changes_to_the_next_rescan() {
        let d = tempfile::tempdir().unwrap();
        let (db, id) = library(d.path());
        crate::library::duplicates::fill_audio_hashes(&db, 10).unwrap();
        edit_songs(&db, &[Edit { id, changes: set_title("Diễm xưa") }]).unwrap();
        let f = crate::library::folders::list_folders(&db).unwrap().remove(0);
        assert_eq!(rescan_folder(&db, f.id, &f.path, 2).unwrap().updated, 0, "no rescan work after our own write");
        assert_eq!(crate::library::duplicates::fill_audio_hashes(&db, 10).unwrap(), 0, "audio unchanged: hash kept");
    }

    #[test]
    fn a_missing_file_is_reported_and_others_still_saved() {
        let d = tempfile::tempdir().unwrap();
        let (db, id) = library(d.path());
        let report = edit_songs(&db, &[Edit { id: 999, changes: set_title("x") }, Edit { id, changes: set_title("ok") }]).unwrap();
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].0, 999);
        assert_eq!(list_songs(&db).unwrap()[0].title, "ok");
    }
}
