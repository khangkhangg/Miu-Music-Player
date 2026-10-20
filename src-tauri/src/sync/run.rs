//! One guarded sync through iTunes (S.3), in this order:
//! 1. back up the phone's music database (CLAUDE.md hard rule 1) and copy off phone-only songs;
//! 2. check iTunes' view of the phone (stop if a sync could delete a song);
//! 3. adopt liked songs already on the phone, plan, and apply the plan through iTunes;
//! 4. `UpdateIPod`, then wait until the phone shows the changes;
//! 5. back up again, diff, and record the run. Songs that vanished without being planned are reported.

use std::collections::HashSet;
use std::time::Duration;

use serde::Serialize;

use crate::library::convert::Policy;
use crate::library::db::Db;
use crate::sync::guard::{diff, phone_only, precheck, Concern, CopyReport, PcSong, PhoneTrack};
use crate::sync::itunes::{adopt, apply, sendable_path, Convert, ITunes};
use crate::sync::plan::{plan, record_run, Step};

/// The phone's files, as a sync needs them.
pub trait PhoneSide {
    /// Back up the music database (kept) and read its tracks.
    fn backup(&self) -> Result<Vec<PhoneTrack>, String>;
    /// Read the tracks now, without keeping a backup (while waiting for the sync to land).
    fn peek(&self) -> Result<Vec<PhoneTrack>, String>;
    /// Copy these songs off the phone (read-only on the phone).
    fn copy_off(&self, tracks: &[&PhoneTrack]) -> Result<CopyReport, String>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "stage")]
