//! Target-session lifetime state. Windows message/COM work is kept outside
//! this state machine so timeout and close paths never borrow across calls.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RequestDecision {
    Sent(u32),
    Coalesced,
    Pending,
    Backoff,
    Closed,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SessionState {
    pub generation: u64,
    pub context_epoch: u64,
    pub pending_sequence: Option<u32>,
    pub deadline_tick: u64,
    pub next_sequence: u32,
    pub closed: bool,
    pub cancelled: bool,
    pub outstanding_callbacks: u32,
}

impl SessionState {
    pub const MAX_CALLBACKS: u32 = 8;
    pub const DEADLINE_MS: u64 = 150;

    pub const fn new(generation: u64) -> Self {
        Self {
            generation,
            context_epoch: 1,
            pending_sequence: None,
            deadline_tick: 0,
            next_sequence: 1,
            closed: false,
            cancelled: false,
            outstanding_callbacks: 0,
        }
    }

    pub fn request(&mut self, now_tick: u64) -> RequestDecision {
        if self.closed {
            return RequestDecision::Closed;
        }
        if self.pending_sequence.is_some() {
            return RequestDecision::Pending;
        }
        if self.outstanding_callbacks >= Self::MAX_CALLBACKS {
            return RequestDecision::Backoff;
        }
        if self.next_sequence >= u32::MAX - 1 {
            self.closed = true;
            self.cancelled = true;
            return RequestDecision::Unavailable;
        }
        let sequence = self.next_sequence;
        self.next_sequence += 1;
        self.pending_sequence = Some(sequence);
        self.deadline_tick = now_tick.saturating_add(Self::DEADLINE_MS);
        self.cancelled = false;
        self.outstanding_callbacks += 1;
        RequestDecision::Sent(sequence)
    }

    pub fn expire(&mut self, now_tick: u64) {
        if self.pending_sequence.is_some() && now_tick > self.deadline_tick {
            self.cancelled = true;
        }
    }

    pub fn complete(
        &mut self,
        sequence: u32,
        generation: u64,
        context_epoch: u64,
        now_tick: u64,
    ) -> bool {
        if self.pending_sequence != Some(sequence) {
            return false;
        }
        let accepted = !self.closed
            && !self.cancelled
            && generation == self.generation
            && context_epoch == self.context_epoch
            && now_tick <= self.deadline_tick;
        self.pending_sequence = None;
        accepted
    }

    pub fn callback_released(&mut self) -> bool {
        if self.outstanding_callbacks == 0 {
            return false;
        }
        self.outstanding_callbacks -= 1;
        true
    }

    pub fn replace_context(&mut self) {
        self.context_epoch = self.context_epoch.wrapping_add(1).max(1);
        self.cancelled = true;
    }

    pub fn close(&mut self) {
        self.closed = true;
        self.cancelled = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_pending_request_and_timeout_keep_callback_bounded() {
        let mut state = SessionState::new(1);
        assert_eq!(state.request(0), RequestDecision::Sent(1));
        assert_eq!(state.request(1), RequestDecision::Pending);
        state.expire(200);
        assert_eq!(state.request(200), RequestDecision::Pending);
        assert_eq!(state.outstanding_callbacks, 1);
    }

    #[test]
    fn stale_wrong_epoch_generation_or_close_cannot_accept_or_clear_newer_work() {
        let mut state = SessionState::new(1);
        assert_eq!(state.request(0), RequestDecision::Sent(1));
        assert!(!state.complete(99, 1, 1, 10));
        assert_eq!(state.pending_sequence, Some(1));
        state.replace_context();
        assert!(!state.complete(1, 1, 1, 10));
        assert_eq!(state.pending_sequence, None);

        let mut state = SessionState::new(1);
        assert_eq!(state.request(0), RequestDecision::Sent(1));
        assert!(!state.complete(1, 2, 1, 10));
        assert_eq!(state.pending_sequence, None);

        let mut state = SessionState::new(1);
        assert_eq!(state.request(0), RequestDecision::Sent(1));
        state.close();
        assert!(!state.complete(1, 1, 1, 10));
        assert_eq!(state.request(20), RequestDecision::Closed);
    }

    #[test]
    fn callback_release_is_separate_and_underflow_is_rejected() {
        let mut state = SessionState::new(1);
        assert!(!state.callback_released());
        assert_eq!(state.request(0), RequestDecision::Sent(1));
        assert!(state.complete(1, 1, 1, 10));
        assert!(state.callback_released());
        assert!(!state.callback_released());
    }

    #[test]
    fn cap_and_rollover_close_without_allocating_more_callbacks() {
        let mut state = SessionState::new(1);
        state.outstanding_callbacks = SessionState::MAX_CALLBACKS;
        assert_eq!(state.request(0), RequestDecision::Backoff);
        state.outstanding_callbacks = 0;
        state.next_sequence = u32::MAX - 1;
        assert_eq!(state.request(0), RequestDecision::Unavailable);
        assert!(state.closed);
    }
}
