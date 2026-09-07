//! Focus probing is independent of storage and image decoding. No retained session
//! is changed here; the UI accepts the epoch and queues adoption before any Execute.
use crate::events::{ActivationResult, Event, Hub};
use echo_presentation::session::Context;
use echo_windows::focus::FocusSnapshot;
use std::sync::{
    atomic::{AtomicU64, Ordering},
    mpsc::{self, SyncSender},
    Arc,
};
use std::thread::JoinHandle;
struct Request {
    epoch: u64,
    context: Context,
    snapshot: Option<FocusSnapshot>,
}
pub(super) struct CaptureLane {
    sender: Option<SyncSender<Request>>,
    thread: Option<JoinHandle<()>>,
}
impl CaptureLane {
    pub(super) fn start(hub: Arc<Hub>, epoch: Arc<AtomicU64>) -> Result<Self, String> {
        let (sender, receiver) = mpsc::sync_channel::<Request>(1);
        let thread = std::thread::Builder::new()
            .name("echo-focus-capture".into())
            .spawn(move || {
                while let Ok(request) = receiver.recv() {
                    if request.epoch != epoch.load(Ordering::Acquire) {
                        continue;
                    }
                    let captured = if request.context == Context::QuickInsert {
                        request.snapshot.as_ref().map(FocusSnapshot::capture_target)
                    } else {
                        None
                    };
                    if request.epoch != epoch.load(Ordering::Acquire) {
                        continue;
                    }
                    let anchor = captured.as_ref().map(|value| value.anchor);
                    let target = captured.and_then(|value| value.target);
                    hub.post(Event::Activated(
                        request.epoch,
                        request.context,
                        Ok(ActivationResult { target, anchor }),
                    ));
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            sender: Some(sender),
            thread: Some(thread),
        })
    }
    pub(super) fn submit(
        &self,
        epoch: u64,
        context: Context,
        snapshot: Option<FocusSnapshot>,
    ) -> Result<(), String> {
        self.sender
            .as_ref()
            .ok_or("Focus capture has stopped")?
            .try_send(Request {
                epoch,
                context,
                snapshot,
            })
            .map_err(|_| "Focus capture is busy; press the shortcut again".into())
    }
    pub(super) fn stop(&mut self) {
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Drop for CaptureLane {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_lane_does_not_need_storage_to_complete_activation() {
        let hub = Arc::new(Hub::default());
        let epoch = Arc::new(AtomicU64::new(7));
        let mut lane = CaptureLane::start(hub.clone(), epoch).unwrap();
        lane.submit(7, Context::Manager, None).unwrap();
        lane.stop();
        let events = hub.take_test_events();
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], Event::Activated(7, Context::Manager, Ok(result)) if result.target.is_none())
        );
        assert!(lane.submit(8, Context::Manager, None).is_err());
    }
    #[test]
    fn cancelled_epoch_cannot_publish_or_adopt_a_target() {
        let hub = Arc::new(Hub::default());
        let mut lane = CaptureLane::start(hub.clone(), Arc::new(AtomicU64::new(8))).unwrap();
        lane.submit(7, Context::Manager, None).unwrap();
        lane.stop();
        assert!(hub.take_test_events().is_empty());
    }
}
