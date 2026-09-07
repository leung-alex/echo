//! Thread-affine Win32 hotkeys with reserve/commit/abort settings updates.
use super::{EventHandler, ShellEvent};
use echo_engine::{GlobalShortcut, ShortcutKey, UiSettings};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering},
    mpsc::{self, Receiver, SyncSender},
    Arc,
};
use std::time::{Duration, Instant};
use windows_sys::Win32::{
    Foundation::*,
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

pub(super) const MESSAGE: u32 = WM_APP + 28;
const FIRST_ID: i32 = 0x4540;
const SECOND_ID: i32 = 0x4541;
const LEASE_TIMER_ID: usize = 0x4542;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(3);
const QUEUED: u8 = 0;
const CLAIMED: u8 = 1;
const COMPLETED: u8 = 2;
const CANCELLED: u8 = 3;

struct Lease {
    cancelled: AtomicBool,
    expires: Instant,
}

impl Lease {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            cancelled: AtomicBool::new(false),
            expires: Instant::now() + REQUEST_TIMEOUT,
        })
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    fn invalid(&self, now: Instant) -> bool {
        self.cancelled.load(Ordering::Acquire) || now >= self.expires
    }
}

struct Gate(AtomicU8);

impl Gate {
    fn new() -> Arc<Self> {
        Arc::new(Self(AtomicU8::new(QUEUED)))
    }

