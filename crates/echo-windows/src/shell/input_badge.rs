//! Owned passive badge chrome. No input activation or main-window hooks.
use super::window::owned;
use windows_sys::Win32::{
    Foundation::*,
    UI::{Shell::*, WindowsAndMessaging::*},
};
fn passive_style(style: u32) -> u32 {
    (style & !WS_EX_APPWINDOW)
        | WS_EX_TOOLWINDOW
        | WS_EX_NOACTIVATE
        | WS_EX_TRANSPARENT
        | WS_EX_LAYERED
}
pub fn configure_input_badge(handle: isize, handler: super::EventHandler) -> Result<(), String> {
    let hwnd = owned(handle)?;
    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, passive_style(style as u32) as isize);
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
        WM_STYLECHANGING if w as i32 == GWL_EXSTYLE && l != 0 => {
            // Winit reapplies its cached flags on show/position/theme changes.
            // Preserve the passive contract across every such update.
            let styles = &mut *(l as *mut STYLESTRUCT);
            styles.styleNew = passive_style(styles.styleNew);
            DefSubclassProc(hwnd, message, w, l)
        }

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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn framework_style_rewrites_preserve_passive_badge_contract() {
        let required = WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_TRANSPARENT | WS_EX_LAYERED;
        for incoming in [0, WS_EX_APPWINDOW, WS_EX_APPWINDOW | WS_EX_TOPMOST] {
            let style = passive_style(incoming);
            assert_eq!(style & required, required);
            assert_eq!(style & WS_EX_APPWINDOW, 0);
            assert_eq!(style & WS_EX_TOPMOST, incoming & WS_EX_TOPMOST);
            assert_eq!(passive_style(style), style);
        }
    }
}
