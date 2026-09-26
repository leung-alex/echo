//! Target-thread message-only scheduler for the standalone observer DLL.
//!
//! The scheduler owns the target-side mapping and the closeable runtime.  A
//! TSF edit session keeps only a `Weak<Runtime>`; mapping/context lifetime is
//! therefore extended by an in-flight callback and cannot be invalidated by a
//! host close or a late COM delivery.

use super::{caret_ffi as ffi, caret_protocol as protocol};
use protocol::{Header, Mailbox, RESPONSE_CLOSED, RESPONSE_PENDING, RESPONSE_UNAVAILABLE};
use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::{
    atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, AtomicUsize, Ordering},
    Arc, OnceLock, Weak,
};

pub const MESSAGE: &str = "Echo.CaretObservation.v1";
pub const PREFIX: &str = "Local\\Echo.CaretObservation.";
pub const REQUEST_MESSAGE: u32 = ffi::WM_APP + 0x5a1;
pub const CLOSE_MESSAGE: u32 = ffi::WM_APP + 0x5a2;
const INIT_MESSAGE: u32 = ffi::WM_APP + 0x5a3;
#[cfg(feature = "native-test")]
pub const LIFECYCLE_DRAIN_MESSAGE: u32 = ffi::WM_APP + 0x5a4;
const WATCHDOG_TIMER: usize = 1;
const WATCHDOG_MS: u32 = 250;
const HEARTBEAT_TIMEOUT_MS: u64 = 1000;
const GET_MODULE_HANDLE_EX_FLAG_PIN: u32 = 0x00000001;
const GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS: u32 = 0x00000004;
const GA_ROOT: u32 = 2;
const WS_OVERLAPPED: u32 = 0;
pub const MAX_CALLBACKS: u32 = 8;

// This count deliberately lives at DLL process scope, rather than in one
// replaceable scheduler host. A new mailbox/session cannot evade the cap.
static CALLBACKS: AtomicU32 = AtomicU32::new(0);
static CALLBACKS_CREATED: AtomicU64 = AtomicU64::new(0);
static CALLBACKS_RELEASED: AtomicU64 = AtomicU64::new(0);
static CALLBACKS_HIGH_WATER: AtomicU32 = AtomicU32::new(0);
static PENDING_HIGH_WATER: AtomicU32 = AtomicU32::new(0);
pub(crate) static API_REQUESTS: AtomicU64 = AtomicU64::new(0);
pub(crate) static REQUEST_EDIT_CALLS: AtomicU64 = AtomicU64::new(0);
pub(crate) static ACCEPTED_SESSIONS: AtomicU64 = AtomicU64::new(0);
pub(crate) static CALLBACK_ENTERED: AtomicU64 = AtomicU64::new(0);
pub(crate) static CALLBACK_COMPLETED: AtomicU64 = AtomicU64::new(0);
pub(crate) static FINAL_RELEASED: AtomicU64 = AtomicU64::new(0);
pub(crate) static CANCELLED: AtomicU64 = AtomicU64::new(0);
pub(crate) static TIMED_OUT: AtomicU64 = AtomicU64::new(0);
pub(crate) static READY_RESULTS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "native-test")]
pub(crate) static MOCK_SESSIONS: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "native-test")]
pub(crate) static MOCK_CALLBACK_ENTERED: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "native-test")]
pub(crate) static MOCK_CALLBACK_COMPLETED: AtomicU64 = AtomicU64::new(0);
#[cfg(feature = "native-test")]
pub(crate) static MOCK_FINAL_RELEASED: AtomicU64 = AtomicU64::new(0);

pub fn callback_count() -> u32 {
    CALLBACKS.load(Ordering::Acquire)
}

pub fn callback_created_count() -> u64 {
    CALLBACKS_CREATED.load(Ordering::Acquire)
}

pub fn callback_released_count() -> u64 {
    CALLBACKS_RELEASED.load(Ordering::Acquire)
}

pub fn callback_high_water() -> u32 {
    CALLBACKS_HIGH_WATER.load(Ordering::Acquire)
}

pub fn pending_high_water() -> u32 {
    PENDING_HIGH_WATER.load(Ordering::Acquire)
}

pub fn api_request_count() -> u64 {
    API_REQUESTS.load(Ordering::Acquire)
}

