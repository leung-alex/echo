//! Geometry is not paste authorization. Never substitute the whole host window
//! for a virtual input element. All returned rectangles are physical pixels.
use super::*;
use windows::core::BOOL;
use windows::Win32::{
    Foundation::{HWND as WinHwnd, LPARAM},
    UI::{
        Accessibility::{
            IUIAutomation, IUIAutomationElement, UIA_ComboBoxControlTypeId,
            UIA_DocumentControlTypeId, UIA_EditControlTypeId, UIA_GroupControlTypeId,
        },
        WindowsAndMessaging::{EnumChildWindows, GetAncestor, GA_ROOT},
    },
};

fn intersect(a: PhysicalRect, b: PhysicalRect) -> Option<PhysicalRect> {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (a.x + a.width).min(b.x + b.width);
    let bottom = (a.y + a.height).min(b.y + b.height);
    (right > x && bottom > y).then_some(PhysicalRect {
        x,
        y,
        width: right - x,
        height: bottom - y,
    })
}

/// Estimate the insertion edge of an editor when the provider exposes a
/// focused writable value but no native/UIA/MSAA caret range. The estimate is
/// deliberately a one-pixel caret-sized rectangle; a whole editor rectangle
/// is never published as geometry. Exact provider rectangles still win over
/// this source in the normal arbitration order.
pub(crate) fn estimate_editor_rect(control: PhysicalRect, value: Option<&str>) -> PhysicalRect {
    estimate_editor_rect_with_metrics(control, value, false)
}

/// WeChat's WebView editor reports a tall composer rectangle even though its
/// text is rendered with the compact chat font.  Treating that whole height
/// as a glyph advance makes the horizontal error grow once more text is typed
/// (for example, 16 Chinese characters were previously measured at roughly
/// twice their rendered width).  Keep the provider-specific correction here,
/// after exact UIA/MSAA/native caret sources have had a chance to win.
fn estimate_editor_rect_with_metrics(
    control: PhysicalRect,
    value: Option<&str>,
    compact_chat_font: bool,
) -> PhysicalRect {
    let height = control.height.max(1);
    let line_height = (height * 7 / 16).clamp(12, 32).min(height);
    let inset = (height / 6).clamp(4, 16).min(control.width.max(1));
    let (line, line_count) = value
        .map(|text| {
            let mut lines = text.split(['\n', '\r']).collect::<Vec<_>>();
            if lines.is_empty() {
                lines.push("");
            }
            let last = lines.last().copied().unwrap_or("");
            (last, lines.len().max(1))
        })
        .unwrap_or(("", 1));
    let advance = estimate_text_advance(line, line_height, compact_chat_font);
    let max_x = control
        .x
        .saturating_add(control.width.max(1).saturating_sub(inset.max(1) + 1));
    let x = control
        .x
        .saturating_add(inset)
        .saturating_add(advance)
        .min(max_x)
        .max(control.x);
    let text_height = line_height.saturating_mul(line_count as i32);
    let top_padding = (height.saturating_sub(text_height).max(0)) / 2;
    let y = control
        .y
        .saturating_add(top_padding)
        .saturating_add(line_height.saturating_mul(line_count.saturating_sub(1) as i32))
        .min(control.y.saturating_add(height.saturating_sub(line_height)));
    PhysicalRect {
        x,
        y,
        width: 1,
        height: line_height.max(1),
    }
}

fn estimate_text_advance(text: &str, line_height: i32, compact_chat_font: bool) -> i32 {
    let wide = if compact_chat_font {
        // The WebView composer is commonly 56–64 physical pixels tall while
        // the actual CJK glyph is about half of the derived line height.
        (line_height / 2).clamp(7, 24)
    } else {
        line_height.max(1)
    };
    let ascii = if compact_chat_font {
        (wide / 2).max(4)
    } else {
        (line_height.saturating_mul(7) / 16).max(4)
    };
    text.chars().fold(0_i32, |total, ch| {
        let width = if ch.is_control() {
            0
        } else if ch.is_ascii_whitespace() {
            (ascii / 2).max(2)
        } else if ch.is_ascii() {
            ascii
        } else {
            wide
        };
        total.saturating_add(width)
    })
}

unsafe fn focused_editable_value(element: &IUIAutomationElement) -> Option<String> {
    let control_type = element.CurrentControlType().ok()?;
    if !matches!(
        control_type,
        UIA_EditControlTypeId
            | UIA_DocumentControlTypeId
            | UIA_ComboBoxControlTypeId
            | UIA_GroupControlTypeId
    ) || !element
        .CurrentIsEnabled()
        .is_ok_and(|value| value.as_bool())
        || !element
            .CurrentHasKeyboardFocus()
            .is_ok_and(|value| value.as_bool())
        || element
            .CurrentIsPassword()
            .map_or(true, |value| value.as_bool())
    {
        return None;
    }
    automation::editable_text_value(element)
}

