//! Loaded only on an acknowledged editor thread. Handles one read-only request;
//! never intercepts keys, subclasses windows, or changes text or IME state.
#![allow(dead_code)]
mod protocol;
mod tsf;
use protocol::*;
use std::{
    ffi::c_void,
    sync::{atomic::Ordering, OnceLock},
};
type Handle = *mut c_void;
#[repr(C)]
struct CallWindow {
    lparam: isize,
    wparam: usize,
    message: u32,
    window: Handle,
}
#[link(name = "user32")]
extern "system" {
    fn CallNextHookEx(hook: Handle, code: i32, wparam: usize, lparam: isize) -> isize;
    fn RegisterWindowMessageW(name: *const u16) -> u32;
    fn GetFocus() -> Handle;
    fn ClientToScreen(window: Handle, point: *mut [i32; 2]) -> i32;
    fn LogicalToPhysicalPointForPerMonitorDPI(window: Handle, point: *mut [i32; 2]) -> i32;
    fn GetAncestor(window: Handle, flags: u32) -> Handle;
    fn GetForegroundWindow() -> Handle;
    fn GetKeyboardLayout(thread: u32) -> Handle;
}
#[link(name = "kernel32")]
extern "system" {
    fn GetCurrentProcessId() -> u32;
    fn GetCurrentThreadId() -> u32;
    fn GetCurrentProcess() -> Handle;
    fn GetProcessTimes(
        process: Handle,
        created: *mut u64,
        exit: *mut u64,
        kernel: *mut u64,
        user: *mut u64,
    ) -> i32;
    fn GlobalGetAtomNameW(atom: u16, name: *mut u16, size: i32) -> u32;
    fn OpenFileMappingW(access: u32, inherit: i32, name: *const u16) -> Handle;
    fn MapViewOfFile(mapping: Handle, access: u32, high: u32, low: u32, size: usize) -> Handle;
    fn UnmapViewOfFile(view: Handle) -> i32;
    fn CloseHandle(handle: Handle) -> i32;
}
#[link(name = "imm32")]
extern "system" {
    fn ImmGetContext(window: Handle) -> Handle;
    fn ImmGetCandidateWindow(context: Handle, index: u32, form: *mut CandidateForm) -> i32;
    fn ImmGetDefaultIMEWnd(window: Handle) -> Handle;
    fn ImmReleaseContext(window: Handle, context: Handle) -> i32;
    fn ImmGetCompositionStringW(context: Handle, index: u32, data: Handle, bytes: u32) -> i32;
    fn ImmGetOpenStatus(context: Handle) -> i32;
    fn ImmGetConversionStatus(context: Handle, conversion: *mut u32, sentence: *mut u32) -> i32;
}
#[repr(C)]
struct CandidateForm {
    index: u32,
    style: u32,
    point: [i32; 2],
    area: [i32; 4],
}
unsafe fn candidate_caret(input: Handle, context: Handle) -> Option<[i32; 4]> {
    let mut form: CandidateForm = std::mem::zeroed();
    if ImmGetCandidateWindow(context, 0, &mut form) == 0 || form.style != 0x80 {
        return None;
    }
    let [left, top, right, bottom] = form.area;
    if bottom <= top || right < left || right - left > 64 || bottom - top > 256 {
        return None;
    }
    let mut a = [left, top];
    let mut b = [right.max(left + 1), bottom];
    if ClientToScreen(input, &mut a) == 0
        || ClientToScreen(input, &mut b) == 0
        || LogicalToPhysicalPointForPerMonitorDPI(input, &mut a) == 0
        || LogicalToPhysicalPointForPerMonitorDPI(input, &mut b) == 0
    {
        return None;
    }
    Some([a[0], a[1], b[0], b[1]])
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
static MESSAGE_ID: OnceLock<u32> = OnceLock::new();

unsafe fn sample(window: Handle, channel: &Channel) -> Sample {
    let input = channel.input_window as Handle;
    let mut value = Sample::unknown();
    value.pid = GetCurrentProcessId();
    value.thread = GetCurrentThreadId();
    value.window = window as u64;
    let (mut created, mut exit, mut kernel, mut user) = (0, 0, 0, 0);
    if value.pid != channel.target_pid
        || value.thread != channel.target_thread
        || value.window != channel.target_window
        || (input != window
            && (channel.tsf_only != STATE_ONLY || ImmGetDefaultIMEWnd(input) != window))
        || GetFocus() != input
        || GetAncestor(input, 2) != GetForegroundWindow()
        || GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exit,
            &mut kernel,
            &mut user,
        ) == 0
        || created != channel.target_started
    {
        return value;
    }
    if matches!(channel.tsf_only, STATE_ONLY | STATE_GEOMETRY) {
        let tsf_mode = tsf::mode();
        let tsf_active = tsf::active();
        let context = ImmGetContext(input);
        let (mut imm_mode, mut imm_active) = (None, None);
        if !context.is_null() {
            let (mut conversion, mut sentence) = (0, 0);
            if ImmGetConversionStatus(context, &mut conversion, &mut sentence) != 0 {
                imm_mode = Some((ImmGetOpenStatus(context) != 0, conversion));
            }
            if channel.tsf_only == STATE_GEOMETRY {
                value.caret = candidate_caret(input, context).unwrap_or([0; 4]);
            }
            // Length only: this operation never copies preedit or input text.
            let bytes = ImmGetCompositionStringW(context, 8, std::ptr::null_mut(), 0);
            if bytes >= 0 {
                imm_active = Some(bytes > 0);
            }
            ImmReleaseContext(input, context);
        }
        value.status = STATE_REPLY;
        if channel.tsf_only == STATE_GEOMETRY {
            value.bounds = tsf::input_bounds(input).unwrap_or([0; 4]);
        }
        value.mode = mode(
            GetKeyboardLayout(0) as usize as u16,
            agree(tsf_mode, imm_mode),
        );
        // An active source wins; TSF is authoritative for a TSF editor's idle state.
        value.composition = if tsf_active == Some(true) || imm_active == Some(true) {
            2
        } else if tsf_active.or(imm_active) == Some(false) {
            1
        } else {
            0
        };
        if GetFocus() != input || GetAncestor(input, 2) != GetForegroundWindow() {
            value = Sample::unknown();
        }
        return value;
    }
    if channel.tsf_only == 1 {
        value.status = match tsf::active() {
            Some(false) => 3,
            Some(true) => 4,
            None => 0,
        };
        // COM can reenter the host. Reject a focus transition during the read.
        if GetFocus() != input || GetAncestor(input, 2) != GetForegroundWindow() {
            value.status = 0;
        }
        return value;
    }
    let context = ImmGetContext(input);
    if context.is_null() {
        return value;
    }
    let bytes = ImmGetCompositionStringW(context, 8, std::ptr::null_mut(), 0);
    if bytes == 0 {
        value.status = 1;
    } else if bytes > 0 && bytes % 2 == 0 && bytes as usize <= MAX_UNITS * 2 {
        let read =
            ImmGetCompositionStringW(context, 8, value.text.as_mut_ptr().cast(), bytes as u32);
        if read == bytes && String::from_utf16(&value.text[..bytes as usize / 2]).is_ok() {
            value.status = 2;
            value.units = bytes as u32 / 2;
        }
    }
    ImmReleaseContext(input, context);
    value
}

