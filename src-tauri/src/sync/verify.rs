//! Post-sync verification (F4.5): after a sync the phone's database is pulled again and every
//! track the plan touched is checked against it, so a silent failure is reported per track.

use std::collections::HashMap;
use std::path::Path;

use unicode_normalization::UnicodeNormalization;

/// What the phone should show for one song after the run.
#[derive(Debug, Clone, PartialEq)]
pub struct Expected {
    pub song_id: i64,
    pub device_pid: Option<u64>,
    pub title: String,
    /// False for songs the run removed.
    pub present: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ProblemKind {
    Missing,
    StillThere,
    WrongTitle { on_phone: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Problem {
    pub song_id: i64,
    pub kind: ProblemKind,
}

fn same(a: &str, b: &str) -> bool {
    a.nfc().eq(b.nfc())
}

/// Every expected track that is not as it should be on the phone (`phone`: pid → title).
pub fn verify(expected: &[Expected], phone: &HashMap<u64, String>) -> Vec<Problem> {
    expected
        .iter()
        .filter_map(|e| {
            let on_phone = e.device_pid.and_then(|p| phone.get(&p));
            let kind = match (e.present, on_phone) {
                (true, None) => ProblemKind::Missing,
                (true, Some(t)) if !same(t, &e.title) => ProblemKind::WrongTitle { on_phone: t.clone() },
                (false, Some(_)) => ProblemKind::StillThere,
                _ => return None,
            };
            Some(Problem { song_id: e.song_id, kind })
        })
        .collect()
}

/// Music tracks (pid → title) in a MediaLibrary.sqlitedb pulled from the phone.
pub fn device_tracks(db: &Path) -> rusqlite::Result<HashMap<u64, String>> {
    let c = rusqlite::Connection::open_with_flags(db, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let mut stmt = c.prepare("SELECT i.item_pid, e.title FROM item i JOIN item_extra e ON e.item_pid = i.item_pid WHERE i.media_type = 8")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)? as u64, r.get::<_, Option<String>>(1)?.unwrap_or_default())))?;
    rows.collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn phone(tracks: &[(u64, &str)]) -> HashMap<u64, String> {
        tracks.iter().map(|(p, t)| (*p, t.to_string())).collect()
    }

    #[test]
    fn everything_where_it_should_be_reports_nothing() {
        let expected = vec![Expected { song_id: 1, device_pid: Some(11), title: "Diễm xưa".into(), present: true }];
        assert!(verify(&expected, &phone(&[(11, "Diễm xưa"), (99, "from 3uTools")])).is_empty());
    }

    #[test]
    fn each_kind_of_mismatch_is_reported_per_track() {
        let expected = vec![
            Expected { song_id: 1, device_pid: Some(11), title: "Diễm xưa".into(), present: true },
            Expected { song_id: 2, device_pid: Some(12), title: "Hạ trắng".into(), present: true },
            Expected { song_id: 3, device_pid: Some(13), title: "Removed".into(), present: false },
            Expected { song_id: 4, device_pid: None, title: "No id".into(), present: true },
        ];
        let got = verify(&expected, &phone(&[(11, "Diem xua (wrong)"), (13, "Removed")]));
        assert_eq!(got, vec![
            Problem { song_id: 1, kind: ProblemKind::WrongTitle { on_phone: "Diem xua (wrong)".into() } },
            Problem { song_id: 2, kind: ProblemKind::Missing },
            Problem { song_id: 3, kind: ProblemKind::StillThere },
            Problem { song_id: 4, kind: ProblemKind::Missing },
        ]);
    }

    #[test]
    fn titles_compare_in_composed_form() {
        // Decomposed "ễ" on the phone is the same title.
        let expected = vec![Expected { song_id: 1, device_pid: Some(11), title: "Diễm".into(), present: true }];
        assert!(verify(&expected, &phone(&[(11, "Die\u{302}\u{303}m")])).is_empty());
    }

    #[test]
    fn device_tracks_are_read_from_a_pulled_database() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("MediaLibrary.sqlitedb");
        let c = rusqlite::Connection::open(&p).unwrap();
        c.execute_batch("CREATE TABLE item (item_pid INTEGER PRIMARY KEY, media_type INTEGER);
                         CREATE TABLE item_extra (item_pid INTEGER PRIMARY KEY, title TEXT);
                         INSERT INTO item VALUES (11, 8), (12, 8), (13, 2);
                         INSERT INTO item_extra VALUES (11, 'Diễm xưa'), (12, 'Hạ trắng'), (13, 'a podcast');").unwrap();
        drop(c);
        let got = device_tracks(&p).unwrap();
        assert_eq!(got, phone(&[(11, "Diễm xưa"), (12, "Hạ trắng")]), "music only");
    }
}
