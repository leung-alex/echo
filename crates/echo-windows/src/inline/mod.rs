//! Global, session-scoped completion. Only the activated input is observed;
//! surrounding text stays on the MTA worker and is never logged or persisted.
use crate::focus::{FocusSnapshot, PopupAnchor};
use echo_engine::{
    ClipboardRepresentation, InlineTicket, PasteDelivery, PasteDeliveryFailure, PasteTarget,
    QueryRange,
};
use std::sync::{
    atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, AtomicU8, Ordering},
    mpsc::{self, SyncSender},
    Arc, Mutex,
};
use std::time::{Duration, Instant};
mod composition;
#[cfg(feature = "native-test")]
pub mod diagnostics;
mod ime_observer;
mod ime_window;
mod key_policy;
mod keyboard;
mod payload;
mod subscriptions;
mod target;
mod text_scope;

pub struct InlineStarted {
    pub ticket: InlineTicket,
    pub query: String,
    pub target: PasteTarget,
    pub anchor: PopupAnchor,
    pub backend: &'static str,
    pub composing: bool,
    pub suspended: bool,
}
pub enum InlineEvent {
    Started(InlineStarted),
    Changed {
        ticket: InlineTicket,
        query: String,
        anchor: Option<PopupAnchor>,
        composing: bool,
        suspended: bool,
    },
    Navigate {
        session: u64,
        delta: i32,
    },
    SwitchSpace {
        session: u64,
        delta: i32,
    },
    Confirm(InlineTicket),
    Cancelled {
        session: u64,
        reason: &'static str,
    },
    Unavailable {
        session: u64,
        reason: String,
        anchor: PopupAnchor,
    },
    Compatibility {
        session: u64,
        reason: String,
    },
    Notice {
        session: u64,
        text: &'static str,
    },
    Suspended {
        session: u64,
        reason: &'static str,
    },
}
impl std::fmt::Debug for InlineEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InlineEvent(<redacted>)")
    }
}
pub type EventHandler = Arc<dyn Fn(InlineEvent) + Send + Sync>;

#[derive(Clone, Debug)]
pub struct TargetCapabilities {
    pub backend: &'static str,
    pub can_read_query: bool,
    pub can_observe_selection: bool,
    pub advertises_exact_selection: bool,
    pub has_text_edit_pattern: bool,
    pub exact_selection_verified: bool,
}

/// Bounded, content-free diagnostics. No key text, composer or clipboard payload.
#[derive(Clone, Debug)]
pub struct InlineTrace {
    pub elapsed_us: u64,
    pub kind: &'static str,
    pub detail: u32,
    pub session: u64,
    pub input_serial: u64,
    pub observed_serial: u64,
}

pub(super) const IME_CLEAR: u8 = 0;
pub(super) const IME_ACTIVE: u8 = 1;
pub(super) const IME_UNKNOWN: u8 = 2;
/// Marker only for Echo's own Ctrl+V injection. Other synthetic input (e.g.
/// accessibility tools and authorized tests) follows the same rules as typing.
pub(crate) const INJECTED_TAG: usize = 0x4543_484f;

