//! Target-thread message-only scheduler for the standalone observer DLL.
//!
//! Bootstrap is deliberately limited to validation, mailbox ownership and
//! scheduler creation. Provider/TSF work runs only from posted scheduler
//! messages, never from the synchronous hook/SendMessage stack.

use super::{caret_ffi as ffi, caret_protocol as protocol};
use protocol::{Mailbox, RESPONSE_CLOSED};
use std::cell::Cell;
use std::ffi::c_void;
use std::ptr::{null, null_mut};
use std::sync::OnceLock;

pub const MESSAGE: &str = "Echo.CaretObservation.v1";
pub const PREFIX: &str = "Local\\Echo.CaretObservation.";
pub const REQUEST_MESSAGE: u32 = ffi::WM_APP + 0x5a1;
pub const CLOSE_MESSAGE: u32 = ffi::WM_APP + 0x5a2;
const INIT_MESSAGE: u32 = ffi::WM_APP + 0x5a3;
const WATCHDOG_TIMER: usize = 1;
const WATCHDOG_MS: u32 = 250;
const HEARTBEAT_TIMEOUT_MS: u64 = 1000;
const GET_MODULE_HANDLE_EX_FLAG_PIN: u32 = 0x00000001;
const GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS: u32 = 0x00000004;
const GA_ROOT: u32 = 2;
const WS_OVERLAPPED: u32 = 0;

thread_local! {
    static SCHEDULER: Cell<*mut SchedulerState> = const { Cell::new(null_mut()) };
}

struct SchedulerState {
    mapping: ffi::Handle,
    view: *mut Mailbox,
    window: ffi::Hwnd,
    input_window: ffi::Hwnd,
    target_pid: u32,
    target_thread: u32,
    target_started: u64,
    closed: bool,
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
    _lparam: ffi::Lparam,
) -> ffi::Lresult {
    let state = ffi::GetWindowLongPtrW(window, ffi::GWLP_USERDATA) as *mut SchedulerState;
    if message == ffi::WM_NCCREATE {
        // The state is installed immediately after CreateWindowExW returns;
        // accepting NCCREATE through DefWindowProc keeps the callback inert.
        return ffi::DefWindowProcW(window, message, wparam, 0);
    }
    if state.is_null() {
        return ffi::DefWindowProcW(window, message, wparam, 0);
    }
    let state_ref = &mut *state;
    match message {
        INIT_MESSAGE => {
            let _ = ffi::SetTimer(window, WATCHDOG_TIMER, WATCHDOG_MS, null_mut());
            0
        }
        REQUEST_MESSAGE => {
            handle_request(state_ref);
            0
        }
        ffi::WM_TIMER if wparam == WATCHDOG_TIMER => {
            let now = ffi::GetTickCount64();
            let heartbeat = (*state_ref.view)
                .heartbeat_tick
                .load(std::sync::atomic::Ordering::Acquire);
            if (*state_ref.view)
                .closed
                .load(std::sync::atomic::Ordering::Acquire)
                != 0
                || heartbeat == 0
                || now.saturating_sub(heartbeat) > HEARTBEAT_TIMEOUT_MS
                || ffi::GetForegroundWindow() != state_ref.input_window
            {
                close_scheduler(state_ref);
            }
            0
        }
        CLOSE_MESSAGE | ffi::WM_CLOSE => {
            close_scheduler(state_ref);
            0
        }
        _ => ffi::DefWindowProcW(window, message, wparam, 0),
    }
}

unsafe fn ensure_class() -> bool {
    let class = ffi::WndClassW {
        style: 0,
        wnd_proc: Some(window_proc),
        cls_extra: 0,
        wnd_extra: 0,
        instance: ffi::GetModuleHandleW(null()),
        icon: null_mut(),
        cursor: null_mut(),
        background: null_mut(),
        menu_name: null(),
        class_name: class_name().as_ptr(),
    };
    ffi::RegisterClassW(&class) != 0 || ffi::GetLastError() == ffi::ERROR_CLASS_ALREADY_EXISTS
}

unsafe fn close_mapping(mapping: ffi::Handle, view: *mut Mailbox) {
    if !view.is_null() {
        ffi::UnmapViewOfFile(view.cast());
    }
    if !mapping.is_null() {
        ffi::CloseHandle(mapping);
    }
}