pub(crate) unsafe fn editor_estimate_for_element(
    snapshot: &FocusSnapshot,
    element: &IUIAutomationElement,
) -> Option<PopupAnchor> {
    if !snapshot.indicator_only || focused_editable_value(element).is_none() {
        return None;
    }
    let host = native::window_rect(win(snapshot.window_id as HWND));
    let control = element.CurrentBoundingRectangle().ok().and_then(|r| {
        let value = focused_editable_value(element);
        let rect = PhysicalRect {
            x: r.left,
            y: r.top,
            width: r.right - r.left,
            height: r.bottom - r.top,
        };
        valid_rect(rect)
            .then_some(rect)
            .and_then(|rect| host.and_then(|host| intersect(rect, host)))
            .map(|rect| (rect, value))
    })?;
    Some(PopupAnchor {
        geometry: geometry(estimate_editor_rect_with_metrics(
            control.0,
            control.1.as_deref(),
            snapshot.is_wechat_indicator_target(),
        )),
        source: AnchorSource::EditorLeadingEdge,
    })
}
pub(crate) fn caret_in_control(c: PhysicalRect, control: Option<PhysicalRect>) -> bool {
    valid_rect(c)
        && c.width <= 64
        && c.height <= 256
        && control.is_none_or(|b| {
            c.x >= b.x - 8
                && c.y >= b.y - 8
                && c.x + c.width <= b.x + b.width + 8
                && c.y + c.height <= b.y + b.height + 8
        })
}
pub(crate) fn anchor_for_element(
    snapshot: &FocusSnapshot,
    element: Option<&IUIAutomationElement>,
) -> PopupAnchor {
    if let Some(exact) = precise_anchor_for_element(snapshot, element) {
        return exact;
    }
    let host = native::window_rect(win(snapshot.window_id as HWND));
    let control = element
        .and_then(|e| unsafe {
            if e.CurrentIsOffscreen().map_or(true, |v| v.as_bool()) {
                return None;
            }
            let r = e.CurrentBoundingRectangle().ok()?;
            let r = PhysicalRect {
                x: r.left,
                y: r.top,
                width: r.right - r.left,
                height: r.bottom - r.top,
            };
            valid_rect(r).then_some(r)
        })
        .and_then(|r| host.and_then(|h| intersect(r, h)));
    if let Some(r) = control {
        return PopupAnchor {
            geometry: geometry(r),
            source: AnchorSource::InputControl,
        };
    }
    snapshot.anchor
}

/// Return only a caret-like anchor.  The passive indicator uses this helper
/// to keep a precise UIA/MSAA result separate from the whole editor rectangle
/// used by activation and diagnostics.
pub(crate) fn precise_anchor_for_element(
    snapshot: &FocusSnapshot,
    element: Option<&IUIAutomationElement>,
) -> Option<PopupAnchor> {
    precise_anchor_for_element_with_uia(snapshot, None, element)
}

pub(crate) fn precise_anchor_for_element_with_uia(
    snapshot: &FocusSnapshot,
    uia: Option<&IUIAutomation>,
    element: Option<&IUIAutomationElement>,
) -> Option<PopupAnchor> {
    let host = native::window_rect(win(snapshot.window_id as HWND));
    let control = element
        .and_then(|e| unsafe {
            if e.CurrentIsOffscreen().map_or(true, |v| v.as_bool()) {
                return None;
            }
            let r = e.CurrentBoundingRectangle().ok()?;
            let r = PhysicalRect {
                x: r.left,
                y: r.top,
                width: r.right - r.left,
                height: r.bottom - r.top,
            };
            valid_rect(r).then_some(r)
        })
        .and_then(|r| host.and_then(|h| intersect(r, h)));
    let accepted = |value: Option<(PhysicalRect, AnchorSource)>| {
        value.filter(|(r, _)| caret_in_control(*r, control.or(host)))
    };
    let native_caret = (snapshot.anchor.source == AnchorSource::NativeCaret)
        .then_some((snapshot.anchor.geometry.target, AnchorSource::NativeCaret));
    // Ask the FOCUSED HWND for OBJID_CARET, not merely the top-level HWND.
    let msaa = || unsafe {
        automation::msaa(win(snapshot.focused_handle as HWND))
            .or_else(|| automation::msaa(win(snapshot.window_id as HWND)))
            .or_else(|| wechat_host_msaa_caret(snapshot))
            .map(|r| (r, AnchorSource::AccessibleCaret))
    };
    let exact = accepted(native_caret)
        .or_else(|| accepted(element.and_then(|e| unsafe { automation::caret(e) })))
        .or_else(|| {
            accepted(uia.and_then(|uia| {
                element.and_then(|e| unsafe { automation::caret_in_scope(uia, e) })
            }))
        })
        .or_else(|| accepted(msaa()));
    if let Some((r, source)) = exact {
        return Some(PopupAnchor {
            geometry: geometry(r),
            source,
        });
    }
    // Keep every focused writable editor visible when it exposes only a
    // ValuePattern. The result is explicitly estimated and is never allowed
    // to outrank an exact native/UIA/MSAA/TSF caret.
    unsafe { editor_estimate_for_element(snapshot, element?) }
}

