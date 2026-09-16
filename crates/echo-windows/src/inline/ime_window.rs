//! Legacy candidate-window compatibility. No text is read and no OS input
//! setting is changed. Only recognized candidate windows next to the active
//! session's own input can suppress Echo's confirmation keys.
use windows_sys::Win32::{Foundation::*, UI::WindowsAndMessaging::*};
pub(crate) unsafe fn candidate(hwnd: HWND, input: HWND) -> bool {
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
pub(crate) unsafe fn visible(input: HWND) -> Option<HWND> {
    search(input, None)
}
/// A passive badge has no active input session. A persistent IME toolbar or
/// another editor's candidate window must not suppress the whole foreground app.
pub(crate) unsafe fn visible_near(
    input: HWND,
    caret: echo_engine::PhysicalRect,
    dpi: u32,
) -> Option<HWND> {
    search(input, Some((caret, dpi)))
}
fn near_caret(candidate: &RECT, caret: echo_engine::PhysicalRect, dpi: u32) -> bool {
    let margin = (64 * dpi.clamp(48, 768) / 96) as i32;
    candidate.right >= caret.x - margin
        && candidate.left <= caret.x + caret.width + margin
        && candidate.bottom >= caret.y - margin
        && candidate.top <= caret.y + caret.height + margin
}
unsafe fn search(input: HWND, anchor: Option<(echo_engine::PhysicalRect, u32)>) -> Option<HWND> {
    struct Search {
        input: HWND,
        found: HWND,
        anchor: Option<(echo_engine::PhysicalRect, u32)>,
    }
    unsafe extern "system" fn visit(hwnd: HWND, param: LPARAM) -> i32 {
        let s = &mut *(param as *mut Search);
        if IsWindowVisible(hwnd) != 0 && candidate(hwnd, s.input) {
            if let Some((caret, dpi)) = s.anchor {
                let mut bounds: RECT = std::mem::zeroed();
                if GetWindowRect(hwnd, &mut bounds) == 0 || !near_caret(&bounds, caret, dpi) {
                    return 1;
                }
            }
            s.found = hwnd;
            return 0;
        }
        1
    }
    let mut state = Search {
        input,
        found: std::ptr::null_mut(),
        anchor,
    };
    EnumWindows(Some(visit), (&mut state as *mut Search) as LPARAM);
    (!state.found.is_null()).then_some(state.found)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "visible owned IME-window fixture; explicit native acceptance"]
    fn distant_native_candidate_is_not_a_passive_badge_suppressor() {
        assert_eq!(std::env::var("ECHO_WINDOWS_ACCEPTANCE").as_deref(), Ok("1"));
        unsafe {
            let instance =
                windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null());
            let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
            let class = wide("OimeDirectUIWindow");
            let title = wide("Echo isolated candidate fixture");
            let mut wc: WNDCLASSW = std::mem::zeroed();
            wc.lpfnWndProc = Some(DefWindowProcW);
            wc.hInstance = instance;
            wc.lpszClassName = class.as_ptr();
            assert_ne!(RegisterClassW(&wc), 0);
            let input = CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                wide("STATIC").as_ptr(),
                title.as_ptr(),
                WS_POPUP | WS_VISIBLE,
                0,
                0,
                1200,
                800,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            );
            let popup = CreateWindowExW(
                WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
                class.as_ptr(),
                title.as_ptr(),
                WS_POPUP | WS_VISIBLE,
                900,
                600,
                160,
                60,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                instance,
                std::ptr::null(),
            );
            assert!(!input.is_null() && !popup.is_null());
            let broad = candidate(popup, input);
            let caret = echo_engine::PhysicalRect {
                x: 30,
                y: 30,
                width: 1,
                height: 20,
            };
            let nearby = visible_near(input, caret, 96);
            DestroyWindow(popup);
            DestroyWindow(input);
            UnregisterClassW(class.as_ptr(), instance);
            assert!(
                broad,
                "the old whole-window rule must reproduce the false positive"
            );
            assert!(
                nearby.is_none(),
                "a distant candidate must not suppress this caret"
            );
        }
    }
    #[test]
    fn persistent_ime_toolbar_does_not_hide_a_distant_caret() {
        let caret = echo_engine::PhysicalRect {
            x: 795,
            y: 1238,
            width: 1,
            height: 28,
        };
        let toolbar = RECT {
            left: 2402,
            top: 1395,
            right: 2558,
            bottom: 1440,
        };
        assert!(!near_caret(&toolbar, caret, 96));
        let other_editor = RECT {
            left: 796,
            top: 322,
            right: 1392,
            bottom: 411,
        };
        assert!(!near_caret(&other_editor, caret, 96));
        let candidate = RECT {
            left: 795,
            top: 1270,
            right: 1392,
            bottom: 1359,
        };
        assert!(near_caret(&candidate, caret, 96));
        let scaled_candidate = RECT {
            left: 795,
            top: 1370,
            right: 1392,
            bottom: 1440,
        };
        assert!(!near_caret(&scaled_candidate, caret, 96));
        assert!(near_caret(&scaled_candidate, caret, 192));
    }
}
