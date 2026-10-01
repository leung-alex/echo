use super::{IME_ACTIVE, IME_UNKNOWN};
use std::time::{Duration, Instant};

/// Search-only spelling for verified Chinese preedit. Never apply to committed
/// text or use the result to measure a replacement range.
pub(super) fn pinyin_search_text(preedit: &str, chinese: bool) -> String {
    let chars: Vec<char> = preedit.chars().collect();
    chars
        .iter()
        .enumerate()
        .filter_map(|(i, &c)| {
            let separator = chinese
                && c == '\''
                && i > 0
                && chars[i - 1].is_ascii_alphabetic()
                && chars.get(i + 1).is_some_and(char::is_ascii_alphabetic);
            (!separator).then_some(c)
        })
        .collect()
}

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
    pub fn can_publish(self, session: u64, serial: u64, previous: Option<Self>) -> bool {
        self.session == session
            && self.input_serial == serial
            && previous
                .is_none_or(|old| old.session != session || old.observed_at <= self.observed_at)
    }
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

    /// Plain-paste confirmation uses a short lease for every observed state.
    /// Inline range inspection keeps its historical Active-only expiry policy;
    /// this separate policy prevents an old Clear sample from authorizing a
    /// later Enter after the worker or event queue was stalled.
    pub fn plain_state_at(self, session: u64, serial: u64, now: Instant, lease: Duration) -> u8 {
        if self.session != session
            || self.input_serial != serial
            || now.saturating_duration_since(self.observed_at) > lease
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
    fn finalized_event_cannot_replace_a_newer_target_read_or_cross_sessions() {
        let now = Instant::now();
        let ended = CompositionEvidence {
            state: super::super::IME_CLEAR,
            source: CompositionSource::TargetThread,
            session: 3,
            input_serial: 7,
            observed_at: now + Duration::from_millis(1),
        };
        let late_event = CompositionEvidence {
            state: IME_UNKNOWN,
            source: CompositionSource::TextEditEvent,
            observed_at: now,
            ..ended
        };
        assert!(!late_event.can_publish(3, 7, Some(ended)));
        assert!(!late_event.can_publish(4, 7, None));
        assert!(!late_event.can_publish(3, 8, None));
        assert!(ended.can_publish(3, 7, Some(late_event)));
        let new_composition = CompositionEvidence {
            state: IME_ACTIVE,
            input_serial: 8,
            observed_at: now + Duration::from_millis(2),
            ..late_event
        };
        assert!(new_composition.can_publish(3, 8, Some(ended)));
    }
    #[test]
    fn repeated_composition_lifecycles_recover_clear_without_reusing_evidence() {
        let now = Instant::now();
        for session in 1..=100 {
            let active = CompositionEvidence {
                state: IME_ACTIVE,
                source: CompositionSource::TargetThread,
                session,
                input_serial: 2,
                observed_at: now,
            };
            let clear = CompositionEvidence {
                state: super::super::IME_CLEAR,
                input_serial: 3,
                observed_at: now + Duration::from_millis(1),
                ..active
            };
            assert_eq!(active.state_at(session, 2, now), IME_ACTIVE);
            assert_eq!(active.state_at(session, 3, now), IME_UNKNOWN);
            assert!(clear.can_publish(session, 3, Some(active)));
            assert_eq!(clear.state_at(session, 3, now), super::super::IME_CLEAR);
            assert_eq!(clear.state_at(session + 1, 3, now), IME_UNKNOWN);
        }
    }
    #[test]
    fn pinyin_separators_are_search_only_and_language_scoped() {
        for (input, expected) in [
            ("e", "e"),
            ("e'ch", "ech"),
            ("w'he", "whe"),
            ("w'hen", "when"),
            ("p'ro", "pro"),
            ("r'e'u", "reu"),
            ("echo", "echo"),
            ("xi'an", "xian"),
            ("'ech'", "'ech'"),
            ("e''ch", "e''ch"),
            ("e’ ch", "e’ ch"),
            ("你'好", "你'好"),
            ("e'1", "e'1"),
        ] {
            assert_eq!(pinyin_search_text(input, true), expected);
            assert_eq!(pinyin_search_text(input, false), input);
        }
    }
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

    #[test]
    fn plain_clear_evidence_expires_and_a_new_clear_recovers() {
        let now = Instant::now();
        let clear = CompositionEvidence {
            state: super::super::IME_CLEAR,
            source: CompositionSource::TargetThread,
            session: 5,
            input_serial: 9,
            observed_at: now,
        };
        let lease = Duration::from_millis(100);
        assert_eq!(
            clear.plain_state_at(5, 9, now, lease),
            super::super::IME_CLEAR
        );
        assert_eq!(
            clear.plain_state_at(5, 9, now + Duration::from_millis(101), lease),
            IME_UNKNOWN
        );
        let refreshed = CompositionEvidence {
            observed_at: now + Duration::from_millis(102),
            ..clear
        };
        assert!(refreshed.can_publish(5, 9, Some(clear)));
        assert_eq!(
            refreshed.plain_state_at(5, 9, now + Duration::from_millis(102), lease),
            super::super::IME_CLEAR
        );
    }

    #[test]
    fn plain_active_and_unknown_never_turn_into_clear_without_new_evidence() {
        let now = Instant::now();
        for state in [super::super::IME_ACTIVE, super::super::IME_UNKNOWN] {
            let evidence = CompositionEvidence {
                state,
                source: CompositionSource::TargetThread,
                session: 6,
                input_serial: 10,
                observed_at: now,
            };
            assert_eq!(
                evidence.plain_state_at(6, 10, now, Duration::from_millis(100)),
                state
            );
            assert_ne!(
                evidence.plain_state_at(
                    6,
                    10,
                    now + Duration::from_millis(101),
                    Duration::from_millis(100)
                ),
                super::super::IME_CLEAR
            );
        }
    }
}