pub fn request_edit_call_count() -> u64 {
    REQUEST_EDIT_CALLS.load(Ordering::Acquire)
}

pub fn accepted_session_count() -> u64 {
    ACCEPTED_SESSIONS.load(Ordering::Acquire)
}

pub fn callback_entered_count() -> u64 {
    CALLBACK_ENTERED.load(Ordering::Acquire)
}

pub fn callback_completed_count() -> u64 {
    CALLBACK_COMPLETED.load(Ordering::Acquire)
}

pub fn callback_final_released_count() -> u64 {
    FINAL_RELEASED.load(Ordering::Acquire)
}

pub fn cancelled_count() -> u64 {
    CANCELLED.load(Ordering::Acquire)
}

pub fn timed_out_count() -> u64 {
    TIMED_OUT.load(Ordering::Acquire)
}

pub fn ready_result_count() -> u64 {
    READY_RESULTS.load(Ordering::Acquire)
}

/// Append target-owned lifecycle counters to the content-free mailbox. These
/// fields are diagnostics only; the host must not infer a callback lifecycle
/// transition from response arrival.
pub fn write_metrics(words: &mut [u32; 64]) {
    let values = [
        api_request_count(),
        accepted_session_count(),
        callback_entered_count(),
        callback_completed_count(),
        callback_final_released_count(),
        cancelled_count(),
        timed_out_count(),
    ];
    for (index, value) in values.into_iter().enumerate() {
        let offset = 36 + index * 2;
        words[offset] = value as u32;
        words[offset + 1] = (value >> 32) as u32;
    }
    words[50] = callback_high_water();
    words[51] = pending_high_water();
    let ready = ready_result_count();
    words[52] = ready as u32;
    words[53] = (ready >> 32) as u32;
    let request_edit_calls = request_edit_call_count();
    words[54] = request_edit_calls as u32;
    words[55] = (request_edit_calls >> 32) as u32;
    #[cfg(feature = "native-test")]
    {
        let values = [
            MOCK_SESSIONS.load(Ordering::Acquire),
            MOCK_CALLBACK_ENTERED.load(Ordering::Acquire),
            MOCK_CALLBACK_COMPLETED.load(Ordering::Acquire),
            MOCK_FINAL_RELEASED.load(Ordering::Acquire),
        ];
        for (index, value) in values.into_iter().enumerate() {
            let offset = 56 + index * 2;
            words[offset] = value as u32;
            words[offset + 1] = (value >> 32) as u32;
        }
    }
}

pub fn reserve_callback() -> bool {
    let mut current = CALLBACKS.load(Ordering::Acquire);
    loop {
        if current >= MAX_CALLBACKS {
            return false;
        }
        match CALLBACKS.compare_exchange_weak(
            current,
            current + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                CALLBACKS_CREATED.fetch_add(1, Ordering::AcqRel);
                update_high_water(current + 1);
                return true;
            }
            Err(next) => current = next,
        }
    }
}

fn update_high_water(value: u32) {
    let mut observed = CALLBACKS_HIGH_WATER.load(Ordering::Acquire);
    while value > observed {
        match CALLBACKS_HIGH_WATER.compare_exchange_weak(
            observed,
            value,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => break,
            Err(next) => observed = next,
        }
    }
}

pub fn release_callback() -> u32 {
    let mut current = CALLBACKS.load(Ordering::Acquire);
    loop {
        if current == 0 {
            return 0;
        }
        match CALLBACKS.compare_exchange_weak(
            current,
            current - 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                CALLBACKS_RELEASED.fetch_add(1, Ordering::AcqRel);
                return current - 1;
            }
            Err(next) => current = next,
        }
    }
}

thread_local! {
    static SCHEDULER: Cell<*mut SchedulerState> = const { Cell::new(null_mut()) };
}

/// Target-thread owned resources. `Drop` runs only after the scheduler and
/// every callback-held `Arc` have gone away.
pub struct Runtime {
    mapping: ffi::Handle,
    view: *mut Mailbox,
    pub header: Header,
    closed: AtomicBool,
    context: AtomicPtr<c_void>,
    document_identity: AtomicUsize,
    context_identity: AtomicUsize,
    context_epoch: AtomicU64,
    pending: AtomicU32,
}