#[no_mangle]
pub unsafe extern "system" fn EchoCompositionObserver(
    code: i32,
    wparam: usize,
    lparam: isize,
) -> isize {
    if code >= 0 && lparam != 0 {
        let message = &*(lparam as *const CallWindow);
        if message.wparam > 0
            && message.wparam <= u16::MAX as usize
            && message.message
                == *MESSAGE_ID.get_or_init(|| RegisterWindowMessageW(wide(MESSAGE).as_ptr()))
        {
            let mut name = [0u16; 256];
            let length =
                GlobalGetAtomNameW(message.wparam as u16, name.as_mut_ptr(), name.len() as i32)
                    as usize;
            if length > 0
                && length < name.len()
                && String::from_utf16_lossy(&name[..length]).starts_with(PREFIX)
            {
                let mapping = OpenFileMappingW(2, 0, name.as_ptr());
                if !mapping.is_null() {
                    let view = MapViewOfFile(mapping, 2, 0, 0, std::mem::size_of::<Channel>());
                    if !view.is_null() {
                        let channel = &mut *view.cast::<Channel>();
                        if channel.magic == MAGIC {
                            let request = channel.request.load(Ordering::Acquire);
                            let observed = sample(message.window, channel);
                            channel.sample = observed;
                            channel.response.store(request, Ordering::Release);
                        }
                        UnmapViewOfFile(view);
                    }
                    CloseHandle(mapping);
                }
            }
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
}
