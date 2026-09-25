//! Standalone-rustc Win32/COM declarations for the target observer DLL.
//!
//! Keep this file std-only. The Cargo host adapter uses `host_ffi.rs`; this
//! module is included from `inline/ime_observer/dll.rs` with an explicit path.

use std::ffi::c_void;

pub type Handle = *mut c_void;
pub type Hwnd = Handle;
pub type Hmodule = Handle;
pub type Hhook = Handle;
pub type Hresult = i32;
pub type Wparam = usize;
pub type Lparam = isize;
pub type Lresult = isize;

pub const S_OK: Hresult = 0;
pub const S_FALSE: Hresult = 1;
pub const E_NOINTERFACE: Hresult = 0x80004002_u32 as i32;
pub const E_FAIL: Hresult = 0x80004005_u32 as i32;
pub const E_INVALIDARG: Hresult = 0x80070057_u32 as i32;
pub const TF_S_ASYNC: Hresult = 0x00040300;
pub const TF_E_LOCKED: Hresult = 0x80040500_u32 as i32;
pub const TF_E_DISCONNECTED: Hresult = 0x80040201_u32 as i32;
pub const TF_E_NOLOCK: Hresult = 0x8004020E_u32 as i32;
pub const TS_E_NOLAYOUT: Hresult = 0x80040206_u32 as i32;

pub const WM_APP: u32 = 0x8000;
pub const WM_NCCREATE: u32 = 0x0081;
pub const WM_TIMER: u32 = 0x0113;
pub const WM_CLOSE: u32 = 0x0010;
pub const WH_CALLWNDPROC: i32 = 4;
pub const HWND_MESSAGE: Hwnd = (-3isize) as Hwnd;
pub const GWLP_USERDATA: i32 = -21;
pub const PM_REMOVE: u32 = 0x0001;
pub const SMTO_ABORTIFHUNG: u32 = 0x0002;
pub const SMTO_BLOCK: u32 = 0x0001;
pub const SMTO_ERRORONEXIT: u32 = 0x0020;
pub const FILE_MAP_READ: u32 = 0x0004;
pub const FILE_MAP_WRITE: u32 = 0x0002;
pub const PAGE_READWRITE: u32 = 0x04;
pub const ERROR_ALREADY_EXISTS: u32 = 183;
pub const ERROR_CLASS_ALREADY_EXISTS: u32 = 1410;
pub const INVALID_HANDLE_VALUE: Handle = (-1isize) as Handle;
pub const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
pub const DPI_AWARENESS_UNAWARE: i32 = 0;
pub const DPI_AWARENESS_SYSTEM: i32 = 1;
pub const DPI_AWARENESS_PER_MONITOR: i32 = 2;

#[repr(C)]
pub struct CallWindow {
    pub lparam: Lparam,
    pub wparam: Wparam,
    pub message: u32,
    pub window: Hwnd,
}

#[repr(C)]
pub struct Point {
    pub x: i32,
    pub y: i32,
}

#[repr(C)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[repr(C)]
pub struct CandidateForm {
    pub index: u32,
    pub style: u32,
    pub point: Point,
    pub area: Rect,
}

#[repr(C)]
pub struct WndClassW {
    pub style: u32,
    pub wnd_proc: Option<unsafe extern "system" fn(Hwnd, u32, Wparam, Lparam) -> Lresult>,
    pub cls_extra: i32,
    pub wnd_extra: i32,
    pub instance: Hmodule,
    pub icon: Handle,
    pub cursor: Handle,
    pub background: Handle,
    pub menu_name: *const u16,
    pub class_name: *const u16,
}

#[repr(C)]
pub struct Msg {
    pub hwnd: Hwnd,
    pub message: u32,
    pub wparam: Wparam,
    pub lparam: Lparam,
    pub time: u32,
    pub point: Point,
}

