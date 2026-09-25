//! Read-only TSF geometry acquisition on the validated target input thread.
//!
//! Edit sessions carry only immutable request metadata and a weak runtime.
//! The runtime owns the mapping and the current context reference until every
//! callback has released it, so a host close cannot turn a late callback into
//! a use-after-unmap/use-after-release.

use super::{
    caret_ffi as ffi, caret_protocol as protocol, caret_target_scheduler as target, sensitivity,
    tsf_abi as abi,
};
use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::null_mut;
use std::sync::{
    atomic::{AtomicU32, Ordering},
    Arc, OnceLock, Weak,
};

const RESPONSE_READY: u32 = protocol::RESPONSE_READY;
const RESPONSE_UNAVAILABLE: u32 = protocol::RESPONSE_UNAVAILABLE;
const GA_ROOT: u32 = 2;
const MAX_WIDTH: i64 = 64;
const MAX_HEIGHT: i64 = 256;

// Content-free reason identifiers. They are stable numeric values in the
// mailbox; the host/diagnostic layer maps them to package reason strings.
pub const REASON_NO_TSF_MODULE: u32 = 1;
pub const REASON_NO_THREAD_MANAGER: u32 = 2;
pub const REASON_CLIENT_ID_UNAVAILABLE: u32 = 3;
pub const REASON_THREAD_NOT_FOCUSED: u32 = 4;
pub const REASON_NO_DOCUMENT: u32 = 5;
pub const REASON_NO_CONTEXT: u32 = 6;
pub const REASON_DISCONNECTED_CONTEXT: u32 = 7;
pub const REASON_UNVERIFIED_VIEW: u32 = 8;
pub const REASON_WRONG_VIEW_OWNER: u32 = 9;
pub const REASON_SENSITIVITY_UNKNOWN: u32 = 10;
pub const REASON_SENSITIVITY_DENIED: u32 = 11;
pub const REASON_READONLY: u32 = 12;
pub const REASON_LOADING: u32 = 13;
pub const REASON_AMBIGUOUS_SELECTION: u32 = 14;
pub const REASON_NO_SELECTION: u32 = 15;
pub const REASON_SELECTION_NOT_COLLAPSED: u32 = 16;
pub const REASON_INTERIM: u32 = 17;
pub const REASON_NO_LAYOUT: u32 = 18;
pub const REASON_CLIPPED: u32 = 19;
pub const REASON_INVALID_RECTANGLE: u32 = 20;
pub const REASON_LATE_RESULT: u32 = 21;
pub const REASON_NO_LOCK: u32 = 22;
pub const REASON_EDIT_LOCKED: u32 = 23;
pub const REASON_EDIT_REQUEST_FAILED: u32 = 24;
pub const REASON_DPI_CONTEXT_MISMATCH: u32 = 25;
pub const REASON_CONTEXT_MISMATCH: u32 = 26;
pub const REASON_COORDINATE_CONVERSION_FAILED: u32 = 27;
pub const REASON_CALLBACK_CAP_REACHED: u32 = 28;
pub const REASON_EDIT_SESSION_MISMATCH: u32 = 29;
pub const REASON_EDIT_CALLBACK_MISSING: u32 = 30;

// The owned acceptance fixture can opt into a bounded fault adapter by
// providing a file path in the target process environment. The default path
// is absent, so production and ordinary shadow/primary operation never reads
// a fixture file. This adapter runs inside the real injected DLL and target
// scheduler; its results remain labelled as MockCOM by the runner and never
// replace the RealTSF evidence.
static MOCK_FAULT_FILE: OnceLock<Option<PathBuf>> = OnceLock::new();

fn mock_fault() -> Option<String> {
    if std::env::var("ECHO_WINDOWS_ACCEPTANCE").as_deref() != Ok("1") {
        return None;
    }
    let path = MOCK_FAULT_FILE
        .get_or_init(|| std::env::var_os("ECHO_CARET_FAULT_FILE").map(PathBuf::from))
        .as_ref()?;
    let value = std::fs::read_to_string(path).ok()?;
    let value = value.trim();
    if value.is_empty() || value == "clear" {
        None
    } else if value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        Some(value.to_owned())
    } else {
        None
    }
}

