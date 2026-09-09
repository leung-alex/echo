//! A one-shot, physical-pixel focus snapshot, captured before Echo can take focus.
use crate::windows_impl as native;
use echo_engine::{InputTargetGeometry, PasteControlIdentity, PasteTarget, PhysicalRect};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HWND as WinHwnd;
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    UI::{HiDpi::*, WindowsAndMessaging::*},
};
mod anchor;
pub(crate) mod automation;
mod placement;
pub(crate) use anchor::{anchor_for_element, resolve_anchor};
pub use placement::{
    expand_popup_stage, place_card, place_inline, place_inline_stage, PopupPlacement,
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnchorSource {
    NativeCaret,
    AutomationCaret,
    AdjacentCharacter,
    AccessibleCaret,
    InputControl,
    Window,
}
impl AnchorSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::NativeCaret => "native-caret",
            Self::AutomationCaret => "uia-caret",
            Self::AdjacentCharacter => "adjacent-character",
            Self::AccessibleCaret => "msaa-caret",
            Self::InputControl => "input-control",
            Self::Window => "window-fallback",
        }
    }
}
#[derive(Debug, Clone, Copy)]
pub struct PopupAnchor {
    pub geometry: InputTargetGeometry,
    pub source: AnchorSource,
}
#[derive(Debug, Clone)]
pub struct FocusSnapshot {
    pub window_id: isize,
    pub process_id: u32,
    pub process_started_at: u64,
    pub anchor: PopupAnchor,
    pub(crate) focused_handle: isize,
    native_input: Option<PasteControlIdentity>,
    native_blocked: bool,
    captured_at: Instant,
}
pub struct CapturedActivation {
    pub target: Option<PasteTarget>,
    pub anchor: PopupAnchor,
}
fn win(hwnd: HWND) -> WinHwnd {
    WinHwnd(hwnd)
}
fn rect(r: RECT) -> PhysicalRect {
    PhysicalRect {
        x: r.left,
        y: r.top,
        width: r.right - r.left,
        height: r.bottom - r.top,
    }
}
pub(crate) fn valid_rect(r: PhysicalRect) -> bool {
    r.width >= 0
        && r.height > 0
        && r.width <= 100_000
        && r.height <= 100_000
        && i64::from(r.x).abs() < 16_000_000
        && i64::from(r.y).abs() < 16_000_000
}
pub(crate) fn geometry(target: PhysicalRect) -> InputTargetGeometry {
    unsafe {
        let r = RECT {
            left: target.x,
            top: target.y,
            right: target.x.saturating_add(target.width.max(1)),
            bottom: target.y.saturating_add(target.height.max(1)),
        };
        let monitor = MonitorFromRect(&r, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        let work_area = if GetMonitorInfoW(monitor, &mut info) != 0 {
            rect(info.rcWork)
        } else {
            PhysicalRect {
                x: 0,
                y: 0,
                width: GetSystemMetrics(SM_CXSCREEN).max(1),
                height: GetSystemMetrics(SM_CYSCREEN).max(1),
            }
        };
        let (mut x, mut y) = (96, 96);
        if GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) < 0 {
            x = 96;
        }
        InputTargetGeometry {
            target,
            work_area,
            dpi: x.clamp(48, 768),
        }
    }
}
struct DpiGuard(DPI_AWARENESS_CONTEXT);
impl Drop for DpiGuard {
    fn drop(&mut self) {
        unsafe {
            SetThreadDpiAwarenessContext(self.0);
        }
    }
}
fn caret_screen(info: &GUITHREADINFO, root: HWND, pid: u32) -> Option<PhysicalRect> {
    if info.hwndCaret.is_null()
        || !native::native_control_belongs_to(win(info.hwndCaret), win(root), pid)
    {
        return None;
    }
    unsafe {
        // rcCaret has the target window's logical coordinate semantics, not ours.
        let previous = SetThreadDpiAwarenessContext(GetWindowDpiAwarenessContext(info.hwndCaret));
        let _guard = DpiGuard(previous);
        let mut a = POINT {
            x: info.rcCaret.left,
            y: info.rcCaret.top,
        };
        let mut b = POINT {
            x: info.rcCaret.right,
            y: info.rcCaret.bottom,
        };
        if ClientToScreen(info.hwndCaret, &mut a) == 0
            || ClientToScreen(info.hwndCaret, &mut b) == 0
        {
            return None;
        }
        if LogicalToPhysicalPointForPerMonitorDPI(info.hwndCaret, &mut a) == 0
            || LogicalToPhysicalPointForPerMonitorDPI(info.hwndCaret, &mut b) == 0
        {
            return None;
        }
        let r = PhysicalRect {
            x: a.x,
            y: a.y,
            width: (b.x - a.x).max(1),
            height: b.y - a.y,
        };
        valid_rect(r).then_some(r)
    }
}
impl FocusSnapshot {
    /// No COM, UI Automation, message sends, or clipboard reads on the hotkey thread.
    pub fn capture() -> Self {
        unsafe {
            let hwnd = GetForegroundWindow();
            let mut pid = 0;
            let tid = GetWindowThreadProcessId(hwnd, &mut pid);
            let mut info: GUITHREADINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
            if tid != 0 {
                GetGUIThreadInfo(tid, &mut info);
            }
            let native_input = native::focused_native_control_from_gui(win(hwnd), pid);
            let native_blocked = !info.hwndFocus.is_null()
                && native::window_class_name(win(info.hwndFocus))
                    .is_some_and(|c| native::is_native_input_class(&c))
                && native_input.is_none();
            let focused_rect = (native_input.is_some() && !info.hwndFocus.is_null())
                .then(|| native::window_rect(win(info.hwndFocus)))
                .flatten()
                .filter(|r| valid_rect(*r));
            let fallback = native::window_rect(win(hwnd))
                .filter(|r| valid_rect(*r))
                .map(|r| PhysicalRect {
                    x: r.x + r.width / 2,
                    y: r.y + r.height / 2,
                    width: 1,
                    height: 1,
                })
                .unwrap_or(PhysicalRect {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                });
            let (target, source) = if let Some(r) = caret_screen(&info, hwnd, pid) {
                (r, AnchorSource::NativeCaret)
            } else if let Some(r) = focused_rect {
                (r, AnchorSource::InputControl)
            } else {
                (fallback, AnchorSource::Window)
            };
            Self {
                window_id: hwnd as isize,
                process_id: pid,
                process_started_at: native::process_started_at(pid).unwrap_or(0),
                anchor: PopupAnchor {
                    geometry: geometry(target),
                    source,
                },
                focused_handle: info.hwndFocus as isize,
                native_input,
                native_blocked,
                captured_at: Instant::now(),
            }
        }
    }
    pub fn is_echo(&self) -> bool {
        self.process_id == std::process::id()
    }
    pub fn still_current(&self) -> bool {
        self.current()
    }
    pub(crate) fn current(&self) -> bool {
        if self.window_id == 0
            || self.process_id == 0
            || self.is_echo()
            || self.process_started_at == 0
            || self.captured_at.elapsed() > Duration::from_secs(1)
        {
            return false;
        }
        unsafe {
            let hwnd = self.window_id as HWND;
            let mut pid = 0;
            let tid = GetWindowThreadProcessId(hwnd, &mut pid);
            if GetForegroundWindow() != hwnd
                || IsWindow(hwnd) == 0
                || pid != self.process_id
                || native::process_started_at(pid) != Some(self.process_started_at)
            {
                return false;
            }
            let mut info: GUITHREADINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
            GetGUIThreadInfo(tid, &mut info) != 0 && info.hwndFocus as isize == self.focused_handle
        }
    }
    pub fn capture_target(&self) -> CapturedActivation {
        let mut result = CapturedActivation {
            target: None,
            anchor: self.anchor,
        };
        if !self.current() || self.native_blocked {
            return result;
        }
        let identity = if let Some(identity) = self.native_input.clone() {
            Some(identity)
        } else {
            // UIA may verify identity even when Win32 already supplied the best anchor.
            // Never replace an exact native caret with a whole input-control rectangle.
            automation::query(
                self.clone(),
                self.anchor.source != AnchorSource::NativeCaret,
            )
            .map(|probe| {
                if let Some((r, source)) = probe.anchor {
                    result.anchor = PopupAnchor {
                        geometry: geometry(r),
                        source,
                    };
                }
                probe.identity
            })
        };
        if !self.current() {
            return result;
        }
        if let Some(identity) = identity {
            result.target = Some(PasteTarget {
                window_id: self.window_id,
                window_class: native::window_class_name(WinHwnd(self.window_id as _))
                    .unwrap_or_default(),
                process_id: self.process_id,
                process_started_at: self.process_started_at,
                focused_control: Some(identity),
                app_name: None,
                selected_text: None,
                is_single_line: None,
                geometry: result.anchor.geometry,
            });
        }
        result
    }
}
/// Bounded MTA validation, used only when the captured identity is a UIA element.
pub fn warm_accessibility() {
    automation::warm();
}

pub(crate) fn focused_identity(window: WinHwnd, pid: u32) -> Option<PasteControlIdentity> {
    let snapshot = FocusSnapshot::capture();
    if snapshot.window_id != window.0 as isize || snapshot.process_id != pid {
        return None;
    }
    automation::query(snapshot, false).map(|probe| probe.identity)
}
/// Escape can restore the original window, but never steals focus from another app.
pub fn restore_after_dismiss(snapshot: &FocusSnapshot) {
    if snapshot.is_echo() || snapshot.window_id == 0 || snapshot.process_started_at == 0 {
        return;
    }
    unsafe {
        let mut fg_pid = 0;
        let fg = GetForegroundWindow();
        GetWindowThreadProcessId(fg, &mut fg_pid);
        if !fg.is_null() && fg_pid != std::process::id() && fg as isize != snapshot.window_id {
            return;
        }
        let hwnd = snapshot.window_id as HWND;
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == snapshot.process_id
            && native::process_started_at(pid) == Some(snapshot.process_started_at)
            && IsWindow(hwnd) != 0
        {
            SetForegroundWindow(hwnd);
        }
    }
}
