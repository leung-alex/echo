//! The hidden timer is advisory: a queued timer must never retire a new session
//! or an input transaction that temporarily hides its surface.
#[derive(Default)]
pub(crate) struct ReclaimBarrier {
    pub epoch: u64,
    pub visible: bool,
    pub capture_pending: bool,
    pub inserting: bool,
    pub inline_pending: bool,
    pub inline_active: bool,
    pub mutating: bool,
    pub modal: bool,
}

impl ReclaimBarrier {
    pub fn permits(&self, requested_epoch: u64) -> bool {
        self.epoch == requested_epoch
            && !self.visible
            && !self.capture_pending
            && !self.inserting
            && !self.inline_pending
            && !self.inline_active
            && !self.mutating
            && !self.modal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queued_hidden_timer_cannot_retire_a_new_activation() {
        let barrier = ReclaimBarrier {
            epoch: 9,
            ..Default::default()
        };
        assert!(!barrier.permits(8));
        assert!(barrier.permits(9));
    }

    #[test]
    fn completing_an_insert_reopens_reclamation_only_for_its_hidden_session() {
        let mut barrier = ReclaimBarrier {
            epoch: 9,
            inserting: true,
            ..Default::default()
        };
        assert!(!barrier.permits(9));
        barrier.inserting = false;
        assert!(barrier.permits(9));
        barrier.epoch = 10;
        assert!(!barrier.permits(9));
        barrier.visible = true;
        assert!(!barrier.permits(10));
    }

    #[test]
    fn temporary_hidden_input_transactions_block_reclamation() {
        for barrier in [
            ReclaimBarrier {
                capture_pending: true,
                ..Default::default()
            },
            ReclaimBarrier {
                inserting: true,
                ..Default::default()
            },
            ReclaimBarrier {
                inline_pending: true,
                ..Default::default()
            },
            ReclaimBarrier {
                inline_active: true,
                ..Default::default()
            },
            ReclaimBarrier {
                mutating: true,
                ..Default::default()
            },
            ReclaimBarrier {
                modal: true,
                ..Default::default()
            },
            ReclaimBarrier {
                visible: true,
                ..Default::default()
            },
        ] {
            assert!(!barrier.permits(0));
        }
    }
}
