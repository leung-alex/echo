use super::{IME_ACTIVE, IME_UNKNOWN};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum CompositionSource {
    TargetRead,
    TargetThread,
    TextEditEvent,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct CompositionEvidence {
    pub state: u8,
    pub source: CompositionSource,
    pub session: u64,
    pub input_serial: u64,
    pub observed_at: Instant,
}
impl CompositionEvidence {
    pub fn state_at(self, session: u64, serial: u64, now: Instant) -> u8 {
        if self.session != session
            || self.input_serial != serial
            || (self.state == IME_ACTIVE
                && now.saturating_duration_since(self.observed_at) > Duration::from_millis(250))
        {
            IME_UNKNOWN
        } else {
            self.state
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn active_evidence_expires_and_cannot_cross_input_or_session() {
        let now = Instant::now();
        let e = CompositionEvidence {
            state: IME_ACTIVE,
            source: CompositionSource::TargetRead,
            session: 1,
            input_serial: 2,
            observed_at: now,
        };
        assert_eq!(e.state_at(1, 2, now), IME_ACTIVE);
        assert_eq!(e.state_at(2, 2, now), IME_UNKNOWN);
        assert_eq!(e.state_at(1, 3, now), IME_UNKNOWN);
        assert_eq!(
            e.state_at(1, 2, now + Duration::from_millis(251)),
            IME_UNKNOWN
        );
    }
}
