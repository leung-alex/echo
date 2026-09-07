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
mod ime_window;
mod keyboard;
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
}
impl std::fmt::Debug for InlineEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InlineEvent(<redacted>)")
    }
}
pub type EventHandler = Arc<dyn Fn(InlineEvent) + Send + Sync>;

pub(super) const IME_CLEAR: u8 = 0;
pub(super) const IME_ACTIVE: u8 = 1;
pub(super) const IME_UNKNOWN: u8 = 2;
/// Marker only for Echo's own Ctrl+V injection. Other synthetic input (e.g.
/// accessibility tools and authorized tests) follows the same rules as typing.
pub(crate) const INJECTED_TAG: usize = 0x4543_484f;

pub(super) struct Shared {
    requested: AtomicU64,
    active: AtomicU64,
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
    ime: AtomicU8,
    may_compose: AtomicBool,
    ime_ui: AtomicU8,
    native_identity: AtomicBool,
    dirty_queued: AtomicBool,
    callback: EventHandler,
}
impl Shared {
    fn new(callback: EventHandler) -> Arc<Self> {
        Arc::new(Self {
            requested: AtomicU64::new(0),
            active: AtomicU64::new(0),
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
            ime: AtomicU8::new(IME_UNKNOWN),
            may_compose: AtomicBool::new(false),
            ime_ui: AtomicU8::new(IME_UNKNOWN),
            native_identity: AtomicBool::new(false),
            dirty_queued: AtomicBool::new(false),
            callback,
        })
    }
    fn ticket(&self) -> InlineTicket {
        InlineTicket {
            session: self.active.load(Ordering::Acquire),
            revision: self.revision.load(Ordering::Acquire),
            input_serial: self.observed_serial.load(Ordering::Acquire),
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
        let (reported, possible) = target.composition();
        let observed = self.ime_ui.load(Ordering::Acquire);
        let state = if observed == IME_ACTIVE {
            IME_ACTIVE
        } else if reported == IME_UNKNOWN && observed == IME_CLEAR {
            IME_CLEAR
        } else {
            reported
        };
        (state, possible)
    }
    fn can_confirm(&self) -> bool {
        let t = self.ticket();
        self.accepts(t)
            && self.selectable.load(Ordering::Acquire)
            && self.displayed_revision.load(Ordering::Acquire) == t.revision
            && self.displayed_serial.load(Ordering::Acquire) == t.input_serial
            && self.ime.load(Ordering::Acquire) == IME_CLEAR
            && !self.committing.load(Ordering::Acquire)
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
    expires: Instant,
}
impl Deadline {
    fn new(timeout: Duration) -> Arc<Self> {
        Arc::new(Self {
            cancelled: AtomicBool::new(false),
            expires: Instant::now() + timeout,
        })
    }
    fn live(&self) -> bool {
        !self.cancelled.load(Ordering::Acquire) && Instant::now() < self.expires
    }
}
pub(super) enum Request {
    Begin(u64, FocusSnapshot),
    Observe(u64),
    Cancel(u64),
    Check(
        InlineTicket,
        String,
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
            .map_err(|_| "Inline input service is busy; use independent search".to_string())
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
    pub fn invalidate_results(&self) {
        self.inner.shared.selectable.store(false, Ordering::Release);
    }
    pub fn live(&self, ticket: InlineTicket) -> bool {
        self.inner.shared.accepts(ticket)
    }
    pub fn preflight(
        &self,
        ticket: InlineTicket,
        payload: &[ClipboardRepresentation],
    ) -> Result<(), String> {
        let text = payload.iter().find(|r| r.format == "text")
            .ok_or("This item has no plain-text representation. Use Copy or independent search; the query was not deleted.")?;
        let text = String::from_utf8(text.bytes.clone())
            .map_err(|_| "Original text is not valid UTF-8")?;
        if text.encode_utf16().count() > echo_engine::MAX_COMPOSER_UNITS || text.contains('\0') {
            return Err(
                "Item is too large for verified inline replacement; use independent search".into(),
            );
        }
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
            Ok(result) => result,
            Err(_) => {
                guard.cancelled.store(true, Ordering::Release);
                // A provider may have completed a native paste before the acknowledgement.
                // Never replay automatically or guess that the target was unchanged.
                Ok(PasteDelivery::Failed(
                    PasteDeliveryFailure::ReplacementUnconfirmed,
                ))
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
    prepared: Option<(InlineTicket, String)>,
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
                        hook.disarm(active.id);
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
                let fallback_snapshot = snapshot.clone();
                let result = begin(id, snapshot, &shared, &hook, &sender, automation.as_ref());
                match result {
                    Ok(active) => session = Some(active),
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
                let result = session
                    .as_mut()
                    .filter(|s| s.id == ticket.session)
                    .ok_or_else(|| "Inline session ended".to_string())
                    .and_then(|s| preflight(s, &shared, ticket, text, &deadline));
                let _ = reply.send(result);
            }
            Ok(Request::Paste(ticket, sequence, deadline, reply)) => {
                let result = session
                    .as_mut()
                    .filter(|s| s.id == ticket.session)
                    .ok_or_else(|| "Inline session ended".to_string())
                    .and_then(|s| paste(s, &shared, ticket, sequence, &deadline));
                shared.committing.store(false, Ordering::Release);
                if matches!(result, Ok(PasteDelivery::Pasted)) {
                    shared.active.store(0, Ordering::Release);
                    hook.disarm(ticket.session);
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
    shared.active.store(id, Ordering::Release);
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
    let serial = shared.input_serial.load(Ordering::Acquire);
    let backend = target::Target::open(&snapshot, uia)?;
    shared
        .native_identity
        .store(backend.backend() == "native-edit", Ordering::Release);
    let initial = backend.snapshot()?;
    let range = QueryRange::begin(&initial).map_err(str::to_string)?;
    let (composition, may_compose) = shared.composition(&backend);
    // A readable input may start with composition active/unknown. Keep focus
    // and filter actual provider text, but do not authorize Enter replacement.
    let anchor = backend.anchor().unwrap_or(snapshot.anchor);
    if shared.requested.load(Ordering::Acquire) != id {
        return Err("Inline activation was cancelled".into());
    }
    if shared.input_serial.load(Ordering::Acquire) != serial {
        return Err("The composer changed while the inline range was being captured. Typed text was kept; use independent search or invoke again.".into());
    }
    shared.ime.store(composition, Ordering::Release);
    shared.may_compose.store(may_compose, Ordering::Release);
    shared.revision.store(range.revision(), Ordering::Release);
    shared.observed_serial.store(serial, Ordering::Release);
    let subscriptions = backend.automation_element().and_then(|(uia, element)| {
        subscriptions::Subscription::new(uia, element, shared.clone(), sender.clone(), id)
    });
    (shared.callback)(InlineEvent::Started(InlineStarted {
        ticket: shared.ticket(),
        query: range.query(),
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
    if shared.committing.load(Ordering::Acquire) {
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
            if session.read_failures < 4 && session.backend.current() {
                session.read_failures += 1;
                shared.selectable.store(false, Ordering::Release);
                return true;
            }
            eprintln!("Echo inline range observation rejected: {error}");
            shared.cancel(
                session.id,
                "Input control or text range is no longer available; typed query kept",
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
        if composition != IME_ACTIVE {
            shared.cancel(session.id, reason);
            return false;
        }
        // Do not expand a protected query across an IME's transient range.
        shared.selectable.store(false, Ordering::Release);
    }
    session.prepared = None;
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
        query: session.range.query(),
        anchor: Some(session.anchor),
        composing: composition == IME_ACTIVE,
        suspended: composition == IME_UNKNOWN,
    });
    false
}
fn preflight(
    session: &mut Session,
    shared: &Shared,
    ticket: InlineTicket,
    text: String,
    guard: &Deadline,
) -> Result<(), String> {
    if !guard.live() || !shared.accepts(ticket) {
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
    if !guard.live() || !shared.accepts(ticket) {
        return Err("The query changed during validation; nothing was replaced".into());
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
    if prepared != ticket || !guard.live() || !shared.accepts(ticket) {
        return Ok(PasteDelivery::Failed(PasteDeliveryFailure::RangeChanged));
    }
    if shared.composition(&session.backend).0 != IME_CLEAR {
        return Ok(PasteDelivery::Failed(PasteDeliveryFailure::CompositionBusy));
    }
    let snapshot = session.backend.snapshot()?;
    let span = session
        .range
        .seal(&snapshot, ticket.revision)
        .map_err(str::to_string)?;
    if !crate::windows_impl::validate_inline_clipboard(sequence, &inserted) {
        return Ok(PasteDelivery::Failed(
            PasteDeliveryFailure::ClipboardChanged,
        ));
    }
    shared.committing.store(true, Ordering::Release);
    session.backend.select(span, &snapshot, guard)?;
    if !guard.live() || !shared.accepts(ticket) {
        return Ok(PasteDelivery::Failed(PasteDeliveryFailure::RangeChanged));
    }
    if !crate::windows_impl::validate_inline_clipboard(sequence, &inserted) {
        return Ok(PasteDelivery::Failed(
            PasteDeliveryFailure::ClipboardChanged,
        ));
    }
    // No mutation was performed before exact selection and all identities were
    // rechecked. The native paste is one edit; no separate query deletion occurs.
    if session.backend.paste_selected().is_err() {
        return Ok(PasteDelivery::Failed(
            PasteDeliveryFailure::ReplacementUnconfirmed,
        ));
    }
    let expected = session.backend.normalize_inserted(&inserted);
    let lf = inserted
        .replace("\r\n", "\n")
        .encode_utf16()
        .collect::<Vec<_>>();
    let crlf = inserted
        .replace("\r\n", "\n")
        .replace('\n', "\r\n")
        .encode_utf16()
        .collect::<Vec<_>>();
    let started = Instant::now();
    while guard.live() && started.elapsed() < Duration::from_millis(300) {
        if !shared.accepts(ticket) {
            break;
        }
        if let Ok(actual) = session.backend.snapshot() {
            if session.range.matches_replacement(&actual.text, &expected)
                || session.range.matches_replacement(&actual.text, &lf)
                || session.range.matches_replacement(&actual.text, &crlf)
            {
                return Ok(PasteDelivery::Pasted);
            }
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    Ok(PasteDelivery::Failed(
        PasteDeliveryFailure::ReplacementUnconfirmed,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