unsafe impl Send for Runtime {}
unsafe impl Sync for Runtime {}

impl Runtime {
    fn new(mapping: ffi::Handle, view: *mut Mailbox, header: Header) -> Self {
        Self {
            mapping,
            view,
            header,
            closed: AtomicBool::new(false),
            context: AtomicPtr::new(null_mut()),
            document_identity: AtomicUsize::new(0),
            context_identity: AtomicUsize::new(0),
            context_epoch: AtomicU64::new(1),
            pending: AtomicU32::new(0),
        }
    }

    pub fn weak(self: &Arc<Self>) -> Weak<Self> {
        Arc::downgrade(self)
    }

    pub fn view(&self) -> *mut Mailbox {
        self.view
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    pub fn context_epoch(&self) -> u64 {
        self.context_epoch.load(Ordering::Acquire)
    }

    pub fn context_identity(&self) -> usize {
        self.context_identity.load(Ordering::Acquire)
    }

    pub fn document_identity(&self) -> usize {
        self.document_identity.load(Ordering::Acquire)
    }

    pub fn context(&self) -> *mut c_void {
        self.context.load(Ordering::Acquire)
    }

    pub fn mark_pending(&self, pending: bool) {
        self.pending.store(u32::from(pending), Ordering::Release);
        if pending {
            let mut observed = PENDING_HIGH_WATER.load(Ordering::Acquire);
            while observed < 1 {
                match PENDING_HIGH_WATER.compare_exchange_weak(
                    observed,
                    1,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => break,
                    Err(next) => observed = next,
                }
            }
        }
    }

    /// Install the current document/context identity and transfer ownership of
    /// the returned context reference into the runtime. A repeated identity
    /// releases the newly returned duplicate and keeps the existing owner.
    pub unsafe fn install_context(
        &self,
        document_identity: usize,
        context: *mut c_void,
        context_identity: usize,
    ) -> Option<(*mut c_void, u64)> {
        if self.is_closed() || context.is_null() || context_identity == 0 {
            super::tsf_abi::release(context);
            return None;
        }
        let old_document = self.document_identity.load(Ordering::Acquire);
        let old_context = self.context_identity.load(Ordering::Acquire);
        if old_document == document_identity && old_context == context_identity {
            super::tsf_abi::release(context);
            return Some((self.context(), self.context_epoch()));
        }
        let old = self.context.swap(context, Ordering::AcqRel);
        self.document_identity
            .store(document_identity, Ordering::Release);
        self.context_identity
            .store(context_identity, Ordering::Release);
        let epoch = if old_document == 0 || old_context == 0 {
            self.context_epoch.load(Ordering::Acquire)
        } else {
            self.context_epoch.fetch_add(1, Ordering::AcqRel) + 1
        };
        if !old.is_null() {
            super::tsf_abi::release(old);
        }
        Some((context, epoch))
    }

    pub unsafe fn mark_closed(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }
        if self.pending.swap(0, Ordering::AcqRel) != 0 {
            CANCELLED.fetch_add(1, Ordering::AcqRel);
        }
        if !self.view.is_null() {
            (*self.view).closed.store(1, Ordering::Release);
            let mut response = [0_u32; 64];
            response[0] = RESPONSE_CLOSED;
            response[30] = callback_count();
            let created = callback_created_count();
            let released = callback_released_count();
            response[32] = created as u32;
            response[33] = (created >> 32) as u32;
            response[34] = released as u32;
            response[35] = (released >> 32) as u32;
            write_metrics(&mut response);
            let _ = (*self.view).response.publish(&response);
        }
    }

    /// Publish the actual callback count after the final COM Release. This is
    /// intentionally independent from geometry/result completion.
    pub unsafe fn publish_callback_count(&self, sequence: u32, count: u32) {
        if self.view.is_null() {
            return;
        }
        let Some(mut response) = (*self.view).response.snapshot() else {
            return;
        };
        if response[1] != sequence {
            return;
        }
        response[29] = self.pending.load(Ordering::Acquire);
        response[30] = count;
        let created = callback_created_count();
        let released = callback_released_count();
        response[32] = created as u32;
        response[33] = (created >> 32) as u32;
        response[34] = released as u32;
        response[35] = (released >> 32) as u32;
        write_metrics(&mut response);
        let _ = (*self.view).response.publish(&response);
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        unsafe {
            let context = self.context.swap(null_mut(), Ordering::AcqRel);
            if !context.is_null() {
                super::tsf_abi::release(context);
            }
            if !self.view.is_null() {
                ffi::UnmapViewOfFile(self.view.cast());
            }
            if !self.mapping.is_null() {
                ffi::CloseHandle(self.mapping);
            }
        }
    }
}

