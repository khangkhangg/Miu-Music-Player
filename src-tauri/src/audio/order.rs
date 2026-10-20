//! Play order (redesign): which queue entry comes next, with shuffle and repeat. Queue indices
//! never change, so the song list, ratings and the timeline keep working; only the order does.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

#[derive(Debug, Clone)]
pub struct PlayOrder {
    /// Queue indices in playing order.
    order: Vec<usize>,
    /// Where each queue index sits in `order`.
    pos: Vec<usize>,
    shuffle: bool,
    repeat: Repeat,
    seed: u64,
}

/// xorshift64*: small, seedable, good enough to shuffle a queue.
fn next_random(state: &mut u64) -> u64 {
    let mut x = (*state).max(1);
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    *state = x;
    x.wrapping_mul(0x2545_f491_4f6c_dd1d)
}

impl PlayOrder {
    /// `len` queue entries; with `shuffle`, `start` plays first and the rest follow at random.
    pub fn new(len: usize, start: usize, shuffle: bool, repeat: Repeat, seed: u64) -> Self {
        let mut o = Self { order: Vec::new(), pos: Vec::new(), shuffle, repeat, seed };
        o.rebuild(len, start);
        o
    }

    fn rebuild(&mut self, len: usize, start: usize) {
        let mut order: Vec<usize> = (0..len).collect();
        if self.shuffle && len > 0 {
            let start = start.min(len - 1);
            order.retain(|&i| i != start);
            for i in (1..order.len()).rev() {
                let j = (next_random(&mut self.seed) % (i as u64 + 1)) as usize;
                order.swap(i, j);
            }
            order.insert(0, start);
        }
        self.pos = vec![0; len];
        for (p, &i) in order.iter().enumerate() {
            self.pos[i] = p;
        }
        self.order = order;
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn repeat(&self) -> Repeat {
        self.repeat
    }

    pub fn set_repeat(&mut self, repeat: Repeat) {
        self.repeat = repeat;
    }

    /// Switch shuffle; `current` keeps playing and the order continues from it.
    pub fn set_shuffle(&mut self, on: bool, current: usize) {
        self.shuffle = on;
        next_random(&mut self.seed);
        self.rebuild(self.order.len(), current);
    }

    fn step(&self, current: usize, by: i64) -> Option<usize> {
        let len = self.order.len() as i64;
        let at = *self.pos.get(current)? as i64 + by;
        if (0..len).contains(&at) {
            Some(self.order[at as usize])
        } else if self.repeat != Repeat::Off && len > 0 {
            Some(self.order[at.rem_euclid(len) as usize])
        } else {
            None
        }
    }

    /// What plays when `current` ends by itself.
    pub fn after(&self, current: usize) -> Option<usize> {
        if self.repeat == Repeat::One { Some(current) } else { self.step(current, 1) }
    }

    /// What Next (+1) or Previous (−1) plays: repeat-one does not hold the user back.
    pub fn skip(&self, current: usize, by: i64) -> Option<usize> {
        self.step(current, by)
    }

    /// Up to `n` entries that follow `current` (what Next would play, in turn).
    pub fn upcoming(&self, current: usize, n: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut at = current;
        while out.len() < n.min(self.order.len().saturating_sub(1)) {
            let Some(next) = self.step(at, 1) else { break };
            out.push(next);
            at = next;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_order_it_stops_at_the_end() {
        let o = PlayOrder::new(4, 0, false, Repeat::Off, 1);
        assert_eq!((o.after(0), o.after(2), o.after(3)), (Some(1), Some(3), None));
    }

    #[test]
    fn repeat_all_wraps_and_repeat_one_stays() {
        let all = PlayOrder::new(4, 0, false, Repeat::All, 1);
        assert_eq!(all.after(3), Some(0));
        let one = PlayOrder::new(4, 0, false, Repeat::One, 1);
        assert_eq!(one.after(2), Some(2), "the song ends: play it again");
        assert_eq!(one.skip(2, 1), Some(3), "pressing Next still moves on");
    }

    #[test]
    fn skipping_back_and_forth_follows_the_order() {
        let o = PlayOrder::new(3, 1, false, Repeat::Off, 1);
        assert_eq!((o.skip(1, -1), o.skip(0, -1), o.skip(2, 1)), (Some(0), None, None));
        let all = PlayOrder::new(3, 1, false, Repeat::All, 1);
        assert_eq!((all.skip(0, -1), all.skip(2, 1)), (Some(2), Some(0)));
    }

    #[test]
    fn shuffle_starts_with_the_chosen_song_and_plays_each_once() {
        let o = PlayOrder::new(20, 7, true, Repeat::Off, 42);
        let mut seen = vec![7];
        let mut at = 7;
        while let Some(next) = o.after(at) {
            seen.push(next);
            at = next;
        }
        let mut sorted = seen.clone();
        sorted.sort();
        assert_eq!(sorted, (0..20).collect::<Vec<_>>(), "every song exactly once");
        assert_ne!(seen, (7..20).chain(0..7).collect::<Vec<_>>(), "really shuffled");
        assert_eq!(seen[0], 7);
    }

    #[test]
    fn the_same_seed_gives_the_same_order() {
        let a = PlayOrder::new(30, 0, true, Repeat::Off, 9);
        let b = PlayOrder::new(30, 0, true, Repeat::Off, 9);
        assert_eq!(a.upcoming(0, 29), b.upcoming(0, 29));
    }

    #[test]
    fn turning_shuffle_on_mid_song_keeps_that_song_and_off_goes_back_to_list_order() {
        let mut o = PlayOrder::new(10, 0, false, Repeat::Off, 5);
        o.set_shuffle(true, 4);
        let next = o.upcoming(4, 9);
        assert_eq!(next.len(), 9);
        assert!(!next.contains(&4));
        o.set_shuffle(false, 4);
        assert_eq!(o.after(4), Some(5));
        assert!(!o.shuffle());
    }

    #[test]
    fn up_next_shows_what_comes_and_wraps_with_repeat_all() {
        let o = PlayOrder::new(5, 0, false, Repeat::Off, 1);
        assert_eq!(o.upcoming(3, 4), vec![4]);
        let mut all = PlayOrder::new(5, 0, false, Repeat::All, 1);
        assert_eq!(all.upcoming(3, 4), vec![4, 0, 1, 2]);
        all.set_repeat(Repeat::One);
        assert_eq!(all.upcoming(3, 2), vec![4, 0], "repeat one: what Next would play");
        assert_eq!(all.repeat(), Repeat::One);
    }

    #[test]
    fn repeat_reads_and_writes_as_words() {
        assert_eq!(serde_json::to_string(&Repeat::All).unwrap(), "\"all\"");
        assert_eq!(serde_json::from_str::<Repeat>("\"one\"").unwrap(), Repeat::One);
    }
}
