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

const COMPARTMENTS: Guid = Guid(
    0x7dcf57ac,
    0x18ad,
    0x438b,
    [0x82, 0x4d, 0x97, 0x9b, 0xff, 0xb7, 0x4b, 0x7c],
);
const OPEN: Guid = Guid(
    0x58273aad,
    0x01bb,
    0x4164,
    [0x95, 0xc6, 0x75, 0x5b, 0xa0, 0xb5, 0x16, 0x2d],
);
const CONVERSION: Guid = Guid(
    0xccf05dd8,
    0x4a87,
    0x11d7,
    [0xa6, 0xe2, 0x00, 0x06, 0x5b, 0x84, 0x43, 0x5c],
);
#[repr(C)]
struct CompartmentMgr {
    base: Unknown,
    get: unsafe extern "system" fn(Handle, *const Guid, *mut Handle) -> i32,
}
#[repr(C)]
struct Variant {
    vt: u16,
    reserved: [u16; 3],
    data: [usize; 2],
}
#[repr(C)]
struct Compartment {
    base: Unknown,
    set: *const c_void,
    get: unsafe extern "system" fn(Handle, *mut Variant) -> i32,
}
#[link(name = "oleaut32")]
extern "system" {
    fn VariantClear(value: *mut Variant) -> i32;
}

unsafe fn thread_manager() -> Option<Com> {
    let module = GetModuleHandleW(super::wide("msctf.dll").as_ptr());
    if module.is_null() {
        return None;
    }
    let address = GetProcAddress(module, b"TF_GetThreadMgr\0".as_ptr());
    if address.is_null() {
        return None;
    }
    let get: unsafe extern "system" fn(*mut Handle) -> i32 = std::mem::transmute(address);
    Com::receive(|out| get(out))
}

pub(super) unsafe fn mode() -> Option<(bool, u32)> {
    let manager = thread_manager()?;
    let compartments =
        Com::receive(|out| (manager.table::<Unknown>().query)(manager.0, &COMPARTMENTS, out))?;
    let read = |guid: &Guid| -> Option<u32> {
        let compartment = Com::receive(|out| {
            (compartments.table::<CompartmentMgr>().get)(compartments.0, guid, out)
        })?;
        let mut value: Variant = std::mem::zeroed();
        let hr = (compartment.table::<Compartment>().get)(compartment.0, &mut value);
        let result = (hr >= 0 && value.vt == 3).then_some(value.data[0] as u32);
        VariantClear(&mut value);
        result
    };
    Some((read(&OPEN)? != 0, read(&CONVERSION)?))
}
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

#[repr(C)]
struct Context {
    base: Unknown,
    unused: [*const c_void; 6],
    view: GetObject,
    enum_views: *const c_void,
    status: unsafe extern "system" fn(Handle, *mut [u32; 2]) -> i32,
}
#[repr(C)]
struct ContextView {
    base: Unknown,
    from_point: *const c_void,
    text_ext: unsafe extern "system" fn(Handle, u32, Handle, *mut [i32; 4], *mut i32) -> i32,
    screen: unsafe extern "system" fn(Handle, *mut [i32; 4]) -> i32,
    window: GetObject,
}
// No text or edit cookie is requested. GetScreenExt is the focused document's
// display rectangle, never an inferred insertion caret.
pub(super) unsafe fn input_bounds(input: Handle) -> Option<[i32; 4]> {
    let manager = thread_manager()?;
    let document = Com::receive(|out| (manager.table::<ThreadMgr>().get_focus)(manager.0, out))?;
    let context = Com::receive(|out| (document.table::<DocumentMgr>().get_top)(document.0, out))?;
    let mut status = [0; 2];
    if (context.table::<Context>().status)(context.0, &mut status) < 0 || status[0] & 1 != 0 {
        return None;
    }
    let view = Com::receive(|out| (context.table::<Context>().view)(context.0, out))?;
    let mut window = null_mut();
    if (view.table::<ContextView>().window)(view.0, &mut window) < 0 || window != input {
        return None;
    }
    let mut bounds = [0; 4];
    if (view.table::<ContextView>().screen)(view.0, &mut bounds) < 0
        || bounds[2] <= bounds[0]
        || bounds[3] <= bounds[1]
    {
        return None;
    }
    Some(bounds)
}