fn response_base(
    status: u32,
    sequence: u32,
    epoch: u64,
    reason: u32,
    api_hr: abi::Hresult,
    session_hr: abi::Hresult,
) -> [u32; 64] {
    let mut words = [0_u32; 64];
    let now = unsafe { ffi::GetTickCount64() };
    words[0] = status;
    words[1] = sequence;
    words[4] = reason;
    words[5] = api_hr as u32;
    words[6] = session_hr as u32;
    words[8] = now as u32;
    words[9] = (now >> 32) as u32;
    words[10] = epoch as u32;
    words[11] = (epoch >> 32) as u32;
    words[30] = target::callback_count();
    let created = target::callback_created_count();
    let released = target::callback_released_count();
    words[32] = created as u32;
    words[33] = (created >> 32) as u32;
    words[34] = released as u32;
    words[35] = (released >> 32) as u32;
    words[31] = 0b11;
    words
}

unsafe fn publish_unavailable(
    runtime: &Arc<target::Runtime>,
    sequence: u32,
    epoch: u64,
    reason: u32,
    api_hr: abi::Hresult,
    session_hr: abi::Hresult,
) {
    publish_unavailable_at(
        runtime,
        sequence,
        epoch,
        reason,
        api_hr,
        session_hr,
        unsafe { ffi::GetTickCount64() },
    );
}

unsafe fn publish_unavailable_at(
    runtime: &Arc<target::Runtime>,
    sequence: u32,
    epoch: u64,
    reason: u32,
    api_hr: abi::Hresult,
    session_hr: abi::Hresult,
    observed_tick: u64,
) {
    if runtime.is_closed() || runtime.view().is_null() {
        return;
    }
    let mut response = response_base(
        RESPONSE_UNAVAILABLE,
        sequence,
        epoch,
        reason,
        api_hr,
        session_hr,
    );
    response[8] = observed_tick as u32;
    response[9] = (observed_tick >> 32) as u32;
    response[29] = 0;
    response[30] = target::callback_count();
    let _ = (*runtime.view()).response.publish(&response);
}

/// Execute one explicit, content-free MockCOM fault while retaining the real
/// target DLL, scheduler window, mailbox, and HWND identity. Returning
/// `Some(false)` tells the scheduler that no TSF edit session was accepted;
/// this keeps the fault path bounded and makes API/session HRESULT handling
/// observable without manufacturing a callback that never existed.
unsafe fn mock_fault_response(
    runtime: &Arc<target::Runtime>,
    sequence: u32,
    epoch: u64,
    fault: &str,
) -> Option<bool> {
    let unavailable = |reason: u32, api_hr: abi::Hresult, session_hr: abi::Hresult| {
        publish_unavailable(runtime, sequence, epoch, reason, api_hr, session_hr);
        false
    };
    match fault {
        "missing-uia" => Some(unavailable(
            REASON_UNVERIFIED_VIEW,
            abi::E_NOINTERFACE,
            abi::S_OK,
        )),
        "password" => Some(unavailable(REASON_SENSITIVITY_DENIED, abi::S_OK, abi::S_OK)),
        "unknown-sensitivity" => Some(unavailable(
            REASON_SENSITIVITY_UNKNOWN,
            abi::S_OK,
            abi::S_OK,
        )),
        "no-layout" => Some(unavailable(REASON_NO_LAYOUT, abi::TS_E_NOLAYOUT, abi::S_OK)),
        "invalid-rect" => Some(unavailable(REASON_INVALID_RECTANGLE, abi::S_OK, abi::S_OK)),
        "different-view" => Some(unavailable(REASON_WRONG_VIEW_OWNER, abi::E_FAIL, abi::S_OK)),
        "focus-race" => Some(unavailable(REASON_LATE_RESULT, abi::S_OK, abi::S_OK)),
        "request-error" => Some(unavailable(
            REASON_EDIT_REQUEST_FAILED,
            abi::E_FAIL,
            abi::S_OK,
        )),
        "late-result" => {
            let observed_tick = unsafe { ffi::GetTickCount64() }.saturating_sub(151);
            publish_unavailable_at(
                runtime,
                sequence,
                epoch,
                REASON_LATE_RESULT,
                abi::S_OK,
                abi::S_OK,
                observed_tick,
            );
            Some(false)
        }
        "never-delivered" => Some(unavailable(
            REASON_EDIT_CALLBACK_MISSING,
            abi::S_OK,
            abi::S_OK,
        )),
        "late-close" => Some(unavailable(REASON_LATE_RESULT, abi::S_OK, abi::S_OK)),
        "selection" => Some(unavailable(
            REASON_SELECTION_NOT_COLLAPSED,
            abi::S_OK,
            abi::S_OK,
        )),
        "reentrancy" => Some(unavailable(
            REASON_EDIT_SESSION_MISMATCH,
            abi::E_FAIL,
            abi::S_OK,
        )),
        "source-conflict" => Some(unavailable(REASON_CONTEXT_MISMATCH, abi::S_OK, abi::S_OK)),
        "protocol" => Some(unavailable(
            REASON_EDIT_REQUEST_FAILED,
            abi::E_INVALIDARG,
            abi::S_OK,
        )),
        "rollover" => Some(unavailable(
            REASON_EDIT_REQUEST_FAILED,
            abi::E_FAIL,
            abi::S_OK,
        )),
        "reuse" => Some(unavailable(REASON_CONTEXT_MISMATCH, abi::S_OK, abi::S_OK)),
        "release-cap" => {
            let mut leases = 0_u32;
            while target::reserve_callback() {
                leases += 1;
            }
            let result = unavailable(REASON_CALLBACK_CAP_REACHED, abi::S_OK, abi::S_OK);
            for _ in 0..leases {
                target::release_callback();
            }
            runtime.publish_callback_count(sequence, target::callback_count());
            Some(result)
        }
        _ => None,
    }
}