pub(super) struct Shared {
    requested: AtomicU64,
    active: AtomicU64,
    editor_session: AtomicU64,
    window: AtomicIsize,
    focus: AtomicIsize,
    process: AtomicU32,
    thread: AtomicU32,
    input_serial: AtomicU64,
    observed_serial: AtomicU64,
    revision: AtomicU64,
    displayed_revision: AtomicU64,
    displayed_serial: AtomicU64,
    selectable: AtomicBool,
    committing: AtomicBool,
    outcome_unknown: AtomicBool,
    range_valid: AtomicBool,
    ime: AtomicU8,
    composition_evidence: Mutex<Option<composition::CompositionEvidence>>,
    may_compose: AtomicBool,
    ime_ui: AtomicU8,
    native_identity: AtomicBool,
    dirty_queued: AtomicBool,
    callback: EventHandler,
    started_at: Instant,
    trace: Mutex<std::collections::VecDeque<InlineTrace>>,
    capabilities: Mutex<Option<TargetCapabilities>>,
}
impl Shared {
    fn new(callback: EventHandler) -> Arc<Self> {
        Arc::new(Self {
            requested: AtomicU64::new(0),
            active: AtomicU64::new(0),
            editor_session: AtomicU64::new(0),
            window: AtomicIsize::new(0),
            focus: AtomicIsize::new(0),
            process: AtomicU32::new(0),
            thread: AtomicU32::new(0),
            input_serial: AtomicU64::new(0),
            observed_serial: AtomicU64::new(0),
            revision: AtomicU64::new(0),
            displayed_revision: AtomicU64::new(0),
            displayed_serial: AtomicU64::new(0),
            selectable: AtomicBool::new(false),
            committing: AtomicBool::new(false),
            outcome_unknown: AtomicBool::new(false),
            range_valid: AtomicBool::new(false),
            ime: AtomicU8::new(IME_UNKNOWN),
            composition_evidence: Mutex::new(None),
            may_compose: AtomicBool::new(false),
            ime_ui: AtomicU8::new(IME_UNKNOWN),
            native_identity: AtomicBool::new(false),
            dirty_queued: AtomicBool::new(false),
            callback,
            started_at: Instant::now(),
            trace: Mutex::new(std::collections::VecDeque::with_capacity(512)),
            capabilities: Mutex::new(None),
        })
    }
    fn ticket(&self) -> InlineTicket {
        InlineTicket {
            session: self.active.load(Ordering::Acquire),
            revision: self.revision.load(Ordering::Acquire),
            input_serial: self.observed_serial.load(Ordering::Acquire),
        }
    }
    fn record(&self, kind: &'static str, detail: u32) {
        if let Ok(mut trace) = self.trace.try_lock() {
            if trace.len() == 512 {
                trace.pop_front();
            }
            trace.push_back(InlineTrace {
                elapsed_us: self.started_at.elapsed().as_micros() as u64,
                kind,
                detail,
                session: self.active.load(Ordering::Acquire),
                input_serial: self.input_serial.load(Ordering::Acquire),
                observed_serial: self.observed_serial.load(Ordering::Acquire),
            });
        }
    }
    fn accepts(&self, ticket: InlineTicket) -> bool {
        ticket.session != 0
            && self.active.load(Ordering::Acquire) == ticket.session
            && self.requested.load(Ordering::Acquire) == ticket.session
            && self.revision.load(Ordering::Acquire) == ticket.revision
            && self.input_serial.load(Ordering::Acquire) == ticket.input_serial
            && self.observed_serial.load(Ordering::Acquire) == ticket.input_serial
    }
    fn composition(&self, target: &target::Target) -> (u8, bool) {
        let session = self.active.load(Ordering::Acquire);
        let serial = self.input_serial.load(Ordering::Acquire);
        let observed_at = Instant::now();
        let (reported, possible) = target.composition();
        let mut state = reported;
        if let Ok(mut evidence) = self.composition_evidence.lock() {
            if reported == IME_UNKNOWN {
                if let Some(e) = *evidence {
                    if e.source == composition::CompositionSource::TextEditEvent {
                        state = e.state_at(session, serial, Instant::now());
                    }
                }
            }
            if reported != IME_UNKNOWN || state == IME_UNKNOWN {
                *evidence = Some(composition::CompositionEvidence {
                    state,
                    session,
                    input_serial: serial,
                    observed_at,
                    source: target.composition_source(),
                });
            }
        }
        if serial != self.input_serial.load(Ordering::Acquire)
            || session != self.active.load(Ordering::Acquire)
        {
            state = IME_UNKNOWN;
        }
        (state, possible)
    }
    // Hook-side read is bounded: contention means Unknown, never a wait.
    fn verified_composition(&self) -> u8 {
        let session = self.active.load(Ordering::Acquire);
        let serial = self.input_serial.load(Ordering::Acquire);
        self.composition_evidence
            .try_lock()
            .ok()
            .and_then(|e| *e)
            .map_or(IME_UNKNOWN, |e| e.state_at(session, serial, Instant::now()))
    }
    fn suspend(&self, session: u64, reason: &'static str) {
        if self.active.load(Ordering::Acquire) == session {
            self.range_valid.store(false, Ordering::Release);
            self.selectable.store(false, Ordering::Release);
            (self.callback)(InlineEvent::Suspended { session, reason });
        }
    }
    fn can_confirm(&self) -> bool {
        let t = self.ticket();
        self.accepts(t)
            && self.selectable.load(Ordering::Acquire)
            && self.displayed_revision.load(Ordering::Acquire) == t.revision
            && self.displayed_serial.load(Ordering::Acquire) == t.input_serial
            && self.ime.load(Ordering::Acquire) == IME_CLEAR
            && !self.committing.load(Ordering::Acquire)
            && !self.outcome_unknown.load(Ordering::Acquire)
            && self.range_valid.load(Ordering::Acquire)
    }
    fn replacement_live(&self, ticket: InlineTicket) -> bool {
        self.accepts(ticket)
            && self.range_valid.load(Ordering::Acquire)
            && !self.outcome_unknown.load(Ordering::Acquire)
    }
    fn input_changed(&self, sender: &SyncSender<Request>, session: u64) {
        if session == 0 || self.active.load(Ordering::Acquire) != session {
            return;
        }
        // Physical/user input is never suppressed by the provider-event guard used
        // around our own selection and paste. It invalidates an in-flight commit.
        self.input_serial.fetch_add(1, Ordering::AcqRel);
        self.selectable.store(false, Ordering::Release);
        if !self.dirty_queued.swap(true, Ordering::AcqRel)
            && sender.try_send(Request::Observe(session)).is_err()
        {
            self.dirty_queued.store(false, Ordering::Release);
        }
    }
    fn dirty(&self, sender: &SyncSender<Request>, session: u64) {
        if session == 0
            || self.active.load(Ordering::Acquire) != session
            || self.committing.load(Ordering::Acquire)
        {
            return;
        }
        self.input_serial.fetch_add(1, Ordering::AcqRel);
        self.selectable.store(false, Ordering::Release);
        if !self.dirty_queued.swap(true, Ordering::AcqRel)
            && sender.try_send(Request::Observe(session)).is_err()
        {
            self.dirty_queued.store(false, Ordering::Release);
        }
    }
    fn cancel(&self, session: u64, reason: &'static str) {
        if self.editor_session.load(Ordering::Acquire) == session {
            return;
        }
        if session == 0 {
            return;
        }
        let _ = self
            .requested
            .compare_exchange(session, 0, Ordering::AcqRel, Ordering::Acquire);
        if self
            .active
            .compare_exchange(session, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.selectable.store(false, Ordering::Release);
            (self.callback)(InlineEvent::Cancelled { session, reason });
        }
    }
}

