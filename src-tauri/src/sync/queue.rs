//! The sync queue (F4.3): steps run one at a time; pause stops after the current step, cancel
//! stops for good, and resuming (even in a later session) continues with the next unfinished step.

use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Running,
    Paused,
    Cancelled,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub done: usize,
    pub failed: usize,
    pub total: usize,
}

/// Steps (by id) waiting, in order; failed ones go to the back for a later retry.
#[derive(Debug)]
pub struct Queue<T> {
    pending: VecDeque<T>,
    current: Option<T>,
    failed: Vec<T>,
    done: usize,
    total: usize,
    paused: bool,
    cancelled: bool,
}

impl<T: Copy + PartialEq> Queue<T> {
    pub fn new(steps: Vec<T>) -> Self {
        Self { total: steps.len(), pending: steps.into(), current: None, failed: Vec::new(), done: 0, paused: false, cancelled: false }
    }

    /// The next step to run, or `None` when paused, cancelled or finished.
    pub fn next(&mut self) -> Option<T> {
        if self.paused || self.cancelled {
            return None;
        }
        self.current = self.pending.pop_front();
        self.current
    }

    pub fn finish(&mut self, step: T, ok: bool) {
        if self.current == Some(step) {
            self.current = None;
        }
        self.pending.retain(|s| *s != step);
        if ok {
            self.done += 1;
        } else {
            self.failed.push(step);
        }
    }

    /// Stop after the step that is running now.
    pub fn pause(&mut self) {
        self.paused = true;
    }

    pub fn resume(&mut self) {
        self.paused = false;
    }

    pub fn cancel(&mut self) {
        self.cancelled = true;
    }

    pub fn state(&self) -> State {
        if self.cancelled {
            State::Cancelled
        } else if self.pending.is_empty() && self.current.is_none() {
            State::Done
        } else if self.paused {
            State::Paused
        } else {
            State::Running
        }
    }

    /// What is left, to save and resume later: untried steps first, then failed ones.
    pub fn unfinished(&self) -> Vec<T> {
        self.current.iter().chain(&self.pending).chain(&self.failed).copied().collect()
    }

    pub fn progress(&self) -> Progress {
        Progress { done: self.done, failed: self.failed.len(), total: self.total }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_come_in_order_until_none_are_left() {
        let mut q = Queue::new(vec![1, 2, 3]);
        assert_eq!(q.next(), Some(1));
        q.finish(1, true);
        assert_eq!(q.next(), Some(2));
        q.finish(2, true);
        q.finish(3, true);
        assert_eq!(q.next(), None);
        assert_eq!(q.state(), State::Done);
    }

    #[test]
    fn pause_holds_and_resume_continues_with_the_next_unfinished_step() {
        let mut q = Queue::new(vec![1, 2, 3]);
        q.next();
        q.finish(1, true);
        q.pause();
        assert_eq!(q.next(), None, "paused");
        assert_eq!(q.state(), State::Paused);
        q.resume();
        assert_eq!(q.next(), Some(2));
    }

    #[test]
    fn a_saved_queue_resumes_where_it_stopped() {
        let mut q = Queue::new(vec![1, 2, 3]);
        q.next();
        q.finish(1, true);
        q.next();
        q.finish(2, false); // failed: retried after the rest
        let saved = q.unfinished();
        assert_eq!(saved, vec![3, 2]);
        let mut again = Queue::new(saved);
        assert_eq!(again.next(), Some(3));
    }

    #[test]
    fn cancel_stops_for_good() {
        let mut q = Queue::new(vec![1, 2]);
        q.cancel();
        q.resume();
        assert_eq!(q.next(), None);
        assert_eq!(q.state(), State::Cancelled);
    }

    #[test]
    fn progress_counts_finished_steps() {
        let mut q = Queue::new(vec![1, 2, 3, 4]);
        q.next();
        q.finish(1, true);
        q.next();
        q.finish(2, false);
        assert_eq!(q.progress(), Progress { done: 1, failed: 1, total: 4 });
    }
}