pub enum Stage {
    Backup,
    CopyOff { songs: usize },
    Check,
    Apply { done: usize, total: usize },
    UpdateDevice,
    Waiting,
    Verify,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncError {
    /// Stopped before anything changed; each reason is a sentence for the user.
    Blocked(Vec<String>),
    Failed(String),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub adopted: usize,
    pub added: usize,
    pub removed: usize,
    /// (song id, why).
    pub failed: Vec<(i64, String)>,
    /// Phone songs that disappeared without Miu asking.
    pub unexpected_removed: Vec<PhoneTrack>,
    /// Worth knowing, not worth stopping for (a copy of the song is kept).
    pub warnings: Vec<String>,
    /// The phone showed every change before the wait ran out.
    pub confirmed: bool,
    pub run_id: Option<i64>,
}

pub struct Options {
    pub poll_every: Duration,
    pub max_polls: usize,
}

#[allow(clippy::too_many_arguments)]
pub fn run_sync(
    db: &Db,
    udid: &str,
    policy: &Policy,
    phone: &dyn PhoneSide,
    itunes: &dyn ITunes,
    convert: Option<Convert>,
    options: &Options,
    sleep: &dyn Fn(Duration),
    progress: &mut dyn FnMut(Stage),
    now_unix: u64,
) -> Result<Report, SyncError> {
    let failed = SyncError::Failed;
    // 1. Back up, and keep a copy of every song only the phone has.
    progress(Stage::Backup);
    let before = phone.backup().map_err(failed)?;
    let pc = pc_songs(db).map_err(failed)?;
    let only = phone_only(&before, &pc);
    progress(Stage::CopyOff { songs: only.len() });
    let copied = phone.copy_off(&only).map_err(failed)?;
    if !copied.failed.is_empty() {
        return Err(SyncError::Blocked(
            copied.failed.iter().map(|(title, why)| format!("Could not copy “{title}” off the iPhone ({why}), so a sync could lose it.")).collect(),
        ));
    }

    // 2. iTunes must list every phone song, or its sync deletes the ones it left out.
    progress(Stage::Check);
    let device = itunes.device_tracks().map_err(failed)?;
    let ids: HashSet<i64> = device.iter().map(|t| t.id).collect();
    let (mut blockers, mut warnings) = (Vec::new(), Vec::new());
    for c in precheck(&before, &ids) {
        match c {
            Concern::MissingFromItunes(t) => blockers.push(format!("iTunes does not list “{}”, so a sync would delete it from the iPhone.", t.title)),
            // A copy is kept (on this PC or copied off above) and the check after the sync reports a loss.
            Concern::LeadingZeroId(t) => warnings.push(format!("“{}” has the kind of ID iTunes lost once before; a copy is kept in case.", t.title)),
        }
    }
    if !blockers.is_empty() {
        return Err(SyncError::Blocked(blockers));
    }

    // 3. Adopt, plan, apply.
    let adopted = adopt(db, udid, &device, now_unix).map_err(failed)?;
    let p = plan(db, udid, policy).map_err(|e| SyncError::Failed(e.to_string()))?;
    let mut report = Report { adopted, confirmed: true, warnings, ..Default::default() };
    if p.steps.is_empty() {
        return Ok(report);
    }
    let source = |id: i64| sendable_path(db, id, policy, convert);
    let mut results = Vec::with_capacity(p.steps.len());
    for (i, step) in p.steps.iter().enumerate() {
        results.extend(apply(itunes, std::slice::from_ref(step), &source));
        progress(Stage::Apply { done: i + 1, total: p.steps.len() });
    }
    let (mut appear, mut vanish) = (HashSet::new(), HashSet::new());
    for (step, result) in p.steps.iter().zip(&results) {
        match (&result.outcome, step) {
            (Err(e), _) => report.failed.push((step.song_id(), e.clone())),
            (Ok(_), Step::Remove { device_pid, .. }) => {
                report.removed += 1;
                vanish.extend(device_pid.map(|p| p as i64));
            }
            (Ok(new), Step::Add { .. } | Step::Replace { .. } | Step::Update { .. }) => {
                report.added += 1;
                appear.extend(new.map(|p| p as i64));
                if let Step::Replace { device_pid, .. } | Step::Update { device_pid, .. } = step {
                    vanish.extend(device_pid.map(|p| p as i64));
                }
            }
        }
    }

    // 4. Let iTunes apply it, and wait until the phone shows it.
    if report.added + report.removed > 0 {
        progress(Stage::UpdateDevice);
        itunes.sync_device().map_err(failed)?;
        progress(Stage::Waiting);
        report.confirmed = false;
        for _ in 0..options.max_polls {
            sleep(options.poll_every);
            let tracks = phone.peek().map_err(failed)?;
            let now: HashSet<i64> = tracks.iter().map(|t| t.sync_id).collect();
            // The entry shows up first; the file follows (size 0 until then, seen on the 5s).
            let with_file: HashSet<i64> = tracks.iter().filter(|t| t.size > 0).map(|t| t.sync_id).collect();
            if appear.is_subset(&with_file) && vanish.is_disjoint(&now) {
                report.confirmed = true;
                break;
            }
        }
    }

    // 5. Back up again, diff, record.
    progress(Stage::Verify);
    let after = phone.backup().map_err(failed)?;
    report.unexpected_removed = diff(&before, &after).removed.into_iter().filter(|t| !vanish.contains(&t.sync_id)).collect();
    report.run_id = Some(record_run(db, udid, &p, &results, now_unix).map_err(|e| SyncError::Failed(e.to_string()))?);
    Ok(report)
}

fn pc_songs(db: &Db) -> Result<Vec<PcSong>, String> {
    let mut stmt = db.conn().prepare("SELECT COALESCE(title, ''), size, duration_ms FROM songs").map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |r| Ok(PcSong { title: r.get(0)?, size: r.get::<_, i64>(1)?.max(0) as u64, duration_ms: r.get(2)? }))
        .map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<_>>().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::folders::add_folder;
    use crate::sync::itunes::DeviceTrack;
    use std::cell::RefCell;
    use std::path::Path;

    const PHONE: &str = "e51e30a5aaaa";

