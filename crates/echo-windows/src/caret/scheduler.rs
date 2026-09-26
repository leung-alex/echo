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

    /// Adopt an epoch reported by the target runtime.  Epoch changes are
    /// invalidations, never evidence that a ready result is current.  The
    /// next request carries the adopted value and the target revalidates it
    /// against its canonical TSF identities.
    pub fn observe_context_epoch(&mut self, epoch: u64) -> bool {
        if epoch == 0 || epoch == self.context_epoch {
            return false;
        }
        self.context_epoch = epoch;
        self.cancelled = true;
        true
    }

    /// Synchronize the host-side diagnostic count with the count published by
    /// the target callback runtime.  The reader must not infer a COM Release
    /// merely because a response arrived.
    pub fn sync_outstanding_callbacks(&mut self, count: u32) {
        self.outstanding_callbacks = count.min(Self::MAX_CALLBACKS);
    }

    /// A host focus generation can advance while the same HWND, process and
    /// TSF document remain active. Rebind the host-side acceptance guard
    /// without tearing down the target scheduler or its COM context.
    pub fn rebind_generation(&mut self, generation: u64) {
        if generation != 0 && generation != self.generation {
            self.generation = generation;
            self.cancelled = true;
        }
    }

    pub fn replace_context(&mut self) {
        self.context_epoch = self.context_epoch.wrapping_add(1).max(1);
        self.cancelled = true;
    }

    pub fn close(&mut self) {
        self.closed = true;
        self.cancelled = true;
    }

    pub fn mark_target_closed(&mut self) {
        self.closed = true;
        self.cancelled = true;
        self.pending_sequence = None;
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
    fn target_closed_clears_pending_but_keeps_terminal_session_state() {
        let mut state = SessionState::new(1);
        assert_eq!(state.request(0), RequestDecision::Sent(1));
        state.mark_target_closed();
        assert!(state.closed);
        assert!(state.cancelled);
        assert_eq!(state.pending_sequence, None);
        assert_eq!(state.request(20), RequestDecision::Closed);
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