pub(super) struct Deadline {
    cancelled: AtomicBool,
    // 0 pending, 1 selecting, 2 write dispatched, 3/4 cancelled before/during
    // selection. A timeout and native-write authorization have one CAS order.
    delivery_stage: AtomicU8,
    expires: Instant,
}
impl Deadline {
    fn new(timeout: Duration) -> Arc<Self> {
        Arc::new(Self {
            cancelled: AtomicBool::new(false),
            delivery_stage: AtomicU8::new(0),
            expires: Instant::now() + timeout,
        })
    }
    fn live(&self) -> bool {
        !self.cancelled.load(Ordering::Acquire) && Instant::now() < self.expires
    }
    fn begin_selection(&self) -> bool {
        self.live()
            && self
                .delivery_stage
                .compare_exchange(0, 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
    }
    fn dispatch_write(&self) -> bool {
        self.live()
            && self
                .delivery_stage
                .compare_exchange(1, 2, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
    }
    fn cancel_delivery(&self) -> PasteDeliveryFailure {
        self.cancelled.store(true, Ordering::Release);
        let previous = self
            .delivery_stage
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |stage| match stage {
                0 => Some(3),
                1 => Some(4),
                _ => None,
            })
            .unwrap_or_else(|stage| stage);
        match previous {
            0 | 3 => PasteDeliveryFailure::RangeUnavailable,
            1 | 4 => PasteDeliveryFailure::SelectionUnconfirmed,
            _ => PasteDeliveryFailure::ReplacementUnconfirmed,
        }
    }
}
pub(super) enum Request {
    Begin(u64, FocusSnapshot),
    Observe(u64),
    Cancel(u64),
    Check(
        InlineTicket,
        payload::InsertPayload,
        Arc<Deadline>,
        SyncSender<Result<(), String>>,
    ),
    Paste(
        InlineTicket,
        u64,
        Arc<Deadline>,
        SyncSender<Result<PasteDelivery, String>>,
    ),
    Stop,
}
struct Inner {
    shared: Arc<Shared>,
    sender: SyncSender<Request>,
    hook: keyboard::InputHook,
    worker: Mutex<Option<std::thread::JoinHandle<()>>>,
}
#[derive(Clone)]
pub struct InlineController {
    inner: Arc<Inner>,
}
impl InlineController {
    /// The callback must enqueue a typed event, never wait for UIA, storage or UI.
    pub fn start(callback: EventHandler) -> Result<Self, String> {
        let shared = Shared::new(callback);
        let (sender, receiver) = mpsc::sync_channel(32);
        let hook = keyboard::InputHook::start(shared.clone(), sender.clone())?;
        let worker_shared = shared.clone();
        let worker_hook = hook.clone();
        let worker_sender = sender.clone();
        let worker = std::thread::Builder::new()
            .name("echo-inline-text-mta".into())
            .spawn(move || run(worker_shared, worker_hook, worker_sender, receiver))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            inner: Arc::new(Inner {
                shared,
                sender,
                hook,
                worker: Mutex::new(Some(worker)),
            }),
        })
    }
    pub fn begin(&self, session: u64, snapshot: FocusSnapshot) -> Result<(), String> {
        if session == 0 {
            return Err("Invalid inline session".into());
        }
        self.inner
            .shared
            .requested
            .store(session, Ordering::Release);
        self.inner
            .sender
            .try_send(Request::Begin(session, snapshot))
            .map_err(|_| "Inline input service is busy; use manual copying".to_string())
    }
    pub fn cancel(&self, session: u64) {
        let s = &self.inner.shared;
        let _ = s
            .requested
            .compare_exchange(session, 0, Ordering::AcqRel, Ordering::Acquire);
        let _ = s
            .active
            .compare_exchange(session, 0, Ordering::AcqRel, Ordering::Acquire);
        s.selectable.store(false, Ordering::Release);
        self.inner.hook.disarm(session);
        let _ = self.inner.sender.try_send(Request::Cancel(session));
    }
    /// Temporarily give an Echo editor the keyboard without retiring the target.
    pub fn set_editor_active(&self, session: u64, active: bool) {
        let s = &self.inner.shared;
        if active {
            s.editor_session.store(session, Ordering::Release);
            s.selectable.store(false, Ordering::Release);
        } else if s
            .editor_session
            .compare_exchange(session, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            s.input_changed(&self.inner.sender, session);
        }
    }
    pub fn results_ready(&self, ticket: InlineTicket, selectable: bool) {
        let s = &self.inner.shared;
        if !s.accepts(ticket) {
            return;
        }
        s.displayed_revision
            .store(ticket.revision, Ordering::Release);
        s.displayed_serial
            .store(ticket.input_serial, Ordering::Release);
        s.selectable.store(selectable, Ordering::Release);
    }
    pub fn can_confirm(&self, ticket: InlineTicket) -> bool {
        self.inner.shared.accepts(ticket) && self.inner.shared.can_confirm()
    }
    /// Non-content readiness counters are useful for an independent UI and tests.
    pub fn readiness(&self) -> [u64; 8] {
        let s = &self.inner.shared;
        [
            s.active.load(Ordering::Acquire),
            s.input_serial.load(Ordering::Acquire),
            s.observed_serial.load(Ordering::Acquire),
            s.revision.load(Ordering::Acquire),
            s.displayed_revision.load(Ordering::Acquire),
            s.displayed_serial.load(Ordering::Acquire),
            u64::from(s.selectable.load(Ordering::Acquire)),
            u64::from(s.can_confirm()),
        ]
    }
    pub fn diagnostics(&self) -> Vec<InlineTrace> {
        self.inner
            .shared
            .trace
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .cloned()
            .collect()
    }
    pub fn capabilities(&self) -> Option<TargetCapabilities> {
        self.inner
            .shared
            .capabilities
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn invalidate_results(&self) {
        self.inner.shared.selectable.store(false, Ordering::Release);
    }
    pub fn live(&self, ticket: InlineTicket) -> bool {
        self.inner.shared.accepts(ticket)
    }
    pub fn replacement_outcome_unknown(&self) -> bool {
        self.inner.shared.outcome_unknown.load(Ordering::Acquire)
    }
    #[cfg(feature = "native-test")]
    pub fn safety_status(&self) -> [bool; 3] {
        let s = &self.inner.shared;
        [
            s.committing.load(Ordering::Acquire),
            s.outcome_unknown.load(Ordering::Acquire),
            s.range_valid.load(Ordering::Acquire),
        ]
    }
    pub fn preflight(
        &self,
        ticket: InlineTicket,
        payload: &[ClipboardRepresentation],
    ) -> Result<(), String> {
        let text = payload::InsertPayload::retain(payload)?;
        let guard = Deadline::new(Duration::from_millis(600));
        let (reply, response) = mpsc::sync_channel(1);
        self.inner
            .sender
            .try_send(Request::Check(ticket, text, guard.clone(), reply))
            .map_err(|_| "Inline service is busy")?;
        match response.recv_timeout(Duration::from_millis(600)) {
            Ok(result) => result,
            Err(_) => {
                guard.cancelled.store(true, Ordering::Release);
                Err("Input validation timed out; no replacement was requested".into())
            }
        }
    }
    pub fn paste(&self, ticket: InlineTicket, sequence: u64) -> Result<PasteDelivery, String> {
        let guard = Deadline::new(Duration::from_millis(900));
        let (reply, response) = mpsc::sync_channel(1);
        self.inner
            .sender
            .try_send(Request::Paste(ticket, sequence, guard.clone(), reply))
            .map_err(|_| "Inline service is busy")?;
        match response.recv_timeout(Duration::from_millis(900)) {
            Ok(result) => {
                self.inner.shared.record("paste-reply-received", 0);
                result
            }
            Err(_) => {
                self.inner.shared.record("paste-reply-timeout", 0);
                let failure = guard.cancel_delivery();
                // Only an authorized native write can make the text outcome
                // unknown. Cancelling the earlier stage also forbids a late
                // selection reply from dispatching that write afterward.
                if failure == PasteDeliveryFailure::ReplacementUnconfirmed
                    && self.inner.shared.active.load(Ordering::Acquire) == ticket.session
                {
                    self.inner
                        .shared
                        .outcome_unknown
                        .store(true, Ordering::Release);
                }
                Ok(PasteDelivery::Failed(failure))
            }
        }
    }
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.shared.requested.store(0, Ordering::Release);
        self.shared.active.store(0, Ordering::Release);
        self.hook.stop();
        let _ = self.sender.try_send(Request::Stop);
        if let Some(worker) = self.worker.lock().unwrap_or_else(|e| e.into_inner()).take() {
            // An unresponsive external accessibility provider must not hold application
            // shutdown hostage. There is still only one worker; no replacement is spawned.
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}

struct Session {
    id: u64,
    backend: target::Target,
    range: QueryRange,
    _subscriptions: Option<subscriptions::Subscription>,
    prepared: Option<(InlineTicket, payload::InsertPayload)>,
    read_failures: u8,
    anchor: PopupAnchor,
}

fn run(
    shared: Arc<Shared>,
    hook: keyboard::InputHook,
    sender: SyncSender<Request>,
    receiver: mpsc::Receiver<Request>,
) {
    use windows::Win32::System::Com::*;
    unsafe {
        if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
            return;
        }
    }
    let automation = target::create_automation();
    let mut session: Option<Session> = None;
    let mut due: Option<Instant> = None;
    loop {
        let request = match due {
            Some(deadline) => {
                receiver.recv_timeout(deadline.saturating_duration_since(Instant::now()))
            }
            None => receiver
                .recv()
                .map_err(|_| mpsc::RecvTimeoutError::Disconnected),
        };
        match request {
            Err(mpsc::RecvTimeoutError::Disconnected) | Ok(Request::Stop) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                due = None;
                shared.dirty_queued.store(false, Ordering::Release);
                if let Some(active) = &mut session {
                    if shared.active.load(Ordering::Acquire) != active.id {
                        // UI cancellation hides the popup before requesting
                        // hook retirement. Query retirement alone cannot do so.
                        session = None;
                        continue;
                    }
                    if observe(active, &shared) {
                        due = Some(Instant::now() + Duration::from_millis(20));
                    }
                }
            }
            Ok(Request::Begin(id, snapshot)) => {
                if shared.requested.load(Ordering::Acquire) != id {
                    continue;
                }
                if let Some(old) = session.take() {
                    hook.disarm(old.id);
                }
                due = None;
                shared.selectable.store(false, Ordering::Release);
                shared.committing.store(false, Ordering::Release);
                shared.outcome_unknown.store(false, Ordering::Release);
                shared.range_valid.store(false, Ordering::Release);
                *shared
                    .capabilities
                    .lock()
                    .unwrap_or_else(|e| e.into_inner()) = None;
                let fallback_snapshot = snapshot.clone();
                let result = begin(id, snapshot, &shared, &hook, &sender, automation.as_ref());
                match result {
                    Ok(active) => {
                        session = Some(active);
                        // Cover the gap between the initial read and subscription:
                        // composition may have ended without a subscribed event.
                        due = Some(Instant::now() + Duration::from_millis(20));
                    }
                    Err(reason) => {
                        // A failed capability probe is NOT permission to focus Echo.
                        // Keep the session hook as an Enter shield until Esc/F6/dismiss.
                        if shared.requested.load(Ordering::Acquire) == id {
                            shared.ime.store(IME_UNKNOWN, Ordering::Release);
                            shared.selectable.store(false, Ordering::Release);
                            if fallback_snapshot.still_current() && hook.arm(id).is_ok() {
                                let anchor = crate::focus::resolve_anchor(
                                    &fallback_snapshot,
                                    automation.as_ref(),
                                );
                                (shared.callback)(InlineEvent::Unavailable {
                                    session: id,
                                    reason,
                                    anchor,
                                });
                            } else {
                                shared.cancel(id,"Input changed during capability check; invoke again in the input");
                                hook.disarm(id);
                            }
                        } else {
                            hook.disarm(id);
                        }
                    }
                }
            }
            Ok(Request::Observe(id)) => {
                if session.as_ref().is_some_and(|s| s.id == id) {
                    // The hook runs before the target processes its keystroke. Delay one
                    // bounded turn and coalesce notifications; never infer text from keys.
                    due.get_or_insert_with(|| Instant::now() + Duration::from_millis(20));
                }
            }
            Ok(Request::Cancel(id)) => {
                if session.as_ref().is_some_and(|s| s.id == id) {
                    session = None;
                    due = None;
                }
                hook.disarm(id);
            }
            Ok(Request::Check(ticket, text, deadline, reply)) => {
                // Composition/range reads can emit redundant provider changes,
                // just like the selection read in Paste. The fresh snapshot
                // still seals the exact range; physical input is never masked.
                shared.committing.store(true, Ordering::Release);
                let result = session
                    .as_mut()
                    .filter(|s| s.id == ticket.session)
                    .ok_or_else(|| "Inline session ended".to_string())
                    .and_then(|s| preflight(s, &shared, ticket, text, &deadline));
                shared.committing.store(false, Ordering::Release);
                if result.is_err()
                    && session
                        .as_ref()
                        .is_some_and(|s| s.id == ticket.session && s.backend.definitely_left())
                {
                    shared.cancel(
                        ticket.session,
                        "Original input control changed; invoke in the new editor",
                    );
                }
                let _ = reply.send(result);
            }
            Ok(Request::Paste(ticket, sequence, deadline, reply)) => {
                let result = session
                    .as_mut()
                    .filter(|s| s.id == ticket.session)
                    .ok_or_else(|| "Inline session ended".to_string())
                    .and_then(|s| paste(s, &shared, ticket, sequence, &deadline));
                if (result.is_err()
                    || matches!(
                        result,
                        Ok(PasteDelivery::Failed(PasteDeliveryFailure::RangeChanged))
                    ))
                    && session
                        .as_ref()
                        .is_some_and(|s| s.id == ticket.session && s.backend.definitely_left())
                {
                    shared.cancel(
                        ticket.session,
                        "Original input control changed; invoke in the new editor",
                    );
                }
                shared.record(
                    "paste-returned",
                    u32::from(matches!(result, Ok(PasteDelivery::Pasted))),
                );
                shared.committing.store(false, Ordering::Release);
                if matches!(
                    result,
                    Ok(PasteDelivery::Failed(
                        PasteDeliveryFailure::ReplacementUnconfirmed
                    ))
                ) {
                    shared.outcome_unknown.store(true, Ordering::Release);
                }
                if matches!(result, Ok(PasteDelivery::Pasted)) {
                    shared.active.store(0, Ordering::Release);
                    // The UI acknowledges delivery by hiding, then cancelling
                    // this lease. Protect the interval before that completion.
                    session = None;
                    due = None;
                }
                let _ = reply.send(result);
            }
        }
    }
    if let Some(active) = session.take() {
        hook.disarm(active.id);
    }
    drop(automation);
    unsafe {
        CoUninitialize();
    }
}

