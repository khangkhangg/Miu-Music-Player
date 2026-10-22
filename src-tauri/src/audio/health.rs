//! Connection health of an output (F7.9): a weak Bluetooth link shows up as the device asking
//! for audio late (a long gap between callbacks) or the second output needing a jump to stay in
//! line. Counting those over the last minute gives good / fair / poor.

use std::collections::VecDeque;
use std::time::Duration;

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Health {
    #[default]
    Good,
    Fair,
    Poor,
}

/// How far back problems count, seconds.
pub const WINDOW_S: f64 = 60.0;

/// A callback that came `gap` after the previous one, when the previous one handed over
/// `frames` at `rate`, was late enough to be heard as a stutter.
pub fn is_glitch(gap: Duration, frames: u64, rate: u32) -> bool {
    let lasts = frames as f64 / rate.max(1) as f64;
    gap.as_secs_f64() > (lasts * 4.0).max(0.06)
}

/// Problems seen on one output, by time (seconds since any fixed start).
#[derive(Debug, Default)]
pub struct HealthLog {
    events: VecDeque<f64>,
}

impl HealthLog {
    pub fn record(&mut self, at: f64) {
        self.events.push_back(at);
    }

    pub fn level(&mut self, now: f64) -> Health {
        while self.events.front().is_some_and(|&t| now - t > WINDOW_S) {
            self.events.pop_front();
        }
        match self.events.len() {
            0 => Health::Good,
            1..=3 => Health::Fair,
            _ => Health::Poor,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regular_callbacks_are_not_glitches() {
        assert!(!is_glitch(Duration::from_millis(10), 441, 44_100));
        assert!(!is_glitch(Duration::from_millis(25), 441, 44_100), "ordinary scheduling jitter");
    }

    #[test]
    fn a_callback_far_later_than_its_buffer_lasts_is_a_glitch() {
        assert!(is_glitch(Duration::from_millis(120), 441, 44_100));
        assert!(!is_glitch(Duration::from_millis(120), 4_410, 44_100), "a 100 ms buffer may wait that long");
        assert!(is_glitch(Duration::from_millis(450), 4_410, 44_100));
    }

    #[test]
    fn no_problems_is_good_a_few_is_fair_many_is_poor() {
        let mut h = HealthLog::default();
        assert_eq!(h.level(0.0), Health::Good);
        h.record(1.0);
        assert_eq!(h.level(2.0), Health::Fair);
        for t in [3.0, 4.0, 5.0] {
            h.record(t);
        }
        assert_eq!(h.level(6.0), Health::Poor);
    }

    #[test]
    fn problems_older_than_a_minute_no_longer_count() {
        let mut h = HealthLog::default();
        for t in [1.0, 2.0, 3.0, 4.0] {
            h.record(t);
        }
        assert_eq!(h.level(62.5), Health::Fair, "only the one at 4 s is left");
        assert_eq!(h.level(70.0), Health::Good);
    }
}

/// The second output counts as gone only if it took no audio for `still` while the first one
/// carried on playing (`first_moved` frames at `first_rate`): if both stopped, the problem is
/// upstream or with the first output, not the second.
pub fn second_stalled(still: Duration, first_moved: u64, first_rate: u32) -> bool {
    still > Duration::from_secs(2) && first_moved as f64 >= first_rate as f64 * (still.as_secs_f64() * 0.5)
}

/// Which output a realignment jump points at: a positive jump (the second output skips ahead)
/// means the second fell behind; a negative one (it waits) means the first did.
pub fn behind(step_s: f64) -> Output {
    if step_s > 0.0 { Output::Second } else { Output::First }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Output {
    First,
    Second,
}

#[cfg(test)]
mod blame_tests {
    use super::*;

    #[test]
    fn the_second_output_is_gone_only_if_the_first_kept_playing() {
        assert!(second_stalled(Duration::from_secs(3), 44_100 * 3, 44_100));
        assert!(!second_stalled(Duration::from_secs(3), 0, 44_100), "both stopped: not the second's fault");
        assert!(!second_stalled(Duration::from_secs(1), 44_100, 44_100), "not still long enough");
    }

    #[test]
    fn a_jump_is_blamed_on_the_output_that_fell_behind() {
        assert_eq!(behind(0.03), Output::Second);
        assert_eq!(behind(-0.03), Output::First);
    }
}

/// Watches how much audio an output takes compared with real time. A Bluetooth link in trouble
/// keeps calling back on time but takes less audio (its own buffer is not draining), so over a
/// second it falls clearly short of real time.
#[derive(Debug, Default)]
pub struct RateWatch {
    last: Option<(f64, u64)>,
}

/// Taking less than this share of real time over a second counts as a dropout.
pub const SLOW_SHARE: f64 = 0.93;

impl RateWatch {
    /// Look at the device's frame counter at time `t` (seconds); returns true when the second
    /// that just ended fell short. Counters that went back (a flush) start over.
    pub fn observe(&mut self, t: f64, frames: u64, rate: u32) -> bool {
        let Some((t0, f0)) = self.last else {
            self.last = Some((t, frames));
            return false;
        };
        if frames < f0 || t < t0 {
            self.last = Some((t, frames));
            return false;
        }
        if t - t0 < 1.0 {
            return false;
        }
        self.last = Some((t, frames));
        let share = (frames - f0) as f64 / rate.max(1) as f64 / (t - t0);
        share < SLOW_SHARE
    }

    /// Paused or restarted: the next second starts fresh.
    pub fn reset(&mut self) {
        self.last = None;
    }
}

#[cfg(test)]
mod rate_tests {
    use super::*;

    #[test]
    fn a_device_taking_audio_in_real_time_is_fine() {
        let mut w = RateWatch::default();
        assert!(!w.observe(0.0, 0, 44_100));
        assert!(!w.observe(0.5, 22_050, 44_100), "less than a second: not judged yet");
        assert!(!w.observe(1.0, 44_100, 44_100));
        assert!(!w.observe(2.0, 87_000, 44_100), "1.4 % short is callback jitter");
    }

    #[test]
    fn a_device_falling_short_of_real_time_is_a_dropout() {
        let mut w = RateWatch::default();
        w.observe(0.0, 0, 44_100);
        assert!(w.observe(1.0, 40_100, 44_100), "91 % of real time");
        assert!(!w.observe(2.0, 84_200, 44_100), "back to real time");
    }

    #[test]
    fn a_flush_or_pause_starts_over() {
        let mut w = RateWatch::default();
        w.observe(0.0, 100_000, 44_100);
        assert!(!w.observe(1.0, 2_000, 44_100), "counter went back: flushed");
        w.reset();
        assert!(!w.observe(5.0, 2_000, 44_100));
        assert!(!w.observe(6.0, 46_100, 44_100));
    }
}
