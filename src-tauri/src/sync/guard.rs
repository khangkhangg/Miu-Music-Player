//! Safeguards around an iTunes sync (Phase 4b). On 2026-10-20 an iTunes sync left a 3uTools song out of
//! its track list and the phone deleted it (docs/AIRTRAFFIC_NOTES.md). So around every sync:
//! 1. copy each phone song that has no copy on this PC off the phone (read-only), so it can be put back;
//! 2. before syncing, check that iTunes' view of the phone holds every phone song;
//! 3. after syncing, diff the phone's songs and report anything that disappeared.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;

use crate::device::files::DeviceFiles;

/// A music track in the phone's MediaLibrary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneTrack {
    pub pid: i64,
    /// iTunes' persistent ID for the track.
    pub sync_id: i64,
    pub title: String,
    /// On the phone's media partition, e.g. `/iTunes_Control/Music/F23/CWXN.mp3`.
    pub path: String,
    pub size: u64,
    pub duration_ms: Option<i64>,
}

/// The music tracks in a backed-up MediaLibrary.sqlitedb (read from a copy; the backup is untouched).
pub fn phone_tracks(backup_dir: &Path) -> Result<Vec<PhoneTrack>, String> {
    let work = tempfile::tempdir().map_err(|e| e.to_string())?;
    let conn = crate::sync::dbdiff::snapshot(&backup_dir.join(crate::sync::backup::DB_NAME), work.path())?;
    let mut stmt = conn
        .prepare(
            "SELECT i.item_pid, s.sync_id, e.title, b.path, e.location, e.file_size, e.total_time_ms
             FROM item i JOIN item_extra e ON e.item_pid = i.item_pid
             JOIN item_store s ON s.item_pid = i.item_pid
             LEFT JOIN base_location b ON b.base_location_id = i.base_location_id
             WHERE i.media_type = 8",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| {
            let dir: Option<String> = r.get(3)?;
            let file: Option<String> = r.get(4)?;
            Ok(PhoneTrack {
                pid: r.get(0)?,
                sync_id: r.get(1)?,
                title: r.get::<_, Option<String>>(2)?.unwrap_or_default(),
                path: format!("/{}/{}", dir.unwrap_or_default(), file.unwrap_or_default()),
                size: r.get::<_, Option<i64>>(5)?.unwrap_or(0).max(0) as u64,
                duration_ms: r.get::<_, Option<f64>>(6)?.map(|ms| ms.round() as i64),
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<_>>().map_err(|e| e.to_string())
}

/// A song in Miu's library, as far as matching a phone file needs.
#[derive(Debug, Clone)]
pub struct PcSong {
    pub title: String,
    pub size: u64,
    pub duration_ms: Option<i64>,
}

/// Phone songs with no copy on this PC: no library file of the same size, and none with the same
/// title and a length within 2 s (a converted copy).
pub fn phone_only<'a>(tracks: &'a [PhoneTrack], pc: &[PcSong]) -> Vec<&'a PhoneTrack> {
    let fold = |t: &str| t.nfc().collect::<String>().to_lowercase();
    let sizes: HashSet<u64> = pc.iter().map(|s| s.size).collect();
    tracks
        .iter()
        .filter(|t| !sizes.contains(&t.size))
        .filter(|t| {
            let title = fold(&t.title);
            !pc.iter().any(|s| {
                fold(&s.title) == title && matches!((s.duration_ms, t.duration_ms), (Some(a), Some(b)) if (a - b).abs() <= 2_000)
            })
        })
        .collect()
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct CopyReport {
    pub copied: usize,
    pub already_there: usize,
    pub failed: Vec<(String, String)>,
}

/// Copy `tracks` off the phone into `dir` (`<sync_id hex>.<ext>`, plus `index.json` with titles).
/// Files already copied with the right size are skipped. Only reads from the phone.
pub fn copy_off(phone: &dyn DeviceFiles, tracks: &[&PhoneTrack], dir: &Path) -> Result<CopyReport, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let index_path = dir.join("index.json");
    let mut index: BTreeMap<String, PhoneTrack> =
        std::fs::read(&index_path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let mut report = CopyReport::default();
    for t in tracks {
        let name = copy_name(t);
        let target = dir.join(&name);
        if std::fs::metadata(&target).is_ok_and(|m| m.len() == t.size) {
            report.already_there += 1;
            index.insert(name, (*t).clone());
            continue;
        }
        match phone.read(&t.path) {
            Ok(data) => {
                let tmp = dir.join(format!("{name}.partial"));
                std::fs::write(&tmp, &data).and_then(|_| std::fs::rename(&tmp, &target)).map_err(|e| e.to_string())?;
                index.insert(name, (*t).clone());
                report.copied += 1;
            }
            Err(e) => report.failed.push((t.title.clone(), e)),
        }
    }
    let json = serde_json::to_vec_pretty(&index).map_err(|e| e.to_string())?;
    std::fs::write(&index_path, json).map_err(|e| e.to_string())?;
    Ok(report)
}

pub fn copy_name(track: &PhoneTrack) -> String {
    let ext = Path::new(&track.path).extension().and_then(|e| e.to_str()).unwrap_or("bin");
    format!("{:016X}.{ext}", track.sync_id as u64)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Concern {
    /// iTunes doesn't list this phone song, so a sync would delete it.
    MissingFromItunes(PhoneTrack),
    /// The only trait of the song iTunes dropped on 2026-10-20: a sync_id whose top byte is zero.
    LeadingZeroId(PhoneTrack),
}

/// Reasons not to start an iTunes sync. `itunes_ids`: persistent IDs of iTunes' view of the phone.
pub fn precheck(tracks: &[PhoneTrack], itunes_ids: &HashSet<i64>) -> Vec<Concern> {
    let mut out = Vec::new();
    for t in tracks {
        if (t.sync_id as u64) >> 56 == 0 {
            out.push(Concern::LeadingZeroId(t.clone()));
        }
        if !itunes_ids.contains(&t.sync_id) {
            out.push(Concern::MissingFromItunes(t.clone()));
        }
    }
    out
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SyncDiff {
    pub added: Vec<PhoneTrack>,
    pub removed: Vec<PhoneTrack>,
}

/// What a sync changed on the phone, by track pid.
pub fn diff(before: &[PhoneTrack], after: &[PhoneTrack]) -> SyncDiff {
    let was: HashSet<i64> = before.iter().map(|t| t.pid).collect();
    let is: HashSet<i64> = after.iter().map(|t| t.pid).collect();
    SyncDiff {
        added: after.iter().filter(|t| !was.contains(&t.pid)).cloned().collect(),
        removed: before.iter().filter(|t| !is.contains(&t.pid)).cloned().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    fn track(pid: i64, sync_id: i64, title: &str, file: &str, size: u64, ms: i64) -> PhoneTrack {
        PhoneTrack { pid, sync_id, title: title.into(), path: format!("/iTunes_Control/Music/F23/{file}"), size, duration_ms: Some(ms) }
    }

    /// A backup folder holding a MediaLibrary.sqlitedb with just the columns the guard reads.
    fn backup_with(rows: &[(i64, i64, i64, &str, &str, u64, f64)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let c = rusqlite::Connection::open(dir.path().join("MediaLibrary.sqlitedb")).unwrap();
        c.execute_batch(
            "CREATE TABLE item (item_pid INTEGER PRIMARY KEY, media_type INTEGER, base_location_id INTEGER);
             CREATE TABLE item_extra (item_pid INTEGER PRIMARY KEY, title TEXT, location TEXT, file_size INTEGER, total_time_ms REAL);
             CREATE TABLE item_store (item_pid INTEGER PRIMARY KEY, sync_id INTEGER);
             CREATE TABLE base_location (base_location_id INTEGER PRIMARY KEY, path TEXT NOT NULL);
             INSERT INTO base_location VALUES (0, ''), (3863, 'iTunes_Control/Music/F23'), (3845, 'iTunes_Control/Music/F05');",
        )
        .unwrap();
        for (pid, media_type, sync_id, title, location, size, ms) in rows {
            let loc = if location.starts_with("KH") { 3845 } else { 3863 };
            c.execute("INSERT INTO item VALUES (?1, ?2, ?3)", rusqlite::params![pid, media_type, loc]).unwrap();
            c.execute("INSERT INTO item_extra VALUES (?1, ?2, ?3, ?4, ?5)", rusqlite::params![pid, title, location, *size as i64, ms]).unwrap();
            c.execute("INSERT INTO item_store VALUES (?1, ?2)", rusqlite::params![pid, sync_id]).unwrap();
        }
        dir
    }

    #[test]
    fn reads_music_tracks_with_their_phone_paths_and_ids() {
        let b = backup_with(&[
            (8519413647570239665, 8, 43170718095898799, "Áo Mới Cà Mau", "CWXN.mp3", 9_296_034, 228_884.0),
            (2099228584612918881, 8, 3464385541651505982, "Ai Rồi Cũng Sẽ Khác", "KHRQ.mp3", 11_016_652, 250_000.0),
            (77, 2, 5, "a podcast", "PODC.mp3", 10, 1.0),
        ]);
        let mut tracks = phone_tracks(b.path()).unwrap();
        tracks.sort_by_key(|t| t.pid);
        assert_eq!(
            tracks,
            vec![
                track(2099228584612918881, 3464385541651505982, "Ai Rồi Cũng Sẽ Khác", "KHRQ.mp3", 11_016_652, 250_000)
                    .with_dir("F05"),
                track(8519413647570239665, 43170718095898799, "Áo Mới Cà Mau", "CWXN.mp3", 9_296_034, 228_884),
            ]
        );
    }

    impl PhoneTrack {
        fn with_dir(mut self, d: &str) -> Self {
            self.path = self.path.replace("F23", d);
            self
        }
    }

    #[test]
    fn a_phone_song_counts_as_on_the_pc_by_size_or_by_title_and_length() {
        let tracks = vec![
            track(1, 11, "Áo Mới Cà Mau", "CWXN.mp3", 9_296_034, 228_884),
            track(2, 12, "Ai Rồi Cũng Sẽ Khác", "KHRQ.mp3", 11_016_652, 250_000),
            track(3, 13, "Diễm Xưa", "DXUA.m4a", 5_000_000, 303_000),
            track(4, 14, "Hạ Trắng", "HTRG.mp3", 4_000_000, 200_000),
        ];
        let pc = vec![
            PcSong { title: "anything".into(), size: 11_016_652, duration_ms: Some(1) },
            PcSong { title: "diễm xưa".into(), size: 30_000_000, duration_ms: Some(304_500) },
            PcSong { title: "Hạ Trắng".into(), size: 9, duration_ms: Some(260_000) },
        ];
        let only: Vec<i64> = phone_only(&tracks, &pc).iter().map(|t| t.pid).collect();
        assert_eq!(only, vec![1, 4], "same title but a different length is another recording");
    }

    #[derive(Default)]
    struct FakePhone {
        files: BTreeMap<String, Vec<u8>>,
        writes: RefCell<usize>,
        reads: RefCell<usize>,
    }

    impl DeviceFiles for FakePhone {
        fn list(&self, _dir: &str) -> Result<Vec<String>, String> {
            Ok(Vec::new())
        }
        fn read(&self, path: &str) -> Result<Vec<u8>, String> {
            *self.reads.borrow_mut() += 1;
            self.files.get(path).cloned().ok_or_else(|| format!("{path} not found"))
        }
        fn write(&self, _: &str, _: &[u8]) -> Result<(), String> {
            *self.writes.borrow_mut() += 1;
            Ok(())
        }
        fn remove(&self, _: &str) -> Result<(), String> {
            *self.writes.borrow_mut() += 1;
            Ok(())
        }
    }

    #[test]
    fn copies_phone_only_songs_off_the_phone_without_writing_to_it() {
        let a = track(1, 43170718095898799, "Áo Mới Cà Mau", "CWXN.mp3", 3, 1);
        let gone = track(2, -941274682976370203, "Bông Cỏ Mây", "LFGG.mp3", 2, 1);
        let mut phone = FakePhone::default();
        phone.files.insert(a.path.clone(), b"abc".to_vec());
        let dir = tempfile::tempdir().unwrap();
        let report = copy_off(&phone, &[&a, &gone], dir.path()).unwrap();
        assert_eq!(report.copied, 1);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].0, "Bông Cỏ Mây");
        assert_eq!(std::fs::read(dir.path().join("00995F8A14CDF8AF.mp3")).unwrap(), b"abc");
        let index: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.path().join("index.json")).unwrap()).unwrap();
        assert_eq!(index["00995F8A14CDF8AF.mp3"]["title"], "Áo Mới Cà Mau");
        assert_eq!(*phone.writes.borrow(), 0, "never writes to the phone");

        let reads = *phone.reads.borrow();
        let again = copy_off(&phone, &[&a], dir.path()).unwrap();
        assert_eq!((again.copied, again.already_there), (0, 1));
        assert_eq!(*phone.reads.borrow(), reads, "a copy with the right size is not read again");
    }

    #[test]
    fn precheck_flags_songs_itunes_does_not_list_and_leading_zero_ids() {
        let kept = track(1, -941274682976370203, "Bông Cỏ Mây", "LFGG.mp3", 1, 1);
        let zero = track(2, 43170718095898799, "Áo Mới Cà Mau", "CWXN.mp3", 1, 1);
        let missing = track(3, 7055259643764605814, "Ai Đưa Em Về", "FFCY.mp3", 1, 1);
        let itunes: HashSet<i64> = [kept.sync_id, zero.sync_id, 3464385541651505982].into();
        assert_eq!(
            precheck(&[kept, zero.clone(), missing.clone()], &itunes),
            vec![Concern::LeadingZeroId(zero), Concern::MissingFromItunes(missing)]
        );
    }

    #[test]
    fn diff_reports_songs_added_and_removed_by_a_sync() {
        let a = track(1, 11, "Áo Mới Cà Mau", "CWXN.mp3", 1, 1);
        let b = track(2, 12, "Bông Cỏ Mây", "LFGG.mp3", 1, 1);
        let c = track(3, 13, "Ai Rồi Cũng Sẽ Khác", "KHRQ.mp3", 1, 1);
        let d = diff(&[a.clone(), b.clone()], &[b, c.clone()]);
        assert_eq!(d, SyncDiff { added: vec![c], removed: vec![a] });
    }
}