unsafe fn target_is_live(header: &protocol::Header, view: ffi::Hwnd) -> bool {
    if view.is_null()
        || ffi::GetCurrentProcessId() != header.target_pid
        || ffi::GetCurrentThreadId() != header.input_thread
        || ffi::GetFocus() != header.input_hwnd as usize as ffi::Hwnd
    {
        return false;
    }
    let mut pid = 0_u32;
    if ffi::GetWindowThreadProcessId(view, &mut pid) != header.input_thread
        || pid != header.target_pid
    {
        return false;
    }
    let root = ffi::GetAncestor(view, GA_ROOT);
    if root as usize as u64 != header.root_hwnd || root != ffi::GetForegroundWindow() {
        return false;
    }
    let mut root_pid = 0_u32;
    if ffi::GetWindowThreadProcessId(root, &mut root_pid) == 0 || root_pid != header.root_pid {
        return false;
    }
    let mut created = 0_u64;
    let mut exit = 0_u64;
    let mut kernel = 0_u64;
    let mut user = 0_u64;
    ffi::GetProcessTimes(
        ffi::GetCurrentProcess(),
        &mut created,
        &mut exit,
        &mut kernel,
        &mut user,
    ) != 0
        && created == header.target_started
}

unsafe fn canonical_identity(object: *mut c_void) -> Option<usize> {
    if object.is_null() {
        return None;
    }
    let table = abi::vtable::<abi::UnknownVtbl>(object);
    if table.is_null() {
        return None;
    }
    let mut unknown = null_mut();
    if ((*table).query_interface)(object, &abi::IID_IUNKNOWN, &mut unknown) < 0 || unknown.is_null()
    {
        return None;
    }
    let identity = unknown as usize;
    abi::release(unknown);
    Some(identity)
}

fn valid_rect(rect: &abi::Rect) -> bool {
    let width = i64::from(rect.right) - i64::from(rect.left);
    let height = i64::from(rect.bottom) - i64::from(rect.top);
    width >= 0 && height > 0 && width <= MAX_WIDTH && height <= MAX_HEIGHT
}

/// TSF returns screen coordinates. For system/unaware views normalize each
/// corner once; for per-monitor aware views the screen values are already
/// physical. The opaque awareness handles are compared semantically.
unsafe fn normalize_rect(view: ffi::Hwnd, raw: &abi::Rect) -> Result<[i32; 4], u32> {
    if !valid_rect(raw) {
        return Err(REASON_INVALID_RECTANGLE);
    }
    let view_context = ffi::GetWindowDpiAwarenessContext(view);
    let thread_context = ffi::GetThreadDpiAwarenessContext();
    if view_context.is_null()
        || thread_context.is_null()
        || ffi::AreDpiAwarenessContextsEqual(view_context, thread_context) == 0
    {
        return Err(REASON_DPI_CONTEXT_MISMATCH);
    }
    let awareness = ffi::GetAwarenessFromDpiAwarenessContext(view_context);
    if awareness < ffi::DPI_AWARENESS_UNAWARE || awareness > ffi::DPI_AWARENESS_PER_MONITOR {
        return Err(REASON_DPI_CONTEXT_MISMATCH);
    }
    let (mut left, mut top, mut right, mut bottom) = (raw.left, raw.top, raw.right, raw.bottom);
    if awareness != ffi::DPI_AWARENESS_PER_MONITOR {
        let mut a = ffi::Point { x: left, y: top };
        let mut b = ffi::Point {
            x: right,
            y: bottom,
        };
        if ffi::LogicalToPhysicalPointForPerMonitorDPI(view, &mut a) == 0
            || ffi::LogicalToPhysicalPointForPerMonitorDPI(view, &mut b) == 0
        {
            return Err(REASON_COORDINATE_CONVERSION_FAILED);
        }
        left = a.x;
        top = a.y;
        right = b.x;
        bottom = b.y;
    }
    if right < left || bottom <= top {
        return Err(REASON_INVALID_RECTANGLE);
    }
    if right == left {
        right = left.checked_add(1).ok_or(REASON_INVALID_RECTANGLE)?;
    }
    let width = i64::from(right) - i64::from(left);
    let height = i64::from(bottom) - i64::from(top);
    if width < 0 || height <= 0 || width > MAX_WIDTH * 8 || height > MAX_HEIGHT * 8 {
        return Err(REASON_INVALID_RECTANGLE);
    }
    Ok([left, top, right, bottom])
}

