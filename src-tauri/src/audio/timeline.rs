//! Which track the listener hears, and where in it (F7.1). Audio is decoded ahead into a buffer,
//! so "now playing" is derived from frames the device has consumed, not frames decoded.

#[derive(Debug, Clone, Copy, PartialEq)]
struct Segment {
    track: usize,
    /// Output frame (since the timeline started) where this segment begins.
    start: u64,
    frames: u64,
    /// Position inside the track where this segment begins (non-zero after a seek).
    offset: u64,
}

#[derive(Debug, Default)]
pub struct Timeline {
    segments: Vec<Segment>,
    total: u64,
}

impl Timeline {
    /// The next frames added belong to `track`, starting `offset_frames` into it.
    pub fn start_track(&mut self, track: usize, offset_frames: u64) {
        self.segments.push(Segment { track, start: self.total, frames: 0, offset: offset_frames });
    }

    pub fn add(&mut self, track: usize, frames: u64) {
        match self.segments.last_mut() {
            Some(s) if s.track == track => s.frames += frames,
            _ => self.segments.push(Segment { track, start: self.total, frames, offset: 0 }),
        }
        self.total += frames;
    }

    /// (track, frame within the track) at output frame `played`.
    pub fn at(&self, played: u64) -> Option<(usize, u64)> {
        let seg = self.segments.iter().rev().find(|s| s.start <= played && s.frames > 0).or(self.segments.last())?;
        let into = (played - seg.start.min(played)).min(seg.frames);
        Some((seg.track, seg.offset + into))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_queued_means_nothing_playing() {
        assert_eq!(Timeline::default().at(0), None);
    }

    #[test]
    fn position_within_the_first_track() {
        let mut t = Timeline::default();
        t.start_track(4, 0);
        t.add(4, 48_000);
        assert_eq!(t.at(24_000), Some((4, 24_000)));
    }

    #[test]
    fn the_track_changes_exactly_at_the_boundary() {
        let mut t = Timeline::default();
        t.start_track(0, 0);
        t.add(0, 1_000);
        t.start_track(1, 0);
        t.add(1, 500);
        assert_eq!(t.at(999), Some((0, 999)));
        assert_eq!(t.at(1_000), Some((1, 0)));
        assert_eq!(t.at(1_200), Some((1, 200)));
    }

    #[test]
    fn a_track_started_after_a_seek_counts_from_the_seek_point() {
        let mut t = Timeline::default();
        t.start_track(2, 24_000); // seeked to 0.5 s at 48 kHz
        t.add(2, 100);
        assert_eq!(t.at(50), Some((2, 24_050)));
    }

    #[test]
    fn past_the_end_stays_on_the_last_track_at_its_end() {
        let mut t = Timeline::default();
        t.start_track(0, 0);
        t.add(0, 100);
        assert_eq!(t.at(500), Some((0, 100)));
    }

    #[test]
    fn several_blocks_of_one_track_add_up() {
        let mut t = Timeline::default();
        t.start_track(0, 0);
        t.add(0, 100);
        t.add(0, 100);
        assert_eq!(t.at(150), Some((0, 150)));
    }
}
