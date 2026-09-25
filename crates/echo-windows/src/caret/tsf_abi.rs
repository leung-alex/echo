//! Checked, std-only declarations for the read-only TSF acquisition path.
//!
//! This file is compiled into the target-thread observer with standalone
//! `rustc`.  Keep interface layouts in the same order as the Windows SDK
//! vtables; callers must never activate a manager or mutate a context.

use super::caret_ffi as ffi;
use std::ffi::c_void;

pub type Hresult = ffi::Hresult;
pub type Bool = i32;
pub type Dword = u32;
pub type Ulong = u32;
pub type TfClientId = u32;
pub type TfEditCookie = u32;
pub type Hwnd = ffi::Hwnd;

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Guid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TfStatus {
    pub dynamic_flags: Dword,
    pub static_flags: Dword,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TfSelectionStyle {
    pub active_selection_end: u32,
    pub interim_char: Bool,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct TfSelection {
    pub range: *mut c_void,
    pub style: TfSelectionStyle,
}

// VARIANT is 16 bytes on x86 and 24 bytes on x64.  The two pointer-sized
// words preserve the SDK union's alignment without exposing any value data.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct Variant {
    pub vt: u16,
    pub reserved1: u16,
    pub reserved2: u16,
    pub reserved3: u16,
    pub data: [usize; 2],
}

impl Default for Variant {
    fn default() -> Self {
        Self {
            vt: 0,
            reserved1: 0,
            reserved2: 0,
            reserved3: 0,
            data: [0; 2],
        }
    }
}

#[repr(C)]
pub struct UnknownVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult,
    pub add_ref: unsafe extern "system" fn(*mut c_void) -> Ulong,
    pub release: unsafe extern "system" fn(*mut c_void) -> Ulong,
}

#[repr(C)]
pub struct ThreadMgrVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult,
    pub add_ref: unsafe extern "system" fn(*mut c_void) -> Ulong,
    pub release: unsafe extern "system" fn(*mut c_void) -> Ulong,
    pub activate: unsafe extern "system" fn(*mut c_void, *mut TfClientId) -> Hresult,
    pub deactivate: unsafe extern "system" fn(*mut c_void) -> Hresult,
    pub create_document_mgr: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> Hresult,
    pub enum_document_mgrs: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> Hresult,
    pub get_focus: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> Hresult,
    pub set_focus: unsafe extern "system" fn(*mut c_void, *mut c_void) -> Hresult,
    pub associate_focus:
        unsafe extern "system" fn(*mut c_void, Hwnd, *mut c_void, *mut *mut c_void) -> Hresult,
    pub is_thread_focus: unsafe extern "system" fn(*mut c_void, *mut Bool) -> Hresult,
    pub get_function_provider:
        unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult,
    pub enum_function_providers:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> Hresult,
    pub get_global_compartment: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> Hresult,
}

#[repr(C)]
pub struct ClientIdVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult,
    pub add_ref: unsafe extern "system" fn(*mut c_void) -> Ulong,
    pub release: unsafe extern "system" fn(*mut c_void) -> Ulong,
    pub get_client_id:
        unsafe extern "system" fn(*mut c_void, *const Guid, *mut TfClientId) -> Hresult,
}

#[repr(C)]
pub struct DocumentMgrVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult,
    pub add_ref: unsafe extern "system" fn(*mut c_void) -> Ulong,
    pub release: unsafe extern "system" fn(*mut c_void) -> Ulong,
    pub create_context: unsafe extern "system" fn(
        *mut c_void,
        TfClientId,
        Dword,
        *mut c_void,
        *mut *mut c_void,
        *mut TfEditCookie,
    ) -> Hresult,
    pub push: unsafe extern "system" fn(*mut c_void, *mut c_void) -> Hresult,
    pub pop: unsafe extern "system" fn(*mut c_void, Dword) -> Hresult,
    pub get_top: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> Hresult,
}

#[repr(C)]
pub struct ContextVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult,
    pub add_ref: unsafe extern "system" fn(*mut c_void) -> Ulong,
    pub release: unsafe extern "system" fn(*mut c_void) -> Ulong,
    pub request_edit_session: unsafe extern "system" fn(
        *mut c_void,
        TfClientId,
        *mut c_void,
        Dword,
        *mut Hresult,
    ) -> Hresult,
    pub in_write_session: unsafe extern "system" fn(*mut c_void, TfClientId, *mut Bool) -> Hresult,
    pub get_selection: unsafe extern "system" fn(
        *mut c_void,
        TfEditCookie,
        u32,
        u32,
        *mut TfSelection,
        *mut u32,
    ) -> Hresult,
    pub set_selection:
        unsafe extern "system" fn(*mut c_void, TfEditCookie, u32, *const TfSelection) -> Hresult,
    pub get_start:
        unsafe extern "system" fn(*mut c_void, TfEditCookie, *mut *mut c_void) -> Hresult,
    pub get_end: unsafe extern "system" fn(*mut c_void, TfEditCookie, *mut *mut c_void) -> Hresult,
    pub get_active_view: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> Hresult,
    pub enum_views: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> Hresult,
    pub get_status: unsafe extern "system" fn(*mut c_void, *mut TfStatus) -> Hresult,
    pub get_property:
        unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult,
    pub get_app_property:
        unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult,
    pub track_properties: unsafe extern "system" fn(
        *mut c_void,
        *const *const Guid,
        u32,
        *const *const Guid,
        u32,
        *mut *mut c_void,
    ) -> Hresult,
    pub enum_properties: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> Hresult,
    pub get_document_mgr: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> Hresult,
    pub create_range_backup: unsafe extern "system" fn(
        *mut c_void,
        TfEditCookie,
        *mut c_void,
        *mut *mut c_void,
    ) -> Hresult,
}