    #[derive(Default)]
    struct World {
        phone: Vec<PhoneTrack>,
        /// iTunes' view leaves these sync_ids out (as with "Áo Mới Cà Mau").
        hidden_from_itunes: Vec<i64>,
        pending_add: Vec<PhoneTrack>,
        pending_remove: Vec<i64>,
        /// The device sync also deletes this sync_id.
        drops_on_sync: Option<i64>,
        sync_never_lands: bool,
        /// Peeks after the device sync before new tracks get their files (size 0 until then).
        files_after_peeks: usize,
        copy_fails: bool,
        next_id: i64,
        log: Vec<String>,
        backups: usize,
    }

    struct Fakes(RefCell<World>);

    fn t(pid: i64, sync_id: i64, title: &str, size: u64) -> PhoneTrack {
        PhoneTrack { pid, sync_id, title: title.into(), path: format!("/iTunes_Control/Music/F01/{pid}.mp3"), size, duration_ms: Some(200_000) }
    }

    impl PhoneSide for Fakes {
        fn backup(&self) -> Result<Vec<PhoneTrack>, String> {
            let mut w = self.0.borrow_mut();
            w.backups += 1;
            w.log.push("backup".into());
            Ok(w.phone.clone())
        }
        fn peek(&self) -> Result<Vec<PhoneTrack>, String> {
            let mut w = self.0.borrow_mut();
            if w.files_after_peeks > 0 {
                w.files_after_peeks -= 1;
                if w.files_after_peeks == 0 {
                    for t in w.phone.iter_mut().filter(|t| t.size == 0) {
                        t.size = 1;
                    }
                }
            }
            Ok(w.phone.clone())
        }
        fn copy_off(&self, tracks: &[&PhoneTrack]) -> Result<CopyReport, String> {
            let mut w = self.0.borrow_mut();
            w.log.push(format!("copy {}", tracks.len()));
            if w.copy_fails {
                return Ok(CopyReport { failed: vec![("Bông Cỏ Mây".into(), "cable pulled".into())], ..Default::default() });
            }
            Ok(CopyReport { copied: tracks.len(), ..Default::default() })
        }
    }

    impl ITunes for Fakes {
        fn device_tracks(&self) -> Result<Vec<DeviceTrack>, String> {
            let w = self.0.borrow();
            Ok(w.phone
                .iter()
                .chain(&w.pending_add)
                .filter(|p| !w.hidden_from_itunes.contains(&p.sync_id) && !w.pending_remove.contains(&p.sync_id))
                .map(|p| DeviceTrack { id: p.sync_id, name: p.title.clone(), size: p.size, duration_ms: p.duration_ms })
                .collect())
        }
        fn add(&self, path: &Path) -> Result<i64, String> {
            let mut w = self.0.borrow_mut();
            w.next_id += 1;
            let id = 0x7000_0000_0000_0000 + w.next_id;
            w.log.push(format!("add {}", path.display()));
            let title = path.file_stem().unwrap().to_string_lossy().into_owned();
            w.pending_add.push(t(id + 1, id, &title, 1));
            Ok(id)
        }
        fn remove(&self, id: i64) -> Result<bool, String> {
            let mut w = self.0.borrow_mut();
            w.log.push(format!("remove {id}"));
            w.pending_remove.push(id);
            Ok(true)
        }
        fn sync_device(&self) -> Result<(), String> {
            let mut w = self.0.borrow_mut();
            w.log.push("UpdateIPod".into());
            if w.sync_never_lands {
                return Ok(());
            }
            let (adds, removes) = (std::mem::take(&mut w.pending_add), std::mem::take(&mut w.pending_remove));
            let drop = w.drops_on_sync;
            w.phone.retain(|p| !removes.contains(&p.sync_id) && Some(p.sync_id) != drop);
            let empty = w.files_after_peeks > 0;
            w.phone.extend(adds.into_iter().map(|mut t| {
                if empty {
                    t.size = 0;
                }
                t
            }));
            Ok(())
        }
    }

