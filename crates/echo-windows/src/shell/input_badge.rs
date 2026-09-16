//! Owned passive badge chrome. No input activation or main-window hooks.
use super::window::owned;
use windows_sys::Win32::{
    Foundation::*,
    UI::{Shell::*, WindowsAndMessaging::*},
};
pub fn configure_input_badge(handle: isize, handler: super::EventHandler) -> Result<(), String> {
    let hwnd = owned(handle)?;
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(
            hwnd,
            GWL_EXSTYLE,
            (style & !(WS_EX_APPWINDOW as isize))
                | (WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED)
                    as isize,
        );
        let callback = Box::into_raw(Box::new(handler));
        if SetWindowSubclass(hwnd, Some(passive), 0x4543494d, callback as usize) == 0 {
            drop(Box::from_raw(callback));
            return Err("Input badge chrome unavailable".into());
        }
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
    Ok(())
}
unsafe extern "system" fn passive(
    hwnd: HWND,
    message: u32,
    w: WPARAM,
    l: LPARAM,
    id: usize,
    data: usize,
) -> LRESULT {
    match message {
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_NCHITTEST => HTTRANSPARENT as LRESULT,
        WM_NCDESTROY => {
            RemoveWindowSubclass(hwnd, Some(passive), id);
            drop(Box::from_raw(data as *mut super::EventHandler));
            DefSubclassProc(hwnd, message, w, l)
        }
        WM_THEMECHANGED | WM_SETTINGCHANGE => {
            let handler = &*(data as *const super::EventHandler);
            handler(super::ShellEvent::ThemeChanged);
            DefSubclassProc(hwnd, message, w, l)
        }
        _ => DefSubclassProc(hwnd, message, w, l),
    }
}
