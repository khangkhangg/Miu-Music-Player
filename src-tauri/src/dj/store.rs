//! Song analysis results (D.1) kept per song in `dj_analysis`. A result belongs to one version
//! of the file: when the file's size or mtime changes the song is analysed again.

use rusqlite::{params, OptionalExtension};

use super::analysis::Analysis;
use super::autodj::Candidate;
use crate::library::db::{Db, Result};

/// A song still waiting for analysis.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub song_id: i64,
    pub path: String,
    pub size: i64,
    pub mtime: i64,
}

pub fn save(db: &Db, song_id: i64, size: i64, mtime: i64, analysis: &Analysis, now_unix: i64) -> Result<()> {
    let data = serde_json::to_string(analysis).expect("analysis serialises");
    db.conn().execute(
        "INSERT INTO dj_analysis (song_id, size, mtime, data, analyzed_at) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (song_id) DO UPDATE SET size = ?2, mtime = ?3, data = ?4, analyzed_at = ?5",
        params![song_id, size, mtime, data, now_unix],
    )?;
    Ok(())
}

/// Records that this version of the file could not be analysed (undecodable), so it is not retried.
pub fn save_failed(db: &Db, song_id: i64, size: i64, mtime: i64, now_unix: i64) -> Result<()> {
    db.conn().execute(
        "INSERT INTO dj_analysis (song_id, size, mtime, data, analyzed_at) VALUES (?1, ?2, ?3, 'null', ?4)
         ON CONFLICT (song_id) DO UPDATE SET size = ?2, mtime = ?3, data = 'null', analyzed_at = ?4",
        params![song_id, size, mtime, now_unix],
    )?;
    Ok(())
}

/// The stored analysis of the song's current file, if there is one.
pub fn get(db: &Db, song_id: i64) -> Result<Option<Analysis>> {
    let data: Option<String> = db
        .conn()
        .query_row(
            "SELECT a.data FROM dj_analysis a JOIN songs s ON s.id = a.song_id
             WHERE a.song_id = ?1 AND a.size = s.size AND a.mtime = s.mtime",
            [song_id],
            |r| r.get(0),
        )
        .optional()?;
    Ok(data.and_then(|d| serde_json::from_str(&d).ok()))
}

/// Songs with no analysis of their current file, liked songs first; disliked songs are skipped
/// (they are never played in DJ mode).
pub fn pending(db: &Db, limit: usize) -> Result<Vec<Pending>> {
    let mut stmt = db.conn().prepare(
        "SELECT s.id, s.path, s.size, s.mtime FROM songs s
         LEFT JOIN dj_analysis a ON a.song_id = s.id AND a.size = s.size AND a.mtime = s.mtime
         WHERE a.song_id IS NULL AND s.rating >= 0
         ORDER BY s.rating DESC, s.id
         LIMIT ?1",
    )?;
    let rows = stmt.query_map([limit as i64], |r| {
        Ok(Pending { song_id: r.get(0)?, path: r.get(1)?, size: r.get(2)?, mtime: r.get(3)? })
    })?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

/// Every song Auto DJ may choose: analysed (current file), not disliked.
pub fn candidates(db: &Db) -> Result<Vec<Candidate>> {
    let mut stmt = db.conn().prepare(
        "SELECT s.id, s.rating, a.data FROM songs s
         JOIN dj_analysis a ON a.song_id = s.id AND a.size = s.size AND a.mtime = s.mtime
         WHERE s.rating >= 0 ORDER BY s.id",
    )?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?)))?;
    let mut out = Vec::new();
    for row in rows {
        let (song_id, rating, data) = row?;
        if let Ok(analysis) = serde_json::from_str(&data) {
            out.push(Candidate { song_id, analysis, liked: rating > 0 });
        }
    }
    Ok(out)
}