#[repr(C)]
struct EditSessionObject {
    vtable: *const abi::EditSessionVtbl,
    refs: AtomicU32,
    runtime: Weak<target::Runtime>,
    header: protocol::Header,
    sequence: u32,
    deadline: u64,
    context_epoch: u64,
    document_identity: usize,
    context_identity: usize,
    entry_tick: u64,
}

unsafe impl Send for EditSessionObject {}

static EDIT_SESSION_VTABLE: abi::EditSessionVtbl = abi::EditSessionVtbl {
    query_interface: edit_query_interface,
    add_ref: edit_add_ref,
    release: edit_release,
    do_edit_session: edit_do,
};

unsafe extern "system" fn edit_query_interface(
    object: *mut c_void,
    iid: *const abi::Guid,
    out: *mut *mut c_void,
) -> abi::Hresult {
    if out.is_null() {
        return abi::E_INVALIDARG;
    }
    *out = null_mut();
    if iid.is_null() {
        return abi::E_NOINTERFACE;
    }
    if *iid == abi::IID_ITF_EDIT_SESSION || *iid == abi::IID_IUNKNOWN {
        *out = object;
        edit_add_ref(object);
        abi::S_OK
    } else {
        abi::E_NOINTERFACE
    }
}

unsafe extern "system" fn edit_add_ref(object: *mut c_void) -> abi::Ulong {
    if object.is_null() {
        return 0;
    }
    let state = &*(object.cast::<EditSessionObject>());
    state.refs.fetch_add(1, Ordering::Relaxed).saturating_add(1)
}

unsafe extern "system" fn edit_release(object: *mut c_void) -> abi::Ulong {
    if object.is_null() {
        return 0;
    }
    let state = &*(object.cast::<EditSessionObject>());
    let mut refs = state.refs.load(Ordering::Acquire);
    loop {
        if refs == 0 {
            return 0;
        }
        match state
            .refs
            .compare_exchange_weak(refs, refs - 1, Ordering::AcqRel, Ordering::Acquire)
        {
            Ok(_) => break,
            Err(next) => refs = next,
        }
    }
    if refs == 1 {
        let boxed = Box::from_raw(object.cast::<EditSessionObject>());
        if let Some(runtime) = boxed.runtime.upgrade() {
            let count = target::release_callback();
            runtime.mark_pending(false);
            runtime.publish_callback_count(boxed.sequence, count);
        } else {
            let _ = target::release_callback();
        }
        0
    } else {
        refs - 1
    }
}

unsafe extern "system" fn edit_do(
    object: *mut c_void,
    edit_cookie: abi::TfEditCookie,
) -> abi::Hresult {
    if object.is_null() {
        return abi::E_INVALIDARG;
    }
    let state = &*(object.cast::<EditSessionObject>());
    let Some(runtime) = state.runtime.upgrade() else {
        return abi::S_OK;
    };
    acquire_geometry(&runtime, state, edit_cookie);
    abi::S_OK
}

unsafe fn same_context(runtime: &target::Runtime, state: &EditSessionObject) -> bool {
    !runtime.is_closed()
        && runtime.context_epoch() == state.context_epoch
        && runtime.document_identity() == state.document_identity
        && runtime.context_identity() == state.context_identity
}