    fn claim(&self) -> bool {
        self.0
            .compare_exchange(QUEUED, CLAIMED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    fn cancel_queued(&self) -> bool {
        self.0
            .compare_exchange(QUEUED, CANCELLED, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }
}

#[derive(Clone)]
pub struct HotkeyController {
    hwnd: isize,
    sender: SyncSender<Request>,
    serial: Arc<AtomicU64>,
}

pub struct HotkeyReservation {
    controller: HotkeyController,
    token: u64,
    lease: Arc<Lease>,
    finished: bool,
}

pub(super) struct Request {
    command: Change,
    reply: SyncSender<Result<(), String>>,
    deadline: Instant,
    gate: Arc<Gate>,
}

enum Change {
    Prepare(u64, Option<GlobalShortcut>, Arc<Lease>),
    Commit(u64, Arc<Lease>),
    Abort(u64, Arc<Lease>),
}

impl Change {
    fn lease(&self) -> &Arc<Lease> {
        match self {
            Self::Prepare(_, _, lease) | Self::Commit(_, lease) | Self::Abort(_, lease) => lease,
        }
    }
}

impl HotkeyController {
    pub(super) fn new(hwnd: isize, sender: SyncSender<Request>) -> Self {
        Self {
            hwnd,
            sender,
            serial: Arc::new(AtomicU64::new(0)),
        }
    }

    fn wake(&self) -> bool {
        unsafe { PostMessageW(self.hwnd as HWND, MESSAGE, 0, 0) != 0 }
    }

    fn request(&self, command: Change) -> Result<(), String> {
        let lease = command.lease().clone();
        let gate = Gate::new();
        let (reply, response) = mpsc::sync_channel(1);
        if self
            .sender
            .try_send(Request {
                command,
                reply,
                deadline: Instant::now() + REQUEST_TIMEOUT,
                gate: gate.clone(),
            })
            .is_err()
        {
            lease.cancel();
            let _ = self.wake();
            return Err("Global shortcut service is busy or stopped".into());
        }
        if !self.wake() {
            if gate.cancel_queued() {
                lease.cancel();
                return Err("Could not reach the global shortcut service".into());
            }
        }
        match response.recv_timeout(REQUEST_TIMEOUT) {
            Ok(result) => result,
            Err(mpsc::RecvTimeoutError::Timeout) if gate.cancel_queued() => {
                lease.cancel();
                let _ = self.wake();
                Err("Global shortcut service did not respond before the request was claimed".into())
            }
            Err(mpsc::RecvTimeoutError::Timeout) => response.recv().map_err(|_| {
                "Global shortcut result channel closed after the operation was claimed; reconcile persisted settings with the reported runtime status".to_string()
            })?,
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                lease.cancel();
                Err("Global shortcut result channel closed; the runtime outcome is unknown and persisted settings must be reconciled".into())
            }
        }
    }

    pub fn prepare(&self, settings: &UiSettings) -> Result<HotkeyReservation, String> {
        let key = GlobalShortcut::parse(&settings.global_hotkey)?;
        let token = self.serial.fetch_add(1, Ordering::Relaxed) + 1;
        let lease = Lease::new();
        self.request(Change::Prepare(
            token,
            settings.global_hotkey_enabled.then_some(key),
            lease.clone(),
        ))?;
        Ok(HotkeyReservation {
            controller: self.clone(),
            token,
            lease,
            finished: false,
        })
    }

    pub fn apply(&self, settings: &UiSettings) -> Result<(), String> {
        self.prepare(settings)?.commit()
    }
}

impl HotkeyReservation {
    pub fn commit(mut self) -> Result<(), String> {
        let result = self
            .controller
            .request(Change::Commit(self.token, self.lease.clone()));
        if result.is_ok() {
            self.finished = true;
        } else {
            self.lease.cancel();
            let _ = self.controller.wake();
        }
        result
    }
}

impl Drop for HotkeyReservation {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        self.lease.cancel();
        let (reply, _) = mpsc::sync_channel(1);
        let _ = self.controller.sender.try_send(Request {
            command: Change::Abort(self.token, self.lease.clone()),
            reply,
            deadline: Instant::now() + REQUEST_TIMEOUT,
            gate: Gate::new(),
        });
        let _ = self.controller.wake();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Binding {
    id: i32,
    key: GlobalShortcut,
}

struct Pending {
    token: u64,
    candidate: Option<Binding>,
    unchanged: bool,
    lease: Arc<Lease>,
}

trait Registrar: Send {
    fn register(&mut self, hwnd: HWND, binding: &Binding) -> Result<(), String>;
    fn unregister(&mut self, hwnd: HWND, binding: &Binding) -> Result<(), String>;
    fn arm_lease(&mut self, hwnd: HWND, expires: Instant) -> Result<(), String>;
    fn disarm_lease(&mut self, hwnd: HWND);
}

struct Win32Registrar;

unsafe extern "system" fn lease_timer(hwnd: HWND, _: u32, timer: usize, _: u32) {
    KillTimer(hwnd, timer);
    PostMessageW(hwnd, MESSAGE, 0, 0);
}

impl Registrar for Win32Registrar {
    fn register(&mut self, hwnd: HWND, binding: &Binding) -> Result<(), String> {
        let (modifiers, vk) = native_key(&binding.key);
        if unsafe { RegisterHotKey(hwnd, binding.id, modifiers | MOD_NOREPEAT, vk) } == 0 {
            let error = unsafe { GetLastError() };
            Err(format!("Windows error {error}"))
        } else {
            Ok(())
        }
    }

    fn unregister(&mut self, hwnd: HWND, binding: &Binding) -> Result<(), String> {
        if unsafe { UnregisterHotKey(hwnd, binding.id) } == 0 {
            let error = unsafe { GetLastError() };
            Err(format!("Windows error {error}"))
        } else {
            Ok(())
        }
    }

    fn arm_lease(&mut self, hwnd: HWND, expires: Instant) -> Result<(), String> {
        let millis = expires
            .saturating_duration_since(Instant::now())
            .as_millis()
            .clamp(1, u32::MAX as u128) as u32;
        if unsafe { SetTimer(hwnd, LEASE_TIMER_ID, millis, Some(lease_timer)) } == 0 {
            let error = unsafe { GetLastError() };
            Err(format!(
                "could not schedule reservation expiry (Windows error {error})"
            ))
        } else {
            Ok(())
        }
    }

    fn disarm_lease(&mut self, hwnd: HWND) {
        unsafe {
            KillTimer(hwnd, LEASE_TIMER_ID);
        }
    }
}

pub(super) struct Host {
    receiver: Receiver<Request>,
    active: Option<Binding>,
    pending: Option<Pending>,
    retained: Vec<Binding>,
    disabled: bool,
    registrar: Box<dyn Registrar>,
}

fn native_key(key: &GlobalShortcut) -> (u32, u32) {
    let modifiers = if key.alt { MOD_ALT } else { 0 }
        | if key.control { MOD_CONTROL } else { 0 }
        | if key.shift { MOD_SHIFT } else { 0 };
    let vk = match key.key {
        ShortcutKey::Character(c) => c as u32,
        ShortcutKey::Function(n) => u32::from(VK_F1) + u32::from(n) - 1,
        ShortcutKey::Space => u32::from(VK_SPACE),
    };
    (modifiers, vk)
}

impl Host {
    pub(super) fn new(receiver: Receiver<Request>) -> Self {
        Self::with_registrar(
            receiver,
            Box::new(Win32Registrar),
            std::env::var("ECHO_DISABLE_GLOBAL_HOTKEY").as_deref() == Ok("1"),
        )
    }

    fn with_registrar(
        receiver: Receiver<Request>,
        registrar: Box<dyn Registrar>,
        disabled: bool,
    ) -> Self {
        Self {
            receiver,
            active: None,
            pending: None,
            retained: Vec::new(),
            disabled,
            registrar,
        }
    }

    fn status(&self) -> String {
        if self.disabled {
            "Global shortcut disabled for this isolated instance (ECHO_DISABLE_GLOBAL_HOTKEY)"
                .into()
        } else if let Some(active) = &self.active {
            format!("Active globally: {}", active.key.canonical())
        } else {
            "Global shortcut is off".into()
        }
    }

    fn release(&mut self, hwnd: HWND, binding: Binding, context: &str) -> Result<(), String> {
        match self.registrar.unregister(hwnd, &binding) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.retained.push(binding);
                Err(format!(
                    "{context}; cleanup could not be confirmed ({error})"
                ))
            }
        }
    }

    fn clear_pending(&mut self, hwnd: HWND, context: &str) -> Result<(), String> {
        self.registrar.disarm_lease(hwnd);
        let Some(pending) = self.pending.take() else {
            return Ok(());
        };
        if pending.unchanged {
            return Ok(());
        }
        match pending.candidate {
            Some(candidate) => self.release(hwnd, candidate, context),
            None => Ok(()),
        }
    }

    fn reap_invalid(&mut self, hwnd: HWND) -> Option<String> {
        let invalid = self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.lease.invalid(Instant::now()));
        invalid.then(|| {
            self.clear_pending(hwnd, "Shortcut reservation expired")
                .err()
                .unwrap_or_else(|| self.status())
        })
    }

