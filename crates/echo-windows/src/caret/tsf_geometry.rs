//! Read-only TSF geometry acquisition on the validated target input thread.
//!
//! The scheduler calls `request` from its target-thread window procedure.  A
//! request creates one asynchronous, read-only `ITfEditSession`; the callback
//! only reads status, sensitivity metadata, collapsed selection state and
//! `GetTextExt`.  It never reads text or changes the real selection.

use super::{caret_ffi as ffi, caret_protocol as protocol, sensitivity, tsf_abi as abi};
use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicU32, Ordering};

const RESPONSE_READY: u32 = protocol::RESPONSE_READY;
const RESPONSE_UNAVAILABLE: u32 = protocol::RESPONSE_UNAVAILABLE;
const GA_ROOT: u32 = 2;
const MAX_WIDTH: i64 = 64;
const MAX_HEIGHT: i64 = 256;
const MAX_SCOPES: usize = 64;

// Content-free reason identifiers used in the fixed response payload.  The
// diagnostic layer maps these values to the package reason strings.
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

fn response_base(status: u32, sequence: u32, deadline: u64, epoch: u64, reason: u32) -> [u32; 64] {
    let mut words = [0_u32; 64];
    words[0] = status;
    words[1] = sequence;
    words[4] = reason;
    words[8] = deadline as u32;
    words[9] = (deadline >> 32) as u32;
    words[10] = epoch as u32;
    words[11] = (epoch >> 32) as u32;
    words
}