#[link(name = "user32")]
extern "system" {
    pub fn CallNextHookEx(hook: Hhook, code: i32, wparam: Wparam, lparam: Lparam) -> Lresult;
    pub fn RegisterWindowMessageW(name: *const u16) -> u32;
    pub fn GetFocus() -> Hwnd;
    pub fn GetAncestor(window: Hwnd, flags: u32) -> Hwnd;
    pub fn GetForegroundWindow() -> Hwnd;
    pub fn GetWindowThreadProcessId(window: Hwnd, pid: *mut u32) -> u32;
    pub fn GetWindowLongPtrW(window: Hwnd, index: i32) -> isize;
    pub fn SetWindowLongPtrW(window: Hwnd, index: i32, value: isize) -> isize;
    pub fn DefWindowProcW(window: Hwnd, message: u32, wparam: Wparam, lparam: Lparam) -> Lresult;
    pub fn RegisterClassW(class: *const WndClassW) -> u16;
    pub fn CreateWindowExW(
        ex_style: u32,
        class_name: *const u16,
        window_name: *const u16,
        style: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        parent: Hwnd,
        menu: Handle,
        instance: Hmodule,
        param: *mut c_void,
    ) -> Hwnd;
    pub fn DestroyWindow(window: Hwnd) -> i32;
    pub fn PostMessageW(window: Hwnd, message: u32, wparam: Wparam, lparam: Lparam) -> i32;
    pub fn SetTimer(window: Hwnd, id: usize, timeout_ms: u32, callback: Handle) -> usize;
    pub fn KillTimer(window: Hwnd, id: usize) -> i32;
    pub fn PeekMessageW(message: *mut Msg, window: Hwnd, min: u32, max: u32, remove: u32) -> i32;
    pub fn TranslateMessage(message: *const Msg) -> i32;
    pub fn DispatchMessageW(message: *const Msg) -> Lresult;
    pub fn SendMessageTimeoutW(
        window: Hwnd,
        message: u32,
        wparam: Wparam,
        lparam: Lparam,
        flags: u32,
        timeout_ms: u32,
        result: *mut usize,
    ) -> usize;
    pub fn ClientToScreen(window: Hwnd, point: *mut Point) -> i32;
    pub fn LogicalToPhysicalPointForPerMonitorDPI(window: Hwnd, point: *mut Point) -> i32;
    pub fn GetWindowDpiAwarenessContext(window: Hwnd) -> Handle;
    pub fn GetThreadDpiAwarenessContext() -> Handle;
    pub fn AreDpiAwarenessContextsEqual(first: Handle, second: Handle) -> i32;
    pub fn GetAwarenessFromDpiAwarenessContext(context: Handle) -> i32;
}

#[link(name = "kernel32")]
extern "system" {
    pub fn GetCurrentProcessId() -> u32;
    pub fn GetCurrentThreadId() -> u32;
    pub fn GetLastError() -> u32;
    pub fn SetLastError(error: u32);
    pub fn GetCurrentProcess() -> Handle;
    pub fn OpenProcess(access: u32, inherit: i32, pid: u32) -> Handle;
    pub fn GetProcessTimes(
        process: Handle,
        created: *mut u64,
        exit: *mut u64,
        kernel: *mut u64,
        user: *mut u64,
    ) -> i32;
    pub fn GetTickCount64() -> u64;
    pub fn GetModuleHandleW(name: *const u16) -> Hmodule;
    pub fn GetModuleHandleExW(flags: u32, address: *const c_void, module: *mut Hmodule) -> i32;
    pub fn GetProcAddress(module: Hmodule, name: *const i8) -> *const c_void;
    pub fn GlobalGetAtomNameW(atom: u16, name: *mut u16, size: i32) -> u32;
    pub fn OpenFileMappingW(access: u32, inherit: i32, name: *const u16) -> Handle;
    pub fn MapViewOfFile(
        mapping: Handle,
        access: u32,
        high: u32,
        low: u32,
        size: usize,
    ) -> *mut c_void;
    pub fn UnmapViewOfFile(view: *const c_void) -> i32;
    pub fn CloseHandle(handle: Handle) -> i32;
    pub fn CoTaskMemFree(pointer: *mut c_void);
}

#[link(name = "imm32")]
extern "system" {
    pub fn ImmGetContext(window: Hwnd) -> Handle;
    pub fn ImmGetCandidateWindow(context: Handle, index: u32, form: *mut CandidateForm) -> i32;
    pub fn ImmGetDefaultIMEWnd(window: Hwnd) -> Hwnd;
    pub fn ImmReleaseContext(window: Hwnd, context: Handle) -> i32;
    pub fn ImmGetCompositionStringW(
        context: Handle,
        index: u32,
        data: *mut c_void,
        bytes: u32,
    ) -> i32;
    pub fn ImmGetOpenStatus(context: Handle) -> i32;
    pub fn ImmGetConversionStatus(context: Handle, conversion: *mut u32, sentence: *mut u32)
        -> i32;
}

pub fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn succeeded(hr: Hresult) -> bool {
    hr >= 0
}