/// WeChat keeps its WebView editor under a same-process host HWND while
/// reporting no native `hwndFocus`.  Query only the bounded, known host class
/// for OBJID_CARET; never use its control rectangle as a fallback caret.
unsafe fn wechat_host_msaa_caret(snapshot: &FocusSnapshot) -> Option<PhysicalRect> {
    if !snapshot.indicator_only
        || snapshot.focused_handle != 0
        || super::native::window_class_name(win(snapshot.window_id as HWND))?.as_str()
            != "WeChatMainWndForPC"
    {
        return None;
    }
    let mut hosts = Vec::<WinHwnd>::new();
    let _ = EnumChildWindows(
        Some(win(snapshot.window_id as HWND)),
        Some(collect_wechat_host),
        LPARAM(&mut hosts as *mut _ as isize),
    );
    if hosts.len() > 16 {
        return None;
    }
    hosts.into_iter().find_map(|host| {
        let mut process = 0;
        (windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(
            host.0,
            &mut process,
        ) != 0
            && process == snapshot.process_id
            && GetAncestor(host, GA_ROOT) == win(snapshot.window_id as HWND))
        .then(|| automation::msaa(host))
        .flatten()
    })
}

unsafe extern "system" fn collect_wechat_host(host: WinHwnd, data: LPARAM) -> BOOL {
    let hosts = &mut *(data.0 as *mut Vec<WinHwnd>);
    let class = super::native::window_class_name(host);
    if class.as_deref() == Some("CWebviewControlHostWnd")
        || class.as_deref() == Some("Chrome_WidgetWin_0")
    {
        hosts.push(host);
    }
    BOOL::from(hosts.len() <= 16)
}
pub(crate) fn resolve_anchor(snapshot: &FocusSnapshot, uia: Option<&IUIAutomation>) -> PopupAnchor {
    let element = uia.and_then(|uia| unsafe {
        let e = uia.GetFocusedElement().ok()?;
        (e.CurrentHasKeyboardFocus().is_ok_and(|v| v.as_bool())
            && snapshot.owns_automation_input(uia, &e))
        .then_some(e)
    });
    anchor_for_element(snapshot, element.as_ref())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_host_caret_is_not_an_input_caret() {
        let b = PhysicalRect {
            x: 300,
            y: 1000,
            width: 700,
            height: 130,
        };
        assert!(!caret_in_control(
            PhysicalRect {
                x: 12,
                y: 0,
                width: 1,
                height: 24
            },
            Some(b)
        ));
        assert!(caret_in_control(
            PhysicalRect {
                x: 320,
                y: 1020,
                width: 1,
                height: 24
            },
            Some(b)
        ));
        assert!(!caret_in_control(b, Some(b)));
    }

    #[test]
    fn editor_estimate_starts_inside_control_and_tracks_text_end() {
        let control = PhysicalRect {
            x: 100,
            y: 200,
            width: 500,
            height: 40,
        };
        let empty = estimate_editor_rect(control, Some(""));
        let short = estimate_editor_rect(control, Some("abc"));
        let wide = estimate_editor_rect(control, Some("中文"));

        assert_eq!(empty.width, 1);
        assert!(empty.x > control.x);
        assert!(empty.y >= control.y);
        assert!(short.x > empty.x);
        assert!(wide.x > empty.x);
        assert!(empty.x < control.x + control.width);
        assert!(short.x < control.x + control.width);
        assert!(wide.x < control.x + control.width);
    }

    #[test]
    fn editor_estimate_uses_last_line_and_clamps_long_values() {
        let control = PhysicalRect {
            x: 10,
            y: 20,
            width: 120,
            height: 48,
        };
        let multiline = estimate_editor_rect(control, Some("first\nlast"));
        let long = estimate_editor_rect(control, Some(&"x".repeat(10_000)));

        assert!(multiline.y > control.y);
        assert!(long.x <= control.x + control.width - 1);
        assert_eq!(long.width, 1);
        assert!(long.height > 0);
    }

    #[test]
    fn compact_chat_font_does_not_double_cjk_advance() {
        let control = PhysicalRect {
            x: 0,
            y: 0,
            width: 800,
            height: 63,
        };
        let text = "那段时间我们体重控制都还不错的吧";
        let regular = estimate_editor_rect_with_metrics(control, Some(text), false);
        let compact = estimate_editor_rect_with_metrics(control, Some(text), true);

        assert!(compact.x > control.x);
        assert!(compact.x < control.x + 300);
        assert!(regular.x > compact.x + 150);
    }
}