#[repr(C)]
pub struct ContextViewVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult,
    pub add_ref: unsafe extern "system" fn(*mut c_void) -> Ulong,
    pub release: unsafe extern "system" fn(*mut c_void) -> Ulong,
    pub get_range_from_point: unsafe extern "system" fn(
        *mut c_void,
        TfEditCookie,
        *const ffi::Point,
        Dword,
        *mut *mut c_void,
    ) -> Hresult,
    pub get_text_ext: unsafe extern "system" fn(
        *mut c_void,
        TfEditCookie,
        *mut c_void,
        *mut Rect,
        *mut Bool,
    ) -> Hresult,
    pub get_screen_ext: unsafe extern "system" fn(*mut c_void, *mut Rect) -> Hresult,
    pub get_wnd: unsafe extern "system" fn(*mut c_void, *mut Hwnd) -> Hresult,
}

#[repr(C)]
pub struct RangeVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult,
    pub add_ref: unsafe extern "system" fn(*mut c_void) -> Ulong,
    pub release: unsafe extern "system" fn(*mut c_void) -> Ulong,
    pub get_text: *const c_void,
    pub set_text: *const c_void,
    pub get_formatted_text: *const c_void,
    pub get_embedded: *const c_void,
    pub insert_embedded: *const c_void,
    pub shift_start: *const c_void,
    pub shift_end: *const c_void,
    pub shift_start_to_range: *const c_void,
    pub shift_end_to_range: *const c_void,
    pub shift_start_region: *const c_void,
    pub shift_end_region: *const c_void,
    pub is_empty: unsafe extern "system" fn(*mut c_void, TfEditCookie, *mut Bool) -> Hresult,
}

#[repr(C)]
pub struct ReadOnlyPropertyVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult,
    pub add_ref: unsafe extern "system" fn(*mut c_void) -> Ulong,
    pub release: unsafe extern "system" fn(*mut c_void) -> Ulong,
    pub get_type: unsafe extern "system" fn(*mut c_void, *mut Guid) -> Hresult,
    pub enum_ranges: *const c_void,
    pub get_value:
        unsafe extern "system" fn(*mut c_void, TfEditCookie, *mut c_void, *mut Variant) -> Hresult,
    pub get_context: unsafe extern "system" fn(*mut c_void, *mut *mut c_void) -> Hresult,
}

#[repr(C)]
pub struct InputScopeVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult,
    pub add_ref: unsafe extern "system" fn(*mut c_void) -> Ulong,
    pub release: unsafe extern "system" fn(*mut c_void) -> Ulong,
    pub get_input_scopes:
        unsafe extern "system" fn(*mut c_void, *mut *mut u32, *mut u32) -> Hresult,
}

#[repr(C)]
pub struct EditSessionVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult,
    pub add_ref: unsafe extern "system" fn(*mut c_void) -> Ulong,
    pub release: unsafe extern "system" fn(*mut c_void) -> Ulong,
    pub do_edit_session: unsafe extern "system" fn(*mut c_void, TfEditCookie) -> Hresult,
}

#[repr(C)]
pub struct SourceVtbl {
    pub query_interface:
        unsafe extern "system" fn(*mut c_void, *const Guid, *mut *mut c_void) -> Hresult,
    pub add_ref: unsafe extern "system" fn(*mut c_void) -> Ulong,
    pub release: unsafe extern "system" fn(*mut c_void) -> Ulong,
    pub advise_sink:
        unsafe extern "system" fn(*mut c_void, *const Guid, *mut c_void, *mut u32) -> Hresult,
    pub unadvise_sink: unsafe extern "system" fn(*mut c_void, u32) -> Hresult,
}