/// (analysed, total) over the songs DJ mode can play, for the progress line.
pub fn progress(db: &Db) -> Result<(u32, u32)> {
    Ok(db.conn().query_row(
        "SELECT COUNT(a.song_id), COUNT(*) FROM songs s
         LEFT JOIN dj_analysis a ON a.song_id = s.id AND a.size = s.size AND a.mtime = s.mtime
         WHERE s.rating >= 0",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(db: &Db, path: &str, rating: i64) -> i64 {
        db.conn()
            .execute(
                "INSERT OR IGNORE INTO watched_folders (id, path, added_at) VALUES (1, 'C:/M', 1)",
                [],
            )
            .unwrap();
        db.conn()
            .execute(
                "INSERT INTO songs (folder_id, path, size, mtime, added_at, rating) VALUES (1, ?1, 10, 100, 1, ?2)",
                params![path, rating],
            )
            .unwrap();
        db.conn().last_insert_rowid()
    }

    fn sample() -> Analysis {
        Analysis {
            bpm: 124.0,
            beat_confidence: 0.6,
            first_beat_s: 0.12,
            downbeat_s: 0.6,
            key: Some("8A".into()),
            energy: 7,
            mix_in_s: 8.0,
            mix_out_s: 200.0,
            duration_s: 215.0,
        }
    }

    #[test]
    fn saved_analysis_comes_back_for_the_same_file() {
        let db = Db::open_in_memory().unwrap();
        let id = song(&db, "C:/M/a.mp3", 0);
        assert_eq!(get(&db, id).unwrap(), None);
        save(&db, id, 10, 100, &sample(), 5).unwrap();
        assert_eq!(get(&db, id).unwrap(), Some(sample()));
    }

    #[test]
    fn a_changed_file_is_analysed_again() {
        let db = Db::open_in_memory().unwrap();
        let id = song(&db, "C:/M/a.mp3", 0);
        save(&db, id, 10, 100, &sample(), 5).unwrap();
        assert!(pending(&db, 10).unwrap().is_empty());
        db.conn().execute("UPDATE songs SET mtime = 200 WHERE id = ?1", [id]).unwrap();
        assert_eq!(get(&db, id).unwrap(), None, "the old result is for the old file");
        let p = pending(&db, 10).unwrap();
        assert_eq!(p, vec![Pending { song_id: id, path: "C:/M/a.mp3".into(), size: 10, mtime: 200 }]);
        save(&db, id, 10, 200, &sample(), 6).unwrap();
        assert!(pending(&db, 10).unwrap().is_empty());
    }

    #[test]
    fn liked_songs_go_first_and_disliked_ones_are_skipped() {
        let db = Db::open_in_memory().unwrap();
        let plain = song(&db, "C:/M/a.mp3", 0);
        let liked = song(&db, "C:/M/b.mp3", 1);
        song(&db, "C:/M/c.mp3", -1);
        let ids: Vec<i64> = pending(&db, 10).unwrap().iter().map(|p| p.song_id).collect();
        assert_eq!(ids, vec![liked, plain]);
        assert_eq!(pending(&db, 1).unwrap().len(), 1);
        assert_eq!(progress(&db).unwrap(), (0, 2));
        save(&db, liked, 10, 100, &sample(), 5).unwrap();
        assert_eq!(progress(&db).unwrap(), (1, 2));
    }

    #[test]
    fn candidates_are_the_analysed_songs_dj_mode_may_play() {
        let db = Db::open_in_memory().unwrap();
        let liked = song(&db, "C:/M/a.mp3", 1);
        let plain = song(&db, "C:/M/b.mp3", 0);
        let disliked = song(&db, "C:/M/c.mp3", -1);
        let failed = song(&db, "C:/M/d.mp3", 0);
        song(&db, "C:/M/e.mp3", 0); // not analysed yet
        for id in [liked, plain, disliked] {
            save(&db, id, 10, 100, &sample(), 5).unwrap();
        }
        save_failed(&db, failed, 10, 100, 5).unwrap();
        let c = candidates(&db).unwrap();
        let ids: Vec<(i64, bool)> = c.iter().map(|c| (c.song_id, c.liked)).collect();
        assert_eq!(ids, vec![(liked, true), (plain, false)]);
        assert_eq!(c[0].analysis, sample());
    }

    #[test]
    fn removing_a_song_removes_its_analysis() {
        let db = Db::open_in_memory().unwrap();
        let id = song(&db, "C:/M/a.mp3", 0);
        save(&db, id, 10, 100, &sample(), 5).unwrap();
        db.conn().execute("DELETE FROM songs WHERE id = ?1", [id]).unwrap();
        let n: i64 = db.conn().query_row("SELECT COUNT(*) FROM dj_analysis", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 0);
    }
}