unsafe fn close_scheduler(state: &mut SchedulerState) {
    if state.closed {
        return;
    }
    state.closed = true;
    (*state.view)
        .closed
        .store(1, std::sync::atomic::Ordering::Release);
    let mut response = [0_u32; 64];
    response[0] = RESPONSE_CLOSED;
    let _ = (*state.view).response.publish(&response);
    let window = state.window;
    let mapping = state.mapping;
    let view = state.view;
    SCHEDULER.with(|slot| slot.set(null_mut()));
    let _ = ffi::KillTimer(window, WATCHDOG_TIMER);
    let _ = ffi::SetWindowLongPtrW(window, ffi::GWLP_USERDATA, 0);
    let _ = ffi::DestroyWindow(window);
    close_mapping(mapping, view);
    drop(Box::from_raw(state));
}

unsafe fn handle_request(state: &mut SchedulerState) {
    if state.closed
        || (*state.view)
            .closed
            .load(std::sync::atomic::Ordering::Acquire)
            != 0
    {
        return;
    }
    let Some(request) = (*state.view).request.snapshot() else {
        return;
    };
    if request[0] != protocol::REQUEST_PROBE {
        if request[0] == protocol::REQUEST_CLOSE {
            close_scheduler(state);
        }
        return;
    }
    let deadline = u64::from(request[2]) | (u64::from(request[3]) << 32);
    let now = ffi::GetTickCount64();
    let _ = (*state.view).response.publish(&{
        let mut pending = [0_u32; 64];
        pending[0] = protocol::RESPONSE_PENDING;
        pending[1] = request[1];
        pending[8] = deadline as u32;
        pending[9] = (deadline >> 32) as u32;
        pending[10] = 1;
        pending
    });
    let _ =
        super::tsf_geometry::request(state.view, &(*state.view).header, request[1], deadline, now);
}

unsafe fn pin_module() {
    let mut module = null_mut();
    let address = pin_module as *const c_void;
    let _ = ffi::GetModuleHandleExW(
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
        address,
        &mut module,
    );
}

unsafe fn live_identity(header: &protocol::Header, input: ffi::Hwnd) -> bool {
    let pid = ffi::GetCurrentProcessId();
    let thread = ffi::GetCurrentThreadId();
    let mut created = 0;
    let mut exit = 0;
    let mut kernel = 0;
    let mut user = 0;
    let root = ffi::GetAncestor(input, GA_ROOT);
    let mut root_pid = 0;
    pid == header.target_pid
        && thread == header.input_thread
        && input as usize as u64 == header.input_hwnd
        && ffi::GetFocus() == input
        && root as usize as u64 == header.root_hwnd
        && root == ffi::GetForegroundWindow()
        && ffi::GetWindowThreadProcessId(root, &mut root_pid) != 0
        && root_pid == header.root_pid
        && ffi::GetProcessTimes(
            ffi::GetCurrentProcess(),
            &mut created,
            &mut exit,
            &mut kernel,
            &mut user,
        ) != 0
        && created == header.target_started
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
    if !protocol::header_shape_valid(&(*view).header)
        || !live_identity(&(*view).header, message.window)
    {
        close_mapping(mapping, view);
        return true;
    }
    pin_module();
    let existing = SCHEDULER.with(|slot| slot.get());
    if !existing.is_null() {
        (*view).scheduler_hwnd.store(
            (*existing).window as usize as u64,
            std::sync::atomic::Ordering::Release,
        );
        close_mapping(mapping, view);
        return true;
    }
    if !ensure_class() {
        close_mapping(mapping, view);
        return true;
    }
    let mut state = Box::new(SchedulerState {
        mapping,
        view,
        window: null_mut(),
        input_window: message.window,
        target_pid: (*view).header.target_pid,
        target_thread: (*view).header.input_thread,
        target_started: (*view).header.target_started,
        closed: false,
    });
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
        close_mapping(mapping, view);
        return true;
    }
    state.window = window;
    let state_ptr = Box::into_raw(state);
    let _ = ffi::SetWindowLongPtrW(window, ffi::GWLP_USERDATA, state_ptr as isize);
    SCHEDULER.with(|slot| slot.set(state_ptr));
    (*view)
        .scheduler_hwnd
        .store(window as usize as u64, std::sync::atomic::Ordering::Release);
    let _ = ffi::PostMessageW(window, INIT_MESSAGE, 0, 0);
    true
}

pub unsafe fn dispatch_hook(message: &ffi::CallWindow) -> bool {
    bootstrap(message)
}