    fn song(db: &Db, title: &str, size: u64, rating: i64) -> i64 {
        let f = match add_folder(db, r"E:\Nhạc", 1) {
            Ok(f) => f.id,
            Err(_) => db.conn().query_row("SELECT id FROM watched_folders", [], |r| r.get(0)).unwrap(),
        };
        db.conn()
            .execute(
                "INSERT INTO songs (folder_id, path, size, mtime, added_at, format, title, duration_ms, rating, tags_read, sample_rate, bit_depth)
                 VALUES (?1, ?2, ?3, 1, 1, 'mp3', ?4, 200000, ?5, 1, 44100, 16)",
                rusqlite::params![f, format!(r"E:\Nhạc\{title}.mp3"), size as i64, title, rating],
            )
            .unwrap();
        db.conn().last_insert_rowid()
    }

    /// The phone holds a liked song (from 3uTools) and one Miu doesn't have; the library has one new liked song.
    fn setup() -> (Db, Fakes) {
        let db = Db::open_in_memory().unwrap();
        song(&db, "Áo Mới Cà Mau", 9_296_034, 1);
        song(&db, "Hạ Trắng", 7, 1);
        song(&db, "Không thích", 8, 0);
        let world = World { phone: vec![t(1, -11, "Áo Mới Cà Mau", 9_296_034), t(2, -12, "Bông Cỏ Mây", 4_000_000)], ..Default::default() };
        (db, Fakes(RefCell::new(world)))
    }

    fn opts() -> Options {
        Options { poll_every: Duration::from_secs(5), max_polls: 3 }
    }

    fn sync(db: &Db, f: &Fakes) -> (Result<Report, SyncError>, Vec<Stage>) {
        let mut stages = Vec::new();
        let r = run_sync(db, PHONE, &Policy::default(), f, f, None, &opts(), &|_| {}, &mut |s| stages.push(s), 100);
        (r, stages)
    }

    #[test]
    fn a_sync_backs_up_copies_off_checks_adopts_applies_updates_and_verifies() {
        let (db, f) = setup();
        let (r, stages) = sync(&db, &f);
        let r = r.unwrap();
        assert_eq!((r.adopted, r.added, r.removed), (1, 1, 0));
        assert!(r.failed.is_empty() && r.unexpected_removed.is_empty() && r.confirmed);
        assert!(r.run_id.is_some());
        let w = f.0.borrow();
        assert_eq!(w.log, vec!["backup", "copy 1", r"add E:\Nhạc\Hạ Trắng.mp3", "UpdateIPod", "backup"]);
        assert_eq!(stages.first(), Some(&Stage::Backup));
        assert!(stages.contains(&Stage::Apply { done: 1, total: 1 }) && stages.contains(&Stage::UpdateDevice));
        assert_eq!(stages.last(), Some(&Stage::Verify));
        drop(w);
        let (again, _) = sync(&db, &f);
        let again = again.unwrap();
        assert_eq!((again.adopted, again.added, again.removed), (0, 0, 0), "the second run has nothing to do");
        assert!(!f.0.borrow().log.iter().skip(5).any(|l| l == "UpdateIPod"), "no device sync when nothing changed");
    }

    #[test]
    fn a_song_missing_from_itunes_view_stops_the_sync_before_anything_changes() {
        let (db, f) = setup();
        f.0.borrow_mut().hidden_from_itunes = vec![-12];
        let (r, _) = sync(&db, &f);
        let Err(SyncError::Blocked(reasons)) = r else { panic!("expected blocked, got {r:?}") };
        assert!(reasons[0].contains("Bông Cỏ Mây"), "{reasons:?}");
        let w = f.0.borrow();
        assert!(!w.log.iter().any(|l| l.starts_with("add") || l.starts_with("remove") || l == "UpdateIPod"), "{:?}", w.log);
        let runs: i64 = db.conn().query_row("SELECT count(*) FROM sync_runs", [], |r| r.get(0)).unwrap();
        assert_eq!(runs, 0);
    }

