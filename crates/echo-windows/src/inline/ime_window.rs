//! Legacy candidate-window compatibility. No text is read and no OS input
//! setting is changed. Only recognized candidate windows next to the active
//! session's own input can suppress Echo's confirmation keys.
use windows_sys::Win32::{Foundation::*, UI::WindowsAndMessaging::*};
pub(super) unsafe fn candidate(hwnd: HWND, input: HWND) -> bool {
    if hwnd.is_null() || input.is_null() || hwnd == input {
        return false;
    }
    let mut class = [0u16; 128];
    let n = GetClassNameW(hwnd, class.as_mut_ptr(), 128);
    if n <= 0 {
        return false;
    }
    let class = String::from_utf16_lossy(&class[..n as usize]);
    if !matches!(
        class.as_str(),
        "OimeDirectUIWindow"
            | "MSCTFIME UI"
            | "IME_Candidate"
            | "Microsoft.IME.UIManager.CandidateWindow.Host"
    ) {
        return false;
    }
    let mut a: RECT = std::mem::zeroed();
    let mut b: RECT = std::mem::zeroed();
    if GetWindowRect(hwnd, &mut a) == 0 || GetWindowRect(input, &mut b) == 0 {
        return false;
    }
    a.right > a.left
        && a.bottom > a.top
        && a.right - a.left < 2048
        && a.bottom - a.top < 1024
        && a.right >= b.left - 64
        && a.left <= b.right + 64
        && a.bottom >= b.top - 128
        && a.top <= b.bottom + 128
}
pub(super) unsafe fn visible(input: HWND) -> Option<HWND> {
    struct Search {
        input: HWND,
        found: HWND,
    }
    unsafe extern "system" fn visit(hwnd: HWND, param: LPARAM) -> i32 {
        let s = &mut *(param as *mut Search);
        if IsWindowVisible(hwnd) != 0 && candidate(hwnd, s.input) {
            s.found = hwnd;
            return 0;
        }
        1
    }
    let mut state = Search {
        input,
        found: std::ptr::null_mut(),
    };
    EnumWindows(Some(visit), (&mut state as *mut Search) as LPARAM);
    (!state.found.is_null()).then_some(state.found)
}
