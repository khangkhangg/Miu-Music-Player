//! Offline rules (P.3): which episodes to fetch on their own, and which downloads to clear away.

use crate::podcast::store::EpisodeRow;

/// Among the newest `keep` unplayed episodes (list is newest first), those not downloaded yet.
pub fn to_download(episodes: &[EpisodeRow], keep: usize) -> Vec<i64> {
    episodes.iter().filter(|e| !e.played).take(keep).filter(|e| e.download_path.is_none()).map(|e| e.id).collect()
}

/// Downloaded episodes played at least `days` days before `now` (seconds since 1970).
pub fn to_clean_up(episodes: &[EpisodeRow], now: i64, days: u32) -> Vec<i64> {
    let cutoff = now - i64::from(days) * 86_400;
    episodes
        .iter()
        .filter(|e| e.download_path.is_some() && e.played && e.played_at.is_some_and(|t| t <= cutoff))
        .map(|e| e.id)
        .collect()
}

/// What to store for a position in an episode: (position to remember, played). The last 30 s or
/// 95 % count as finished, and a finished episode starts from the top next time.
pub fn progress(position_ms: u64, duration_ms: Option<u64>) -> (u64, bool) {
    match duration_ms {
        Some(d) if d > 0 && (position_ms + 30_000 >= d || position_ms * 100 >= d * 95) => (0, true),
        _ => (position_ms, false),
    }
}

/// Where to start an episode: a few seconds before where the listener stopped; `None` = the top.
pub fn resume_at(position_ms: i64, played: bool) -> Option<u64> {
    (!played && position_ms > 5_000).then(|| (position_ms - 3_000) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::podcast::store::EpisodeRow;

    const DAY: i64 = 86_400;

    /// Newest first, like `list_episodes`.
    fn ep(id: i64, played: bool, downloaded: bool, played_at: Option<i64>) -> EpisodeRow {
        EpisodeRow { id, podcast_id: 1, guid: id.to_string(), title: String::new(), description: None, published: Some(1_000 - id),
                     duration_ms: None, audio_url: String::new(), position_ms: 0, played, played_at,
                     download_path: downloaded.then(|| format!("{id}.mp3")) }
    }

    #[test]
    fn the_newest_unplayed_episodes_are_fetched_up_to_the_limit() {
        let eps = vec![ep(1, false, false, None), ep(2, true, false, None), ep(3, false, true, None), ep(4, false, false, None), ep(5, false, false, None)];
        assert_eq!(to_download(&eps, 3), vec![1, 4], "the newest 3 unplayed are 1, 3, 4; 3 is already here");
        assert_eq!(to_download(&eps, 0), Vec::<i64>::new());
    }

    #[test]
    fn played_downloads_go_after_the_chosen_number_of_days() {
        let now = 100 * DAY;
        let eps = vec![
            ep(1, true, true, Some(now - 8 * DAY)),  // played long ago: delete
            ep(2, true, true, Some(now - 2 * DAY)),  // played recently: keep
            ep(3, false, true, None),                // not played: keep
            ep(4, true, false, Some(now - 30 * DAY)), // nothing on disk
        ];
        assert_eq!(to_clean_up(&eps, now, 7), vec![1]);
        assert_eq!(to_clean_up(&eps, now, 0), vec![1, 2], "0 days: as soon as it is played");
    }
}

#[cfg(test)]
mod listening_tests {
    use super::*;

    #[test]
    fn listening_near_the_end_counts_as_played_and_restarts_next_time() {
        let hour = 3_600_000;
        assert_eq!(progress(10 * 60_000, Some(hour)), (10 * 60_000, false));
        assert_eq!(progress(hour - 20_000, Some(hour)), (0, true), "last 30 seconds");
        assert_eq!(progress(95 * 600, Some(60_000)), (0, true), "95 % of a short episode");
        assert_eq!(progress(10 * 60_000, None), (10 * 60_000, false), "length unknown: never guessed played");
    }

    #[test]
    fn episodes_resume_where_you_stopped_unless_barely_started_or_finished() {
        assert_eq!(resume_at(754_000, false), Some(751_000), "a few seconds back, to pick up the thread");
        assert_eq!(resume_at(4_000, false), None, "barely started: from the top");
        assert_eq!(resume_at(754_000, true), None, "played: from the top");
    }
}