struct SchedulerState {
    runtime: Arc<Runtime>,
    window: ffi::Hwnd,
    input_window: ffi::Hwnd,
    root_window: ffi::Hwnd,
    closed: bool,
    last_request_sequence: u32,
    pending_sequence: Option<u32>,
}

unsafe impl Send for SchedulerState {}

static CLASS: OnceLock<Vec<u16>> = OnceLock::new();
static MESSAGE_ID: OnceLock<u32> = OnceLock::new();

fn class_name() -> &'static [u16] {
    CLASS.get_or_init(|| ffi::wide("Echo.CaretScheduler.v1"))
}

unsafe extern "system" fn window_proc(
    window: ffi::Hwnd,
    message: u32,
    wparam: ffi::Wparam,
    lparam: ffi::Lparam,
) -> ffi::Lresult {
    if message == ffi::WM_NCCREATE {
        // Install the state pointer before Windows sends any subsequent create
        // messages.  The pointer is only a lookup token here; every handler
        // below takes a short borrow and never keeps it across COM/Win32 work.
        let create = lparam as *const ffi::CreateStructW;
        if !create.is_null() {
            let state = (*create).create_params as *mut SchedulerState;
            let _ = ffi::SetWindowLongPtrW(window, ffi::GWLP_USERDATA, state as isize);
        }
        return ffi::DefWindowProcW(window, message, wparam, lparam);
    }
    let state = ffi::GetWindowLongPtrW(window, ffi::GWLP_USERDATA) as *mut SchedulerState;
    if state.is_null() {
        return ffi::DefWindowProcW(window, message, wparam, lparam);
    }
    match message {
        INIT_MESSAGE => {
            let _ = ffi::SetTimer(window, WATCHDOG_TIMER, WATCHDOG_MS, null_mut());
            0
        }
        REQUEST_MESSAGE => {
            handle_request(state);
            0
        }
        #[cfg(feature = "native-test")]
        LIFECYCLE_DRAIN_MESSAGE => {
            super::tsf_geometry::drain_lifecycle_queue(&(*state).runtime);
            0
        }
        ffi::WM_TIMER if wparam == WATCHDOG_TIMER => {
            let now = ffi::GetTickCount64();
            let should_close = {
                let state_ref = &*state;
                let view = state_ref.runtime.view();
                let heartbeat = if view.is_null() {
                    0
                } else {
                    (*view).heartbeat_tick.load(Ordering::Acquire)
                };
                state_ref.runtime.is_closed()
                    || view.is_null()
                    || (*view).closed.load(Ordering::Acquire) != 0
                    || heartbeat == 0
                    || now.saturating_sub(heartbeat) > HEARTBEAT_TIMEOUT_MS
                    || !live_identity(
                        &state_ref.runtime.header,
                        state_ref.input_window,
                        state_ref.root_window,
                    )
            };
            if should_close {
                close_scheduler(state);
            }
            0
        }
        CLOSE_MESSAGE | ffi::WM_CLOSE => {
            close_scheduler(state);
            0
        }
        _ => ffi::DefWindowProcW(window, message, wparam, 0),
    }
}

unsafe fn ensure_class() -> bool {
    let instance = ffi::GetModuleHandleW(null());
    let class = ffi::WndClassW {
        style: 0,
        wnd_proc: Some(window_proc),
        cls_extra: 0,
        wnd_extra: 0,
        instance,
        icon: null_mut(),
        cursor: null_mut(),
        background: null_mut(),
        menu_name: null(),
        class_name: class_name().as_ptr(),
    };
    let atom = ffi::RegisterClassW(&class);
    let error = ffi::GetLastError();
    atom != 0 || error == ffi::ERROR_CLASS_ALREADY_EXISTS
}