pub const S_OK: Hresult = 0;
pub const S_FALSE: Hresult = 1;
pub const E_NOINTERFACE: Hresult = 0x8000_4002_u32 as i32;
pub const E_FAIL: Hresult = 0x8000_4005_u32 as i32;
pub const E_INVALIDARG: Hresult = 0x8007_0057_u32 as i32;
pub const TF_E_LOCKED: Hresult = 0x8004_0500_u32 as i32;
pub const TF_E_NOLOCK: Hresult = 0x8004_0201_u32 as i32;
pub const TF_E_DISCONNECTED: Hresult = 0x8004_0504_u32 as i32;
pub const TF_E_NOSELECTION: Hresult = 0x8004_0205_u32 as i32;
pub const TS_E_NOLAYOUT: Hresult = 0x8004_0206_u32 as i32;
pub const TF_S_ASYNC: Hresult = 0x0004_0300;
pub const TF_ES_READ: Dword = 0x2;
pub const TF_ES_ASYNC: Dword = 0x8;
pub const TF_DEFAULT_SELECTION: u32 = u32::MAX;
pub const TF_SD_READONLY: Dword = 0x1;
pub const TF_SD_LOADING: Dword = 0x2;
pub const TF_SS_DISJOINTSEL: Dword = 0x1;
pub const VT_UNKNOWN: u16 = 13;
pub const IS_PASSWORD: u32 = 31;

pub const IID_ITF_CLIENT_ID: Guid = Guid {
    data1: 0xd60a7b49,
    data2: 0x1b9f,
    data3: 0x4be2,
    data4: [0xb7, 0x02, 0x47, 0xe9, 0xdc, 0x05, 0xde, 0xc3],
};
pub const IID_IUNKNOWN: Guid = Guid {
    data1: 0x00000000,
    data2: 0x0000,
    data3: 0x0000,
    data4: [0xc0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46],
};
pub const IID_ITF_EDIT_SESSION: Guid = Guid {
    data1: 0xaa80e803,
    data2: 0x2021,
    data3: 0x11d2,
    data4: [0x93, 0xe0, 0x00, 0x60, 0xb0, 0x67, 0xb8, 0x6e],
};
pub const IID_ITF_SOURCE: Guid = Guid {
    data1: 0x4ea48a35,
    data2: 0x60ae,
    data3: 0x446f,
    data4: [0x8f, 0xd6, 0xe6, 0xa8, 0xd8, 0x24, 0x59, 0xf7],
};
pub const IID_ITF_INPUT_SCOPE: Guid = Guid {
    data1: 0xfde1eaee,
    data2: 0x6924,
    data3: 0x4cdf,
    data4: [0x91, 0xe7, 0xda, 0x38, 0xcf, 0xf5, 0x55, 0x9d],
};
pub const GUID_PROP_INPUTSCOPE: Guid = Guid {
    data1: 0x1713dd5a,
    data2: 0x68e7,
    data3: 0x4a5b,
    data4: [0x9a, 0xf6, 0x59, 0x2a, 0x59, 0x5c, 0x77, 0x8d],
};
pub const CLSID_ECHO_CARET_OBSERVER: Guid = Guid {
    data1: 0x05b48319,
    data2: 0x26c2,
    data3: 0x5d88,
    data4: [0x92, 0x8b, 0x81, 0xd1, 0x55, 0x88, 0x8f, 0xc4],
};

pub unsafe fn vtable<T>(object: *mut c_void) -> *const T {
    if object.is_null() {
        std::ptr::null()
    } else {
        *(object.cast::<*const T>())
    }
}

pub unsafe fn release(object: *mut c_void) {
    if !object.is_null() {
        let table = vtable::<UnknownVtbl>(object);
        if !table.is_null() {
            ((*table).release)(object);
        }
    }
}

pub unsafe fn add_ref(object: *mut c_void) {
    if !object.is_null() {
        let table = vtable::<UnknownVtbl>(object);
        if !table.is_null() {
            ((*table).add_ref)(object);
        }
    }
}

#[link(name = "oleaut32")]
extern "system" {
    pub fn VariantClear(value: *mut Variant) -> Hresult;
}

#[link(name = "ole32")]
extern "system" {
    pub fn CoTaskMemFree(value: *mut c_void);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{align_of, size_of};

    #[test]
    fn sdk_layouts_match_x64_reference() {
        assert_eq!(size_of::<TfStatus>(), 8);
        assert_eq!(size_of::<TfSelectionStyle>(), 8);
        assert_eq!(size_of::<TfSelection>(), 16);
        assert_eq!(size_of::<Rect>(), 16);
        assert_eq!(
            size_of::<Variant>(),
            if cfg!(target_pointer_width = "64") {
                24
            } else {
                16
            }
        );
        assert_eq!(
            align_of::<Variant>(),
            if cfg!(target_pointer_width = "64") {
                8
            } else {
                4
            }
        );
        assert_eq!(
            std::mem::size_of::<ThreadMgrVtbl>() / std::mem::size_of::<usize>(),
            14
        );
        assert_eq!(
            std::mem::size_of::<ClientIdVtbl>() / std::mem::size_of::<usize>(),
            4
        );
        assert_eq!(
            std::mem::size_of::<ContextViewVtbl>() / std::mem::size_of::<usize>(),
            7
        );
        assert_eq!(
            std::mem::size_of::<EditSessionVtbl>() / std::mem::size_of::<usize>(),
            4
        );
    }
}