unsafe fn acquire_geometry(
    runtime: &Arc<target::Runtime>,
    state: &EditSessionObject,
    edit_cookie: abi::TfEditCookie,
) {
    if !same_context(runtime, state)
        || runtime.view().is_null()
        || (*runtime.view()).closed.load(Ordering::Acquire) != 0
        || ffi::GetTickCount64().saturating_sub(state.entry_tick) > 150
    {
        publish_unavailable(
            runtime,
            state.sequence,
            runtime.context_epoch(),
            if same_context(runtime, state) {
                REASON_LATE_RESULT
            } else {
                REASON_CONTEXT_MISMATCH
            },
            abi::S_OK,
            abi::S_OK,
        );
        return;
    }
    let context = runtime.context();
    let context_table = abi::vtable::<abi::ContextVtbl>(context);
    if context_table.is_null() {
        publish_unavailable(
            runtime,
            state.sequence,
            state.context_epoch,
            REASON_DISCONNECTED_CONTEXT,
            abi::E_FAIL,
            abi::S_OK,
        );
        return;
    }
    let mut status = abi::TfStatus::default();
    let status_hr = ((*context_table).get_status)(context, &mut status);
    if status_hr < 0 {
        publish_unavailable(
            runtime,
            state.sequence,
            state.context_epoch,
            REASON_DISCONNECTED_CONTEXT,
            status_hr,
            abi::S_OK,
        );
        return;
    }
    let (reason, blocked) = if status.dynamic_flags & abi::TF_SD_READONLY != 0 {
        (REASON_READONLY, true)
    } else if status.dynamic_flags & abi::TF_SD_LOADING != 0 {
        (REASON_LOADING, true)
    } else if status.static_flags & abi::TF_SS_DISJOINTSEL != 0 {
        (REASON_AMBIGUOUS_SELECTION, true)
    } else {
        (0, false)
    };
    if blocked {
        publish_unavailable(
            runtime,
            state.sequence,
            state.context_epoch,
            reason,
            status_hr,
            abi::S_OK,
        );
        return;
    }

    let mut selection = abi::TfSelection::default();
    let mut fetched = 0_u32;
    let selection_hr = ((*context_table).get_selection)(
        context,
        edit_cookie,
        abi::TF_DEFAULT_SELECTION,
        1,
        &mut selection,
        &mut fetched,
    );
    if selection_hr < 0 || fetched != 1 || selection.range.is_null() {
        if !selection.range.is_null() {
            abi::release(selection.range);
        }
        publish_unavailable(
            runtime,
            state.sequence,
            state.context_epoch,
            REASON_NO_SELECTION,
            selection_hr,
            abi::S_OK,
        );
        return;
    }
    if selection.style.interim_char != 0 {
        abi::release(selection.range);
        publish_unavailable(
            runtime,
            state.sequence,
            state.context_epoch,
            REASON_INTERIM,
            abi::S_OK,
            abi::S_OK,
        );
        return;
    }
    let range_table = abi::vtable::<abi::RangeVtbl>(selection.range);
    let mut empty = 0_i32;
    let empty_hr = if range_table.is_null() {
        abi::E_NOINTERFACE
    } else {
        ((*range_table).is_empty)(selection.range, edit_cookie, &mut empty)
    };
    if empty_hr < 0 || empty == 0 {
        abi::release(selection.range);
        publish_unavailable(
            runtime,
            state.sequence,
            state.context_epoch,
            if empty_hr < 0 {
                REASON_NO_SELECTION
            } else {
                REASON_SELECTION_NOT_COLLAPSED
            },
            empty_hr,
            abi::S_OK,
        );
        return;
    }

    let mut property = null_mut();
    let property_hr =
        ((*context_table).get_app_property)(context, &abi::GUID_PROP_INPUTSCOPE, &mut property);
    let safety = if property_hr >= 0 && !property.is_null() {
        sensitivity::from_property(property, edit_cookie, selection.range)
    } else {
        sensitivity::Sensitivity::Unknown
    };
    abi::release(property);
    if safety != sensitivity::Sensitivity::Allowed {
        abi::release(selection.range);
        publish_unavailable(
            runtime,
            state.sequence,
            state.context_epoch,
            if safety == sensitivity::Sensitivity::Denied {
                REASON_SENSITIVITY_DENIED
            } else {
                REASON_SENSITIVITY_UNKNOWN
            },
            property_hr,
            abi::S_OK,
        );
        return;
    }

    let mut view = null_mut();
    let view_hr = ((*context_table).get_active_view)(context, &mut view);
    let view_table = abi::vtable::<abi::ContextViewVtbl>(view);
    let mut view_window = null_mut();
    let window_hr = if view_table.is_null() || view.is_null() {
        abi::E_NOINTERFACE
    } else {
        ((*view_table).get_wnd)(view, &mut view_window)
    };
    if view_hr < 0
        || window_hr < 0
        || view_window.is_null()
        || !target_is_live(&state.header, view_window)
    {
        abi::release(view);
        abi::release(selection.range);
        publish_unavailable(
            runtime,
            state.sequence,
            state.context_epoch,
            if view_window.is_null() {
                REASON_UNVERIFIED_VIEW
            } else {
                REASON_WRONG_VIEW_OWNER
            },
            if view_hr < 0 { view_hr } else { window_hr },
            abi::S_OK,
        );
        return;
    }
    let mut raw = abi::Rect::default();
    let mut clipped = 0_i32;
    let text_ext_hr =
        ((*view_table).get_text_ext)(view, edit_cookie, selection.range, &mut raw, &mut clipped);
    let normalized = normalize_rect(view_window, &raw);
    let epoch_still_current = same_context(runtime, state);
    let live_after = target_is_live(&state.header, view_window)
        && !runtime.view().is_null()
        && (*runtime.view()).closed.load(Ordering::Acquire) == 0;
    if text_ext_hr < 0 || text_ext_hr == abi::TS_E_NOLAYOUT {
        publish_unavailable(
            runtime,
            state.sequence,
            state.context_epoch,
            REASON_NO_LAYOUT,
            text_ext_hr,
            abi::S_OK,
        );
    } else if clipped != 0 {
        publish_unavailable(
            runtime,
            state.sequence,
            state.context_epoch,
            REASON_CLIPPED,
            text_ext_hr,
            abi::S_OK,
        );
    } else if let Err(reason) = normalized {
        publish_unavailable(
            runtime,
            state.sequence,
            state.context_epoch,
            reason,
            text_ext_hr,
            abi::S_OK,
        );
    } else if !epoch_still_current || !live_after {
        publish_unavailable(
            runtime,
            state.sequence,
            runtime.context_epoch(),
            if epoch_still_current {
                REASON_LATE_RESULT
            } else {
                REASON_CONTEXT_MISMATCH
            },
            text_ext_hr,
            abi::S_OK,
        );
    } else if ffi::GetTickCount64().saturating_sub(state.entry_tick) > 150 {
        publish_unavailable(
            runtime,
            state.sequence,
            state.context_epoch,
            REASON_LATE_RESULT,
            text_ext_hr,
            abi::S_OK,
        );
    } else if let Ok([left, top, right, bottom]) = normalized {
        let mut response = response_base(
            RESPONSE_READY,
            state.sequence,
            state.context_epoch,
            0,
            text_ext_hr,
            abi::S_OK,
        );
        let flags = 0_u32;
        response[2] = 1; // TsfCaret
        response[3] = 1; // Exact
        response[7] = flags;
        response[8] = state.entry_tick as u32;
        response[9] = (state.entry_tick >> 32) as u32;
        response[12] = view_window as usize as u64 as u32;
        response[13] = (view_window as usize as u64 >> 32) as u32;
        response[14] = raw.left as u32;
        response[15] = raw.top as u32;
        response[16] = raw.right as u32;
        response[17] = raw.bottom as u32;
        response[18] = left as u32;
        response[19] = top as u32;
        response[20] = right as u32;
        response[21] = bottom as u32;
        response[26] = 96;
        response[27] = 2; // physical-screen
        response[28] = 1; // Allowed
        response[29] = 1;
        response[30] = target::callback_count();
        response[31] = 0b11;
        let _ = (*runtime.view()).response.publish(&response);
    }
    abi::release(view);
    abi::release(selection.range);
}