unsafe fn close_scheduler(state: *mut SchedulerState) {
    if state.is_null() {
        return;
    }
    let (window, runtime) = {
        let state_ref = &mut *state;
        if state_ref.closed {
            return;
        }
        state_ref.closed = true;
        (state_ref.window, state_ref.runtime.clone())
    };
    // Clear both lookup paths before DestroyWindow.  DestroyWindow can send
    // synchronous messages back to this window; those messages must observe a
    // closed scheduler and must never reach a freed Box.
    SCHEDULER.with(|slot| {
        if slot.get() == state {
            slot.set(null_mut());
        }
    });
    runtime.mark_closed();
    let _ = ffi::KillTimer(window, WATCHDOG_TIMER);
    if !window.is_null() {
        let _ = ffi::SetWindowLongPtrW(window, ffi::GWLP_USERDATA, 0);
        let _ = ffi::DestroyWindow(window);
    }
    // Dropping the host state releases only its Arc. An edit-session callback
    // that already upgraded its Weak keeps the runtime/mapping alive.
    drop(Box::from_raw(state));
}

unsafe fn publish_bootstrap_unavailable(view: *mut Mailbox, reason: u32) {
    if view.is_null() {
        return;
    }
    let mut response = [0_u32; 64];
    response[0] = RESPONSE_UNAVAILABLE;
    response[4] = reason;
    response[30] = callback_count();
    let created = callback_created_count();
    let released = callback_released_count();
    response[32] = created as u32;
    response[33] = (created >> 32) as u32;
    response[34] = released as u32;
    response[35] = (released >> 32) as u32;
    write_metrics(&mut response);
    let _ = (*view).response.publish(&response);
}

struct PreparedRequest {
    state: *mut SchedulerState,
    runtime: Arc<Runtime>,
    sequence: u32,
    deadline: u64,
    entry_tick: u64,
    request_epoch: u64,
}

/// Prepare a request while holding a short scheduler-state borrow.  The
/// returned Arc is the only runtime handle used during the external TSF call.
/// No `&mut SchedulerState` escapes this function.
unsafe fn prepare_request(state: *mut SchedulerState) -> Option<PreparedRequest> {
    if state.is_null() {
        return None;
    }
    let state_ref = &mut *state;
    if state_ref.closed || state_ref.runtime.is_closed() || state_ref.runtime.view().is_null() {
        return None;
    }
    let runtime = state_ref.runtime.clone();
    let view = runtime.view();
    if (*view).closed.load(Ordering::Acquire) != 0 {
        return None;
    }
    // A duplicate/reentrant post while Pending cannot create another session.
    if let Some(pending) = state_ref.pending_sequence {
        let finished = (*view)
            .response
            .snapshot()
            .is_some_and(|response| response[1] == pending && response[0] != RESPONSE_PENDING);
        if finished {
            state_ref.pending_sequence = None;
            runtime.mark_pending(false);
        } else {
            return None;
        }
    }
    let Some(request) = (*view).request.snapshot() else {
        return None;
    };
    if request[0] == protocol::REQUEST_CLOSE {
        drop(state_ref);
        close_scheduler(state);
        return None;
    }
    if request[0] != protocol::REQUEST_PROBE || request[1] == 0 {
        return None;
    }
    let sequence = request[1];
    if sequence <= state_ref.last_request_sequence {
        if sequence < state_ref.last_request_sequence {
            publish_bootstrap_unavailable(view, 34); // sequence-rollover/replay
        }
        return None;
    }
    state_ref.last_request_sequence = sequence;
    let deadline = u64::from(request[2]) | (u64::from(request[3]) << 32);
    let now = ffi::GetTickCount64();
    let request_epoch = u64::from(request[12]) | (u64::from(request[13]) << 32);
    let mut pending = [0_u32; 64];
    pending[0] = RESPONSE_PENDING;
    pending[1] = sequence;
    pending[8] = deadline as u32;
    pending[9] = (deadline >> 32) as u32;
    pending[10] = request_epoch as u32;
    pending[11] = (request_epoch >> 32) as u32;
    pending[29] = 1;
    pending[30] = callback_count();
    let created = callback_created_count();
    let released = callback_released_count();
    pending[32] = created as u32;
    pending[33] = (created >> 32) as u32;
    pending[34] = released as u32;
    pending[35] = (released >> 32) as u32;
    write_metrics(&mut pending);
    let _ = (*view).response.publish(&pending);
    state_ref.pending_sequence = Some(sequence);
    runtime.mark_pending(true);
    Some(PreparedRequest {
        state,
        runtime,
        sequence,
        deadline,
        entry_tick: now,
        request_epoch,
    })
}