unsafe fn publish_unavailable(
    mailbox: *mut protocol::Mailbox,
    sequence: u32,
    deadline: u64,
    epoch: u64,
    reason: u32,
    api_hr: abi::Hresult,
    session_hr: abi::Hresult,
) {
    if mailbox.is_null() {
        return;
    }
    let mut response = response_base(RESPONSE_UNAVAILABLE, sequence, deadline, epoch, reason);
    response[6] = api_hr as u32;
    response[7] = session_hr as u32;
    let _ = (*mailbox).response.publish(&response);
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
    if ffi::GetWindowThreadProcessId(view, &mut pid) != header.target_thread()
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

trait HeaderThread {
    fn target_thread(&self) -> u32;
}

impl HeaderThread for protocol::Header {
    fn target_thread(&self) -> u32 {
        self.input_thread
    }
}

fn valid_rect(rect: &abi::Rect) -> Option<[i32; 4]> {
    let left = i64::from(rect.left);
    let top = i64::from(rect.top);
    let right = i64::from(rect.right);
    let bottom = i64::from(rect.bottom);
    let width = right.checked_sub(left)?;
    let height = bottom.checked_sub(top)?;
    if width < 0 || height <= 0 || width > MAX_WIDTH || height > MAX_HEIGHT {
        return None;
    }
    let normalized_right = if width == 0 {
        left.checked_add(1)?
    } else {
        right
    };
    if normalized_right < i64::from(i32::MIN)
        || normalized_right > i64::from(i32::MAX)
        || bottom < i64::from(i32::MIN)
        || bottom > i64::from(i32::MAX)
    {
        return None;
    }
    Some([rect.left, rect.top, normalized_right as i32, rect.bottom])
}

#[repr(C)]
struct EditSessionObject {
    vtable: *const abi::EditSessionVtbl,
    refs: AtomicU32,
    mailbox: *mut protocol::Mailbox,
    context: *mut c_void,
    header: protocol::Header,
    sequence: u32,
    deadline: u64,
    context_epoch: u64,
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
    let iid = *iid;
    let iunknown = abi::Guid {
        data1: 0,
        data2: 0,
        data3: 0,
        data4: [0xc0, 0, 0, 0, 0, 0, 0, 0x46],
    };
    if iid == abi::IID_ITF_EDIT_SESSION || iid == iunknown {
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
    let previous = state.refs.fetch_sub(1, Ordering::AcqRel);
    if previous == 1 {
        let boxed = Box::from_raw(object.cast::<EditSessionObject>());
        abi::release(boxed.context);
        0
    } else {
        previous.saturating_sub(1)
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
    acquire_geometry(state, edit_cookie);
    abi::S_OK
}

unsafe fn acquire_geometry(state: &EditSessionObject, edit_cookie: abi::TfEditCookie) {
    if state.mailbox.is_null()
        || (*state.mailbox).closed.load(Ordering::Acquire) != 0
        || ffi::GetTickCount64().saturating_sub(state.entry_tick) > 150
    {
        publish_unavailable(
            state.mailbox,
            state.sequence,
            state.deadline,
            state.context_epoch,
            REASON_LATE_RESULT,
            abi::S_OK,
            abi::S_OK,
        );
        return;
    }
    let mut status = abi::TfStatus::default();
    let context_table = abi::vtable::<abi::ContextVtbl>(state.context);
    if context_table.is_null() || ((*context_table).get_status)(state.context, &mut status) < 0 {
        publish_unavailable(
            state.mailbox,
            state.sequence,
            state.deadline,
            state.context_epoch,
            REASON_DISCONNECTED_CONTEXT,
            abi::E_FAIL,
            abi::E_FAIL,
        );
        return;
    }
    if status.dynamic_flags & abi::TF_SD_READONLY != 0 {
        publish_unavailable(
            state.mailbox,
            state.sequence,
            state.deadline,
            state.context_epoch,
            REASON_READONLY,
            abi::S_OK,
            abi::S_OK,
        );
        return;
    }
    if status.dynamic_flags & abi::TF_SD_LOADING != 0 {
        publish_unavailable(
            state.mailbox,
            state.sequence,
            state.deadline,
            state.context_epoch,
            REASON_LOADING,
            abi::S_OK,
            abi::S_OK,
        );
        return;
    }
    if status.static_flags & abi::TF_SS_DISJOINTSEL != 0 {
        publish_unavailable(
            state.mailbox,
            state.sequence,
            state.deadline,
            state.context_epoch,
            REASON_AMBIGUOUS_SELECTION,
            abi::S_OK,
            abi::S_OK,
        );
        return;
    }

    let mut selection = abi::TfSelection::default();
    let mut fetched = 0_u32;
    let selection_hr = ((*context_table).get_selection)(
        state.context,
        edit_cookie,
        abi::TF_DEFAULT_SELECTION,
        1,
        &mut selection,
        &mut fetched,
    );
    if selection_hr < 0 || fetched != 1 || selection.range.is_null() {
        publish_unavailable(
            state.mailbox,
            state.sequence,
            state.deadline,
            state.context_epoch,
            REASON_NO_SELECTION,
            selection_hr,
            abi::S_OK,
        );
        if !selection.range.is_null() {
            abi::release(selection.range);
        }
        return;
    }
    if selection.style.interim_char != 0 {
        abi::release(selection.range);
        publish_unavailable(
            state.mailbox,
            state.sequence,
            state.deadline,
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
            state.mailbox,
            state.sequence,
            state.deadline,
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
    let property_hr = ((*context_table).get_app_property)(
        state.context,
        &abi::GUID_PROP_INPUTSCOPE,
        &mut property,
    );
    let safety = if property_hr >= 0 && !property.is_null() {
        sensitivity::from_property(property, edit_cookie, selection.range)
    } else {
        sensitivity::Sensitivity::Unknown
    };
    abi::release(property);
    if safety != sensitivity::Sensitivity::Allowed {
        abi::release(selection.range);
        publish_unavailable(
            state.mailbox,
            state.sequence,
            state.deadline,
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
    let view_hr = ((*context_table).get_active_view)(state.context, &mut view);
    if view_hr < 0 || view.is_null() {
        abi::release(selection.range);
        publish_unavailable(
            state.mailbox,
            state.sequence,
            state.deadline,
            state.context_epoch,
            REASON_UNVERIFIED_VIEW,
            view_hr,
            abi::S_OK,
        );
        return;
    }
    let view_table = abi::vtable::<abi::ContextViewVtbl>(view);
    let mut view_window = null_mut();
    if view_table.is_null()
        || ((*view_table).get_wnd)(view, &mut view_window) < 0
        || view_window.is_null()
        || !target_is_live(&state.header, view_window)
    {
        abi::release(view);
        abi::release(selection.range);
        publish_unavailable(
            state.mailbox,
            state.sequence,
            state.deadline,
            state.context_epoch,
            REASON_WRONG_VIEW_OWNER,
            abi::E_FAIL,
            abi::S_OK,
        );
        return;
    }
    let mut rect = abi::Rect::default();
    let mut clipped = 0_i32;
    let text_ext_hr =
        ((*view_table).get_text_ext)(view, edit_cookie, selection.range, &mut rect, &mut clipped);
    let normalized = valid_rect(&rect);
    let late = ffi::GetTickCount64().saturating_sub(state.entry_tick) > 150;
    let live_after = target_is_live(&state.header, view_window)
        && (*state.mailbox).closed.load(Ordering::Acquire) == 0;
    if text_ext_hr < 0 || text_ext_hr == abi::TS_E_NOLAYOUT {
        publish_unavailable(
            state.mailbox,
            state.sequence,
            state.deadline,
            state.context_epoch,
            REASON_NO_LAYOUT,
            text_ext_hr,
            abi::S_OK,
        );
    } else if clipped != 0 {
        publish_unavailable(
            state.mailbox,
            state.sequence,
            state.deadline,
            state.context_epoch,
            REASON_CLIPPED,
            text_ext_hr,
            abi::S_OK,
        );
    } else if normalized.is_none() {
        publish_unavailable(
            state.mailbox,
            state.sequence,
            state.deadline,
            state.context_epoch,
            REASON_INVALID_RECTANGLE,
            text_ext_hr,
            abi::S_OK,
        );
    } else if late || !live_after {
        publish_unavailable(
            state.mailbox,
            state.sequence,
            state.deadline,
            state.context_epoch,
            REASON_LATE_RESULT,
            text_ext_hr,
            abi::S_OK,
        );
    } else if let Some([left, top, right, bottom]) = normalized {
        let mut response = response_base(
            RESPONSE_READY,
            state.sequence,
            state.deadline,
            state.context_epoch,
            0,
        );
        response[2] = left as u32;
        response[3] = top as u32;
        response[4] = right as u32;
        response[5] = bottom as u32;
        response[6] = state.entry_tick as u32;
        response[7] = (state.entry_tick >> 32) as u32;
        response[12] = 1; // TsfCaret
        response[13] = 1; // Exact
        let _ = (*state.mailbox).response.publish(&response);
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

/// Start one read-only TSF request.  `true` means the request was accepted by
/// TSF and the callback owns the edit-session object; failures publish one
/// bounded unavailable response immediately.
pub unsafe fn request(
    mailbox: *mut protocol::Mailbox,
    header: &protocol::Header,
    sequence: u32,
    deadline: u64,
    entry_tick: u64,
) -> bool {
    let manager = match thread_manager() {
        Ok(value) => value,
        Err(reason) => {
            publish_unavailable(
                mailbox,
                sequence,
                deadline,
                1,
                reason,
                abi::E_FAIL,
                abi::E_FAIL,
            );
            return false;
        }
    };
    let manager_table = abi::vtable::<abi::ThreadMgrVtbl>(manager);
    if manager_table.is_null() {
        abi::release(manager);
        publish_unavailable(
            mailbox,
            sequence,
            deadline,
            1,
            REASON_NO_THREAD_MANAGER,
            abi::E_FAIL,
            abi::E_FAIL,
        );
        return false;
    }
    let mut client_ids = null_mut();
    let qi_hr =
        ((*manager_table).query_interface)(manager, &abi::IID_ITF_CLIENT_ID, &mut client_ids);
    if qi_hr < 0 || client_ids.is_null() {
        abi::release(manager);
        publish_unavailable(
            mailbox,
            sequence,
            deadline,
            1,
            REASON_CLIENT_ID_UNAVAILABLE,
            qi_hr,
            abi::E_FAIL,
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
            mailbox,
            sequence,
            deadline,
            1,
            REASON_CLIENT_ID_UNAVAILABLE,
            client_hr,
            abi::S_OK,
        );
        return false;
    }
    let mut focused = 0_i32;
    if ((*manager_table).is_thread_focus)(manager, &mut focused) < 0 || focused == 0 {
        abi::release(manager);
        publish_unavailable(
            mailbox,
            sequence,
            deadline,
            1,
            REASON_THREAD_NOT_FOCUSED,
            abi::E_FAIL,
            abi::S_OK,
        );
        return false;
    }
    let mut document = null_mut();
    let focus_hr = ((*manager_table).get_focus)(manager, &mut document);
    if focus_hr < 0 || document.is_null() {
        abi::release(manager);
        publish_unavailable(
            mailbox,
            sequence,
            deadline,
            1,
            REASON_NO_DOCUMENT,
            focus_hr,
            abi::S_OK,
        );
        return false;
    }
    let document_table = abi::vtable::<abi::DocumentMgrVtbl>(document);
    let mut context = null_mut();
    let top_hr = if document_table.is_null() {
        abi::E_NOINTERFACE
    } else {
        ((*document_table).get_top)(document, &mut context)
    };
    abi::release(document);
    if top_hr < 0 || context.is_null() {
        abi::release(manager);
        publish_unavailable(
            mailbox,
            sequence,
            deadline,
            1,
            REASON_NO_CONTEXT,
            top_hr,
            abi::S_OK,
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
        abi::release(context);
        abi::release(manager);
        publish_unavailable(
            mailbox,
            sequence,
            deadline,
            1,
            REASON_DISCONNECTED_CONTEXT,
            status_hr,
            abi::S_OK,
        );
        return false;
    }
    if status.dynamic_flags & abi::TF_SD_READONLY != 0 {
        abi::release(context);
        abi::release(manager);
        publish_unavailable(
            mailbox,
            sequence,
            deadline,
            1,
            REASON_READONLY,
            status_hr,
            abi::S_OK,
        );
        return false;
    }
    if status.dynamic_flags & abi::TF_SD_LOADING != 0 {
        abi::release(context);
        abi::release(manager);
        publish_unavailable(
            mailbox,
            sequence,
            deadline,
            1,
            REASON_LOADING,
            status_hr,
            abi::S_OK,
        );
        return false;
    }
    if status.static_flags & abi::TF_SS_DISJOINTSEL != 0 {
        abi::release(context);
        abi::release(manager);
        publish_unavailable(
            mailbox,
            sequence,
            deadline,
            1,
            REASON_AMBIGUOUS_SELECTION,
            status_hr,
            abi::S_OK,
        );
        return false;
    }
    let mut view = null_mut();
    let view_hr = ((*context_table).get_active_view)(context, &mut view);
    let mut view_window = null_mut();
    let view_table = abi::vtable::<abi::ContextViewVtbl>(view);
    let window_hr = if view_table.is_null() || view.is_null() {
        abi::E_NOINTERFACE
    } else {
        ((*view_table).get_wnd)(view, &mut view_window)
    };
    abi::release(view);
    if view_hr < 0 || window_hr < 0 || view_window.is_null() || !target_is_live(header, view_window)
    {
        abi::release(context);
        abi::release(manager);
        publish_unavailable(
            mailbox,
            sequence,
            deadline,
            1,
            if view_window.is_null() {
                REASON_UNVERIFIED_VIEW
            } else {
                REASON_WRONG_VIEW_OWNER
            },
            view_hr.max(window_hr),
            abi::S_OK,
        );
        return false;
    }
    if ffi::GetWindowDpiAwarenessContext(view_window) != ffi::GetThreadDpiAwarenessContext() {
        abi::release(context);
        abi::release(manager);
        publish_unavailable(
            mailbox,
            sequence,
            deadline,
            1,
            REASON_DPI_CONTEXT_MISMATCH,
            abi::S_OK,
            abi::S_OK,
        );
        return false;
    }

    abi::add_ref(context);
    let object = Box::new(EditSessionObject {
        vtable: &EDIT_SESSION_VTABLE,
        refs: AtomicU32::new(1),
        mailbox,
        context,
        header: *header,
        sequence,
        deadline,
        context_epoch: 1,
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
    abi::release(context);
    abi::release(manager);
    if api_hr < 0 {
        edit_release(session.cast());
        publish_unavailable(
            mailbox,
            sequence,
            deadline,
            1,
            if api_hr == abi::TF_E_LOCKED {
                REASON_EDIT_LOCKED
            } else if api_hr == abi::TF_E_NOLOCK {
                REASON_NO_LOCK
            } else {
                REASON_EDIT_REQUEST_FAILED
            },
            api_hr,
            session_hr,
        );
        false
    } else {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_width_caret_is_normalized_after_screen_validation() {
        assert_eq!(
            valid_rect(&abi::Rect {
                left: -20,
                top: 40,
                right: -20,
                bottom: 58
            }),
            Some([-20, 40, -19, 58])
        );
    }

    #[test]
    fn malformed_rectangles_are_rejected() {
        assert!(valid_rect(&abi::Rect {
            left: 0,
            top: 0,
            right: 65,
            bottom: 10
        })
        .is_none());
        assert!(valid_rect(&abi::Rect {
            left: 0,
            top: 0,
            right: 10,
            bottom: 0
        })
        .is_none());
        assert!(valid_rect(&abi::Rect {
            left: 10,
            top: 0,
            right: 0,
            bottom: 10
        })
        .is_none());
    }
}
