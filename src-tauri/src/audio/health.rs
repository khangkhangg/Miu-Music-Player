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