    fn retry_retained_cleanup(&mut self, hwnd: HWND) -> Result<(), String> {
        let mut errors = Vec::new();
        for binding in std::mem::take(&mut self.retained) {
            if let Err(error) = self.release(hwnd, binding, "Previous shortcut cleanup is pending")
            {
                errors.push(error);
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; "))
        }
    }

    fn prepare(
        &mut self,
        hwnd: HWND,
        token: u64,
        key: Option<GlobalShortcut>,
        lease: Arc<Lease>,
    ) -> Result<(), String> {
        if lease.invalid(Instant::now()) {
            return Err("Shortcut reservation expired before preparation".into());
        }
        if self.pending.is_some() {
            return Err("Another shortcut update is in progress".into());
        }
        // Never recycle a Win32 registration ID whose earlier unregister failed.
        // Retries are event-driven by the next change, not a resident polling loop.
        self.retry_retained_cleanup(hwnd)?;
        let key = if self.disabled { None } else { key };
        let unchanged = self.active.as_ref().map(|a| &a.key) == key.as_ref();
        let candidate = if unchanged {
            self.active.clone()
        } else if let Some(key) = key {
            let id = if self.active.as_ref().is_some_and(|a| a.id == FIRST_ID) {
                SECOND_ID
            } else {
                FIRST_ID
            };
            let binding = Binding { id, key };
            self.registrar.register(hwnd, &binding).map_err(|error| {
                format!("Cannot register {} ({error}). Another app or Windows may own it. {}. Choose another shortcut; your previous binding is unchanged.", binding.key.canonical(), self.status())
            })?;
            Some(binding)
        } else {
            None
        };
        if let Err(error) = self.registrar.arm_lease(hwnd, lease.expires) {
            if !unchanged {
                if let Some(candidate) = candidate {
                    return self
                        .release(hwnd, candidate, "Reservation timer setup failed")
                        .and(Err(error));
                }
            }
            return Err(error);
        }
        self.pending = Some(Pending {
            token,
            candidate,
            unchanged,
            lease,
        });
        Ok(())
    }

    fn commit(&mut self, hwnd: HWND, token: u64, lease: &Lease) -> Result<(), String> {
        if !self.pending.as_ref().is_some_and(|p| p.token == token) {
            return Err("Shortcut reservation expired".into());
        }
        if lease.invalid(Instant::now()) {
            let cleanup = self.clear_pending(hwnd, "Expired shortcut reservation");
            return cleanup.and(Err("Shortcut reservation expired".into()));
        }
        if self.pending.as_ref().is_some_and(|p| p.unchanged) {
            self.pending.take();
            self.registrar.disarm_lease(hwnd);
            return Ok(());
        }

        // Successful removal of the old binding is the native commit linearization point.
        if let Some(old) = self.active.as_ref() {
            self.registrar.unregister(hwnd, old).map_err(|error| {
                format!(
                    "Could not replace {}; the previous binding remains active ({error})",
                    old.key.canonical()
                )
            })?;
        }
        let pending = self.pending.take().expect("token checked above");
        self.registrar.disarm_lease(hwnd);
        self.active = pending.candidate;
        Ok(())
    }

    fn abort(&mut self, hwnd: HWND, token: u64) -> Result<(), String> {
        if self.pending.as_ref().is_some_and(|p| p.token == token) {
            self.clear_pending(hwnd, "Could not release the cancelled shortcut reservation")
        } else {
            Ok(())
        }
    }

    pub(super) fn drain(&mut self, hwnd: HWND, handler: &EventHandler) {
        if let Some(status) = self.reap_invalid(hwnd) {
            (handler)(ShellEvent::HotkeyStatus(status));
        }
        while let Ok(request) = self.receiver.try_recv() {
            if !request.gate.claim() {
                if let Change::Commit(token, _) | Change::Abort(token, _) = &request.command {
                    let _ = self.abort(hwnd, *token);
                }
                continue;
            }
            let result = if Instant::now() >= request.deadline {
                match &request.command {
                    Change::Commit(token, _) | Change::Abort(token, _) => {
                        let cleanup = self.abort(hwnd, *token);
                        cleanup.and(Err("Shortcut request expired before processing".into()))
                    }
                    Change::Prepare(_, _, _) => {
                        Err("Shortcut request expired before processing".into())
                    }
                }
            } else {
                match request.command {
                    Change::Prepare(token, key, lease) => self.prepare(hwnd, token, key, lease),
                    Change::Commit(token, lease) => self.commit(hwnd, token, &lease),
                    Change::Abort(token, _) => self.abort(hwnd, token),
                }
            };
            let status = result
                .as_ref()
                .err()
                .cloned()
                .unwrap_or_else(|| self.status());
            request.gate.0.store(COMPLETED, Ordering::Release);
            let _ = request.reply.send(result);
            (handler)(ShellEvent::HotkeyStatus(status));
        }
        if let Some(status) = self.reap_invalid(hwnd) {
            (handler)(ShellEvent::HotkeyStatus(status));
        }
    }

    pub(super) fn matches(&self, id: usize, data: isize) -> bool {
        self.active.as_ref().is_some_and(|active| {
            let (mods, key) = native_key(&active.key);
            active.id as usize == id && ((data as u32) >> 16) == key && (data as u32 & 0xf) == mods
        })
    }

    pub(super) fn stop(&mut self, hwnd: HWND) {
        let _ = self.clear_pending(hwnd, "Shutdown reservation cleanup failed");
        let mut bindings = std::mem::take(&mut self.retained);
        if let Some(active) = self.active.take() {
            bindings.push(active);
        }
        for binding in bindings {
            if let Err(error) = self.registrar.unregister(hwnd, &binding) {
                eprintln!(
                    "Echo could not unregister hotkey {} during shutdown: {error}",
                    binding.key.canonical()
                );
            }
        }
        while let Ok(request) = self.receiver.try_recv() {
            request.gate.cancel_queued();
            request.command.lease().cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeState {
        registered: Vec<Binding>,
        calls: Vec<String>,
        unregister_results: VecDeque<Result<(), String>>,
        register_results: VecDeque<Result<(), String>>,
    }

    struct FakeRegistrar(Arc<Mutex<FakeState>>);

    impl Registrar for FakeRegistrar {
        fn register(&mut self, _: HWND, binding: &Binding) -> Result<(), String> {
            let mut state = self.0.lock().unwrap();
            state.calls.push(format!("register:{}", binding.id));
            if let Some(result) = state.register_results.pop_front() {
                result?;
            }
            state.registered.push(binding.clone());
            Ok(())
        }
        fn unregister(&mut self, _: HWND, binding: &Binding) -> Result<(), String> {
            let mut state = self.0.lock().unwrap();
            state.calls.push(format!("unregister:{}", binding.id));
            if let Some(result) = state.unregister_results.pop_front() {
                result?;
            }
            state.registered.retain(|item| item.id != binding.id);
            Ok(())
        }
        fn arm_lease(&mut self, _: HWND, _: Instant) -> Result<(), String> {
            Ok(())
        }
        fn disarm_lease(&mut self, _: HWND) {}
    }

    fn host() -> (Host, SyncSender<Request>, Arc<Mutex<FakeState>>) {
        let (sender, receiver) = mpsc::sync_channel(8);
        let state = Arc::new(Mutex::new(FakeState::default()));
        (
            Host::with_registrar(receiver, Box::new(FakeRegistrar(state.clone())), false),
            sender,
            state,
        )
    }

    fn lease() -> Arc<Lease> {
        Lease::new()
    }

    #[test]
    fn prepare_commit_and_disable_use_production_transitions() {
        let (mut host, _, state) = host();
        let first = lease();
        host.prepare(
            0 as HWND,
            1,
            Some(GlobalShortcut::parse("Alt+V").unwrap()),
            first.clone(),
        )
        .unwrap();
        assert!(host.active.is_none());
        host.commit(0 as HWND, 1, &first).unwrap();
        assert_eq!(host.status(), "Active globally: Alt+V");
        let disable = lease();
        host.prepare(0 as HWND, 2, None, disable.clone()).unwrap();
        host.commit(0 as HWND, 2, &disable).unwrap();
        assert!(host.active.is_none());
        assert!(state.lock().unwrap().registered.is_empty());
    }

    #[test]
    fn conflict_and_unregister_failure_preserve_old_binding() {
        let (mut host, _, state) = host();
        host.active = Some(Binding {
            id: FIRST_ID,
            key: GlobalShortcut::parse("Alt+V").unwrap(),
        });
        state
            .lock()
            .unwrap()
            .registered
            .push(host.active.clone().unwrap());
        let next = lease();
        host.prepare(
            0 as HWND,
            2,
            Some(GlobalShortcut::parse("Ctrl+Space").unwrap()),
            next.clone(),
        )
        .unwrap();
        state
            .lock()
            .unwrap()
            .unregister_results
            .push_back(Err("denied".into()));
        assert!(host
            .commit(0 as HWND, 2, &next)
            .unwrap_err()
            .contains("remains active"));
        assert_eq!(host.status(), "Active globally: Alt+V");
        next.cancel();
        host.reap_invalid(0 as HWND);
        assert_eq!(state.lock().unwrap().registered.len(), 1);
    }

    #[test]
    fn abort_expiry_and_unchanged_release_candidates_correctly() {
        let (mut host, _, state) = host();
        let dropped = lease();
        host.prepare(
            0 as HWND,
            1,
            Some(GlobalShortcut::parse("Alt+V").unwrap()),
            dropped.clone(),
        )
        .unwrap();
        dropped.cancel();
        host.reap_invalid(0 as HWND);
        assert!(host.pending.is_none());
        assert!(state.lock().unwrap().registered.is_empty());

        host.active = Some(Binding {
            id: FIRST_ID,
            key: GlobalShortcut::parse("Alt+V").unwrap(),
        });
        let same = lease();
        host.prepare(
            0 as HWND,
            2,
            Some(GlobalShortcut::parse("Alt+V").unwrap()),
            same.clone(),
        )
        .unwrap();
        host.commit(0 as HWND, 2, &same).unwrap();
        assert_eq!(host.status(), "Active globally: Alt+V");
    }

    #[test]
    fn expired_commit_and_abort_clear_pending() {
        let (mut host, _, state) = host();
        let expired = Arc::new(Lease {
            cancelled: AtomicBool::new(true),
            expires: Instant::now(),
        });
        host.pending = Some(Pending {
            token: 7,
            candidate: Some(Binding {
                id: FIRST_ID,
                key: GlobalShortcut::parse("Alt+V").unwrap(),
            }),
            unchanged: false,
            lease: expired.clone(),
        });
        state
            .lock()
            .unwrap()
            .registered
            .push(host.pending.as_ref().unwrap().candidate.clone().unwrap());
        assert!(host.commit(0 as HWND, 7, &expired).is_err());
        assert!(host.pending.is_none());
        assert!(state.lock().unwrap().registered.is_empty());
        assert!(host.abort(0 as HWND, 7).is_ok());
    }

    #[test]
    fn stale_expired_commit_cannot_clear_a_newer_reservation() {
        let (mut host, _, _) = host();
        let current = lease();
        host.prepare(
            0 as HWND,
            8,
            Some(GlobalShortcut::parse("Alt+V").unwrap()),
            current,
        )
        .unwrap();
        let stale = Arc::new(Lease {
            cancelled: AtomicBool::new(true),
            expires: Instant::now(),
        });
        assert!(host.commit(0 as HWND, 7, &stale).is_err());
        assert_eq!(host.pending.as_ref().map(|pending| pending.token), Some(8));
    }

    #[test]
    fn expired_prepare_never_registers_a_candidate() {
        let (mut host, _, state) = host();
        let expired = Arc::new(Lease {
            cancelled: AtomicBool::new(false),
            expires: Instant::now(),
        });
        assert!(host
            .prepare(
                0 as HWND,
                1,
                Some(GlobalShortcut::parse("Alt+V").unwrap()),
                expired
            )
            .is_err());
        assert!(host.pending.is_none());
        assert!(state.lock().unwrap().registered.is_empty());
    }

    #[test]
    fn drop_cancels_even_when_abort_queue_is_full() {
        let (sender, _receiver) = mpsc::sync_channel(1);
        let filler_lease = lease();
        let (reply, _) = mpsc::sync_channel(1);
        sender
            .send(Request {
                command: Change::Abort(99, filler_lease),
                reply,
                deadline: Instant::now() + REQUEST_TIMEOUT,
                gate: Gate::new(),
            })
            .unwrap();
        let reservation_lease = lease();
        let reservation = HotkeyReservation {
            controller: HotkeyController::new(0, sender),
            token: 1,
            lease: reservation_lease.clone(),
            finished: false,
        };
        drop(reservation);
        assert!(reservation_lease.cancelled.load(Ordering::Acquire));
    }

    #[test]
    fn cancellation_before_execution_and_closed_result_cleanup() {
        let (mut host, sender, state) = host();
        let l = lease();
        let gate = Gate::new();
        let (reply, response) = mpsc::sync_channel(1);
        drop(response);
        sender
            .send(Request {
                command: Change::Prepare(
                    1,
                    Some(GlobalShortcut::parse("Alt+V").unwrap()),
                    l.clone(),
                ),
                reply,
                deadline: Instant::now() + REQUEST_TIMEOUT,
                gate: gate.clone(),
            })
            .unwrap();
        assert!(gate.cancel_queued());
        let handler: EventHandler = Arc::new(|_| {});
        host.drain(0 as HWND, &handler);
        assert!(state.lock().unwrap().registered.is_empty());

        let l = lease();
        let gate = Gate::new();
        let (reply, response) = mpsc::sync_channel(1);
        drop(response);
        sender
            .send(Request {
                command: Change::Prepare(2, Some(GlobalShortcut::parse("Alt+V").unwrap()), l),
                reply,
                deadline: Instant::now() + REQUEST_TIMEOUT,
                gate,
            })
            .unwrap();
        host.drain(0 as HWND, &handler);
        assert!(host.pending.is_some());
        host.pending.as_ref().unwrap().lease.cancel();
        host.reap_invalid(0 as HWND);
        assert!(state.lock().unwrap().registered.is_empty());
    }

    #[test]
    fn late_ack_claim_cannot_be_reported_as_cancelled() {
        let gate = Gate::new();
        assert!(gate.claim());
        assert!(!gate.cancel_queued());
        gate.0.store(COMPLETED, Ordering::Release);
        assert_eq!(gate.0.load(Ordering::Acquire), COMPLETED);
    }

    #[test]
    fn shutdown_attempts_every_owned_id_after_failure() {
        let (mut host, _, state) = host();
        host.active = Some(Binding {
            id: FIRST_ID,
            key: GlobalShortcut::parse("Alt+V").unwrap(),
        });
        host.retained.push(Binding {
            id: SECOND_ID,
            key: GlobalShortcut::parse("Ctrl+Space").unwrap(),
        });
        state
            .lock()
            .unwrap()
            .unregister_results
            .push_back(Err("first failed".into()));
        host.stop(0 as HWND);
        let calls = &state.lock().unwrap().calls;
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.starts_with("unregister:"))
                .count(),
            2
        );
    }

    #[test]
    fn failed_candidate_cleanup_prevents_id_reuse_until_confirmed() {
        let (mut host, _, state) = host();
        let first = lease();
        host.prepare(
            0 as HWND,
            1,
            Some(GlobalShortcut::parse("Alt+V").unwrap()),
            first.clone(),
        )
        .unwrap();
        state
            .lock()
            .unwrap()
            .unregister_results
            .push_back(Err("denied".into()));
        assert!(host.abort(0 as HWND, 1).is_err());
        assert_eq!(host.retained.len(), 1);
        let calls_before = state.lock().unwrap().calls.len();
        state
            .lock()
            .unwrap()
            .unregister_results
            .push_back(Err("still denied".into()));
        assert!(host
            .prepare(
                0 as HWND,
                2,
                Some(GlobalShortcut::parse("Ctrl+Alt+J").unwrap()),
                lease()
            )
            .is_err());
        let state_after = state.lock().unwrap();
        assert_eq!(state_after.registered.len(), 1);
        assert!(state_after.calls[calls_before..]
            .iter()
            .all(|call| call.starts_with("unregister:")));
        drop(state_after);
        let retry = lease();
        host.prepare(
            0 as HWND,
            3,
            Some(GlobalShortcut::parse("Ctrl+Alt+J").unwrap()),
            retry.clone(),
        )
        .unwrap();
        assert!(host.retained.is_empty());
        host.commit(0 as HWND, 3, &retry).unwrap();
        assert_eq!(host.status(), "Active globally: Ctrl+Alt+J");
        assert_eq!(state.lock().unwrap().registered.len(), 1);
        host.stop(0 as HWND);
        assert!(state.lock().unwrap().registered.is_empty());
    }

    #[test]
    fn occupied_candidate_never_replaces_an_active_binding() {
        let (mut host, _, state) = host();
        let first = lease();
        host.prepare(
            0 as HWND,
            1,
            Some(GlobalShortcut::parse("Alt+V").unwrap()),
            first.clone(),
        )
        .unwrap();
        host.commit(0 as HWND, 1, &first).unwrap();
        state
            .lock()
            .unwrap()
            .register_results
            .push_back(Err("hotkey already registered".into()));
        let error = host
            .prepare(
                0 as HWND,
                2,
                Some(GlobalShortcut::parse("Ctrl+Alt+J").unwrap()),
                lease(),
            )
            .unwrap_err();
        assert!(error.contains("previous binding is unchanged"));
        assert_eq!(host.status(), "Active globally: Alt+V");
        assert!(host.pending.is_none());
        assert_eq!(state.lock().unwrap().registered.len(), 1);
        assert!(host.matches(FIRST_ID as usize, (0x56 << 16) | 1));
        host.stop(0 as HWND);
    }

    #[test]
    fn shortcut_maps_without_keyboard_hooks() {
        assert_eq!(
            native_key(&GlobalShortcut::parse("Alt+V").unwrap()),
            (MOD_ALT, 0x56)
        );
        assert_eq!(
            native_key(&GlobalShortcut::parse("Ctrl+Shift+F24").unwrap()),
            (MOD_CONTROL | MOD_SHIFT, 0x87)
        );
    }

    #[test]
    fn queued_messages_from_old_binding_are_rejected() {
        let (mut host, _, _) = host();
        host.active = Some(Binding {
            id: FIRST_ID,
            key: GlobalShortcut::parse("Alt+V").unwrap(),
        });
        assert!(host.matches(FIRST_ID as usize, (0x56 << 16) | 1));
        assert!(!host.matches(SECOND_ID as usize, (0x56 << 16) | 1));
        assert!(!host.matches(FIRST_ID as usize, (0x56 << 16) | 2));
    }
}