    #[test]
    fn a_song_that_could_not_be_copied_off_stops_the_sync() {
        let (db, f) = setup();
        f.0.borrow_mut().copy_fails = true;
        let (r, _) = sync(&db, &f);
        let Err(SyncError::Blocked(reasons)) = r else { panic!("expected blocked, got {r:?}") };
        assert!(reasons[0].contains("Bông Cỏ Mây") && reasons[0].contains("cable pulled"), "{reasons:?}");
        assert!(!f.0.borrow().log.iter().any(|l| l == "UpdateIPod"));
    }

    #[test]
    fn a_song_the_sync_deleted_without_being_asked_is_reported() {
        let (db, f) = setup();
        f.0.borrow_mut().drops_on_sync = Some(-12);
        let r = sync(&db, &f).0.unwrap();
        assert_eq!(r.unexpected_removed.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), vec!["Bông Cỏ Mây"]);
    }

    #[test]
    fn with_a_converter_flacs_go_as_converted_copies() {
        let (db, f) = setup();
        let flac = song(&db, "Diễm Xưa", 9, 1);
        db.conn().execute("UPDATE songs SET format = 'flac' WHERE id = ?1", [flac]).unwrap();
        let convert = |s: &crate::sync::itunes::SongSource| Ok(std::path::PathBuf::from(format!(r"C:\cache\{}.m4a", s.audio_key.len())));
        let r = run_sync(&db, PHONE, &Policy::default(), &f, &f, Some(&convert), &opts(), &|_| {}, &mut |_| {}, 100).unwrap();
        assert_eq!((r.added, r.failed.len()), (2, 0));
        assert!(f.0.borrow().log.iter().any(|l| l.starts_with(r"add C:\cache\")), "{:?}", f.0.borrow().log);
    }

    #[test]
    fn a_leading_zero_id_only_warns_because_a_copy_is_kept() {
        let (db, f) = setup();
        f.0.borrow_mut().phone[1].sync_id = 0x0099_5F8A_14CD_F8AF;
        let r = sync(&db, &f).0.unwrap();
        assert_eq!(r.added, 1);
        assert_eq!(r.warnings.len(), 1);
        assert!(r.warnings[0].contains("Bông Cỏ Mây"), "{:?}", r.warnings);
    }

    #[test]
    fn a_sync_is_confirmed_only_once_the_files_have_arrived() {
        let (db, f) = setup();
        f.0.borrow_mut().files_after_peeks = 3;
        let polls = RefCell::new(0);
        let r = run_sync(&db, PHONE, &Policy::default(), &f, &f, None, &opts(), &|_| *polls.borrow_mut() += 1, &mut |_| {}, 100).unwrap();
        assert!(r.confirmed);
        assert_eq!(*polls.borrow(), 3, "kept waiting while the new track had no file");

        let (db, f) = setup();
        f.0.borrow_mut().files_after_peeks = 10;
        let r = run_sync(&db, PHONE, &Policy::default(), &f, &f, None, &opts(), &|_| {}, &mut |_| {}, 100).unwrap();
        assert!(!r.confirmed, "entries without files after the last poll are not confirmed");
    }

    #[test]
    fn a_sync_that_does_not_land_in_time_is_recorded_unconfirmed() {
        let (db, f) = setup();
        f.0.borrow_mut().sync_never_lands = true;
        let r = sync(&db, &f).0.unwrap();
        assert!(!r.confirmed);
        assert!(r.run_id.is_some(), "what iTunes accepted is still recorded");
    }

    #[test]
    fn songs_itunes_could_not_take_are_listed_and_the_rest_still_go() {
        let (db, f) = setup();
        let flac = song(&db, "Diễm Xưa", 9, 1);
        db.conn().execute("UPDATE songs SET format = 'flac' WHERE id = ?1", [flac]).unwrap();
        let r = sync(&db, &f).0.unwrap();
        assert_eq!(r.added, 1);
        assert_eq!(r.failed.len(), 1);
        assert_eq!(r.failed[0].0, flac);
        assert!(r.failed[0].1.contains("convert"));
    }
}
