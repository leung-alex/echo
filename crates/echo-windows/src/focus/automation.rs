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
        Foundation::{HWND, LPARAM},
        System::{Com::*, Ole::*, Variant::*},
        UI::Accessibility::*,
        UI::WindowsAndMessaging::EnumChildWindows,
    },
};
pub(crate) struct Probe {
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
                    // Retain the provider connection, not its authorization. XAML
                    // hosted inputs can take over a second to connect cold. Each
                    // request still revalidates native ownership and live focus.
                    let mut provider = None;
                    while let Ok(request) = receiver.recv() {
                        let value = automation.as_ref().and_then(|uia| {
                            probe(uia, &request.snapshot, request.geometry, &mut provider)
                        });
                        working.store(false, Ordering::Release);
                        let _ = request.reply.send(value);
                    }
                    drop(provider);
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
    let timeout = Duration::from_millis(if snapshot.is_hosted_input() {
        2500
    } else {
        200
    });
    begin_query(snapshot, geometry)?
        .recv_timeout(timeout)
        .ok()
        .flatten()
}
pub(crate) fn begin_query(
    snapshot: FocusSnapshot,
    geometry: bool,
) -> Option<mpsc::Receiver<Option<Probe>>> {
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
    // A dropped receiver does not cancel COM or spawn another worker.
    Some(response)
}
unsafe fn writable(element: &IUIAutomationElement) -> bool {
    if let Ok(value) = element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
    {
        return value.CurrentIsReadOnly().is_ok_and(|v| !v.as_bool());
    }
    writable_text(element)
}
unsafe fn writable_text(element: &IUIAutomationElement) -> bool {
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
// Rich chat editors can expose Group instead of Edit/Document. Quick Insert
// needs a verified writable input, not inline completion's range replacement
// contract. Keep this extension local to both capture and delivery revalidation.
unsafe fn quick_insert_editable(element: &IUIAutomationElement) -> bool {
    if element.CurrentControlType().ok() == Some(UIA_GroupControlTypeId) {
        native::automation_element_has_input_focus(element) && writable_text(element)
    } else {
        native::automation_element_is_editable(element) && writable(element)
    }
}
unsafe fn probe(
    uia: &IUIAutomation,
    snapshot: &FocusSnapshot,
    geometry: bool,
    provider: &mut Option<(isize, isize, IUIAutomationElement)>,
) -> Option<Probe> {
    if provider.as_ref().is_some_and(|(window, focus, _)| {
        *window != snapshot.window_id || *focus != snapshot.focused_handle
    }) {
        *provider = None;
    }
    if !snapshot.current() {
        return None;
    }
    if snapshot.indicator_only
        && native::window_class_name(HWND(snapshot.window_id as _)).as_deref()
            == Some("ConsoleWindowClass")
    {
        return console_caret(uia, snapshot);
    }
    let retained = provider.as_ref().and_then(|(_, _, element)| {
        native::automation_element_has_input_focus(element).then(|| (element.clone(), false))
    });
    let (element, legacy) = retained.or_else(|| focused_element(uia, snapshot))?;
    *provider = Some((snapshot.window_id, snapshot.focused_handle, element.clone()));
    probe_element(uia, snapshot, geometry, &element, legacy)
}
// Some embedded providers return their native host from GetFocusedElement and
// misreport UIA IsKeyboardFocusable on the actual input. Follow MSAA's direct
// focus chain instead of scanning the entire subtree. Capture and delivery use
// the same resolver, and the converted element still needs UIA identity checks.
unsafe fn focused_element(
    uia: &IUIAutomation,
    snapshot: &FocusSnapshot,
) -> Option<(IUIAutomationElement, bool)> {
    let element = uia.GetFocusedElement().ok()?;
    if native::automation_element_has_input_focus(&element) {
        return Some((element, false));
    }
    let host = HWND(snapshot.focused_handle as _);
    if !snapshot.current()
        || !native::native_control_belongs_to(
            host,
            HWND(snapshot.window_id as _),
            snapshot.process_id,
        )
    {
        return None;
    }
    if let Some(element) = legacy_focused_element(uia, host) {
        return Some((element, true));
    }
    // The GUI thread can focus a container HWND while its embedded provider lives
    // on a child HWND. Bound the native host list, and require one unique focused
    // input; never walk the potentially enormous accessibility tree.
    let mut hosts = Vec::<HWND>::new();
    let _ = EnumChildWindows(
        Some(host),
        Some(collect_host),
        LPARAM(&mut hosts as *mut _ as isize),
    );
    if hosts.len() > 32 {
        return None;
    }
    let mut found: Option<IUIAutomationElement> = None;
    for child in hosts {
        if !snapshot.current() {
            return None;
        }
        if !native::native_control_belongs_to(
            child,
            HWND(snapshot.window_id as _),
            snapshot.process_id,
        ) {
            continue;
        }
        if let Some(candidate) = legacy_focused_element(uia, child) {
            if let Some(previous) = found.as_ref() {
                if !uia
                    .CompareElements(previous, &candidate)
                    .is_ok_and(|v| v.as_bool())
                {
                    return None;
                }
            } else {
                found = Some(candidate);
            }
        }
    }
    found.map(|element| (element, true))
}
unsafe extern "system" fn collect_host(host: HWND, data: LPARAM) -> BOOL {
    let hosts = &mut *(data.0 as *mut Vec<HWND>);
    hosts.push(host);
    BOOL::from(hosts.len() <= 32)
}
unsafe fn legacy_focused_element(uia: &IUIAutomation, host: HWND) -> Option<IUIAutomationElement> {
    let started = std::time::Instant::now();
    let mut object = std::ptr::null_mut();
    AccessibleObjectFromWindow(host, 0xffff_fffc, &IAccessible::IID, &mut object).ok()?;
    if object.is_null() {
        return None;
    }
    let mut accessible = IAccessible::from_raw(object);
    for _ in 0..8 {
        let mut focus = accessible.accFocus().ok()?;
        let kind = focus.Anonymous.Anonymous.vt;
        let next = if kind == VT_DISPATCH {
            focus
                .Anonymous
                .Anonymous
                .Anonymous
                .pdispVal
                .as_ref()
                .and_then(|dispatch| dispatch.cast::<IAccessible>().ok())
        } else {
            None
        };
        let child = (kind == VT_I4).then(|| focus.Anonymous.Anonymous.Anonymous.lVal);
        let _ = VariantClear(&mut focus);
        if let Some(next) = next {
            accessible = next;
            continue;
        }
        let child = child?;
        if child < 0 || started.elapsed() > Duration::from_secs(1) {
            return None;
        }
        let child_variant = VARIANT::from(child);
        let role = variant_i32(accessible.get_accRole(&child_variant).ok()?)?;
        let state = variant_i32(accessible.get_accState(&child_variant).ok()?)?;
        if !legacy_writable_focus(role, state) {
            return None;
        }
        let element = uia.ElementFromIAccessible(&accessible, child).ok()?;
        return Some(element);
    }
    None
}
unsafe fn variant_i32(mut value: VARIANT) -> Option<i32> {
    let result =
        (value.Anonymous.Anonymous.vt == VT_I4).then(|| value.Anonymous.Anonymous.Anonymous.lVal);
    let _ = VariantClear(&mut value);
    result
}
fn legacy_writable_focus(role: i32, state: i32) -> bool {
    // MSAA ROLE_SYSTEM_TEXT; require FOCUSED + FOCUSABLE and reject
    // UNAVAILABLE, READONLY and PROTECTED. UIA writability is checked as well.
    role == 42 && state & 0x0010_0004 == 0x0010_0004 && state & (0x1 | 0x40 | 0x2000_0000) == 0
}
pub(super) unsafe fn plain_paste_probe(
    uia: &IUIAutomation,
    snapshot: &FocusSnapshot,
) -> Option<Probe> {
    let (element, legacy) = focused_element(uia, snapshot)?;
    let has_text = element
        .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
        .is_ok();
    let read_only = element
        .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
        .ok()
        .and_then(|value| value.CurrentIsReadOnly().ok())
        .map(|v| v.as_bool());
    if !prefers_plain_paste(has_text, read_only, legacy) {
        return None;
    }
    probe_element(uia, snapshot, true, &element, legacy)
}
fn prefers_plain_paste(has_text: bool, value_read_only: Option<bool>, legacy: bool) -> bool {
    // The MSAA-to-UIA bridge may synthesize TextPattern. That does not establish
    // inline selection/replacement support in the original focused provider.
    (legacy || !has_text) && value_read_only == Some(false)
}
unsafe fn probe_element(
    uia: &IUIAutomation,
    snapshot: &FocusSnapshot,
    geometry: bool,
    element: &IUIAutomationElement,
    legacy: bool,
) -> Option<Probe> {
    if !snapshot.current() {
        return None;
    }
    if element
        .CurrentIsOffscreen()
        .map_or(true, |value| value.as_bool())
    {
        return None;
    }
    let belongs = snapshot.owns_automation_input(uia, element);
    let editable = quick_insert_editable(element)
        || (legacy
            && element.CurrentControlType().ok() == Some(UIA_EditControlTypeId)
            && element.CurrentIsEnabled().is_ok_and(|v| v.as_bool())
            && element.CurrentHasKeyboardFocus().is_ok_and(|v| v.as_bool())
            && element.CurrentIsPassword().is_ok_and(|v| !v.as_bool())
            && writable(element));
    if !belongs || !editable {
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
#[cfg(test)]
mod capability_tests {
    use super::{legacy_writable_focus, prefers_plain_paste};
    #[test]
    fn legacy_focus_requires_writable_text_with_actual_keyboard_focus() {
        let focused = 0x0010_0004;
        assert!(legacy_writable_focus(42, 1074790404));
        assert!(!legacy_writable_focus(42, focused & !4));
        assert!(!legacy_writable_focus(42, focused & !0x0010_0000));
        for blocked in [0x1, 0x40, 0x2000_0000] {
            assert!(!legacy_writable_focus(42, focused | blocked));
        }
        assert!(!legacy_writable_focus(9, focused));
    }
    #[test]
    fn value_only_writable_inputs_use_plain_paste_without_an_app_allowlist() {
        assert!(prefers_plain_paste(false, Some(false), false));
        assert!(!prefers_plain_paste(true, Some(false), false));
        assert!(!prefers_plain_paste(false, Some(true), false));
        assert!(!prefers_plain_paste(false, None, false));
        assert!(prefers_plain_paste(true, Some(false), true));
        assert!(!prefers_plain_paste(true, Some(true), true));
        assert!(!prefers_plain_paste(true, None, true));
    }
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

// Console text is exposed as a read-only document even at its input cursor.
// This geometry-only exception never authorizes paste or range replacement.
unsafe fn console_caret(uia: &IUIAutomation, snapshot: &FocusSnapshot) -> Option<Probe> {
    let root = uia.ElementFromHandle(HWND(snapshot.window_id as _)).ok()?;
    let mut owner = 0;
    let ime = windows_sys::Win32::UI::Input::Ime::ImmGetDefaultIMEWnd(snapshot.window_id as _);
    windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId(ime, &mut owner);
    if owner == 0 || root.CurrentProcessId().ok()? != owner as i32 {
        return None;
    }
    let walker = uia.ControlViewWalker().ok()?;
    let mut element = walker.GetFirstChildElement(&root).ok()?;
    for _ in 0..32 {
        if element.CurrentControlType().ok() == Some(UIA_DocumentControlTypeId)
            && element.CurrentProcessId().ok() == Some(owner as i32)
            && element.CurrentHasKeyboardFocus().is_ok_and(|v| v.as_bool())
            && element.CurrentIsEnabled().is_ok_and(|v| v.as_bool())
            && element.CurrentIsPassword().is_ok_and(|v| !v.as_bool())
            && element.CurrentIsOffscreen().is_ok_and(|v| !v.as_bool())
        {
            let anchor = caret(&element)?;
            if !super::anchor::caret_in_control(
                anchor.0,
                native::window_rect(HWND(snapshot.window_id as _)),
            ) || !snapshot.current()
            {
                return None;
            }
            return Some(Probe {
                identity: PasteControlIdentity::AutomationRuntimeId(native::automation_runtime_id(
                    &element,
                )?),
                anchor: Some(anchor),
            });
        }
        element = walker.GetNextSiblingElement(&element).ok()?;
    }
    None
}