unsafe fn thread_manager() -> Result<*mut c_void, u32> {
    let module = ffi::GetModuleHandleW(ffi::wide("msctf.dll").as_ptr());
    if module.is_null() {
        return Err(REASON_NO_TSF_MODULE);
    }
    let address = ffi::GetProcAddress(module, b"TF_GetThreadMgr\0".as_ptr().cast());
    if address.is_null() {
        return Err(REASON_NO_TSF_MODULE);
    }
    let get: unsafe extern "system" fn(*mut *mut c_void) -> abi::Hresult =
        std::mem::transmute(address);
    let mut manager = null_mut();
    if get(&mut manager) < 0 || manager.is_null() {
        return Err(REASON_NO_THREAD_MANAGER);
    }
    Ok(manager)
}

/// Start one forced-asynchronous read-only TSF request. `true` means both the
/// API and session HRESULTs indicate that TSF accepted the callback.
pub unsafe fn request(
    runtime: &Arc<target::Runtime>,
    sequence: u32,
    deadline: u64,
    entry_tick: u64,
    requested_epoch: u64,
) -> bool {
    if runtime.is_closed() || runtime.context_epoch() != requested_epoch {
        publish_unavailable(
            runtime,
            sequence,
            runtime.context_epoch(),
            REASON_CONTEXT_MISMATCH,
            abi::S_OK,
            abi::S_OK,
        );
        return false;
    }
    if let Some(fault) = mock_fault() {
        if let Some(accepted) = mock_fault_response(runtime, sequence, requested_epoch, &fault) {
            return accepted;
        }
    }
    let manager = match thread_manager() {
        Ok(value) => value,
        Err(reason) => {
            publish_unavailable(runtime, sequence, requested_epoch, reason, 0, 0);
            return false;
        }
    };
    let manager_table = abi::vtable::<abi::ThreadMgrVtbl>(manager);
    if manager_table.is_null() {
        abi::release(manager);
        publish_unavailable(
            runtime,
            sequence,
            requested_epoch,
            REASON_NO_THREAD_MANAGER,
            abi::E_FAIL,
            0,
        );
        return false;
    }
    let mut client_ids = null_mut();
    let qi_hr =
        ((*manager_table).query_interface)(manager, &abi::IID_ITF_CLIENT_ID, &mut client_ids);
    if qi_hr < 0 || client_ids.is_null() {
        abi::release(manager);
        publish_unavailable(
            runtime,
            sequence,
            requested_epoch,
            REASON_CLIENT_ID_UNAVAILABLE,
            qi_hr,
            0,
        );
        return false;
    }
    let client_table = abi::vtable::<abi::ClientIdVtbl>(client_ids);
    let mut client_id = 0_u32;
    let client_hr = if client_table.is_null() {
        abi::E_NOINTERFACE
    } else {
        ((*client_table).get_client_id)(client_ids, &abi::CLSID_ECHO_CARET_OBSERVER, &mut client_id)
    };
    abi::release(client_ids);
    if client_hr < 0 || client_id == 0 {
        abi::release(manager);
        publish_unavailable(
            runtime,
            sequence,
            requested_epoch,
            REASON_CLIENT_ID_UNAVAILABLE,
            client_hr,
            0,
        );
        return false;
    }
    let mut focused = 0_i32;
    let focus_hr = ((*manager_table).is_thread_focus)(manager, &mut focused);
    if focus_hr < 0 || focused == 0 {
        abi::release(manager);
        publish_unavailable(
            runtime,
            sequence,
            requested_epoch,
            REASON_THREAD_NOT_FOCUSED,
            focus_hr,
            0,
        );
        return false;
    }
    let mut document = null_mut();
    let focus_hr = ((*manager_table).get_focus)(manager, &mut document);
    if focus_hr < 0 || document.is_null() {
        abi::release(manager);
        publish_unavailable(
            runtime,
            sequence,
            requested_epoch,
            REASON_NO_DOCUMENT,
            focus_hr,
            0,
        );
        return false;
    }
    let document_identity = canonical_identity(document);
    let document_table = abi::vtable::<abi::DocumentMgrVtbl>(document);
    let mut context = null_mut();
    let top_hr = if document_table.is_null() {
        abi::E_NOINTERFACE
    } else {
        ((*document_table).get_top)(document, &mut context)
    };
    abi::release(document);
    let Some(document_identity) = document_identity else {
        abi::release(context);
        abi::release(manager);
        publish_unavailable(
            runtime,
            sequence,
            requested_epoch,
            REASON_NO_DOCUMENT,
            top_hr,
            0,
        );
        return false;
    };
    if top_hr < 0 || context.is_null() {
        abi::release(manager);
        publish_unavailable(
            runtime,
            sequence,
            requested_epoch,
            REASON_NO_CONTEXT,
            top_hr,
            0,
        );
        return false;
    }
    let context_identity = canonical_identity(context);
    let Some(context_identity) = context_identity else {
        abi::release(context);
        abi::release(manager);
        publish_unavailable(
            runtime,
            sequence,
            requested_epoch,
            REASON_NO_CONTEXT,
            abi::E_NOINTERFACE,
            0,
        );
        return false;
    };
    let Some((context, epoch)) =
        runtime.install_context(document_identity, context, context_identity)
    else {
        abi::release(manager);
        return false;
    };
    if epoch != requested_epoch {
        abi::release(manager);
        publish_unavailable(
            runtime,
            sequence,
            epoch,
            REASON_CONTEXT_MISMATCH,
            abi::S_OK,
            0,
        );
        return false;
    }
    let context_table = abi::vtable::<abi::ContextVtbl>(context);
    let mut status = abi::TfStatus::default();
    let status_hr = if context_table.is_null() {
        abi::E_NOINTERFACE
    } else {
        ((*context_table).get_status)(context, &mut status)
    };
    if status_hr < 0 {
        abi::release(manager);
        publish_unavailable(
            runtime,
            sequence,
            epoch,
            REASON_DISCONNECTED_CONTEXT,
            status_hr,
            0,
        );
        return false;
    }
    let (reason, blocked) = if status.dynamic_flags & abi::TF_SD_READONLY != 0 {
        (REASON_READONLY, true)
    } else if status.dynamic_flags & abi::TF_SD_LOADING != 0 {
        (REASON_LOADING, true)
    } else if status.static_flags & abi::TF_SS_DISJOINTSEL != 0 {
        (REASON_AMBIGUOUS_SELECTION, true)
    } else {
        (0, false)
    };
    if blocked {
        abi::release(manager);
        publish_unavailable(runtime, sequence, epoch, reason, status_hr, 0);
        return false;
    }

    let mut view = null_mut();
    let view_hr = ((*context_table).get_active_view)(context, &mut view);
    let view_table = abi::vtable::<abi::ContextViewVtbl>(view);
    let mut view_window = null_mut();
    let window_hr = if view_table.is_null() || view.is_null() {
        abi::E_NOINTERFACE
    } else {
        ((*view_table).get_wnd)(view, &mut view_window)
    };
    abi::release(view);
    if view_hr < 0
        || window_hr < 0
        || view_window.is_null()
        || !target_is_live(&runtime.header, view_window)
    {
        abi::release(manager);
        publish_unavailable(
            runtime,
            sequence,
            epoch,
            if view_window.is_null() {
                REASON_UNVERIFIED_VIEW
            } else {
                REASON_WRONG_VIEW_OWNER
            },
            if view_hr < 0 { view_hr } else { window_hr },
            0,
        );
        return false;
    }
    let view_context = ffi::GetWindowDpiAwarenessContext(view_window);
    let thread_context = ffi::GetThreadDpiAwarenessContext();
    if view_context.is_null()
        || thread_context.is_null()
        || ffi::AreDpiAwarenessContextsEqual(view_context, thread_context) == 0
    {
        abi::release(manager);
        publish_unavailable(
            runtime,
            sequence,
            epoch,
            REASON_DPI_CONTEXT_MISMATCH,
            abi::S_OK,
            0,
        );
        return false;
    }

    if !target::reserve_callback() {
        abi::release(manager);
        publish_unavailable(
            runtime,
            sequence,
            epoch,
            REASON_CALLBACK_CAP_REACHED,
            abi::S_OK,
            0,
        );
        return false;
    }
    let object = Box::new(EditSessionObject {
        vtable: &EDIT_SESSION_VTABLE,
        refs: AtomicU32::new(1),
        runtime: runtime.weak(),
        header: runtime.header,
        sequence,
        deadline,
        context_epoch: epoch,
        document_identity,
        context_identity,
        entry_tick,
    });
    let session = Box::into_raw(object);
    let mut session_hr = abi::S_OK;
    let api_hr = ((*context_table).request_edit_session)(
        context,
        client_id,
        session.cast(),
        abi::TF_ES_READ | abi::TF_ES_ASYNC,
        &mut session_hr,
    );
    // Balance the caller-owned reference on every return path. TSF owns any
    // additional reference it retained and will invoke the real Release.
    edit_release(session.cast());
    abi::release(manager);
    let accepted = api_hr >= 0 && session_hr == abi::TF_S_ASYNC;
    if !accepted {
        publish_unavailable(
            runtime,
            sequence,
            epoch,
            if api_hr < 0 {
                if api_hr == abi::TF_E_LOCKED {
                    REASON_EDIT_LOCKED
                } else if api_hr == abi::TF_E_NOLOCK {
                    REASON_NO_LOCK
                } else {
                    REASON_EDIT_REQUEST_FAILED
                }
            } else {
                REASON_EDIT_SESSION_MISMATCH
            },
            api_hr,
            session_hr,
        );
        runtime.mark_pending(false);
    }
    accepted
}
