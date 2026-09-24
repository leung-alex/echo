//! Small Win32 transport wrappers used by the host-side CaretObserver.

use super::protocol::Mailbox;
use std::{ffi::c_void, mem::size_of, ptr::null_mut};
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND, INVALID_HANDLE_VALUE,
        LPARAM, LRESULT, WPARAM,
    },
    System::{
        DataExchange::{GlobalAddAtomW, GlobalDeleteAtom},
        Memory::*,
    },
    UI::WindowsAndMessaging::{HHOOK, *},
};

pub struct Mapping {
    pub handle: HANDLE,
    pub view: MEMORY_MAPPED_VIEW_ADDRESS,
}

impl Mapping {
    pub unsafe fn create(name: *const u16) -> Result<Self, String> {
        let handle = CreateFileMappingW(
            INVALID_HANDLE_VALUE,
            null_mut(),
            PAGE_READWRITE,
            0,
            size_of::<Mailbox>() as u32,
            name,
        );
        if handle.is_null() || GetLastError() == ERROR_ALREADY_EXISTS {
            if !handle.is_null() {
                CloseHandle(handle);
            }
            return Err("caret mailbox mapping unavailable".into());
        }
        let view = MapViewOfFile(
            handle,
            FILE_MAP_READ | FILE_MAP_WRITE,
            0,
            0,
            size_of::<Mailbox>(),
        );
        if view.Value.is_null() {
            CloseHandle(handle);
            return Err("caret mailbox mapping failed".into());
        }
        Ok(Self { handle, view })
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        unsafe {
            if !self.view.Value.is_null() {
                UnmapViewOfFile(self.view);
            }
            if !self.handle.is_null() {
                CloseHandle(self.handle);
            }
        }
    }
}

pub unsafe fn register_message(name: *const u16) -> u32 {
    RegisterWindowMessageW(name)
}

pub unsafe fn add_atom(name: *const u16) -> u16 {
    GlobalAddAtomW(name)
}

pub unsafe fn delete_atom(atom: u16) {
    if atom != 0 {
        GlobalDeleteAtom(atom);
    }
}

pub unsafe fn bootstrap(
    window: HWND,
    message: u32,
    atom: u16,
    timeout_ms: u32,
) -> Result<(), String> {
    let mut result = 0;
    let ok = SendMessageTimeoutW(
        window,
        message,
        atom as WPARAM,
        0,
        SMTO_ABORTIFHUNG | SMTO_BLOCK | SMTO_ERRORONEXIT,
        timeout_ms,
        &mut result,
    );
    if ok == 0 {
        return Err(format!("caret bootstrap timeout/error={}", GetLastError()));
    }
    Ok(())
}

pub unsafe fn post_scheduler(window: HWND, message: u32, command: usize) -> bool {
    PostMessageW(window, message, command, 0) != 0
}

pub unsafe fn install_hook(
    callback: unsafe extern "system" fn(i32, WPARAM, LPARAM) -> LRESULT,
    thread: u32,
    module: isize,
) -> Result<HHOOK, String> {
    let hook = SetWindowsHookExW(
        WH_CALLWNDPROC,
        Some(std::mem::transmute(callback)),
        module as _,
        thread,
    );
    if hook.is_null() {
        Err(format!("caret hook installation failed={}", GetLastError()))
    } else {
        Ok(hook)
    }
}

pub unsafe fn remove_hook(hook: HHOOK) {
    if !hook.is_null() {
        UnhookWindowsHookEx(hook);
    }
}

pub fn hwnd_as_u64(hwnd: HWND) -> u64 {
    hwnd as usize as u64
}

pub fn u64_as_hwnd(value: u64) -> HWND {
    value as usize as *mut c_void
}
