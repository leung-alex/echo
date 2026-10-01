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
use crate::ime_observer;
pub(crate) mod ime_window;
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

/// A plain-paste confirmation token deliberately carries no query range or
/// inline replacement ticket.  It binds the physical confirmation to the
/// captured target, the active session and the current result/IME sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlainPasteTicket {
    pub session: u64,
    pub revision: u64,
    pub input_serial: u64,
}

pub enum InlineEvent {
    Started(InlineStarted),
    PlainPasteStarted {
        ticket: PlainPasteTicket,
        target: PasteTarget,
        anchor: PopupAnchor,
        backend: &'static str,
        composing: bool,
        suspended: bool,
        composition_source: &'static str,
    },
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
    PlainPasteChanged {
        ticket: PlainPasteTicket,
        composing: bool,
        suspended: bool,
        composition_source: &'static str,
    },
    PlainPasteConfirm(PlainPasteTicket),
    Cancelled {
        session: u64,
        reason: &'static str,
    },
    Unavailable {
        session: u64,
        reason: String,
        anchor: PopupAnchor,
        captured_target: Option<PasteTarget>,
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
pub(super) const MODE_NONE: u8 = 0;
pub(super) const MODE_INLINE: u8 = 1;
pub(super) const MODE_PLAIN_PASTE: u8 = 2;
const PLAIN_COMPOSITION_LEASE: Duration = Duration::from_millis(250);
const PLAIN_CONFIRM_LEASE: Duration = Duration::from_millis(350);
const MAX_OBSERVER_RETRIES: u8 = 3;
const MAX_TARGET_OPEN_RETRIES: u8 = 3;
/// Marker only for Echo's own Ctrl+V injection. Other synthetic input (e.g.
/// accessibility tools and authorized tests) follows the same rules as typing.
pub(crate) const INJECTED_TAG: usize = 0x4543_484f;

pub(super) struct Shared {
    lifecycle: Mutex<()>,
    requested: AtomicU64,
    active: AtomicU64,
    editor_session: AtomicU64,
    navigation_session: AtomicU64,
    window: AtomicIsize,
    focus: AtomicIsize,
    process: AtomicU32,
    thread: AtomicU32,
    input_serial: AtomicU64,
    observed_serial: AtomicU64,
    revision: AtomicU64,
    displayed_revision: AtomicU64,
    displayed_serial: AtomicU64,
    mode: AtomicU8,
    selectable: AtomicBool,
    committing: AtomicBool,
    outcome_unknown: AtomicBool,
    plain_claim: Mutex<Option<PlainPasteClaim>>,
    range_valid: AtomicBool,
    ime: AtomicU8,
    composition_evidence: Mutex<Option<composition::CompositionEvidence>>,
    may_compose: AtomicBool,
    ime_ui: AtomicU8,
    native_identity: AtomicBool,
    hosted_input: AtomicBool,
    dirty_queued: AtomicBool,
    callback: EventHandler,
    started_at: Instant,
    trace: Mutex<std::collections::VecDeque<InlineTrace>>,
    trace_log: Mutex<Option<std::fs::File>>,
    capabilities: Mutex<Option<TargetCapabilities>>,
}

#[derive(Clone, Copy, Debug)]
struct PlainPasteClaim {
    ticket: PlainPasteTicket,
    expires_at: Instant,
}

impl Shared {
    fn new(callback: EventHandler) -> Arc<Self> {
        Arc::new(Self {
            lifecycle: Mutex::new(()),
            requested: AtomicU64::new(0),
            active: AtomicU64::new(0),
            editor_session: AtomicU64::new(0),
            navigation_session: AtomicU64::new(0),
            window: AtomicIsize::new(0),
            focus: AtomicIsize::new(0),
            process: AtomicU32::new(0),
            thread: AtomicU32::new(0),
            input_serial: AtomicU64::new(0),
            observed_serial: AtomicU64::new(0),
            revision: AtomicU64::new(0),
            displayed_revision: AtomicU64::new(0),
            displayed_serial: AtomicU64::new(0),
            mode: AtomicU8::new(MODE_NONE),
            selectable: AtomicBool::new(false),
            committing: AtomicBool::new(false),
            outcome_unknown: AtomicBool::new(false),
            plain_claim: Mutex::new(None),
            range_valid: AtomicBool::new(false),
            ime: AtomicU8::new(IME_UNKNOWN),
            composition_evidence: Mutex::new(None),
            may_compose: AtomicBool::new(false),
            ime_ui: AtomicU8::new(IME_UNKNOWN),
            native_identity: AtomicBool::new(false),
            hosted_input: AtomicBool::new(false),
            dirty_queued: AtomicBool::new(false),
            callback,
            started_at: Instant::now(),
            trace: Mutex::new(std::collections::VecDeque::with_capacity(512)),
            trace_log: Mutex::new(None),
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
    fn plain_ticket(&self) -> PlainPasteTicket {
        PlainPasteTicket {
            session: self.active.load(Ordering::Acquire),
            revision: self.revision.load(Ordering::Acquire),
            input_serial: self.observed_serial.load(Ordering::Acquire),
        }
    }
    fn record(&self, kind: &'static str, detail: u32) {
        let entry = InlineTrace {
            elapsed_us: self.started_at.elapsed().as_micros() as u64,
            kind,
            detail,
            session: self.active.load(Ordering::Acquire),
            input_serial: self.input_serial.load(Ordering::Acquire),
            observed_serial: self.observed_serial.load(Ordering::Acquire),
        };
        if let Ok(mut trace) = self.trace.try_lock() {
            if trace.len() == 512 {
                trace.pop_front();
            }
            trace.push_back(entry.clone());
        }
        // Best-effort JSONL mirror for field diagnosis. try_lock keeps the
        // keyboard hook non-blocking; a stalled write just drops one line.
        if let Ok(mut sink) = self.trace_log.try_lock() {
            if let Some(file) = sink.as_mut() {
                use std::io::Write;
                let _ = writeln!(
                    file,
                    "{}",
                    serde_json::json!({
                        "us": entry.elapsed_us,
                        "kind": entry.kind,
                        "detail": entry.detail,
                        "session": entry.session,
                        "input_serial": entry.input_serial,
                        "observed_serial": entry.observed_serial,
                    })
                );
            }
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
    fn plain_accepts(&self, ticket: PlainPasteTicket) -> bool {
        ticket.session != 0
            && self.mode.load(Ordering::Acquire) == MODE_PLAIN_PASTE
            && self.active.load(Ordering::Acquire) == ticket.session
            && self.requested.load(Ordering::Acquire) == ticket.session
            && self.revision.load(Ordering::Acquire) == ticket.revision
            && self.input_serial.load(Ordering::Acquire) == ticket.input_serial
            && self.observed_serial.load(Ordering::Acquire) == ticket.input_serial
    }
    fn plain_evidence_state(&self, ticket: PlainPasteTicket, now: Instant) -> u8 {
        self.composition_evidence
            .try_lock()
            .ok()
            .and_then(|evidence| *evidence)
            .map_or(IME_UNKNOWN, |evidence| {
                evidence.plain_state_at(
                    ticket.session,
                    ticket.input_serial,
                    now,
                    PLAIN_COMPOSITION_LEASE,
                )
            })
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
    fn plain_can_confirm(&self) -> bool {
        let ticket = self.plain_ticket();
        self.plain_can_confirm_ticket(ticket)
    }
    fn plain_can_confirm_ticket(&self, ticket: PlainPasteTicket) -> bool {
        self.plain_accepts(ticket)
            && self.selectable.load(Ordering::Acquire)
            && self.displayed_revision.load(Ordering::Acquire) == ticket.revision
            && self.displayed_serial.load(Ordering::Acquire) == ticket.input_serial
            && self.plain_evidence_state(ticket, Instant::now()) == IME_CLEAR
            && !self.committing.load(Ordering::Acquire)
            && !self.outcome_unknown.load(Ordering::Acquire)
    }
    fn plain_live(&self, ticket: PlainPasteTicket) -> bool {
        self.plain_accepts(ticket)
            && !self.outcome_unknown.load(Ordering::Acquire)
            && !self.committing.load(Ordering::Acquire)
            && self
                .plain_claim
                .try_lock()
                .ok()
                .and_then(|claim| *claim)
                .is_some_and(|claim| claim.ticket == ticket && Instant::now() < claim.expires_at)
    }

    fn reap_expired_plain_claim(&self) {
        if let Ok(mut claim) = self.plain_claim.try_lock() {
            if claim.is_some_and(|value| Instant::now() >= value.expires_at) {
                *claim = None;
            }
        }
    }

    /// Reserve one confirmation after the hook has made its pure decision.
    /// The reservation is separate from `selectable`: the latter is the UI
    /// readiness bit and is cleared before the asynchronous event is queued.
    fn claim_plain_confirmation(&self, ticket: PlainPasteTicket) -> bool {
        if !self.plain_can_confirm_ticket(ticket) {
            return false;
        }
        let Ok(mut claim) = self.plain_claim.try_lock() else {
            return false;
        };
        if claim.is_some() || !self.plain_can_confirm_ticket(ticket) {
            return false;
        }
        *claim = Some(PlainPasteClaim {
            ticket,
            expires_at: Instant::now() + PLAIN_CONFIRM_LEASE,
        });
        self.selectable.store(false, Ordering::Release);
        true
    }

    fn release_plain_claim(&self, ticket: PlainPasteTicket) {
        if let Ok(mut claim) = self.plain_claim.try_lock() {
            if claim.is_some_and(|value| value.ticket == ticket) {
                *claim = None;
            }
        }
    }

    fn consume_plain_confirmation(&self, ticket: PlainPasteTicket) -> bool {
        let Ok(mut claim) = self.plain_claim.try_lock() else {
            return false;
        };
        let valid = claim
            .as_ref()
            .is_some_and(|value| value.ticket == ticket && Instant::now() < value.expires_at)
            && self.plain_accepts(ticket)
            && !self.outcome_unknown.load(Ordering::Acquire)
            && !self.committing.load(Ordering::Acquire);
        if !valid {
            if claim.as_ref().is_some_and(|value| value.ticket == ticket) {
                *claim = None;
            }
            return false;
        }
        *claim = None;
        self.selectable.store(false, Ordering::Release);
        self.committing.store(true, Ordering::Release);
        true
    }

    fn clear_plain_claim(&self) {
        if let Ok(mut claim) = self.plain_claim.try_lock() {
            *claim = None;
        }
    }

    fn publish_composition(
        &self,
        session: u64,
        serial: u64,
        state: u8,
        source: composition::CompositionSource,
    ) -> u8 {
        if session == 0
            || self.active.load(Ordering::Acquire) != session
            || self.input_serial.load(Ordering::Acquire) != serial
        {
            return IME_UNKNOWN;
        }
        let evidence = composition::CompositionEvidence {
            state,
            session,
            input_serial: serial,
            observed_at: Instant::now(),
            source,
        };
        if let Ok(mut current) = self.composition_evidence.lock() {
            if evidence.can_publish(session, serial, *current) {
                *current = Some(evidence);
            }
            let published = current
                .map(|value| value.state_at(session, serial, Instant::now()))
                .unwrap_or(IME_UNKNOWN);
            self.ime.store(published, Ordering::Release);
            published
        } else {
            self.ime.store(IME_UNKNOWN, Ordering::Release);
            IME_UNKNOWN
        }
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
    /// Provider-side notifications (UIA value/property/selection churn) are not
    /// physical input: attachment uploads and live regions emit them without
    /// touching the query. Queue a coalesced observation and let the snapshot
    /// diff — via range revision and a re-issued ticket — decide what changed.
    /// Bumping input_serial here outruns every in-flight ticket while an upload
    /// is still animating, permanently blocking Enter and click confirmation.
    fn provider_changed(&self, sender: &SyncSender<Request>, session: u64) {
        if session == 0 || self.active.load(Ordering::Acquire) != session {
            return;
        }
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
        if self.release_if_owner(session) {
            let code = match reason {
                "Input focus changed; inline completion cancelled" => 1,
                "Inline completion cancelled; typed query kept" => 2,
                "Foreground application changed; inline completion cancelled" => 3,
                "Native input focus changed; inline completion cancelled" => 4,
                "Input changed during capability check; invoke again in the input" => 5,
                "Original input control changed; invoke in the new editor" => 6,
                "The original editor lost focus; inline completion cancelled" => 7,
                "The original input changed; plain paste was cancelled" => 8,
                _ => 9,
            };
            self.record("cancel", code);
            (self.callback)(InlineEvent::Cancelled { session, reason });
        }
    }

    /// Lifecycle state is split across atomics for hook reads, so its writes
    /// need one ownership gate. A stale cancellation may retire its own hook,
    /// but it cannot clear a newer requested or active session's mode.
    fn release_if_owner(&self, session: u64) -> bool {
        let _lifecycle = self.lifecycle.lock().unwrap_or_else(|e| e.into_inner());
        let requested = self.requested.load(Ordering::Acquire);
        let active = self.active.load(Ordering::Acquire);
        if requested != session && active != session {
            return false;
        }
        // A newer request owns the transition. Leave the old active lease for
        // the worker's serialized Begin path instead of clearing its state.
        if requested != 0 && requested != session {
            return false;
        }
        if requested == session {
            self.requested.store(0, Ordering::Release);
        }
        let active_owned = active == session;
        if active_owned {
            self.active.store(0, Ordering::Release);
        }
        self.clear_plain_claim();
        if active_owned || requested == session {
            self.selectable.store(false, Ordering::Release);
            self.mode.store(MODE_NONE, Ordering::Release);
            let _ = self.navigation_session.compare_exchange(
                session,
                0,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
        active_owned
    }

    fn request_session(&self, session: u64) {
        let _lifecycle = self.lifecycle.lock().unwrap_or_else(|e| e.into_inner());
        self.clear_plain_claim();
        self.mode.store(MODE_NONE, Ordering::Release);
        self.requested.store(session, Ordering::Release);
    }

    fn reset_for_begin(&self, session: u64) -> bool {
        let _lifecycle = self.lifecycle.lock().unwrap_or_else(|e| e.into_inner());
        if self.requested.load(Ordering::Acquire) != session {
            return false;
        }
        self.clear_plain_claim();
        self.active.store(session, Ordering::Release);
        self.mode.store(MODE_NONE, Ordering::Release);
        true
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
        self.inner.shared.request_session(session);
        self.inner
            .sender
            .try_send(Request::Begin(session, snapshot))
            .map_err(|_| "Inline input service is busy; use manual copying".to_string())
    }
    pub fn cancel(&self, session: u64) {
        let s = &self.inner.shared;
        s.record("cancel-ui", session as u32);
        s.release_if_owner(session);
        self.inner.hook.disarm(session);
        let _ = self.inner.sender.try_send(Request::Cancel(session));
    }
    /// Plain-paste browsing owns navigation without claiming an editable range or IME evidence.
    pub fn own_plain_paste_navigation(&self, session: u64) {
        let s = &self.inner.shared;
        if session != 0
            && s.active.load(Ordering::Acquire) == session
            && s.requested.load(Ordering::Acquire) == session
        {
            s.navigation_session.store(session, Ordering::Release);
        }
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
    pub fn plain_results_ready(&self, ticket: PlainPasteTicket, selectable: bool) {
        let s = &self.inner.shared;
        if !s.plain_accepts(ticket) {
            return;
        }
        s.reap_expired_plain_claim();
        s.displayed_revision
            .store(ticket.revision, Ordering::Release);
        s.displayed_serial
            .store(ticket.input_serial, Ordering::Release);
        if s.plain_claim
            .try_lock()
            .ok()
            .is_some_and(|claim| claim.is_none())
        {
            s.selectable.store(selectable, Ordering::Release);
        }
    }
    pub fn can_confirm(&self, ticket: InlineTicket) -> bool {
        self.inner.shared.accepts(ticket) && self.inner.shared.can_confirm()
    }
    pub fn plain_live(&self, ticket: PlainPasteTicket) -> bool {
        self.inner.shared.plain_live(ticket)
    }
    /// Reserve a one-shot confirmation for a mouse or keyboard path. A
    /// keyboard Enter normally reserves this in the hook before it posts its
    /// event; the UI may call this method when a direct click needs the same
    /// freshness and composition checks.
    pub fn reserve_plain_confirmation(&self, ticket: PlainPasteTicket) -> bool {
        self.inner.shared.claim_plain_confirmation(ticket)
    }
    /// Consume the reserved confirmation exactly once before queueing the
    /// ordinary quick-insert operation. The token is never replayed by a
    /// later Enter or click, even if the worker queue is delayed.
    pub fn consume_plain_confirmation(&self, ticket: PlainPasteTicket) -> bool {
        self.inner.shared.consume_plain_confirmation(ticket)
    }
    pub fn finish_plain_confirmation(&self) {
        self.inner.shared.committing.store(false, Ordering::Release);
    }
    pub fn release_plain_confirmation(&self, ticket: PlainPasteTicket) {
        self.inner.shared.release_plain_claim(ticket);
    }
    /// Non-content readiness counters are useful for an independent UI and tests.
    pub fn readiness(&self) -> [u64; 8] {
        let s = &self.inner.shared;
        let ready = if s.mode.load(Ordering::Acquire) == MODE_PLAIN_PASTE {
            s.plain_can_confirm()
        } else {
            s.can_confirm()
        };
        [
            s.active.load(Ordering::Acquire),
            s.input_serial.load(Ordering::Acquire),
            s.observed_serial.load(Ordering::Acquire),
            s.revision.load(Ordering::Acquire),
            s.displayed_revision.load(Ordering::Acquire),
            s.displayed_serial.load(Ordering::Acquire),
            u64::from(s.selectable.load(Ordering::Acquire)),
            u64::from(ready),
        ]
    }
    /// Mirror the bounded in-memory trace into a JSONL file. Content-free
    /// (static labels and counters only); used for field diagnosis of target
    /// admission and Enter decisions where no synthetic editor exists.
    pub fn set_trace_log(&self, path: impl AsRef<std::path::Path>) {
        if let Ok(mut sink) = self.inner.shared.trace_log.lock() {
            *sink = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .ok();
        }
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
        self.inner.shared.clear_plain_claim();
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
        let timeout =
            Duration::from_millis(if self.inner.shared.hosted_input.load(Ordering::Acquire) {
                3000
            } else {
                600
            });
        let guard = Deadline::new(timeout);
        let (reply, response) = mpsc::sync_channel(1);
        self.inner
            .sender
            .try_send(Request::Check(ticket, text, guard.clone(), reply))
            .map_err(|_| "Inline service is busy")?;
        match response.recv_timeout(timeout) {
            Ok(result) => result,
            Err(_) => {
                guard.cancelled.store(true, Ordering::Release);
                Err("Input validation timed out; no replacement was requested".into())
            }
        }
    }
    pub fn paste(&self, ticket: InlineTicket, sequence: u64) -> Result<PasteDelivery, String> {
        let timeout =
            Duration::from_millis(if self.inner.shared.hosted_input.load(Ordering::Acquire) {
                3000
            } else {
                900
            });
        let guard = Deadline::new(timeout);
        let (reply, response) = mpsc::sync_channel(1);
        self.inner
            .sender
            .try_send(Request::Paste(ticket, sequence, guard.clone(), reply))
            .map_err(|_| "Inline service is busy")?;
        match response.recv_timeout(timeout) {
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

struct PlainPasteSession {
    id: u64,
    target: PasteTarget,
    anchor: PopupAnchor,
    automation: windows::Win32::UI::Accessibility::IUIAutomation,
    endpoint: Option<crate::focus::InputStatusEndpoint>,
    observer: Option<ime_observer::Observer>,
    observer_attempts: u8,
    observer_retry_at: Instant,
    observer_blocked: bool,
    last_composition: u8,
    last_serial: u64,
}

fn observer_retryable(error: &str) -> bool {
    ![
        "not approved",
        "matching process architecture",
        "target identity unavailable",
        "identity unavailable",
        "entry point unavailable",
        "requires a matching",
    ]
    .iter()
    .any(|marker| error.contains(marker))
}

impl PlainPasteSession {
    fn new(
        id: u64,
        target: PasteTarget,
        anchor: PopupAnchor,
        automation: windows::Win32::UI::Accessibility::IUIAutomation,
        snapshot: &FocusSnapshot,
    ) -> Self {
        let endpoint = snapshot.input_endpoint();
        let mut observer_attempts = 0;
        let mut observer_blocked = endpoint.is_none();
        let observer = endpoint.and_then(|endpoint| {
            observer_attempts = 1;
            match ime_observer::Observer::status_at(endpoint) {
                Ok(observer) => Some(observer),
                Err(error) => {
                    observer_blocked = !observer_retryable(&error);
                    None
                }
            }
        });
        Self {
            id,
            target,
            anchor,
            automation,
            endpoint,
            observer,
            observer_attempts,
            observer_retry_at: Instant::now() + Duration::from_millis(20),
            observer_blocked,
            last_composition: IME_UNKNOWN,
            last_serial: 0,
        }
    }

    fn current(&self) -> bool {
        let snapshot = FocusSnapshot::capture();
        if snapshot.window_id != self.target.window_id
            || snapshot.process_id != self.target.process_id
            || snapshot.process_started_at != self.target.process_started_at
        {
            return false;
        }
        snapshot
            .capture_plain_paste_target(&self.automation)
            .is_some_and(|(target, _)| target.focused_control == self.target.focused_control)
    }

    fn composition(&mut self, shared: &Shared) -> (u8, &'static str) {
        if self.observer.is_none()
            && !self.observer_blocked
            && self.observer_attempts < MAX_OBSERVER_RETRIES
            && Instant::now() >= self.observer_retry_at
        {
            if let Some(endpoint) = self.endpoint {
                self.observer_attempts = self.observer_attempts.saturating_add(1);
                shared.record("plain-observer-retry", u32::from(self.observer_attempts));
                match ime_observer::Observer::status_at(endpoint) {
                    Ok(observer) => {
                        self.observer = Some(observer);
                        shared.record("plain-observer-recovered", self.observer_attempts.into());
                    }
                    Err(error) => {
                        if observer_retryable(&error) {
                            let backoff =
                                20_u64.saturating_mul(1_u64 << self.observer_attempts.min(5));
                            self.observer_retry_at =
                                Instant::now() + Duration::from_millis(backoff);
                        } else {
                            self.observer_blocked = true;
                            shared.record("plain-observer-blocked", self.observer_attempts.into());
                        }
                    }
                }
            }
        }
        let Some(observer) = &self.observer else {
            return (IME_UNKNOWN, "target-thread-observer-unavailable");
        };
        match observer.input_state() {
            Some((_, echo_engine::CompositionState::Idle)) => (IME_CLEAR, "target-thread-idle"),
            Some((_, echo_engine::CompositionState::Composing)) => {
                (IME_ACTIVE, "target-thread-composing")
            }
            _ => (IME_UNKNOWN, "target-thread-unknown"),
        }
    }

    fn refresh(&mut self, shared: &Shared) -> Option<(PlainPasteTicket, u8, &'static str, bool)> {
        if !self.current() {
            return None;
        }
        let serial = shared.input_serial.load(Ordering::Acquire);
        let (observed, source) = self.composition(shared);
        let composition = shared.publish_composition(
            self.id,
            serial,
            observed,
            if self.observer.is_some() {
                composition::CompositionSource::TargetThread
            } else {
                composition::CompositionSource::TargetRead
            },
        );
        let changed = self.last_composition != composition || self.last_serial != serial;
        if changed {
            shared.revision.fetch_add(1, Ordering::AcqRel);
            shared.selectable.store(false, Ordering::Release);
        }
        self.last_composition = composition;
        self.last_serial = serial;
        shared.observed_serial.store(serial, Ordering::Release);
        Some((shared.plain_ticket(), composition, source, changed))
    }

    fn keep_observing(&self, composition: u8) -> bool {
        self.observer.is_some()
            || (!self.observer_blocked && self.observer_attempts < MAX_OBSERVER_RETRIES)
            || composition != IME_CLEAR
    }
}

enum ActiveSession {
    Inline(Session),
    Plain(PlainPasteSession),
}

impl ActiveSession {
    fn id(&self) -> u64 {
        match self {
            Self::Inline(session) => session.id,
            Self::Plain(session) => session.id,
        }
    }
}

enum BeginResult {
    Inline(Session),
    Plain(PlainPasteSession),
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
    let mut session: Option<ActiveSession> = None;
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
                    if shared.active.load(Ordering::Acquire) != active.id() {
                        // UI cancellation hides the popup before requesting
                        // hook retirement. Query retirement alone cannot do so.
                        session = None;
                        continue;
                    }
                    let keep_observing = match active {
                        ActiveSession::Inline(inline) => observe(inline, &shared),
                        ActiveSession::Plain(plain) => observe_plain(plain, &shared),
                    };
                    if keep_observing {
                        due = Some(Instant::now() + Duration::from_millis(20));
                    }
                }
            }
            Ok(Request::Begin(id, snapshot)) => {
                if shared.requested.load(Ordering::Acquire) != id {
                    continue;
                }
                if let Some(old) = session.take() {
                    hook.disarm(old.id());
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
                    Ok(BeginResult::Inline(active)) => {
                        session = Some(ActiveSession::Inline(active));
                        // Cover the gap between the initial read and subscription:
                        // composition may have ended without a subscribed event.
                        due = Some(Instant::now() + Duration::from_millis(20));
                    }
                    Ok(BeginResult::Plain(active)) => {
                        session = Some(ActiveSession::Plain(active));
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
                                    captured_target: None,
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
                if session.as_ref().is_some_and(|s| s.id() == id) {
                    // The hook runs before the target processes its keystroke. Delay one
                    // bounded turn and coalesce notifications; never infer text from keys.
                    due.get_or_insert_with(|| Instant::now() + Duration::from_millis(20));
                }
            }
            Ok(Request::Cancel(id)) => {
                if session.as_ref().is_some_and(|s| s.id() == id) {
                    session = None;
                    due = None;
                }
                hook.disarm(id);
            }
            Ok(Request::Check(ticket, text, deadline, reply)) => {
                shared.record("check-entry", ticket.session as u32);
                // Composition/range reads can emit redundant provider changes,
                // just like the selection read in Paste. The fresh snapshot
                // still seals the exact range; physical input is never masked.
                shared.committing.store(true, Ordering::Release);
                let result = match session.as_mut() {
                    Some(ActiveSession::Inline(s)) if s.id == ticket.session => {
                        preflight(s, &shared, ticket, text, &deadline)
                    }
                    Some(ActiveSession::Plain(_))
                        if shared.active.load(Ordering::Acquire) == ticket.session =>
                    {
                        Err("Plain paste uses the ordinary insert path".to_string())
                    }
                    _ => Err("Inline session ended".to_string()),
                };
                shared.committing.store(false, Ordering::Release);
                if result.is_err()
                    && session
                        .as_ref()
                        .is_some_and(|s| {
                            matches!(s, ActiveSession::Inline(inline) if inline.id == ticket.session && inline.backend.definitely_left())
                        })
                {
                    shared.cancel(
                        ticket.session,
                        "Original input control changed; invoke in the new editor",
                    );
                }
                let _ = reply.send(result);
            }
            Ok(Request::Paste(ticket, sequence, deadline, reply)) => {
                let result = match session.as_mut() {
                    Some(ActiveSession::Inline(s)) if s.id == ticket.session => {
                        paste(s, &shared, ticket, sequence, &deadline)
                    }
                    Some(ActiveSession::Plain(_))
                        if shared.active.load(Ordering::Acquire) == ticket.session =>
                    {
                        Err("Plain paste uses the ordinary insert path".to_string())
                    }
                    _ => Err("Inline session ended".to_string()),
                };
                if (result.is_err()
                    || matches!(
                        result,
                        Ok(PasteDelivery::Failed(PasteDeliveryFailure::RangeChanged))
                    ))
                    && session
                        .as_ref()
                        .is_some_and(|s| {
                            matches!(s, ActiveSession::Inline(inline) if inline.id == ticket.session && inline.backend.definitely_left())
                        })
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
                    shared.mode.store(MODE_NONE, Ordering::Release);
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
        hook.disarm(active.id());
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
) -> Result<BeginResult, String> {
    {
        let mut evidence = shared
            .composition_evidence
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if !shared.reset_for_begin(id) {
            return Err("Inline activation became stale".into());
        }
        *evidence = None;
        shared.ime.store(IME_UNKNOWN, Ordering::Release);
    }
    shared.window.store(snapshot.window_id, Ordering::Release);
    shared
        .hosted_input
        .store(snapshot.is_hosted_input(), Ordering::Release);
    shared
        .focus
        .store(snapshot.focused_handle, Ordering::Release);
    shared.process.store(
        snapshot
            .input_endpoint()
            .map_or(snapshot.process_id, |input| input.process),
        Ordering::Release,
    );
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
    shared.displayed_revision.store(0, Ordering::Release);
    shared.displayed_serial.store(0, Ordering::Release);
    shared.dirty_queued.store(false, Ordering::Release);
    shared.ime.store(IME_UNKNOWN, Ordering::Release);
    shared.native_identity.store(false, Ordering::Release);
    hook.arm(id)?;
    shared.record("armed", shared.process.load(Ordering::Acquire) as u32);
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
    let backend = match open_target_with_retries(&snapshot, uia, shared) {
        Ok(backend) => backend,
        Err(error) => {
            // Rich Chromium editors must get the inline range first. A legacy
            // UIA/MSAA element may also expose a writable ValuePattern, but
            // choosing plain paste before attempting Target::open strands the
            // keyboard in the host and makes every letter bypass Echo search.
            // Value-only controls still use the ordinary paste path after the
            // strict range attempt has failed.
            if let Some((target, anchor)) =
                uia.and_then(|uia| snapshot.capture_plain_paste_target(uia))
            {
                if shared.requested.load(Ordering::Acquire) != id || !snapshot.still_current() {
                    return Err("Input changed during plain paste activation".into());
                }
                let automation = uia
                    .cloned()
                    .ok_or("Plain paste observation is unavailable")?;
                let mut plain = PlainPasteSession::new(id, target, anchor, automation, &snapshot);
                let Some((ticket, composition, source, _)) = plain.refresh(shared) else {
                    return Err("Plain paste target changed during activation".into());
                };
                shared.mode.store(MODE_PLAIN_PASTE, Ordering::Release);
                shared.navigation_session.store(id, Ordering::Release);
                shared.may_compose.store(true, Ordering::Release);
                shared.record("plain-paste-session-started", 0);
                (shared.callback)(InlineEvent::PlainPasteStarted {
                    ticket,
                    target: plain.target.clone(),
                    anchor: plain.anchor,
                    backend: "plain-paste-value",
                    composing: composition == IME_ACTIVE,
                    suspended: composition == IME_UNKNOWN,
                    composition_source: source,
                });
                return Ok(BeginResult::Plain(plain));
            }
            shared.record("target-open-failed", target_failure_code(&error));
            return Err(error);
        }
    };
    shared.record("target-open", 0);
    shared
        .native_identity
        .store(backend.backend() == "native-edit", Ordering::Release);
    let initial = backend.snapshot()?;
    // Field diagnosis only: when a non-empty document survives normalization,
    // the gate mask shows which readonly-hint condition rejected it.
    if !initial.text.is_empty() {
        if let Some(mask) = backend.readonly_hint_gates() {
            shared.record("hint-gates", mask);
        }
    }
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
    shared.mode.store(MODE_INLINE, Ordering::Release);
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
    Ok(BeginResult::Inline(Session {
        id,
        backend,
        range,
        _subscriptions: subscriptions,
        prepared: None,
        read_failures: 0,
        anchor,
    }))
}
fn target_open_retryable(error: &str) -> bool {
    [
        "Windows text accessibility is unavailable",
        "The input does not expose its focused text element",
        "Input owner unavailable",
        "The focused input is an offscreen proxy",
        "The input has no stable accessibility identity",
    ]
    .iter()
    .any(|marker| error.contains(marker))
}

fn open_target_with_retries(
    snapshot: &FocusSnapshot,
    uia: Option<&windows::Win32::UI::Accessibility::IUIAutomation>,
    shared: &Shared,
) -> Result<target::Target, String> {
    let mut last = None;
    for attempt in 0..=MAX_TARGET_OPEN_RETRIES {
        if !snapshot.still_current() {
            return Err("Original input focus changed during capability check".into());
        }
        match target::Target::open(snapshot, uia) {
            Ok(backend) => return Ok(backend),
            Err(error) if attempt < MAX_TARGET_OPEN_RETRIES && target_open_retryable(&error) => {
                shared.record("target-open-retry", u32::from(attempt + 1));
                last = Some(error);
                std::thread::sleep(Duration::from_millis(20 * u64::from(attempt + 1)));
            }
            Err(error) => return Err(error),
        }
    }
    Err(last.unwrap_or_else(|| "Input capability check failed".into()))
}

fn target_failure_code(error: &str) -> u32 {
    // Stable, content-free buckets for local diagnostics. Do not persist the
    // provider's name, window title, query, or any text from the editor.
    if error.contains("focused text element") || error.contains("verified text editor") {
        1
    } else if error.contains("offscreen proxy") || error.contains("offscreen") {
        2
    } else if error.contains("exact query range") || error.contains("selection") {
        3
    } else if error.contains("stable accessibility identity") {
        4
    } else if error.contains("Input owner") || error.contains("input focus") {
        5
    } else {
        255
    }
}

/// Returns true when input raced the provider read and needs one more deferred sample.
fn observe(session: &mut Session, shared: &Shared) -> bool {
    shared.record("observe-enter", 0);
    if shared.editor_session.load(Ordering::Acquire) == session.id {
        shared.record("observe-stop", 1);
        return false;
    }
    if shared.committing.load(Ordering::Acquire) {
        shared.record("observe-stop", 2);
        return false;
    }
    if session.backend.definitely_left() {
        shared.record("observe-stop", 3);
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
        shared.record("observe-stop", 4);
        shared.suspend(
            session.id,
            "Replacement outcome is unknown. Check the input, then Esc/F6; no repeat paste is allowed in this session.",
        );
        return false;
    }
    let serial = shared.input_serial.load(Ordering::Acquire);
    let previous_revision = session.range.revision();
    let previous_ime = shared.ime.load(Ordering::Acquire);
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
                shared.record("observe-retry", u32::from(session.read_failures));
                shared.selectable.store(false, Ordering::Release);
                return true;
            }
            eprintln!("Echo inline range observation rejected: {error}");
            shared.record("observe-stop", 5);
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
        shared.record("observe-race", 1);
        return true;
    }
    let mut rebound = false;
    if let Err(reason) = session.range.observe(&snapshot) {
        // While the captured query never changed (revision is still 1), a
        // context mismatch means the recorded prefix/suffix was generated
        // decoration (for example a placeholder paragraph exposed as document
        // text) rather than user text. Rebind the range to the live snapshot
        // instead of suspending; replacement still seals against the current
        // text before pasting.
        rebound = reason == "Text outside the inline query changed; nothing was replaced"
            && session.range.revision() == 1
            && QueryRange::resume(&snapshot)
                .map(|range| session.range = range)
                .is_ok();
        if rebound {
            shared.record("range-rebind", snapshot.text.len() as u32);
        } else {
            shared.range_valid.store(false, Ordering::Release);
            shared.record(
                "range-reject",
                match reason {
                    "Text outside the inline query changed; nothing was replaced" => 1,
                    "The caret left the inline query; nothing was replaced" => 2,
                    _ => 3,
                },
            );
            if composition != IME_ACTIVE {
                shared.record("observe-stop", 6);
                shared.suspend(session.id, reason);
                return false;
            }
            // Do not expand a protected query across an IME's transient range.
            shared.selectable.store(false, Ordering::Release);
        }
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
        shared.record("observe-race", 2);
        return true;
    }
    shared
        .revision
        .store(session.range.revision(), Ordering::Release);
    shared.observed_serial.store(serial, Ordering::Release);
    shared.record("observed", (composition as u32) | (u32::from(possible) << 8));
    let query_changed = rebound || session.range.revision() != previous_revision;
    let composition_changed = composition != previous_ime;
    if query_changed || composition_changed {
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
    }
    // Some Chromium editors do not publish UIA text-change events for ordinary
    // typing. Keep a bounded poll alive for every active inline session so the
    // provider snapshot, rather than the physical key, remains authoritative.
    let _ = possible;
    true
}

fn observe_plain(session: &mut PlainPasteSession, shared: &Shared) -> bool {
    if shared.editor_session.load(Ordering::Acquire) == session.id {
        return false;
    }
    let Some((ticket, composition, source, changed)) = session.refresh(shared) else {
        shared.cancel(
            session.id,
            "The original input changed; plain paste was cancelled",
        );
        return false;
    };
    if changed {
        shared.clear_plain_claim();
        shared.record(
            "plain-paste-composition",
            match composition {
                IME_CLEAR => 0,
                IME_ACTIVE => 1,
                _ => 2,
            },
        );
        (shared.callback)(InlineEvent::PlainPasteChanged {
            ticket,
            composing: composition == IME_ACTIVE,
            suspended: composition == IME_UNKNOWN,
            composition_source: source,
        });
    }
    session.keep_observing(composition)
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
        let mut last_text: Option<u32> = None;
        let mut last_objects: Option<u32> = None;
        while guard.live() && shared.accepts(ticket) {
            if let (Ok(actual), Ok(after)) =
                (session.backend.snapshot(), session.backend.image_objects())
            {
                last_text = Some(actual.text.len() as u32);
                last_objects = Some(
                    ((before.objects.len() as u32) << 8) | after.objects.len() as u32,
                );
                let embedded = session.range.matches_replacement_except(
                    &actual.text,
                    session.backend.embedded_image_text(),
                    text_scope::structural_text_unit,
                );
                let attachment = session
                    .range
                    .matches_replacement_except(&actual.text, &[], text_scope::structural_text_unit)
                    && (after.adds_to(&before) || after.grows_in_scope(&before));
                if embedded || attachment {
                    return Ok(PasteDelivery::Pasted);
                }
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        shared.record("image-readback", last_text.unwrap_or(u32::MAX));
        shared.record("image-objects", last_objects.unwrap_or(u32::MAX));
        for ct in session.backend.image_scope_control_types() {
            shared.record("image-scope-ct", ct as u32);
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
    fn target_open_retries_only_transient_provider_states() {
        assert!(target_open_retryable(
            "The input does not expose its focused text element"
        ));
        assert!(target_open_retryable(
            "Windows text accessibility is unavailable"
        ));
        assert!(target_open_retryable("Input owner unavailable"));
        assert!(!target_open_retryable(
            "The input is protected, read-only, or not a verified text editor"
        ));
        assert!(!target_open_retryable(
            "The input cannot select an exact query range"
        ));
    }
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
        assert!(observed(1, &[10, 11]).adds_to(&before));
        // A preview plus its overlay controls still proves receipt; losing a
        // pre-existing object or switching containers never does.
        assert!(observed(1, &[10, 11, 12]).adds_to(&before));
        assert!(!observed(2, &[10, 11]).adds_to(&before));
        assert!(!observed(1, &[10]).adds_to(&before));
        assert!(!observed(1, &[11]).adds_to(&before));
        assert!(!observed(1, &[11, 12]).adds_to(&before));
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
    fn plain_paste_confirmation_uses_target_state_without_query_range() {
        let s = Shared::new(Arc::new(|_| {}));
        s.active.store(7, Ordering::Relaxed);
        s.requested.store(7, Ordering::Relaxed);
        s.mode.store(MODE_PLAIN_PASTE, Ordering::Relaxed);
        s.input_serial.store(3, Ordering::Relaxed);
        s.observed_serial.store(3, Ordering::Relaxed);
        s.revision.store(2, Ordering::Relaxed);
        s.displayed_revision.store(2, Ordering::Relaxed);
        s.displayed_serial.store(3, Ordering::Relaxed);
        s.ime.store(IME_CLEAR, Ordering::Relaxed);
        *s.composition_evidence.lock().unwrap() = Some(composition::CompositionEvidence {
            state: IME_CLEAR,
            source: composition::CompositionSource::TargetThread,
            session: 7,
            input_serial: 3,
            observed_at: Instant::now(),
        });
        s.selectable.store(true, Ordering::Relaxed);
        // Plain paste deliberately has no QueryRange/range_valid capability.
        assert!(!s.range_valid.load(Ordering::Relaxed));
        assert!(s.plain_can_confirm());

        *s.composition_evidence.lock().unwrap() = Some(composition::CompositionEvidence {
            state: IME_UNKNOWN,
            source: composition::CompositionSource::TargetThread,
            session: 7,
            input_serial: 3,
            observed_at: Instant::now(),
        });
        assert!(!s.plain_can_confirm());
        *s.composition_evidence.lock().unwrap() = Some(composition::CompositionEvidence {
            state: IME_ACTIVE,
            source: composition::CompositionSource::TargetThread,
            session: 7,
            input_serial: 3,
            observed_at: Instant::now(),
        });
        assert!(!s.plain_can_confirm());
    }

    #[test]
    fn stale_cancel_cannot_clear_a_new_plain_session_lease() {
        let s = Shared::new(Arc::new(|_| {}));
        s.request_session(8);
        s.reset_for_begin(8);
        s.mode.store(MODE_PLAIN_PASTE, Ordering::Release);
        s.navigation_session.store(8, Ordering::Release);
        s.selectable.store(true, Ordering::Release);
        s.clear_plain_claim();

        assert!(!s.release_if_owner(7));
        assert_eq!(s.requested.load(Ordering::Acquire), 8);
        assert_eq!(s.active.load(Ordering::Acquire), 8);
        assert_eq!(s.mode.load(Ordering::Acquire), MODE_PLAIN_PASTE);
        assert!(s.selectable.load(Ordering::Acquire));
        assert_eq!(s.navigation_session.load(Ordering::Acquire), 8);
    }

    #[test]
    fn old_cancel_after_new_request_does_not_cross_the_begin_boundary() {
        let s = Shared::new(Arc::new(|_| {}));
        s.request_session(7);
        s.reset_for_begin(7);
        s.mode.store(MODE_PLAIN_PASTE, Ordering::Release);
        s.selectable.store(true, Ordering::Release);
        s.request_session(8);

        assert!(!s.release_if_owner(7));
        assert_eq!(s.requested.load(Ordering::Acquire), 8);
        assert_eq!(s.active.load(Ordering::Acquire), 7);
        assert_eq!(s.mode.load(Ordering::Acquire), MODE_NONE);
        assert!(s.selectable.load(Ordering::Acquire));

        s.reset_for_begin(8);
        assert_eq!(s.active.load(Ordering::Acquire), 8);
        assert_eq!(s.requested.load(Ordering::Acquire), 8);
    }

    #[test]
    fn late_started_event_cannot_replace_a_newer_requested_session() {
        let s = Shared::new(Arc::new(|_| {}));
        s.request_session(10);
        assert!(s.reset_for_begin(10));
        s.mode.store(MODE_PLAIN_PASTE, Ordering::Release);
        s.navigation_session.store(10, Ordering::Release);
        s.selectable.store(true, Ordering::Release);

        s.request_session(11);
        assert!(!s.reset_for_begin(10));
        assert_eq!(s.active.load(Ordering::Acquire), 10);
        assert_eq!(s.requested.load(Ordering::Acquire), 11);
        assert_eq!(s.mode.load(Ordering::Acquire), MODE_NONE);
        assert!(s.selectable.load(Ordering::Acquire));
        assert_eq!(s.navigation_session.load(Ordering::Acquire), 10);

        assert!(s.reset_for_begin(11));
        s.mode.store(MODE_PLAIN_PASTE, Ordering::Release);
        s.navigation_session.store(11, Ordering::Release);
        s.selectable.store(true, Ordering::Release);
        assert!(!s.release_if_owner(10));
        assert_eq!(s.active.load(Ordering::Acquire), 11);
        assert_eq!(s.mode.load(Ordering::Acquire), MODE_PLAIN_PASTE);
        assert!(s.selectable.load(Ordering::Acquire));
        assert_eq!(s.navigation_session.load(Ordering::Acquire), 11);
    }

    #[test]
    fn plain_confirmation_claim_is_one_time_and_expires_without_rearming_old_ticket() {
        let s = Shared::new(Arc::new(|_| {}));
        s.request_session(9);
        s.reset_for_begin(9);
        s.mode.store(MODE_PLAIN_PASTE, Ordering::Release);
        s.input_serial.store(1, Ordering::Release);
        s.observed_serial.store(1, Ordering::Release);
        s.revision.store(1, Ordering::Release);
        s.displayed_revision.store(1, Ordering::Release);
        s.displayed_serial.store(1, Ordering::Release);
        s.selectable.store(true, Ordering::Release);
        *s.composition_evidence.lock().unwrap() = Some(composition::CompositionEvidence {
            state: IME_CLEAR,
            source: composition::CompositionSource::TargetThread,
            session: 9,
            input_serial: 1,
            observed_at: Instant::now(),
        });
        let ticket = s.plain_ticket();
        assert!(s.claim_plain_confirmation(ticket));
        assert!(!s.claim_plain_confirmation(ticket));
        assert!(s.plain_live(ticket));
        s.release_plain_claim(ticket);
        assert!(!s.plain_live(ticket));
    }

    #[test]
    fn plain_confirmation_consumption_is_one_shot_and_marks_commit_in_flight() {
        let s = Shared::new(Arc::new(|_| {}));
        s.request_session(12);
        assert!(s.reset_for_begin(12));
        s.mode.store(MODE_PLAIN_PASTE, Ordering::Release);
        s.input_serial.store(2, Ordering::Release);
        s.observed_serial.store(2, Ordering::Release);
        s.revision.store(3, Ordering::Release);
        s.displayed_revision.store(3, Ordering::Release);
        s.displayed_serial.store(2, Ordering::Release);
        s.selectable.store(true, Ordering::Release);
        *s.composition_evidence.lock().unwrap() = Some(composition::CompositionEvidence {
            state: IME_CLEAR,
            source: composition::CompositionSource::TargetThread,
            session: 12,
            input_serial: 2,
            observed_at: Instant::now(),
        });
        let ticket = s.plain_ticket();
        assert!(s.claim_plain_confirmation(ticket));
        assert!(s.consume_plain_confirmation(ticket));
        assert!(!s.consume_plain_confirmation(ticket));
        assert!(!s.plain_live(ticket));
        assert!(!s.selectable.load(Ordering::Acquire));
        assert!(s.committing.load(Ordering::Acquire));
        s.committing.store(false, Ordering::Release);
    }

    #[test]
    fn observer_retry_policy_preserves_unknown_for_blocked_targets() {
        assert!(observer_retryable(
            "IME observer target did not acknowledge a read"
        ));
        assert!(observer_retryable("IME observer channel unavailable"));
        assert!(!observer_retryable(
            "IME observer target identity is not approved"
        ));
        assert!(!observer_retryable(
            "IME observer requires a matching process architecture"
        ));
    }
    #[test]
    fn plain_paste_ticket_expires_on_input_or_session_change() {
        let s = Shared::new(Arc::new(|_| {}));
        s.active.store(4, Ordering::Relaxed);
        s.requested.store(4, Ordering::Relaxed);
        s.mode.store(MODE_PLAIN_PASTE, Ordering::Relaxed);
        s.input_serial.store(1, Ordering::Relaxed);
        s.observed_serial.store(1, Ordering::Relaxed);
        s.revision.store(1, Ordering::Relaxed);
        let ticket = s.plain_ticket();
        assert!(s.plain_accepts(ticket));
        s.input_serial.store(2, Ordering::Relaxed);
        assert!(!s.plain_accepts(ticket));
        s.active.store(5, Ordering::Relaxed);
        s.requested.store(5, Ordering::Relaxed);
        assert!(!s.plain_accepts(ticket));
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

    /// Temporary regression probe: prints the Quick Insert decision chain for
    /// each newly-foregrounded window (never text content). Focus the composer
    /// under test while this runs.
    #[test]
    #[ignore = "live probe; requires ECHO_WINDOWS_ACCEPTANCE=1 and explicit focus"]
    fn live_probe_decision_chain() {
        use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};
        assert_eq!(
            std::env::var("ECHO_WINDOWS_ACCEPTANCE").as_deref(),
            Ok("1")
        );
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok().unwrap();
        }
        let own = std::process::id();
        let automation = target::create_automation();
        let started_at = Instant::now();
        let deadline = started_at + Duration::from_secs(120);
        while Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1200));
            let snapshot = crate::focus::FocusSnapshot::capture();
            if snapshot.window_id == 0 || snapshot.process_id == own {
                continue;
            }
            let name = crate::windows_impl::process_path(snapshot.process_id)
                .and_then(|path| {
                    std::path::Path::new(&path)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                })
                .unwrap_or_default();
            let mut input_pid = 0u32;
            unsafe {
                windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(
                    snapshot.focused_handle as _,
                    &mut input_pid,
                );
            }
            let input_name = crate::windows_impl::process_path(input_pid)
                .and_then(|path| {
                    std::path::Path::new(&path)
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                })
                .unwrap_or_default();
            eprintln!("=== +{:.0}s {name} window={:#x} class={:?} focus={:#x}->{input_name} hosted={} anchor={:?} current={} native_input={}",
                started_at.elapsed().as_secs_f32(),
                snapshot.window_id,
                crate::windows_impl::window_class_name(
                    windows::Win32::Foundation::HWND(snapshot.window_id as _)
                ),
                snapshot.focused_handle,
                snapshot.is_hosted_input(),
                snapshot.anchor.source,
                snapshot.current(),
                snapshot.input_endpoint().is_some());
            let captured = snapshot.capture_target();
            eprintln!(
                "  capture_target: target={} control={} anchor={:?}",
                captured.target.is_some(),
                captured
                    .target
                    .as_ref()
                    .is_some_and(|t| t.focused_control.is_some()),
                captured.anchor.source
            );
            if let Some(uia) = automation.as_ref() {
                use windows::Win32::UI::Accessibility::*;
                unsafe {
                    let direct = uia.GetFocusedElement().ok();
                    let resolved =
                        crate::focus::automation::focused_element_for_inline(uia, &snapshot);
                    let doc_units = |e: &IUIAutomationElement| {
                        e.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
                            .ok()
                            .and_then(|p| p.DocumentRange().ok())
                            .and_then(|r| r.GetText(-1).ok())
                            .map(|t| t.to_string().encode_utf16().count())
                    };
                    match (direct, resolved) {
                        (Some(d), Some((r, legacy))) => {
                            let same = uia
                                .CompareElements(&d, &r)
                                .map(|v| v.as_bool())
                                .unwrap_or(false);
                            eprintln!(
                                "  resolver: legacy={legacy} same={same} focused_units={:?} resolved_units={:?} resolved_ct={:?}",
                                doc_units(&d),
                                doc_units(&r),
                                r.CurrentControlType().ok(),
                            );
                        }
                        (d, r) => eprintln!(
                            "  resolver: direct={} resolved={}",
                            d.is_some(),
                            r.is_some()
                        ),
                    }
                    if let Some((r, _)) = crate::focus::automation::focused_element_for_inline(uia, &snapshot) {
                        describe_editor_shape(uia, &r);
                    }
                }
            }
            match target::Target::open(&snapshot, automation.as_ref()) {
                Ok(opened) => {
                    eprintln!(
                        "  Target::open: OK backend={} current={} left={}",
                        opened.backend(),
                        opened.current(),
                        opened.definitely_left()
                    );
                    match opened.snapshot() {
                        Ok(read) => eprintln!(
                            "  snapshot: units={} selection={}..{}",
                            read.text.len(),
                            read.selection.start,
                            read.selection.end
                        ),
                        Err(error) => eprintln!("  snapshot: ERR {error}"),
                    }
                    let (state, possible) = opened.composition();
                    eprintln!("  composition: state={state} possible={possible}");
                }
                Err(error) => eprintln!("  Target::open: ERR {error}"),
            }
            if let Some(uia) = automation.as_ref() {
                eprintln!(
                    "  plain_paste_target: {}",
                    snapshot.capture_plain_paste_target(uia).is_some()
                );
            }
        }
        unsafe {
            CoUninitialize();
        }
    }

    /// Content-free shape dump for the placeholder-normalization gates in
    /// text_scope.rs: only booleans, counts and enum ids, never text.
    #[cfg(test)]
    unsafe fn describe_editor_shape(
        uia: &windows::Win32::UI::Accessibility::IUIAutomation,
        editor: &windows::Win32::UI::Accessibility::IUIAutomationElement,
    ) {
        use windows::Win32::UI::Accessibility::*;
        let has_token = |hay: &str, needle: &str| {
            hay.split_whitespace().any(|c| c == needle)
        };
        let role = editor
            .CurrentAriaRole()
            .map(|r| r.to_string())
            .unwrap_or_default();
        let name = editor.CurrentName().map(|n| n.to_vec()).unwrap_or_default();
        let autoid = editor
            .CurrentAutomationId()
            .map(|n| n.to_string())
            .unwrap_or_default();
        let class = editor
            .CurrentClassName()
            .map(|c| c.to_string())
            .unwrap_or_default();
        eprintln!(
            "  editor: ct={:?} role_textbox={} name_len={} name_chatgpt={} autoid_prompt={} pm={} ek_container={} innerdoc={}",
            editor.CurrentControlType().ok(),
            role == "textbox" || role == "searchbox",
            name.len(),
            name == "Chat with ChatGPT".encode_utf16().collect::<Vec<u16>>(),
            autoid == "prompt-textarea",
            has_token(&class, "ProseMirror"),
            has_token(&class, "editor-kit-container"),
            has_token(&class, "innerdocbody"),
        );
        let Ok(pattern) =
            editor.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
        else {
            eprintln!("  editor: no TextPattern");
            return;
        };
        let Ok(doc) = pattern.DocumentRange() else {
            eprintln!("  editor: no DocumentRange");
            return;
        };
        let raw = doc.GetText(520).map(|t| t.to_vec()).unwrap_or_default();
        let selected = pattern
            .GetSelection()
            .ok()
            .and_then(|s| s.GetElement(0).ok());
        let (collapsed, at_doc_start) = match &selected {
            Some(sel) => (
                sel.CompareEndpoints(
                    TextPatternRangeEndpoint_Start,
                    sel,
                    TextPatternRangeEndpoint_End,
                )
                .ok()
                    == Some(0),
                sel.CompareEndpoints(
                    TextPatternRangeEndpoint_Start,
                    &doc,
                    TextPatternRangeEndpoint_Start,
                )
                .ok()
                    == Some(0),
            ),
            None => (false, false),
        };
        let value = editor
            .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            .and_then(|p| p.CurrentValue())
            .map(|v| v.to_vec());
        // aria-placeholder is the semantic carrier of generated hint text;
        // report only whether its value reconstructs the document text.
        let aria_placeholder_units = editor
            .CurrentAriaProperties()
            .ok()
            .and_then(|props| {
                props
                    .to_string()
                    .split(';')
                    .find_map(|pair| pair.strip_prefix("placeholder=").map(str::to_string))
            })
            .map(|v| v.encode_utf16().collect::<Vec<_>>());
        eprintln!(
            "  doc: units={} collapsed={} at_doc_start={} value_eq_doc={} nl_name={} crlf_name={} aria_ph={:?} aria_ph_eq_doc={}",
            raw.len(),
            collapsed,
            at_doc_start,
            value.as_ref().is_ok_and(|v| *v == raw),
            raw.strip_prefix(&[10]) == Some(name.as_slice()),
            raw.strip_prefix(&[13, 10]) == Some(name.as_slice()),
            aria_placeholder_units.as_ref().map(|v| v.len()),
            aria_placeholder_units.as_ref().is_some_and(|v| *v == raw),
        );
        let Ok(walker) = uia.RawViewWalker() else { return };
        let Some(paragraph) = walker.GetFirstChildElement(editor).ok() else {
            eprintln!("  children: none");
            return;
        };
        // Per-child text contribution: which subtree carries the placeholder units.
        {
            let mut child = Some(paragraph.clone());
            let mut index = 0usize;
            while let Some(c) = child {
                let units = pattern
                    .RangeFromChild(&c)
                    .and_then(|r| r.GetText(520))
                    .map(|t| t.to_vec().len())
                    .unwrap_or(usize::MAX);
                let c_class = c
                    .CurrentClassName()
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                let c_aria = c
                    .CurrentAriaProperties()
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                eprintln!(
                    "  child[{index}]: ct={:?} units={units} name_len={} class_placeholder={} class_empty={} readonly={} aria_ph={}",
                    c.CurrentControlType().ok(),
                    c.CurrentName().map(|n| n.to_vec().len()).unwrap_or(0),
                    has_token(&c_class, "placeholder"),
                    c_class.is_empty(),
                    c_aria.split(';').any(|p| p == "readonly=true"),
                    c_aria.split(';').any(|p| p.starts_with("placeholder=")),
                );
                if let Ok(grand) = walker.GetFirstChildElement(&c) {
                    let mut g: Option<IUIAutomationElement> = Some(grand);
                    let mut gi = 0usize;
                    while let Some(ge) = g {
                        let g_units = pattern
                            .RangeFromChild(&ge)
                            .and_then(|r| r.GetText(520))
                            .map(|t| t.to_vec().len())
                            .unwrap_or(usize::MAX);
                        let g_class = ge
                            .CurrentClassName()
                            .map(|s| s.to_string())
                            .unwrap_or_default();
                        let g_aria = ge
                            .CurrentAriaProperties()
                            .map(|s| s.to_string())
                            .unwrap_or_default();
                        eprintln!(
                            "    child[{index}][{gi}]: ct={:?} units={g_units} name_len={} class_placeholder={} class_empty={} readonly={} aria_ph={}",
                            ge.CurrentControlType().ok(),
                            ge.CurrentName().map(|n| n.to_vec().len()).unwrap_or(0),
                            has_token(&g_class, "placeholder"),
                            g_class.is_empty(),
                            g_aria.split(';').any(|p| p == "readonly=true"),
                            g_aria.split(';').any(|p| p.starts_with("placeholder=")),
                        );
                        gi += 1;
                        if gi > 8 {
                            break;
                        }
                        g = walker.GetNextSiblingElement(&ge).ok();
                    }
                }
                index += 1;
                if index > 8 {
                    break;
                }
                child = walker.GetNextSiblingElement(&c).ok();
            }
        }
        let second = walker.GetNextSiblingElement(&paragraph).ok();
        let p_class = paragraph
            .CurrentClassName()
            .map(|c| c.to_string())
            .unwrap_or_default();
        let enclosing = doc.GetEnclosingElement().ok();
        let enc_is_paragraph = enclosing.as_ref().is_some_and(|e| {
            uia.CompareElements(e, &paragraph)
                .map(|v| v.as_bool())
                .unwrap_or(false)
        });
        eprintln!(
            "  paragraph: ct={:?} second_sibling={} class_placeholder={} enc_is_paragraph={} enc_ct={:?} enc_name_eq_doc={}",
            paragraph.CurrentControlType().ok(),
            second.is_some(),
            has_token(&p_class, "placeholder"),
            enc_is_paragraph,
            enclosing.as_ref().and_then(|e| e.CurrentControlType().ok()),
            enclosing.as_ref().is_some_and(|e| {
                e.CurrentName().map(|n| n.to_vec()).unwrap_or_default() == raw
            }),
        );
        if let Ok(decoration) = walker.GetFirstChildElement(&paragraph) {
            let d_next = walker.GetNextSiblingElement(&decoration).ok();
            let d_class = decoration
                .CurrentClassName()
                .map(|c| c.to_string())
                .unwrap_or_default();
            let d_name = decoration
                .CurrentName()
                .map(|n| n.to_vec())
                .unwrap_or_default();
            let leaf_is_enc = enclosing.as_ref().is_some_and(|e| {
                uia.CompareElements(e, &decoration)
                    .map(|v| v.as_bool())
                    .unwrap_or(false)
            });
            eprintln!(
                "  decoration: ct={:?} next_sibling={} name_empty={} class_empty={} leaf_eq_enclosing={}",
                decoration.CurrentControlType().ok(),
                d_next.is_some(),
                d_name.is_empty(),
                d_class.is_empty(),
                leaf_is_enc,
            );
            if let Ok(leaf) = walker.GetFirstChildElement(&decoration) {
                let l_next = walker.GetNextSiblingElement(&leaf).ok();
                let leaf2_is_enc = enclosing.as_ref().is_some_and(|e| {
                    uia.CompareElements(e, &leaf)
                        .map(|v| v.as_bool())
                        .unwrap_or(false)
                });
                eprintln!(
                    "  leaf: ct={:?} next_sibling={} leaf_eq_enclosing={}",
                    leaf.CurrentControlType().ok(),
                    l_next.is_some(),
                    leaf2_is_enc,
                );
            }
        } else {
            eprintln!("  decoration: none");
        }
        // Evaluate the real normalization gates plus per-leaf blank flags so a
        // field repro can bisect the failing condition without text content.
        if let Some(sel) = &selected {
            let blank = |units: &[u16]| {
                units.iter().all(|u| {
                    *u == 0x200b || char::from_u32(u32::from(*u)).is_some_and(char::is_whitespace)
                })
            };
            let mut pieces: Vec<u16> = Vec::new();
            let mut leaf_kinds: Vec<String> = Vec::new();
            let mut leaf_child = walker.GetFirstChildElement(&paragraph).ok();
            for _ in 0..8 {
                let Some(le) = leaf_child else { break };
                let units = pattern
                    .RangeFromChild(&le)
                    .and_then(|r| r.GetText(-1))
                    .map(|t| t.to_vec())
                    .unwrap_or_default();
                let ro = le
                    .CurrentAriaProperties()
                    .ok()
                    .is_some_and(|a| a.to_string().split(';').any(|p| p == "readonly=true"));
                leaf_kinds.push(format!(
                    "{:?}/{}u/ro={}/blank={}",
                    le.CurrentControlType().ok().map(|c| c.0),
                    units.len(),
                    ro,
                    blank(&units)
                ));
                pieces.extend_from_slice(&units);
                leaf_child = walker.GetNextSiblingElement(&le).ok();
            }
            eprintln!(
                "  leaves: [{}] pieces_eq_doc={} gates: decorated={} readonly_hint={} value={} paragraph={}",
                leaf_kinds.join(","),
                pieces == raw,
                super::text_scope::empty_decorated_paragraph_caret(uia, editor, &pattern, &doc, sel),
                super::text_scope::empty_readonly_hint_caret(uia, editor, &pattern, &doc, sel),
                super::text_scope::empty_value_caret(editor, sel),
                super::text_scope::empty_paragraph_caret(editor, &doc, sel),
            );
        }
        // Attachment evidence: histogram of substantial descendants (>=32px on
        // either axis) per ancestor container, by control type id.
        {
            let Ok(true_condition) = uia.CreateTrueCondition() else {
                return;
            };
            let mut scope = editor.clone();
            for depth in 0..3 {
                let Ok(all) = scope.FindAll(TreeScope_Descendants, &true_condition) else {
                    break;
                };
                let Ok(count) = all.Length() else { break };
                let mut histogram = std::collections::BTreeMap::<i32, (u32, u32)>::new();
                let mut sizeable = 0u32;
                for index in 0..count.min(400) {
                    let Ok(element) = all.GetElement(index) else { continue };
                    let Ok(ct) = element.CurrentControlType() else { continue };
                    let entry = histogram.entry(ct.0).or_default();
                    entry.0 += 1;
                    if let Ok(rect) = element.CurrentBoundingRectangle() {
                        if rect.right - rect.left >= 32 && rect.bottom - rect.top >= 32 {
                            entry.1 += 1;
                            sizeable += 1;
                        }
                    }
                }
                let summary: Vec<String> = histogram
                    .iter()
                    .map(|(ct, (total, big))| format!("{ct}:{total}/{big}"))
                    .collect();
                eprintln!(
                    "  scope[{depth}]: descendants={} sizeable={} [{}]",
                    count, sizeable, summary.join(" ")
                );
                let Ok(next) = walker.GetParentElement(&scope) else { break };
                if next.CurrentProcessId().ok() != editor.CurrentProcessId().ok() {
                    break;
                }
                scope = next;
            }
        }
    }
}