fn begin(
    id: u64,
    snapshot: FocusSnapshot,
    shared: &Arc<Shared>,
    hook: &keyboard::InputHook,
    sender: &SyncSender<Request>,
    uia: Option<&windows::Win32::UI::Accessibility::IUIAutomation>,
) -> Result<Session, String> {
    {
        let mut evidence = shared
            .composition_evidence
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        shared.active.store(id, Ordering::Release);
        *evidence = None;
        shared.ime.store(IME_UNKNOWN, Ordering::Release);
    }
    shared.window.store(snapshot.window_id, Ordering::Release);
    shared
        .focus
        .store(snapshot.focused_handle, Ordering::Release);
    shared.process.store(snapshot.process_id, Ordering::Release);
    shared.thread.store(
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(
                snapshot.window_id as _,
                std::ptr::null_mut(),
            )
        },
        Ordering::Release,
    );
    shared.input_serial.store(1, Ordering::Release);
    shared.observed_serial.store(0, Ordering::Release);
    shared.revision.store(0, Ordering::Release);
    shared.dirty_queued.store(false, Ordering::Release);
    shared.ime.store(IME_UNKNOWN, Ordering::Release);
    shared.native_identity.store(false, Ordering::Release);
    hook.arm(id)?;
    shared.record("armed", 0);
    #[cfg(feature = "native-test")]
    {
        let delay = diagnostics::acquisition_delay();
        if delay != 0 {
            shared.record("acquisition-fault-start", delay);
            std::thread::sleep(Duration::from_millis(u64::from(delay)));
            shared.record("acquisition-fault-end", delay);
        }
    }
    let serial = shared.input_serial.load(Ordering::Acquire);
    let backend = target::Target::open(&snapshot, uia)?;
    shared.record("target-open", 0);
    shared
        .native_identity
        .store(backend.backend() == "native-edit", Ordering::Release);
    let initial = backend.snapshot()?;
    *shared
        .capabilities
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = Some(backend.capabilities());
    let range = QueryRange::begin(&initial).map_err(str::to_string)?;
    let (composition, may_compose) = shared.composition(&backend);
    // A readable input may start with composition active/unknown. Keep focus
    // and filter actual provider text, but do not authorize Enter replacement.
    let anchor = backend.anchor().unwrap_or(snapshot.anchor);
    if shared.requested.load(Ordering::Acquire) != id {
        return Err("Inline activation was cancelled".into());
    }
    if shared.input_serial.load(Ordering::Acquire) != serial {
        shared.record("acquisition-raced", 0);
        return Err("The composer changed while the inline range was being captured. Typed text was kept; use manual copying or invoke again.".into());
    }
    shared.ime.store(composition, Ordering::Release);
    shared.may_compose.store(may_compose, Ordering::Release);
    shared.revision.store(range.revision(), Ordering::Release);
    shared.range_valid.store(true, Ordering::Release);
    shared.observed_serial.store(serial, Ordering::Release);
    let subscriptions = backend.automation_element().and_then(|(uia, element)| {
        subscriptions::Subscription::new(uia, element, shared.clone(), sender.clone(), id)
    });
    (shared.callback)(InlineEvent::Started(InlineStarted {
        ticket: shared.ticket(),
        query: if composition == IME_ACTIVE {
            backend
                .preview_query(&range, &initial)
                .unwrap_or_else(|| range.query())
        } else {
            range.query()
        },
        target: backend.paste_target.clone(),
        anchor,
        backend: backend.backend(),
        composing: composition == IME_ACTIVE,
        suspended: composition == IME_UNKNOWN,
    }));
    Ok(Session {
        id,
        backend,
        range,
        _subscriptions: subscriptions,
        prepared: None,
        read_failures: 0,
        anchor,
    })
}
/// Returns true when input raced the provider read and needs one more deferred sample.
fn observe(session: &mut Session, shared: &Shared) -> bool {
    if shared.editor_session.load(Ordering::Acquire) == session.id {
        return false;
    }
    if shared.committing.load(Ordering::Acquire) {
        return false;
    }
    if session.backend.definitely_left() {
        shared.cancel(
            session.id,
            "The original editor lost focus; inline completion cancelled",
        );
        return false;
    }
    // A later caret/provider notification cannot prove that a dispatched write
    // did not happen. Retain the unknown-outcome notice and Enter protection
    // until explicit cancellation; do not reinterpret the edited text as a
    // fresh query or overwrite the warning with "nothing was replaced".
    if shared.outcome_unknown.load(Ordering::Acquire) {
        shared.suspend(
            session.id,
            "Replacement outcome is unknown. Check the input, then Esc/F6; no repeat paste is allowed in this session.",
        );
        return false;
    }
    let serial = shared.input_serial.load(Ordering::Acquire);
    let snapshot = match session.backend.snapshot() {
        Ok(value) => {
            session.read_failures = 0;
            value
        }
        Err(error) => {
            // A target may expose the new text and the old UTF-16 selection for
            // one message turn (notably surrogate-pair backspace). Retry a bounded
            // number of turns; Enter stays disabled until a coherent read arrives.
            if session.read_failures < 4 {
                session.read_failures += 1;
                shared.selectable.store(false, Ordering::Release);
                return true;
            }
            eprintln!("Echo inline range observation rejected: {error}");
            shared.suspend(
                session.id,
                "Input range is temporarily unavailable; Enter stays protected. Edit the query or use Esc/F6.",
            );
            return false;
        }
    };
    let (composition, possible) = shared.composition(&session.backend);
    shared.ime.store(composition, Ordering::Release);
    shared.may_compose.store(possible, Ordering::Release);
    if shared.input_serial.load(Ordering::Acquire) != serial {
        return true;
    }
    if let Err(reason) = session.range.observe(&snapshot) {
        shared.range_valid.store(false, Ordering::Release);
        if composition != IME_ACTIVE {
            shared.suspend(session.id, reason);
            return false;
        }
        // Do not expand a protected query across an IME's transient range.
        shared.selectable.store(false, Ordering::Release);
    } else {
        shared.range_valid.store(true, Ordering::Release);
    }
    // Providers also notify for unchanged text/caret (for example placeholder
    // decorations). Such a notification must not erase the handoff between
    // preflight and paste. Real input, range or composition changes invalidate
    // it here; paste still seals the current snapshot and consumes it once.
    if session.prepared.as_ref().is_some_and(|(ticket, _)| {
        ticket.revision != session.range.revision()
            || !shared.replacement_live(*ticket)
            || composition != IME_CLEAR
    }) {
        session.prepared = None;
    }
    if let Some(anchor) = session.backend.anchor() {
        session.anchor = anchor;
    }
    if shared.input_serial.load(Ordering::Acquire) != serial {
        return true;
    }
    shared
        .revision
        .store(session.range.revision(), Ordering::Release);
    shared.observed_serial.store(serial, Ordering::Release);
    (shared.callback)(InlineEvent::Changed {
        ticket: shared.ticket(),
        query: if composition == IME_ACTIVE {
            session
                .backend
                .preview_query(&session.range, &snapshot)
                .unwrap_or_else(|| session.range.query())
        } else {
            session.range.query()
        },
        anchor: Some(session.anchor),
        composing: composition == IME_ACTIVE,
        suspended: composition == IME_UNKNOWN
            || !shared.range_valid.load(Ordering::Acquire)
            || shared.outcome_unknown.load(Ordering::Acquire),
    });
    composition == IME_ACTIVE || (composition == IME_UNKNOWN && possible)
}
fn preflight(
    session: &mut Session,
    shared: &Shared,
    ticket: InlineTicket,
    text: payload::InsertPayload,
    guard: &Deadline,
) -> Result<(), String> {
    if shared.outcome_unknown.load(Ordering::Acquire) {
        return Err("Replacement outcome is unknown. Check the input, then Esc/F6; no repeat paste is allowed in this session.".into());
    }
    if !guard.live() || !shared.replacement_live(ticket) {
        return Err(
            "The query changed before insertion. Enter was not sent; choose a current result."
                .into(),
        );
    }
    crate::windows_impl::modifiers_released()
        .map_err(|_| "ModifierKeysBusy: release held modifier keys")?;
    if shared.composition(&session.backend).0 != IME_CLEAR {
        return Err("CompositionBusy: finish the input-method composition first".into());
    }
    let snapshot = session.backend.snapshot()?;
    session
        .range
        .seal(&snapshot, ticket.revision)
        .map_err(str::to_string)?;
    if !guard.live() || !shared.replacement_live(ticket) {
        return Err("The query changed during validation; nothing was replaced".into());
    }
    if matches!(text, payload::InsertPayload::Image(_)) {
        session.backend.image_objects()?;
    }
    session.prepared = Some((ticket, text));
    Ok(())
}
fn paste(
    session: &mut Session,
    shared: &Shared,
    ticket: InlineTicket,
    sequence: u64,
    guard: &Deadline,
) -> Result<PasteDelivery, String> {
    let Some((prepared, inserted)) = session.prepared.take() else {
        return Err("Inline replacement was not preflighted".into());
    };
    if prepared != ticket || !guard.live() || !shared.replacement_live(ticket) {
        return Ok(PasteDelivery::Failed(PasteDeliveryFailure::RangeChanged));
    }
    // Reading a rich editor can itself publish redundant provider selection
    // notifications. Guard the complete verified operation, including its first
    // read, while physical input still invalidates the ticket independently.
    // The request loop clears this guard on every success/error return.
    shared.committing.store(true, Ordering::Release);
    if shared.composition(&session.backend).0 != IME_CLEAR {
        return Ok(PasteDelivery::Failed(PasteDeliveryFailure::CompositionBusy));
    }
    let snapshot = session.backend.snapshot()?;
    let span = session
        .range
        .seal(&snapshot, ticket.revision)
        .map_err(str::to_string)?;
    if !inserted.clipboard_matches(sequence) {
        return Ok(PasteDelivery::Failed(
            PasteDeliveryFailure::ClipboardChanged,
        ));
    }
    if !guard.begin_selection() {
        return Ok(PasteDelivery::Failed(
            PasteDeliveryFailure::RangeUnavailable,
        ));
    }
    session.backend.select(span.clone(), &snapshot, guard)?;
    shared.record("selection-verified", snapshot.text.len() as u32);
    if let Some(c) = shared
        .capabilities
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_mut()
    {
        c.exact_selection_verified = true;
    }
    if !guard.live() || !shared.replacement_live(ticket) {
        return Ok(PasteDelivery::Failed(PasteDeliveryFailure::RangeChanged));
    }
    if !inserted.clipboard_matches(sequence) {
        return Ok(PasteDelivery::Failed(
            PasteDeliveryFailure::ClipboardChanged,
        ));
    }
    if matches!(inserted, payload::InsertPayload::Image(_)) {
        let before = session.backend.image_objects()?;
        if !guard.live()
            || !shared.replacement_live(ticket)
            || !inserted.clipboard_matches(sequence)
        {
            return Ok(PasteDelivery::Failed(PasteDeliveryFailure::RangeChanged));
        }
        if !guard.dispatch_write() {
            return Ok(PasteDelivery::Failed(
                PasteDeliveryFailure::SelectionUnconfirmed,
            ));
        }
        if session.backend.paste_image().is_err() {
            return Ok(PasteDelivery::Failed(
                PasteDeliveryFailure::ReplacementUnconfirmed,
            ));
        }
        // Never retry the paste. An exact embedded-object replacement, or a
        // new attachment plus preserved surrounding text, acknowledges receipt.
        while guard.live() && shared.accepts(ticket) {
            if let (Ok(actual), Ok(after)) =
                (session.backend.snapshot(), session.backend.image_objects())
            {
                let embedded = session
                    .range
                    .matches_replacement(&actual.text, session.backend.embedded_image_text());
                let attachment = session.range.matches_replacement(&actual.text, &[])
                    && after.adds_one_to(&before);
                if embedded || attachment {
                    return Ok(PasteDelivery::Pasted);
                }
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        return Ok(PasteDelivery::Failed(
            PasteDeliveryFailure::ReplacementUnconfirmed,
        ));
    }
    let payload::InsertPayload::Text(inserted) = inserted else {
        unreachable!()
    };
    // No mutation was performed before exact selection and all identities were
    // rechecked. Standard Edit uses its native range operation; rich editors
    // retain native clipboard paste. Neither route separately deletes the query.
    shared.record(
        "replacement-method",
        u32::from(session.backend.uses_native_range_replace()),
    );
    if !guard.dispatch_write() {
        return Ok(PasteDelivery::Failed(
            PasteDeliveryFailure::SelectionUnconfirmed,
        ));
    }
    if session.backend.paste_selected(&inserted).is_err() {
        shared.record("paste-request-error", 0);
        return Ok(PasteDelivery::Failed(
            PasteDeliveryFailure::ReplacementUnconfirmed,
        ));
    }
    let expected = session.backend.normalize_inserted(&inserted);
    let single_line = session.backend.single_line_inserted(&inserted);
    let browser_line_breaks = session.backend.chromium_text_input();
    shared.record("paste-requested", expected.len() as u32);
    let lf = inserted
        .replace("\r\n", "\n")
        .encode_utf16()
        .collect::<Vec<_>>();
    let crlf = inserted
        .replace("\r\n", "\n")
        .replace('\n', "\r\n")
        .encode_utf16()
        .collect::<Vec<_>>();
    // The request already has a bounded 900 ms deadline. A separate 300 ms cap
    // discarded useful remaining time after an acknowledged edit when the host
    // briefly stalled its readback. Spend the remaining request budget only on
    // observation; never repeat the mutation and never extend that deadline.
    while guard.live() {
        if !shared.accepts(ticket) {
            break;
        }
        shared.record("paste-readback-start", 0);
        if let Ok(actual) = session.backend.receipt_text() {
            shared.record("paste-readback", actual.len() as u32);
            if session.range.matches_replacement(&actual, &expected)
                || session.range.matches_replacement(&actual, &lf)
                || session.range.matches_replacement(&actual, &crlf)
                || single_line.as_ref().is_some_and(|forms| {
                    forms
                        .iter()
                        .any(|form| session.range.matches_replacement(&actual, form))
                })
                || (browser_line_breaks
                    && matches_browser_line_breaks(
                        &session.range,
                        &actual,
                        &expected,
                        span.start,
                        snapshot.text.len() - span.end,
                    ))
            {
                return Ok(PasteDelivery::Pasted);
            }
        } else {
            shared.record("paste-readback-error", 0);
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    Ok(PasteDelivery::Failed(
        PasteDeliveryFailure::ReplacementUnconfirmed,
    ))
}

/// Chromium can expose additional paragraph separators after rich clipboard
/// paste. Compare only the inserted span this way; the frozen context remains
/// byte-for-byte exact and no non-line-break character may disappear or change.
fn matches_browser_line_breaks(
    range: &QueryRange,
    actual: &[u16],
    inserted: &[u16],
    prefix_units: usize,
    suffix_units: usize,
) -> bool {
    if !inserted.iter().any(|c| matches!(c, 10 | 13)) {
        return false;
    }
    let Some(end) = actual.len().checked_sub(suffix_units) else {
        return false;
    };
    let Some(middle) = actual.get(prefix_units..end) else {
        return false;
    };
    range.matches_replacement(actual, middle)
        && middle
            .iter()
            .filter(|c| !matches!(c, 10 | 13))
            .eq(inserted.iter().filter(|c| !matches!(c, 10 | 13)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chromium_rich_paste_accepts_paragraph_breaks_but_preserves_content_and_context() {
        let u = |s: &str| s.encode_utf16().collect::<Vec<_>>();
        let range = QueryRange::begin(&echo_engine::ComposerSnapshot {
            text: u("pre\n|q|\npost"),
            selection: 5..6,
        })
        .unwrap();
        let payload = u("Echo第一行ABC\r\nEcho第二行123");
        for actual in [
            "pre\n|Echo第一行ABC\n\nEcho第二行123|\npost",
            "pre\n|Echo第一行ABCEcho第二行123|\npost",
        ] {
            assert!(matches_browser_line_breaks(
                &range,
                &u(actual),
                &payload,
                5,
                6
            ));
        }
        for actual in [
            "pre|Echo第一行ABCEcho第二行123|post",
            "pre\n|Echo第一行ABCEcho第二行12|\npost",
            "pre\n|Echo第一行ABC Echo第二行123|\npost",
        ] {
            assert!(!matches_browser_line_breaks(
                &range,
                &u(actual),
                &payload,
                5,
                6
            ));
        }
        assert!(!matches_browser_line_breaks(
            &range,
            &u("pre\n|a\nb|\npost"),
            &u("ab"),
            5,
            6
        ));
    }
    #[test]
    fn image_receipt_requires_one_new_attachment_in_the_original_container() {
        fn observed(scope: i32, images: &[i32]) -> target::ImageObservation {
            target::ImageObservation {
                scope: vec![scope],
                objects: images.iter().map(|id| vec![*id]).collect(),
            }
        }
        let before = observed(1, &[10]);
        assert!(observed(1, &[10, 11]).adds_one_to(&before));
        assert!(!observed(2, &[10, 11]).adds_one_to(&before));
        assert!(!observed(1, &[10]).adds_one_to(&before));
        assert!(!observed(1, &[11]).adds_one_to(&before));
        assert!(!observed(1, &[10, 11, 12]).adds_one_to(&before));
    }
    #[test]
    fn validation_guard_preserves_physical_input_invalidation() {
        let s = Shared::new(Arc::new(|_| {}));
        s.active.store(1, Ordering::Relaxed);
        s.requested.store(1, Ordering::Relaxed);
        s.range_valid.store(true, Ordering::Relaxed);
        let ticket = s.ticket();
        let (sender, _receiver) = mpsc::sync_channel(2);
        s.committing.store(true, Ordering::Relaxed);
        s.dirty(&sender, 1);
        assert!(s.replacement_live(ticket));
        s.input_changed(&sender, 1);
        assert!(!s.replacement_live(ticket));
    }
    #[cfg(feature = "native-test")]
    #[test]
    #[ignore = "requires an explicitly selected synthetic editor draft"]
    fn authorized_provider_notification_retains_preflight() {
        use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
        assert_eq!(std::env::var("ECHO_WINDOWS_ACCEPTANCE").as_deref(), Ok("1"));
        let hwnd: isize = std::env::var("ECHO_TEST_TARGET_HWND")
            .unwrap()
            .parse()
            .unwrap();
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap();
        }
        let focus = FocusSnapshot::capture();
        assert_eq!(focus.window_id, hwnd);
        let automation = target::create_automation().unwrap();
        let backend = target::Target::open(&focus, Some(&automation)).unwrap();
        let initial = backend.snapshot().unwrap();
        let range = QueryRange::begin(&initial).unwrap();
        let shared = Shared::new(Arc::new(|_| {}));
        shared.active.store(1, Ordering::Release);
        shared.requested.store(1, Ordering::Release);
        shared.revision.store(range.revision(), Ordering::Release);
        shared.range_valid.store(true, Ordering::Release);
        let ticket = shared.ticket();
        let mut session = Session {
            id: 1,
            backend,
            range,
            _subscriptions: None,
            prepared: Some((ticket, payload::InsertPayload::Text("synthetic".into()))),
            read_failures: 0,
            anchor: focus.anchor,
        };
        observe(&mut session, &shared);
        assert_eq!(shared.ticket(), ticket);
        assert!(
            session.prepared.is_some(),
            "unchanged provider event discarded preflight"
        );
        drop(session);
        drop(automation);
        unsafe {
            CoUninitialize();
        }
    }
    #[test]
    fn timeout_distinguishes_selection_from_dispatched_write() {
        let pending = Deadline::new(Duration::from_secs(1));
        assert_eq!(
            pending.cancel_delivery(),
            PasteDeliveryFailure::RangeUnavailable
        );
        assert!(!pending.begin_selection());
        let selecting = Deadline::new(Duration::from_secs(1));
        assert!(selecting.begin_selection());
        assert_eq!(
            selecting.cancel_delivery(),
            PasteDeliveryFailure::SelectionUnconfirmed
        );
        assert!(!selecting.dispatch_write());
        let writing = Deadline::new(Duration::from_secs(1));
        assert!(writing.begin_selection());
        assert!(writing.dispatch_write());
        assert_eq!(
            writing.cancel_delivery(),
            PasteDeliveryFailure::ReplacementUnconfirmed
        );
    }
    #[test]
    fn timeout_cannot_report_selection_only_after_authorizing_a_write() {
        for _ in 0..100 {
            let deadline = Deadline::new(Duration::from_secs(2));
            assert!(deadline.begin_selection());
            let barrier = Arc::new(std::sync::Barrier::new(2));
            let worker_deadline = deadline.clone();
            let worker_barrier = barrier.clone();
            let worker = std::thread::spawn(move || {
                worker_barrier.wait();
                worker_deadline.dispatch_write()
            });
            barrier.wait();
            let result = deadline.cancel_delivery();
            let dispatched = worker.join().unwrap();
            assert_eq!(
                dispatched,
                result == PasteDeliveryFailure::ReplacementUnconfirmed
            );
            assert!(!deadline.dispatch_write());
        }
    }
    #[test]
    fn ready_result_must_match_query_and_every_observed_input_generation() {
        let s = Shared::new(Arc::new(|_| {}));
        s.active.store(7, Ordering::Relaxed);
        s.requested.store(7, Ordering::Relaxed);
        s.input_serial.store(3, Ordering::Relaxed);
        s.observed_serial.store(3, Ordering::Relaxed);
        s.revision.store(2, Ordering::Relaxed);
        s.displayed_revision.store(2, Ordering::Relaxed);
        s.displayed_serial.store(3, Ordering::Relaxed);
        s.ime.store(IME_CLEAR, Ordering::Relaxed);
        s.selectable.store(true, Ordering::Relaxed);
        s.range_valid.store(true, Ordering::Relaxed);
        assert!(s.can_confirm());
        s.input_serial.store(4, Ordering::Relaxed);
        assert!(!s.can_confirm());
        s.observed_serial.store(4, Ordering::Relaxed);
        assert!(!s.can_confirm());
        s.displayed_serial.store(4, Ordering::Relaxed);
        assert!(s.can_confirm());
        s.ime.store(IME_ACTIVE, Ordering::Relaxed);
        assert!(!s.can_confirm());
        s.ime.store(IME_UNKNOWN, Ordering::Relaxed);
        assert!(!s.can_confirm());
    }
    #[test]
    fn cancelled_session_cannot_commit_a_late_result() {
        let s = Shared::new(Arc::new(|_| {}));
        s.active.store(4, Ordering::Relaxed);
        s.requested.store(4, Ordering::Relaxed);
        let ticket = s.ticket();
        s.cancel(4, "test");
        assert!(!s.accepts(ticket));
        s.active.store(5, Ordering::Relaxed);
        s.requested.store(5, Ordering::Relaxed);
        s.cancel(4, "stale");
        assert_eq!(s.active.load(Ordering::Relaxed), 5);
    }
    #[test]
    fn paused_range_and_unknown_delivery_reject_a_matching_ticket() {
        let s = Shared::new(Arc::new(|_| {}));
        s.active.store(4, Ordering::Relaxed);
        s.requested.store(4, Ordering::Relaxed);
        s.range_valid.store(true, Ordering::Relaxed);
        let ticket = s.ticket();
        assert!(s.replacement_live(ticket));
        s.suspend(4, "provider read failed");
        assert!(s.accepts(ticket));
        assert!(!s.replacement_live(ticket));
        s.range_valid.store(true, Ordering::Relaxed);
        assert!(s.replacement_live(ticket));
        s.outcome_unknown.store(true, Ordering::Relaxed);
        assert!(!s.replacement_live(ticket));
    }
}