/// Reconcile after TSF/COM returns through a live scheduler lookup.  A
/// synchronous close or replacement clears the TLS slot and frees the state;
/// in that case this function only drops the local Arc and does not dereference
/// the stale state pointer.
unsafe fn reconcile_request(prepared: PreparedRequest, accepted: bool) {
    let live = SCHEDULER.with(|slot| slot.get());
    if live.is_null() || live != prepared.state {
        return;
    }
    let state_ref = &mut *live;
    if state_ref.closed || !Arc::ptr_eq(&state_ref.runtime, &prepared.runtime) {
        return;
    }
    if state_ref.pending_sequence != Some(prepared.sequence) {
        return;
    }
    if !accepted {
        state_ref.pending_sequence = None;
        prepared.runtime.mark_pending(false);
        prepared
            .runtime
            .publish_callback_count(prepared.sequence, callback_count());
    }
}

unsafe fn handle_request(state: *mut SchedulerState) {
    let Some(prepared) = prepare_request(state) else {
        return;
    };
    // The scheduler state borrow ended before this call.  A provider can call
    // CLOSE_MESSAGE synchronously and destroy the scheduler here.
    let accepted = super::tsf_geometry::request(
        &prepared.runtime,
        prepared.sequence,
        prepared.deadline,
        prepared.entry_tick,
        prepared.request_epoch,
    );
    reconcile_request(prepared, accepted);
}

unsafe fn pin_module() -> bool {
    let mut module = null_mut();
    let address = pin_module as *const c_void;
    ffi::GetModuleHandleExW(
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
        address,
        &mut module,
    ) != 0
}

