//! The F2.3 handshake as a pure state machine: wait for `SyncAllowed` → send HostInfo → after 3 s
//! send the Media SyncRequest → stop at `ReadyForSync` / `SyncFailed` / timeout. It never asks for
//! anything beyond that (no metadata, no files). Times are ms since the connection was created.

const SYNC_ALLOWED_TIMEOUT_MS: u64 = 10_000;
const HOST_INFO_SETTLE_MS: u64 = 3_000;
const READY_QUIET_TIMEOUT_MS: u64 = 15_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    WaitingForSyncAllowed,
    WaitingForReadyForSync,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    ReadyForSync,
    SyncFailed,
    Timeout(Phase),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Wait,
    SendHostInfo,
    SendSyncRequest,
    Stop(Outcome),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    WaitingForSyncAllowed { since: u64 },
    HostInfoSent { at: u64 },
    RequestSent { last_message: u64 },
    Done(Outcome),
}

#[derive(Debug)]
pub struct Handshake {
    state: State,
}

impl Handshake {
    pub fn new(now_ms: u64) -> Self {
        Self { state: State::WaitingForSyncAllowed { since: now_ms } }
    }

    pub fn on_message(&mut self, name: &str, now_ms: u64) -> Action {
        if let State::Done(outcome) = self.state {
            return Action::Stop(outcome);
        }
        if name == "SyncFailed" {
            return self.finish(Outcome::SyncFailed);
        }
        match (self.state, name) {
            (State::WaitingForSyncAllowed { .. }, "SyncAllowed") => {
                self.state = State::HostInfoSent { at: now_ms };
                Action::SendHostInfo
            }
            (State::RequestSent { .. }, "ReadyForSync") => self.finish(Outcome::ReadyForSync),
            (State::RequestSent { .. }, _) => {
                self.state = State::RequestSent { last_message: now_ms };
                Action::Wait
            }
            _ => Action::Wait,
        }
    }

    pub fn on_tick(&mut self, now_ms: u64) -> Action {
        match self.state {
            State::Done(outcome) => Action::Stop(outcome),
            State::WaitingForSyncAllowed { since } if now_ms >= since + SYNC_ALLOWED_TIMEOUT_MS => {
                self.finish(Outcome::Timeout(Phase::WaitingForSyncAllowed))
            }
            State::HostInfoSent { at } if now_ms >= at + HOST_INFO_SETTLE_MS => {
                self.state = State::RequestSent { last_message: now_ms };
                Action::SendSyncRequest
            }
            State::RequestSent { last_message } if now_ms >= last_message + READY_QUIET_TIMEOUT_MS => {
                self.finish(Outcome::Timeout(Phase::WaitingForReadyForSync))
            }
            _ => Action::Wait,
        }
    }

    fn finish(&mut self, outcome: Outcome) -> Action {
        self.state = State::Done(outcome);
        Action::Stop(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_for_sync_allowed_before_sending_anything() {
        let mut h = Handshake::new(0);
        assert_eq!(h.on_message("InstalledAssets", 230), Action::Wait);
        assert_eq!(h.on_message("AssetMetrics", 231), Action::Wait);
        assert_eq!(h.on_tick(5_000), Action::Wait);
    }

    #[test]
    fn sync_allowed_triggers_host_info() {
        let mut h = Handshake::new(0);
        assert_eq!(h.on_message("SyncAllowed", 250), Action::SendHostInfo);
    }

    #[test]
    fn sync_request_follows_three_seconds_after_host_info() {
        let mut h = Handshake::new(0);
        h.on_message("SyncAllowed", 250);
        assert_eq!(h.on_tick(3_249), Action::Wait);
        assert_eq!(h.on_tick(3_250), Action::SendSyncRequest);
        assert_eq!(h.on_tick(3_300), Action::Wait, "sent only once");
    }

    #[test]
    fn ready_for_sync_after_the_request_ends_the_handshake() {
        let mut h = Handshake::new(0);
        h.on_message("SyncAllowed", 250);
        h.on_tick(3_250);
        assert_eq!(h.on_message("ReadyForSync", 4_000), Action::Stop(Outcome::ReadyForSync));
    }

    #[test]
    fn sync_failed_stops_at_any_point() {
        let mut h = Handshake::new(0);
        assert_eq!(h.on_message("SyncFailed", 100), Action::Stop(Outcome::SyncFailed));
        let mut h = Handshake::new(0);
        h.on_message("SyncAllowed", 250);
        h.on_tick(3_250);
        assert_eq!(h.on_message("SyncFailed", 3_500), Action::Stop(Outcome::SyncFailed));
    }

    #[test]
    fn no_sync_allowed_within_ten_seconds_times_out() {
        let mut h = Handshake::new(0);
        assert_eq!(h.on_tick(9_999), Action::Wait);
        assert_eq!(h.on_tick(10_000), Action::Stop(Outcome::Timeout(Phase::WaitingForSyncAllowed)));
    }

    #[test]
    fn fifteen_quiet_seconds_after_the_request_time_out_and_messages_reset_the_clock() {
        let mut h = Handshake::new(0);
        h.on_message("SyncAllowed", 250);
        h.on_tick(3_250);
        assert_eq!(h.on_message("Ping", 10_000), Action::Wait);
        assert_eq!(h.on_tick(24_999), Action::Wait);
        assert_eq!(h.on_tick(25_000), Action::Stop(Outcome::Timeout(Phase::WaitingForReadyForSync)));
    }

    #[test]
    fn ready_for_sync_before_our_request_is_only_logged() {
        let mut h = Handshake::new(0);
        h.on_message("SyncAllowed", 250);
        assert_eq!(h.on_message("ReadyForSync", 1_000), Action::Wait);
    }

    #[test]
    fn after_stopping_it_stays_stopped() {
        let mut h = Handshake::new(0);
        h.on_message("SyncFailed", 100);
        assert_eq!(h.on_message("SyncAllowed", 200), Action::Stop(Outcome::SyncFailed));
        assert_eq!(h.on_tick(99_999), Action::Stop(Outcome::SyncFailed));
    }
}
