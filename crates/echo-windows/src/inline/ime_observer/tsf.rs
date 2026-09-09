//! Read the existing TSF context on its owning thread. Never create/activate a
//! thread manager, request an edit session, or change focus/composition.
use super::Handle;
use std::{ffi::c_void, ptr::null_mut};

#[repr(C)]
struct Guid(u32, u16, u16, [u8; 8]);
const CONTEXT_COMPOSITION: Guid = Guid(
    0xd40c8aae,
    0xac92,
    0x4fc7,
    [0x9a, 0x11, 0x0e, 0xe0, 0xe2, 0x3a, 0xa3, 0x9b],
);
type GetObject = unsafe extern "system" fn(Handle, *mut Handle) -> i32;
#[repr(C)]
struct Unknown {
    query: unsafe extern "system" fn(Handle, *const Guid, *mut Handle) -> i32,
    add_ref: unsafe extern "system" fn(Handle) -> u32,
    release: unsafe extern "system" fn(Handle) -> u32,
}
// Prefixes of the SDK's msctf.h vtables, through the last method used here.
#[repr(C)]
struct ThreadMgr {
    base: Unknown,
    unused: [*const c_void; 4], // Activate, Deactivate, CreateDocumentMgr, EnumDocumentMgrs
    get_focus: GetObject,
}
#[repr(C)]
struct DocumentMgr {
    base: Unknown,
    unused: [*const c_void; 3], // CreateContext, Push, Pop
    get_top: GetObject,
}
#[repr(C)]
struct Composition {
    base: Unknown,
    start: *const c_void,
    enumerate: GetObject,
}
#[repr(C)]
struct Enumerator {
    base: Unknown,
    clone: *const c_void,
    next: unsafe extern "system" fn(Handle, u32, *mut Handle, *mut u32) -> i32,
}
struct Com(Handle);
impl Com {
    unsafe fn table<T>(&self) -> &T {
        &**self.0.cast::<*const T>()
    }
    unsafe fn receive(call: impl FnOnce(*mut Handle) -> i32) -> Option<Self> {
        let mut raw = null_mut();
        let hr = call(&mut raw);
        let value = (!raw.is_null()).then(|| Self(raw));
        if hr >= 0 {
            value
        } else {
            None
        }
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        unsafe {
            (self.table::<Unknown>().release)(self.0);
        }
    }
}
#[link(name = "kernel32")]
extern "system" {
    fn GetModuleHandleW(name: *const u16) -> Handle;
    fn GetProcAddress(module: Handle, name: *const u8) -> *const c_void;
}

pub(super) unsafe fn active() -> Option<bool> {
    // TF_GetThreadMgr has no import library. Only use the module already loaded
    // by this editor; absence is Unknown, not permission to install TSF.
    let module = GetModuleHandleW(super::wide("msctf.dll").as_ptr());
    if module.is_null() {
        return None;
    }
    let address = GetProcAddress(module, b"TF_GetThreadMgr\0".as_ptr());
    if address.is_null() {
        return None;
    }
    let get: unsafe extern "system" fn(*mut Handle) -> i32 = std::mem::transmute(address);
    let manager = Com::receive(|out| get(out))?;
    let document = Com::receive(|out| (manager.table::<ThreadMgr>().get_focus)(manager.0, out))?;
    let context = Com::receive(|out| (document.table::<DocumentMgr>().get_top)(document.0, out))?;
    let composition = Com::receive(|out| {
        (context.table::<Unknown>().query)(context.0, &CONTEXT_COMPOSITION, out)
    })?;
    let enumerator =
        Com::receive(|out| (composition.table::<Composition>().enumerate)(composition.0, out))?;
    let mut raw = null_mut();
    let mut count = 0;
    let hr = (enumerator.table::<Enumerator>().next)(enumerator.0, 1, &mut raw, &mut count);
    let view = (!raw.is_null()).then(|| Com(raw));
    match (hr, count, view.is_some()) {
        (0, 1, true) => Some(true),
        (1, 0, false) => Some(false), // S_FALSE: enumeration completed without a composition
        _ => None,
    }
}
