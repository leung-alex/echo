//! Geometry is not paste authorization. Never substitute the whole host window
//! for a virtual input element. All returned rectangles are physical pixels.
use super::*;
use windows::Win32::UI::Accessibility::{IUIAutomation, IUIAutomationElement};

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
            .map(|r| (r, AnchorSource::AccessibleCaret))
    };
    let exact = accepted(native_caret)
        .or_else(|| accepted(element.and_then(|e| unsafe { automation::caret(e) })))
        .or_else(|| accepted(msaa()));
    if let Some((r, source)) = exact {
        return PopupAnchor {
            geometry: geometry(r),
            source,
        };
    }
    if let Some(r) = control {
        return PopupAnchor {
            geometry: geometry(r),
            source: AnchorSource::InputControl,
        };
    }
    snapshot.anchor
}
pub(crate) fn resolve_anchor(snapshot: &FocusSnapshot, uia: Option<&IUIAutomation>) -> PopupAnchor {
    let element = uia.and_then(|uia| unsafe {
        let e = uia.GetFocusedElement().ok()?;
        (e.CurrentProcessId().ok() == Some(snapshot.process_id as i32)
            && e.CurrentHasKeyboardFocus().is_ok_and(|v| v.as_bool())
            && native::automation_element_belongs_to(uia, &e, win(snapshot.window_id as HWND)))
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
}
