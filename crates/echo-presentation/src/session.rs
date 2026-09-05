//! Explicit session epochs distinguish dismissal from a temporary hide for paste.
use echo_engine::{QuickInsertAction, QuickInsertOutcome};
use std::collections::VecDeque;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Context {
    Manager,
    QuickInsert,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Operation {
    pub epoch: u64,
    pub serial: u64,
    pub action: QuickInsertAction,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Completion {
    Stale,
    Inserted,
    Copied,
    Staged,
    Restore,
}
pub struct Session {
    pub context: Context,
    pub has_target: bool,
    pub epoch: u64,
    serial: u64,
    pending: Option<Operation>,
}
impl Default for Session {
    fn default() -> Self {
        Self {
            context: Context::Manager,
            has_target: false,
            epoch: 0,
            serial: 0,
            pending: None,
        }
    }
}
impl Session {
    pub fn activate(&mut self, context: Context) -> u64 {
        self.epoch = self.epoch.wrapping_add(1);
        self.context = context;
        self.has_target = false;
        self.pending = None;
        self.epoch
    }
    pub fn capture_finished(&mut self, epoch: u64, has_target: bool) -> bool {
        if epoch != self.epoch {
            return false;
        }
        self.has_target = has_target;
        true
    }
    pub fn dismiss(&mut self) {
        self.activate(Context::Manager);
    }
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn begin(&mut self, action: QuickInsertAction) -> Option<Operation> {
        if self.pending.is_some()
            || (action == QuickInsertAction::Insert && self.context != Context::QuickInsert)
        {
            return None;
        }
        self.serial = self.serial.wrapping_add(1);
        let op = Operation {
            epoch: self.epoch,
            serial: self.serial,
            action,
        };
        self.pending = Some(op);
        Some(op)
    }
    pub fn finish(&mut self, op: Operation, outcome: Result<QuickInsertOutcome, ()>) -> Completion {
        if self.pending != Some(op) || op.epoch != self.epoch {
            return Completion::Stale;
        }
        self.pending = None;
        match outcome {
            Ok(QuickInsertOutcome::Inserted) => {
                self.has_target = false;
                Completion::Inserted
            }
            Ok(QuickInsertOutcome::Copied) => Completion::Copied,
            Ok(QuickInsertOutcome::ClipboardStaged) => {
                self.has_target = false;
                Completion::Staged
            }
            Err(()) => Completion::Restore,
        }
    }
}
pub struct RecentActivations {
    ids: VecDeque<String>,
    capacity: usize,
}
impl Default for RecentActivations {
    fn default() -> Self {
        Self {
            ids: VecDeque::new(),
            capacity: 128,
        }
    }
}
impl RecentActivations {
    pub fn admit(&mut self, id: &str) -> bool {
        if id.is_empty() || id.len() > 256 || self.ids.iter().any(|x| x == id) {
            return false;
        }
        if self.ids.len() == self.capacity {
            self.ids.pop_front();
        }
        self.ids.push_back(id.into());
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manager_never_inserts() {
        assert!(Session::default()
            .begin(QuickInsertAction::Insert)
            .is_none());
    }
    #[test]
    fn repeat_enter_is_single_flight() {
        let mut s = Session::default();
        s.activate(Context::QuickInsert);
        assert!(s.begin(QuickInsertAction::Insert).is_some());
        assert!(s.begin(QuickInsertAction::Insert).is_none());
    }
    #[test]
    fn dismiss_invalidates_late_failure() {
        let mut s = Session::default();
        s.activate(Context::QuickInsert);
        let t = s.begin(QuickInsertAction::Insert).unwrap();
        s.dismiss();
        assert_eq!(s.finish(t, Err(())), Completion::Stale);
    }
    #[test]
    fn failure_restores_without_losing_target() {
        let mut s = Session::default();
        let e = s.activate(Context::QuickInsert);
        s.capture_finished(e, true);
        let t = s.begin(QuickInsertAction::Insert).unwrap();
        assert_eq!(s.finish(t, Err(())), Completion::Restore);
        assert!(s.has_target);
        assert!(!s.busy());
    }
    #[test]
    fn insertion_success_clears_target() {
        let mut s = Session::default();
        let e = s.activate(Context::QuickInsert);
        s.capture_finished(e, true);
        let t = s.begin(QuickInsertAction::Insert).unwrap();
        assert_eq!(
            s.finish(t, Ok(QuickInsertOutcome::Inserted)),
            Completion::Inserted
        );
        assert!(!s.has_target);
    }
    #[test]
    fn late_capture_is_ignored() {
        let mut s = Session::default();
        let e = s.activate(Context::QuickInsert);
        s.dismiss();
        assert!(!s.capture_finished(e, true));
    }
    #[test]
    fn copy_staged_does_not_claim_insertion() {
        let mut s = Session::default();
        let t = s.begin(QuickInsertAction::Copy).unwrap();
        assert_eq!(
            s.finish(t, Ok(QuickInsertOutcome::ClipboardStaged)),
            Completion::Staged
        );
    }
    #[test]
    fn request_dedup_is_bounded() {
        let mut r = RecentActivations::default();
        assert!(r.admit("first"));
        assert!(!r.admit("first"));
        for n in 0..500 {
            assert!(r.admit(&n.to_string()));
        }
        assert_eq!(r.ids.len(), 128);
        assert!(r.admit("first"));
        assert!(!r.admit(""));
    }
}