unsafe fn process_started(pid: u32) -> Option<u64> {
    if pid == 0 {
        return None;
    }
    if pid == ffi::GetCurrentProcessId() {
        let mut created = 0_u64;
        let mut exit = 0_u64;
        let mut kernel = 0_u64;
        let mut user = 0_u64;
        return (ffi::GetProcessTimes(
            ffi::GetCurrentProcess(),
            &mut created,
            &mut exit,
            &mut kernel,
            &mut user,
        ) != 0)
            .then_some(created);
    }
    let process = ffi::OpenProcess(ffi::PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
    if process.is_null() {
        return None;
    }
    let mut created = 0_u64;
    let mut exit = 0_u64;
    let mut kernel = 0_u64;
    let mut user = 0_u64;
    let ok = ffi::GetProcessTimes(process, &mut created, &mut exit, &mut kernel, &mut user) != 0;
    ffi::CloseHandle(process);
    ok.then_some(created)
}

unsafe fn live_identity(header: &Header, input: ffi::Hwnd, root: ffi::Hwnd) -> bool {
    if input.is_null()
        || root.is_null()
        || ffi::GetCurrentProcessId() != header.target_pid
        || ffi::GetCurrentThreadId() != header.input_thread
        || input as usize as u64 != header.input_hwnd
        || root as usize as u64 != header.root_hwnd
        || ffi::GetFocus() != input
        || ffi::GetAncestor(input, GA_ROOT) != root
        || ffi::GetForegroundWindow() != root
    {
        return false;
    }
    let mut input_pid = 0_u32;
    let input_thread = ffi::GetWindowThreadProcessId(input, &mut input_pid);
    if input_thread != header.input_thread || input_pid != header.target_pid {
        return false;
    }
    let mut root_pid = 0_u32;
    if ffi::GetWindowThreadProcessId(root, &mut root_pid) == 0 || root_pid != header.root_pid {
        return false;
    }
    process_started(header.target_pid) == Some(header.target_started)
        && process_started(header.root_pid) == Some(header.root_started)
}

unsafe fn bootstrap(message: &ffi::CallWindow) -> bool {
    let expected =
        *MESSAGE_ID.get_or_init(|| ffi::RegisterWindowMessageW(ffi::wide(MESSAGE).as_ptr()));
    if message.message != expected
        || message.window.is_null()
        || message.wparam == 0
        || message.wparam > u16::MAX as usize
    {
        return false;
    }
    let mut name = [0_u16; 256];
    let length =
        ffi::GlobalGetAtomNameW(message.wparam as u16, name.as_mut_ptr(), name.len() as i32)
            as usize;
    if length == 0
        || length >= name.len()
        || !String::from_utf16_lossy(&name[..length]).starts_with(PREFIX)
    {
        return false;
    }
    let mapping = ffi::OpenFileMappingW(ffi::FILE_MAP_READ | ffi::FILE_MAP_WRITE, 0, name.as_ptr());
    if mapping.is_null() {
        return true;
    }
    let view = ffi::MapViewOfFile(
        mapping,
        ffi::FILE_MAP_READ | ffi::FILE_MAP_WRITE,
        0,
        0,
        std::mem::size_of::<Mailbox>(),
    )
    .cast::<Mailbox>();
    if view.is_null() {
        ffi::CloseHandle(mapping);
        return true;
    }
    let header = (*view).header;
    if !protocol::header_shape_valid(&header)
        || !live_identity(
            &header,
            message.window,
            ffi::GetAncestor(message.window, GA_ROOT),
        )
    {
        publish_bootstrap_unavailable(view, 32); // bad-header/identity
        ffi::UnmapViewOfFile(view.cast());
        ffi::CloseHandle(mapping);
        return true;
    }

    let existing = SCHEDULER.with(|slot| slot.get());
    if !existing.is_null() {
        let existing_header = (*(*existing).runtime.view()).header;
        if protocol::header_identity_matches(&existing_header, &header) {
            (*view)
                .scheduler_hwnd
                .store((*existing).window as usize as u64, Ordering::Release);
            ffi::UnmapViewOfFile(view.cast());
            ffi::CloseHandle(mapping);
            return true;
        }
        // A different nonce/generation must not reuse the old scheduler. Its
        // runtime remains alive only while late callbacks still hold an Arc.
        close_scheduler(existing);
    }

    if !pin_module() {
        publish_bootstrap_unavailable(view, 33); // explicit pin/build failure
        ffi::UnmapViewOfFile(view.cast());
        ffi::CloseHandle(mapping);
        return true;
    }
    if !ensure_class() {
        publish_bootstrap_unavailable(view, 36); // scheduler-class-registration
        ffi::UnmapViewOfFile(view.cast());
        ffi::CloseHandle(mapping);
        return true;
    }
    let runtime = Arc::new(Runtime::new(mapping, view, header));
    let mut state = Box::new(SchedulerState {
        runtime: runtime.clone(),
        window: null_mut(),
        input_window: message.window,
        root_window: ffi::GetAncestor(message.window, GA_ROOT),
        closed: false,
        last_request_sequence: 0,
        pending_sequence: None,
    });
    ffi::SetLastError(0);
    let window = ffi::CreateWindowExW(
        0,
        class_name().as_ptr(),
        class_name().as_ptr(),
        WS_OVERLAPPED,
        0,
        0,
        0,
        0,
        ffi::HWND_MESSAGE,
        null_mut(),
        ffi::GetModuleHandleW(null()),
        state.as_mut() as *mut SchedulerState as *mut c_void,
    );
    if window.is_null() {
        // `state` still owns the sole strong runtime reference here. Mark the
        // mailbox closed before dropping it so a host cannot wait forever for
        // a scheduler acknowledgement, then let `Runtime::Drop` unmap the
        // view and close the mapping handle.
        state.runtime.mark_closed();
        drop(state);
        return true;
    }
    state.window = window;
    let state_ptr = Box::into_raw(state);
    let _ = ffi::SetWindowLongPtrW(window, ffi::GWLP_USERDATA, state_ptr as isize);
    SCHEDULER.with(|slot| slot.set(state_ptr));
    (*view)
        .scheduler_hwnd
        .store(window as usize as u64, Ordering::Release);
    let _ = ffi::PostMessageW(window, INIT_MESSAGE, 0, 0);
    true
}

pub unsafe fn dispatch_hook(message: &ffi::CallWindow) -> bool {
    bootstrap(message)
}
