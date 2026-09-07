//! One no-window MTA worker; a stalled external provider cannot accumulate threads.
use super::{native, valid_rect, AnchorSource, FocusSnapshot};
use echo_engine::{PasteControlIdentity, PhysicalRect};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, SyncSender},
    Arc, OnceLock,
};
use std::time::Duration;
use windows::{
    core::{Interface, BOOL},
    Win32::{
        Foundation::HWND,
        System::{Com::*, Ole::*, Variant::*},
        UI::Accessibility::*,
    },
};
pub(super) struct Probe {
    pub identity: PasteControlIdentity,
    pub anchor: Option<(PhysicalRect, AnchorSource)>,
}
struct Request {
    snapshot: FocusSnapshot,
    geometry: bool,
    reply: SyncSender<Option<Probe>>,
}
struct Broker {
    sender: SyncSender<Request>,
    busy: Arc<AtomicBool>,
}
static BROKER: OnceLock<Option<Broker>> = OnceLock::new();
fn broker() -> Option<&'static Broker> {
    BROKER
        .get_or_init(|| {
            let (sender, receiver) = mpsc::sync_channel::<Request>(1);
            let busy = Arc::new(AtomicBool::new(false));
            let working = busy.clone();
            std::thread::Builder::new()
                .name("echo-focus-uia-mta".into())
                .spawn(move || unsafe {
                    if CoInitializeEx(None, COINIT_MULTITHREADED).is_err() {
                        return;
                    }
                    let automation: Option<IUIAutomation> =
                        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok();
                    while let Ok(request) = receiver.recv() {
                        let value = automation
                            .as_ref()
                            .and_then(|uia| probe(uia, &request.snapshot, request.geometry));
                        let _ = request.reply.send(value);
                        working.store(false, Ordering::Release);
                    }
                    drop(automation);
                    CoUninitialize();
                })
                .ok()?;
            Some(Broker { sender, busy })
        })
        .as_ref()
}
pub(super) fn warm() {
    let _ = broker();
}
pub(super) fn query(snapshot: FocusSnapshot, geometry: bool) -> Option<Probe> {
    let broker = broker()?;
    if broker
        .busy
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return None;
    }
    let (reply, response) = mpsc::sync_channel(1);
    if broker
        .sender
        .try_send(Request {
            snapshot,
            geometry,
            reply,
        })
        .is_err()
    {
        broker.busy.store(false, Ordering::Release);
        return None;
    }
    // A timeout discards the result, NOT the still-running COM call. Busy remains set
    // until the one worker actually returns. No fresh worker is spawned on timeout.
    response
        .recv_timeout(Duration::from_millis(55))
        .ok()
        .flatten()
}
unsafe fn writable(element: &IUIAutomationElement) -> bool {
    if let Ok(value) = element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
    {
        return value.CurrentIsReadOnly().is_ok_and(|v| !v.as_bool());
    }
    let Ok(text) = element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
    else {
        return false;
    };
    let Ok(range) = text.DocumentRange() else {
        return false;
    };
    let Ok(mut value) = range.GetAttributeValue(UIA_IsReadOnlyAttributeId) else {
        return false;
    };
    let result = value.Anonymous.Anonymous.vt == VT_BOOL
        && value.Anonymous.Anonymous.Anonymous.boolVal.0 == 0;
    let _ = VariantClear(&mut value);
    result
}
unsafe fn probe(uia: &IUIAutomation, snapshot: &FocusSnapshot, geometry: bool) -> Option<Probe> {
    if !snapshot.current() {
        return None;
    }
    let element = uia.GetFocusedElement().ok()?;
    let window = HWND(snapshot.window_id as _);
    if element.CurrentProcessId().ok()? as u32 != snapshot.process_id
        || !native::automation_element_belongs_to(uia, &element, window)
        || !native::automation_element_is_editable(&element)
        || !writable(&element)
    {
        return None;
    }
    let identity =
        native::automation_runtime_id(&element).map(PasteControlIdentity::AutomationRuntimeId)?;
    let anchor = geometry.then(|| {
        let result = super::anchor_for_element(snapshot, Some(&element));
        (result.geometry.target, result.source)
    });
    if !snapshot.current() {
        return None;
    }
    Some(Probe { identity, anchor })
}
pub(crate) unsafe fn caret(element: &IUIAutomationElement) -> Option<(PhysicalRect, AnchorSource)> {
    // TextPattern2 may be advertised but fail for an empty/new control.
    // In that case fall back to the single collapsed selection, not the host rect.
    let range = element
        .GetCurrentPatternAs::<IUIAutomationTextPattern2>(UIA_TextPattern2Id)
        .ok()
        .and_then(|pattern| {
            let mut active = BOOL(0);
            pattern
                .GetCaretRange(&mut active)
                .ok()
                .filter(|_| active.as_bool())
        })
        .or_else(|| {
            let pattern = element
                .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
                .ok()?;
            let selection = pattern.GetSelection().ok()?;
            if selection.Length().ok()? != 1 {
                return None;
            }
            selection.GetElement(0).ok()
        })?;
    // Never infer an insertion endpoint from a non-empty selection.
    if range
        .CompareEndpoints(
            TextPatternRangeEndpoint_Start,
            &range,
            TextPatternRangeEndpoint_End,
        )
        .ok()?
        != 0
    {
        return None;
    }
    if let Some(r) = bounds(&range) {
        return Some((r, AnchorSource::AutomationCaret));
    }
    // Some providers expose no rectangle for a degenerate range. A cloned range
    // supplies an approximate character edge without moving the actual selection.
    let next = range.Clone().ok()?;
    if next
        .MoveEndpointByUnit(TextPatternRangeEndpoint_End, TextUnit_Character, 1)
        .ok()?
        == 1
    {
        if let Some(mut r) = bounds(&next) {
            r.width = 1;
            return Some((r, AnchorSource::AdjacentCharacter));
        }
    }
    let previous = range.Clone().ok()?;
    if previous
        .MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, -1)
        .ok()?
        == -1
    {
        let mut r = bounds(&previous)?;
        r.x = r.x.saturating_add(r.width);
        r.width = 1;
        return Some((r, AnchorSource::AdjacentCharacter));
    }
    None
}
unsafe fn bounds(range: &IUIAutomationTextRange) -> Option<PhysicalRect> {
    let array = range.GetBoundingRectangles().ok()?;
    if array.is_null() {
        return None;
    }
    let result = (|| {
        if SafeArrayGetDim(array) != 1 || SafeArrayGetElemsize(array) != 8 {
            return None;
        }
        let lo = SafeArrayGetLBound(array, 1).ok()?;
        let hi = SafeArrayGetUBound(array, 1).ok()?;
        let count = hi.checked_sub(lo)?.checked_add(1)?;
        if !(4..=4096).contains(&count) || count % 4 != 0 {
            return None;
        }
        let mut data = std::ptr::null_mut();
        SafeArrayAccessData(array, &mut data).ok()?;
        let result = if data.is_null() {
            None
        } else {
            let values = std::slice::from_raw_parts(data.cast::<f64>(), count as usize);
            values.chunks_exact(4).find_map(rect_from_values)
        };
        let _ = SafeArrayUnaccessData(array);
        result
    })();
    let _ = SafeArrayDestroy(array);
    result
}
fn rect_from_values(v: &[f64]) -> Option<PhysicalRect> {
    if v.len() != 4
        || v.iter().any(|n| !n.is_finite() || n.abs() >= 16_000_000.0)
        || v[2] < 0.0
        || v[3] <= 0.0
    {
        return None;
    }
    let r = PhysicalRect {
        x: v[0].round() as i32,
        y: v[1].round() as i32,
        width: (v[2].ceil() as i32).max(1),
        height: v[3].ceil() as i32,
    };
    valid_rect(r).then_some(r)
}
pub(crate) unsafe fn msaa(hwnd: HWND) -> Option<PhysicalRect> {
    let mut object = std::ptr::null_mut();
    AccessibleObjectFromWindow(hwnd, 0xfffffff8, &IAccessible::IID, &mut object).ok()?;
    if object.is_null() {
        return None;
    }
    let accessible = IAccessible::from_raw(object);
    let mut child = VARIANT::default();
    (*child.Anonymous.Anonymous).vt = VT_I4;
    (*child.Anonymous.Anonymous).Anonymous.lVal = 0;
    let (mut x, mut y, mut width, mut height) = (0, 0, 0, 0);
    accessible
        .accLocation(&mut x, &mut y, &mut width, &mut height, &child)
        .ok()?;
    let r = PhysicalRect {
        x,
        y,
        width: width.max(1),
        height,
    };
    valid_rect(r).then_some(r)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_provider_rectangles_are_rejected() {
        assert!(rect_from_values(&[f64::NAN, 0.0, 1.0, 20.0]).is_none());
        assert!(rect_from_values(&[0.0, 0.0, -1.0, 20.0]).is_none());
        assert!(rect_from_values(&[0.0, 0.0, 1.0, 0.0]).is_none());
        assert_eq!(
            rect_from_values(&[-1200.0, 80.0, 0.0, 24.0]).unwrap().width,
            1
        );
    }
}
