//! Loaded only on an acknowledged editor thread. Handles one read-only request;
//! never intercepts keys, subclasses windows, or changes text or IME state.
#![allow(dead_code)]
mod protocol;
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
    fn GetAncestor(window: Handle, flags: u32) -> Handle;
    fn GetForegroundWindow() -> Handle;
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
    fn ImmReleaseContext(window: Handle, context: Handle) -> i32;
    fn ImmGetCompositionStringW(context: Handle, index: u32, data: Handle, bytes: u32) -> i32;
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
static MESSAGE_ID: OnceLock<u32> = OnceLock::new();

unsafe fn sample(window: Handle, channel: &Channel) -> Sample {
    let mut value = Sample::unknown();
    value.pid = GetCurrentProcessId();
    value.thread = GetCurrentThreadId();
    value.window = window as u64;
    let (mut created, mut exit, mut kernel, mut user) = (0, 0, 0, 0);
    if value.pid != channel.target_pid
        || value.thread != channel.target_thread
        || value.window != channel.target_window
        || GetFocus() != window
        || GetAncestor(window, 2) != GetForegroundWindow()
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
    let context = ImmGetContext(window);
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
    ImmReleaseContext(window, context);
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
