//! Pure confirmation policy. No provider calls or event delivery on the hook.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum KeyDecision {
    PassToTarget,
    PassToVerifiedIme,
    ConsumeOnly,
    ConsumeAndConfirm,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct KeyState {
    pub guarded: bool,
    pub consumed_sequence: bool,
    pub ime_sequence: bool,
    pub down: bool,
    pub repeat: bool,
    pub modified: bool,
    pub composing: bool,
    pub ready: bool,
}

pub(super) fn decide_key(s: KeyState) -> KeyDecision {
    if s.consumed_sequence {
        return KeyDecision::ConsumeOnly;
    }
    if s.ime_sequence {
        return if s.down {
            KeyDecision::ConsumeOnly
        } else {
            KeyDecision::PassToVerifiedIme
        };
    }
    if !s.guarded || !s.down {
        return KeyDecision::PassToTarget;
    }
    if s.modified || s.repeat {
        return KeyDecision::ConsumeOnly;
    }
    if s.composing {
        return KeyDecision::PassToVerifiedIme;
    }
    if s.ready {
        KeyDecision::ConsumeAndConfirm
    } else {
        KeyDecision::ConsumeOnly
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state() -> KeyState {
        KeyState {
            guarded: true,
            consumed_sequence: false,
            ime_sequence: false,
            down: true,
            repeat: false,
            modified: false,
            composing: false,
            ready: false,
        }
    }
    #[test]
    fn filtering_empty_unknown_and_busy_are_protected() {
        assert_eq!(decide_key(state()), KeyDecision::ConsumeOnly);
        assert_eq!(
            decide_key(KeyState {
                ready: true,
                ..state()
            }),
            KeyDecision::ConsumeAndConfirm
        );
    }
    #[test]
    fn modified_enter_never_leaks_even_during_composition() {
        for ready in [false, true] {
            for composing in [false, true] {
                assert_eq!(
                    decide_key(KeyState {
                        modified: true,
                        ready,
                        composing,
                        ..state()
                    }),
                    KeyDecision::ConsumeOnly
                );
            }
        }
    }
    #[test]
    fn retiring_key_owns_repeat_and_release_after_session_ends() {
        for down in [false, true] {
            assert_eq!(
                decide_key(KeyState {
                    guarded: false,
                    consumed_sequence: true,
                    down,
                    ..state()
                }),
                KeyDecision::ConsumeOnly
            );
        }
        assert_eq!(
            decide_key(KeyState {
                guarded: false,
                ..state()
            }),
            KeyDecision::PassToTarget
        );
    }
    #[test]
    fn ime_gets_only_first_down_and_matching_release() {
        assert_eq!(
            decide_key(KeyState {
                composing: true,
                ..state()
            }),
            KeyDecision::PassToVerifiedIme
        );
        assert_eq!(
            decide_key(KeyState {
                ime_sequence: true,
                repeat: true,
                ..state()
            }),
            KeyDecision::ConsumeOnly
        );
        assert_eq!(
            decide_key(KeyState {
                ime_sequence: true,
                down: false,
                ..state()
            }),
            KeyDecision::PassToVerifiedIme
        );
    }

    #[test]
    fn thousand_modified_and_ime_confirmation_lifecycles() {
        // Modifier bits represent Control/Shift/Alt combinations. Physical
        // message/extended-key translation is covered by the callback suite.
        for iteration in 0..1000 {
            let modified = iteration % 8 != 0;
            let composing = iteration % 3 == 0;
            let ready = iteration % 5 == 0;
            let first = decide_key(KeyState {
                modified,
                composing,
                ready,
                ..state()
            });
            let ime_owns = composing && !modified;
            assert_eq!(first == KeyDecision::PassToVerifiedIme, ime_owns);
            assert_ne!(first, KeyDecision::PassToTarget);
            if modified || (!ready && !ime_owns) {
                assert_eq!(first, KeyDecision::ConsumeOnly);
            }
            let sequence = KeyState {
                consumed_sequence: !ime_owns,
                ime_sequence: ime_owns,
                guarded: false,
                repeat: true,
                modified,
                composing,
                ready,
                ..state()
            };
            assert_eq!(decide_key(sequence), KeyDecision::ConsumeOnly);
            assert_eq!(
                decide_key(KeyState {
                    down: false,
                    ..sequence
                }),
                if ime_owns {
                    KeyDecision::PassToVerifiedIme
                } else {
                    KeyDecision::ConsumeOnly
                }
            );
            assert_eq!(
                decide_key(KeyState {
                    guarded: false,
                    ..state()
                }),
                KeyDecision::PassToTarget
            );
        }
    }
}
