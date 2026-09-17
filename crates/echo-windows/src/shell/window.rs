use super::{
    common::{error, wide},
    EventHandler, ShellEvent,
};
use std::{
    ptr::null_mut,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::*, Gdi::*},
    System::{Registry::*, Threading::*},
    UI::{
        Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW},
        Controls::MARGINS,
        HiDpi::GetDpiForWindow,
        Input::KeyboardAndMouse::ReleaseCapture,
        Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass},
        WindowsAndMessaging::*,
    },
};
pub(super) fn owned(handle: isize) -> Result<HWND, String> {
    unsafe {
        let hwnd = handle as HWND;
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if hwnd.is_null() || IsWindow(hwnd) == 0 || pid != GetCurrentProcessId() {
            Err("window is unavailable or belongs to another process".into())
        } else {
            Ok(hwnd)
        }
    }
}
fn registry_number(name: &str) -> Option<u32> {
    unsafe {
        let mut value = 0_u32;
        let mut bytes = 4_u32;
        let path = wide("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize");
        let name = wide(name);
        (RegGetValueW(
            HKEY_CURRENT_USER,
            path.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            null_mut(),
            (&mut value as *mut u32).cast(),
            &mut bytes,
        ) == ERROR_SUCCESS)
            .then_some(value)
    }
}
pub fn apply_theme(handle: isize, dark: bool, request_mica: bool) -> bool {
    let Ok(hwnd) = owned(handle) else {
        return false;
    };
    unsafe {
        let dark = i32::from(dark);
        DwmSetWindowAttribute(hwnd, 20, (&dark as *const i32).cast(), 4);
        let corner = 2_u32;
        DwmSetWindowAttribute(hwnd, 33, (&corner as *const u32).cast(), 4);
        let mut hc: HIGHCONTRASTW = std::mem::zeroed();
        hc.cbSize = std::mem::size_of::<HIGHCONTRASTW>() as u32;
        let high_contrast = SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            hc.cbSize,
            (&mut hc as *mut HIGHCONTRASTW).cast(),
            0,
        ) != 0
            && hc.dwFlags & HCF_HIGHCONTRASTON != 0;
        let enabled =
            request_mica && !high_contrast && registry_number("EnableTransparency") != Some(0);
        let backdrop = if enabled { 2_u32 } else { 1_u32 };
        let result = DwmSetWindowAttribute(hwnd, 38, (&backdrop as *const u32).cast(), 4);
        if enabled && result >= 0 {
            let margins = MARGINS {
                cxLeftWidth: -1,
                cxRightWidth: -1,
                cyTopHeight: -1,
                cyBottomHeight: -1,
            };
            return DwmExtendFrameIntoClientArea(hwnd, &margins) >= 0;
        }
        false
    }
}
pub fn set_owner(child: isize, owner: isize) -> Result<(), String> {
    let child = owned(child)?;
    let owner = owned(owner)?;
    unsafe {
        SetLastError(0);
        let result = SetWindowLongPtrW(child, GWLP_HWNDPARENT, owner as isize);
        if result == 0 && GetLastError() != 0 {
            return Err(error());
        }
        let style = GetWindowLongPtrW(child, GWL_EXSTYLE);
        SetWindowLongPtrW(
            child,
            GWL_EXSTYLE,
            (style | WS_EX_TOOLWINDOW as isize) & !(WS_EX_APPWINDOW as isize),
        );
        SetWindowPos(
            child,
            null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
    }
    Ok(())
}
pub fn focus_window(handle: isize) -> Result<(), String> {
    let hwnd = owned(handle)?;
    unsafe {
        if SetForegroundWindow(hwnd) == 0 {
            return Err("Windows did not grant foreground focus".into());
        }
    }
    Ok(())
}
pub fn start_drag(handle: isize) -> Result<(), String> {
    let hwnd = owned(handle)?;
    unsafe {
        ReleaseCapture();
        SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as usize, 0);
    }
    Ok(())
}
fn clamp_position(desired: (i32, i32), size: (i32, i32), work: RECT) -> (i32, i32) {
    let max_x = (work.right - size.0).max(work.left);
    let max_y = (work.bottom - size.1).max(work.top);
    (
        desired.0.clamp(work.left, max_x),
        desired.1.clamp(work.top, max_y),
    )
}
// Prefer the left attachment, but never overlap the main surface when the
// other side has space. All arguments are physical screen coordinates.
fn adjacent_position(
    main: RECT,
    size: (i32, i32),
    work: RECT,
    gap: i32,
    offset: i32,
) -> (i32, i32) {
    let left = main.left - size.0 - gap;
    let right = main.right + gap;
    let x = if left >= work.left {
        left
    } else if right + size.0 <= work.right {
        right
    } else {
        left
    };
    clamp_position((x, main.top + offset), size, work)
}
fn pair_main_position(main: (i32, i32), favorite: (i32, i32), work: RECT, gap: i32) -> (i32, i32) {
    let total = main.0 + favorite.0 + gap;
    let available = work.right - work.left;
    let x = if total <= available {
        work.left + (available - total) / 2 + favorite.0 + gap
    } else {
        work.left + (available - main.0) / 2
    };
    clamp_position(
        (x, work.top + (work.bottom - work.top - main.1) / 2),
        main,
        work,
    )
}
pub fn reposition_favorites(main: isize, favorites: isize) -> Result<(), String> {
    let main = owned(main)?;
    let favorites = owned(favorites)?;
    unsafe {
        let mut a: RECT = std::mem::zeroed();
        let mut b: RECT = std::mem::zeroed();
        if GetWindowRect(main, &mut a) == 0 || GetWindowRect(favorites, &mut b) == 0 {
            return Err(error());
        }
        let monitor = MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return Err(error());
        }
        let scale = f64::from(GetDpiForWindow(main).max(96)) / 96.0;
        let (x, y) = adjacent_position(
            a,
            (b.right - b.left, b.bottom - b.top),
            info.rcWork,
            (8.0 * scale).round() as i32,
            (93.0 * scale).round() as i32,
        );
        if x != b.left || y != b.top {
            if SetWindowPos(
                favorites,
                null_mut(),
                x,
                y,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            ) == 0
            {
                return Err(error());
            }
        }
    }
    Ok(())
}
pub fn center_composition(handle: isize, favorite: isize) -> Result<(), String> {
    let hwnd = owned(handle)?;
    let favorite = owned(favorite)?;
    unsafe {
        let mut r: RECT = std::mem::zeroed();
        GetWindowRect(hwnd, &mut r);
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return Err(error());
        }
        let mut f: RECT = std::mem::zeroed();
        if GetWindowRect(favorite, &mut f) == 0 {
            return Err(error());
        }
        let gap = (8.0 * f64::from(GetDpiForWindow(hwnd).max(96)) / 96.0).round() as i32;
        let (x, y) = pair_main_position(
            (r.right - r.left, r.bottom - r.top),
            (f.right - f.left, f.bottom - f.top),
            info.rcWork,
            gap,
        );
        SetWindowPos(
            hwnd,
            null_mut(),
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
    Ok(())
}
fn caption_hit(bounds: Option<[i32; 4]>, point: [i32; 2]) -> bool {
    bounds
        .is_some_and(|[l, t, r, b]| point[0] >= l && point[0] < r && point[1] >= t && point[1] < b)
}

struct HookData {
    ime_mode: super::ime_mode::ImeMode,
    handler: EventHandler,
    main: bool,
    composing: Arc<AtomicBool>,
    resize_bounds: std::cell::Cell<Option<[i32; 4]>>,
    caption_bounds: std::cell::Cell<Option<[i32; 4]>>,
    inline: std::cell::Cell<bool>,
    prior_popup_style: std::cell::Cell<Option<isize>>,
}
pub struct WindowHook {
    hwnd: isize,
    data: Box<HookData>,
    _main_thread: Rc<()>,
}
impl WindowHook {
    /// Inline suggestions are interactive with the pointer, but never own the
    /// external text input's activation/IME. Normal manager behavior is restored.
    pub fn set_inline_popup(&self, enabled: bool) -> Result<(), String> {
        let changed = self.data.inline.get() != enabled;
        if !changed && !enabled {
            return Ok(());
        }
        let hwnd = owned(self.hwnd)?;
        unsafe {
            let current = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let mask = (WS_EX_NOACTIVATE | WS_EX_TOPMOST) as isize;
            let previous_popup_style = self.data.prior_popup_style.get();
            let desired = if enabled {
                if changed {
                    self.data.prior_popup_style.set(Some(current & mask));
                }
                current | WS_EX_NOACTIVATE as isize
            } else {
                (current & !mask) | self.data.prior_popup_style.take().unwrap_or(0)
            };
            let previous_inline = self.data.inline.replace(enabled);
            SetLastError(0);
            if SetWindowLongPtrW(hwnd, GWL_EXSTYLE, desired) == 0 && GetLastError() != 0 {
                self.data.inline.set(previous_inline);
                self.data.prior_popup_style.set(previous_popup_style);
                return Err(error());
            }
            let top = enabled || desired & WS_EX_TOPMOST as isize != 0;
            if SetWindowPos(
                hwnd,
                if top { HWND_TOPMOST } else { HWND_NOTOPMOST },
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
            ) == 0
            {
                let failure = error();
                self.data.inline.set(previous_inline);
                self.data.prior_popup_style.set(previous_popup_style);
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, current);
                SetWindowPos(
                    hwnd,
                    if current & WS_EX_TOPMOST as isize != 0 {
                        HWND_TOPMOST
                    } else {
                        HWND_NOTOPMOST
                    },
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                );
                return Err(failure);
            }
        }
        Ok(())
    }

    pub fn set_resize_bounds(&self, bounds: Option<[i32; 4]>) {
        self.data.resize_bounds.set(bounds);
    }
    /// Optional native title drag region in physical client coordinates.
    /// Resize edges take precedence; `None` leaves every control as client input.
    pub fn set_caption_bounds(&self, bounds: Option<[i32; 4]>) {
        self.data.caption_bounds.set(bounds);
    }
    /// Recheck at event delivery; ignore obsolete deactivation notifications.
    pub fn foreground_is_external(&self) -> bool {
        unsafe {
            let foreground = GetForegroundWindow();
            let mut pid = 0;
            GetWindowThreadProcessId(foreground, &mut pid);
            !foreground.is_null() && pid != 0 && pid != GetCurrentProcessId()
        }
    }
    pub fn is_composing(&self) -> bool {
        self.data.composing.load(Ordering::Acquire)
    }
}
impl Drop for WindowHook {
    fn drop(&mut self) {
        unsafe {
            if IsWindow(self.hwnd as HWND) != 0 {
                RemoveWindowSubclass(self.hwnd as HWND, Some(subclass), 1);
            }
        }
    }
}
pub fn attach_window(
    handle: isize,
    is_main: bool,
    handler: EventHandler,
) -> Result<WindowHook, String> {
    let hwnd = owned(handle)?;
    unsafe {
        if GetWindowThreadProcessId(hwnd, null_mut()) != GetCurrentThreadId() {
            return Err("window hook must be installed on its UI thread".into());
        }
        // Custom chrome supplies its own controls; suppress phantom DWM caption buttons.
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        SetWindowLongPtrW(
            hwnd,
            GWL_STYLE,
            style & !((WS_SYSMENU | WS_MINIMIZEBOX | WS_MAXIMIZEBOX) as isize),
        );
        SetWindowPos(
            hwnd,
            null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
        );
        let mut data = Box::new(HookData {
            handler,
            main: is_main,
            composing: Arc::new(AtomicBool::new(false)),
            resize_bounds: std::cell::Cell::new(None),
            caption_bounds: std::cell::Cell::new(None),
            inline: std::cell::Cell::new(false),
            prior_popup_style: std::cell::Cell::new(None),
            ime_mode: Default::default(),
        });
        if SetWindowSubclass(
            hwnd,
            Some(subclass),
            1,
            (&mut *data as *mut HookData) as usize,
        ) == 0
        {
            return Err(error());
        }
        Ok(WindowHook {
            hwnd: handle,
            data,
            _main_thread: Rc::new(()),
        })
    }
}
unsafe extern "system" fn subclass(
    hwnd: HWND,
    msg: u32,
    w: WPARAM,
    l: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    let state = &*(data as *const HookData);
    match msg {
        WM_INPUTLANGCHANGE => state.ime_mode.clear(),
        WM_IME_NOTIFY if !state.inline.get() => {
            if w == windows_sys::Win32::UI::Input::Ime::IMN_SETOPENSTATUS as usize
                || w == windows_sys::Win32::UI::Input::Ime::IMN_SETCONVERSIONMODE as usize
            {
                state.ime_mode.remember(hwnd);
            }
        }
        WM_IME_SETCONTEXT if !state.inline.get() => {
            if state.composing.load(Ordering::Acquire) {
                return DefSubclassProc(hwnd, msg, w, l);
            }
            return state
                .ime_mode
                .context_changed(hwnd, w != 0, || DefSubclassProc(hwnd, msg, w, l));
        }
        // Winit reapplies its cached extended style on visibility/flag changes.
        // Preserve the native inline contract across those framework updates.
        WM_STYLECHANGING if state.inline.get() && w as i32 == GWL_EXSTYLE && l != 0 => {
            (*(l as *mut STYLESTRUCT)).styleNew |= WS_EX_NOACTIVATE;
        }
        WM_WINDOWPOSCHANGING if state.inline.get() && l != 0 => {
            // Framework visibility/flag updates must not demote an active inline
            // popup behind the editor that deliberately retains foreground.
            let position = &mut *(l as *mut WINDOWPOS);
            position.hwndInsertAfter = HWND_TOPMOST;
            position.flags = (position.flags & !SWP_NOZORDER) | SWP_NOACTIVATE;
        }
        WM_MOUSEACTIVATE if state.inline.get() => return MA_NOACTIVATE as LRESULT,
        WM_NCHITTEST if state.inline.get() => return HTCLIENT as LRESULT,
        WM_SYSKEYDOWN if w == 0x73 => {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
            return 0;
        }
        WM_ACTIVATEAPP if w == 0 && state.main => {
            (state.handler)(ShellEvent::FocusLost);
        }
        WM_IME_STARTCOMPOSITION => {
            state.composing.store(true, Ordering::Release);
        }
        WM_IME_ENDCOMPOSITION => {
            state.composing.store(false, Ordering::Release);
        }
        WM_SETTINGCHANGE | WM_THEMECHANGED | WM_POWERBROADCAST => {
            (state.handler)(ShellEvent::ThemeChanged);
        }
        WM_WINDOWPOSCHANGED | WM_DPICHANGED if state.main => {
            (state.handler)(ShellEvent::GeometryChanged);
        }
        WM_NCHITTEST => {
            let mut r: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut r);
            let x = (l as u32 & 0xffff) as i16 as i32;
            let y = ((l as u32 >> 16) & 0xffff) as i16 as i32;
            let edge = (6.0 * f64::from(GetDpiForWindow(hwnd).max(96)) / 96.0).round() as i32;
            let bounds = state
                .resize_bounds
                .get()
                .map(|b| [r.left + b[0], r.top + b[1], r.left + b[2], r.top + b[3]])
                .unwrap_or([r.left, r.top, r.right, r.bottom]);
            let code = super::card_window::resize_hit(bounds, [x, y], edge);
            if code != 0 {
                return code as LRESULT;
            }
            if caption_hit(state.caption_bounds.get(), [x - r.left, y - r.top]) {
                return HTCAPTION as LRESULT;
            }
        }
        _ => {}
    }
    DefSubclassProc(hwnd, msg, w, l)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn caption_region_excludes_controls_and_can_be_disabled_for_modals() {
        // Settings reserves the trailing 60px for the custom close button.
        // That control must stay client hit-testable instead of becoming HTCAPTION.
        let bounds = Some([280, 24, 1260, 82]);
        assert!(caption_hit(bounds, [320, 56]));
        assert!(caption_hit(bounds, [900, 58]));
        assert!(!caption_hit(bounds, [1260, 56]));
        assert!(!caption_hit(bounds, [1290, 56]));
        assert!(!caption_hit(bounds, [400, 120]));
        assert!(!caption_hit(bounds, [279, 56]));
        assert!(!caption_hit(bounds, [1320, 56]));
        assert!(!caption_hit(None, [320, 56]));
    }
    #[test]
    fn geometry_handles_negative_monitor_coordinates() {
        let r = RECT {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1040,
        };
        assert_eq!(clamp_position((-2100, 900), (310, 575), r), (-1920, 465));
    }
    #[test]
    fn oversize_window_never_panics() {
        let r = RECT {
            left: 0,
            top: 0,
            right: 100,
            bottom: 100,
        };
        assert_eq!(clamp_position((9, 9), (310, 575), r), (0, 0));
    }
    #[test]
    fn centered_pair_fits_without_overlap() {
        let w = RECT {
            left: 0,
            top: 0,
            right: 1366,
            bottom: 1040,
        };
        let (x, y) = pair_main_position((824, 814), (310, 575), w, 8);
        let a = RECT {
            left: x,
            top: y,
            right: x + 824,
            bottom: y + 814,
        };
        let (fx, _) = adjacent_position(a, (310, 575), w, 8, 93);
        assert!(fx >= 0 && fx + 310 + 8 <= x && x + 824 <= w.right);
    }
    #[test]
    fn attachment_switches_right_near_left_monitor_edge() {
        let w = RECT {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1040,
        };
        let a = RECT {
            left: -1900,
            top: 100,
            right: -1076,
            bottom: 914,
        };
        let (x, y) = adjacent_position(a, (310, 575), w, 8, 93);
        assert_eq!((x, y), (-1068, 193));
    }
    #[test]
    fn invalid_window_is_rejected() {
        assert!(focus_window(0).is_err());
        assert!(!apply_theme(0, false, true));
    }
}
