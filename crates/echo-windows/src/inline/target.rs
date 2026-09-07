//! Native EDIT and UI Automation text ranges. This worker is the only owner of
//! COM elements/ranges. It never uses Ctrl+A, backspace counts, or Value.SetValue.
use super::*;
use crate::windows_impl as native;
use echo_engine::{ComposerSnapshot, PasteControlIdentity, MAX_COMPOSER_UNITS};
use std::{ops::Range, ptr::null_mut};
use windows::{
    core::{Interface, BSTR},
    Win32::{
        Foundation::HWND,
        System::{Com::*, Variant::*},
        UI::Accessibility::*,
    },
};
use windows_sys::Win32::UI::Controls::{EM_GETSEL, EM_SETSEL};
use windows_sys::Win32::UI::{
    Input::{Ime::*, KeyboardAndMouse::GetKeyboardLayout},
    WindowsAndMessaging::*,
};

pub(super) fn create_automation() -> Option<IUIAutomation> {
    unsafe {
        let uia: IUIAutomation = CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)
            .or_else(|_| CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER))
            .ok()?;
        if let Ok(settings) = uia.cast::<IUIAutomation2>() {
            let _ = settings.SetAutoSetFocus(false);
            let _ = settings.SetConnectionTimeout(200);
            let _ = settings.SetTransactionTimeout(200);
        }
        Some(uia)
    }
}
struct AutomationTarget {
    uia: IUIAutomation,
    element: IUIAutomationElement,
    edit: Option<IUIAutomationTextEditPattern>,
}
pub(super) struct Target {
    pub(super) paste_target: PasteTarget,
    control: isize,
    thread: u32,
    automation: Option<AutomationTarget>,
    native_composition: Option<IUIAutomationTextEditPattern>,
    native_element: Option<IUIAutomationElement>,
}
impl Target {
    pub(super) fn open(
        snapshot: &FocusSnapshot,
        uia: Option<&IUIAutomation>,
    ) -> Result<Self, String> {
        if !snapshot.current() {
            return Err("Original input focus changed before activation".into());
        }
        let window = HWND(snapshot.window_id as _);
        native::validate_target_integrity(snapshot.process_id)
            .map_err(|_| "Elevated input is not available for inline replacement")?;
        let mut automation = None;
        let identity = if let Some(identity) =
            native::focused_native_control_from_gui(window, snapshot.process_id)
        {
            identity
        } else {
            let uia = uia.ok_or("Windows text accessibility is unavailable")?;
            unsafe {
                let element = uia
                    .GetFocusedElement()
                    .map_err(|_| "The input does not expose its focused text element")?;
                if element.CurrentProcessId().ok() != Some(snapshot.process_id as i32)
                    || !native::automation_element_belongs_to(uia, &element, window)
                    || !native::automation_element_is_editable(&element)
                    || !writable(&element)
                {
                    return Err(
                        "The input is protected, read-only, or not a verified text editor".into(),
                    );
                }
                if element.CurrentIsOffscreen().map_or(true, |v| v.as_bool()) {
                    return Err("The focused input is an offscreen proxy, not a safely replaceable composer".into());
                }
                let (text, _) = super::text_scope::resolve(uia, &element)?;
                if text.SupportedTextSelection().ok() == Some(SupportedTextSelection_None) {
                    return Err("This input cannot select an exact query range".into());
                }
                let edit = element
                    .GetCurrentPatternAs::<IUIAutomationTextEditPattern>(UIA_TextEditPatternId)
                    .ok();
                let id = native::automation_runtime_id(&element)
                    .ok_or("The input has no stable accessibility identity")?;
                automation = Some(AutomationTarget {
                    uia: uia.clone(),
                    element,
                    edit,
                });
                PasteControlIdentity::AutomationRuntimeId(id)
            }
        };
        let control = match &identity {
            PasteControlIdentity::NativeWindow { handle, .. } => *handle,
            _ => snapshot.focused_handle,
        };
        let thread = unsafe { GetWindowThreadProcessId(window.0, null_mut()) };
        // Native Edit is still the range backend. A native provider may additionally
        // expose TextEdit composition state, so do not unnecessarily force IME
        // users through the less capable default-IME-window compatibility path.
        let native_element = if automation.is_none() {
            uia.and_then(|uia| unsafe {
                let e = uia.ElementFromHandle(HWND(control as _)).ok()?;
                native::automation_element_belongs_to(uia, &e, window).then_some(e)
            })
        } else {
            None
        };
        let native_composition = native_element.as_ref().and_then(|e| unsafe {
            e.GetCurrentPatternAs::<IUIAutomationTextEditPattern>(UIA_TextEditPatternId)
                .ok()
        });
        Ok(Self {
            paste_target: PasteTarget {
                window_id: snapshot.window_id,
                window_class: native::window_class_name(window).unwrap_or_default(),
                process_id: snapshot.process_id,
                process_started_at: snapshot.process_started_at,
                focused_control: Some(identity),
                app_name: None,
                selected_text: None,
                is_single_line: None,
                geometry: snapshot.anchor.geometry,
            },
            control,
            thread,
            automation,
            native_composition,
            native_element,
        })
    }
    pub(super) fn backend(&self) -> &'static str {
        if self.automation.is_some() {
            "uia-text-range"
        } else {
            "native-edit"
        }
    }
    pub(super) fn automation_element(&self) -> Option<(&IUIAutomation, &IUIAutomationElement)> {
        self.automation.as_ref().map(|a| (&a.uia, &a.element))
    }
    pub(super) fn current(&self) -> bool {
        let Ok(hwnd) = native::validate_target_identity(&self.paste_target) else {
            return false;
        };
        if unsafe { GetForegroundWindow() } != hwnd.0 {
            return false;
        }
        if native::validate_target_integrity(self.paste_target.process_id).is_err() {
            return false;
        }
        if let Some(a) = &self.automation {
            unsafe {
                a.uia.GetFocusedElement().ok().is_some_and(|f| {
                    a.uia
                        .CompareElements(&f, &a.element)
                        .is_ok_and(|same| same.as_bool())
                }) && native::automation_element_is_editable(&a.element)
                    && writable(&a.element)
            }
        } else {
            native::focused_native_control_from_gui(hwnd, self.paste_target.process_id)
                == self.paste_target.focused_control
        }
    }
    pub(super) fn snapshot(&self) -> Result<ComposerSnapshot, String> {
        if !self.current() {
            return Err("Original input control changed; nothing was replaced".into());
        }
        let value = if let Some(a) = &self.automation {
            unsafe { automation_snapshot(a)? }
        } else {
            let length = message(self.control, WM_GETTEXTLENGTH, 0, 0)?;
            if length > MAX_COMPOSER_UNITS {
                return Err("Composer exceeds the bounded inline text budget".into());
            }
            let mut text = vec![0u16; length + 1];
            let count = message(
                self.control,
                WM_GETTEXT,
                text.len(),
                text.as_mut_ptr() as isize,
            )?;
            if count > length {
                return Err("Text changed while being read".into());
            }
            text.truncate(count);
            let (mut start, mut end) = (0u32, 0u32);
            message(
                self.control,
                EM_GETSEL,
                &mut start as *mut u32 as usize,
                &mut end as *mut u32 as isize,
            )?;
            ComposerSnapshot {
                text,
                selection: start as usize..end as usize,
            }
        };
        value.validate().map_err(str::to_string)?;
        Ok(value)
    }
    pub(super) fn composition(&self) -> (u8, bool) {
        let layout = unsafe { GetKeyboardLayout(self.thread) };
        let language = (layout as usize & 0x3ff) as u16;
        let possible = matches!(language, 0x04 | 0x11 | 0x12) || unsafe { ImmIsIME(layout) != 0 };
        if let Some(edit) = self
            .automation
            .as_ref()
            .and_then(|a| a.edit.as_ref())
            .or(self.native_composition.as_ref())
        {
            unsafe {
                let mut raw = null_mut();
                let hr = (edit.vtable().GetActiveComposition)(edit.as_raw(), &mut raw);
                if hr.is_ok() {
                    if raw.is_null() {
                        return (IME_CLEAR, possible);
                    }
                    let range = IUIAutomationTextRange::from_raw(raw);
                    let active = range
                        .CompareEndpoints(
                            TextPatternRangeEndpoint_Start,
                            &range,
                            TextPatternRangeEndpoint_End,
                        )
                        .map_or(true, |difference| difference != 0);
                    return (if active { IME_ACTIVE } else { IME_CLEAR }, possible);
                }
            }
        }
        if !possible {
            return (IME_CLEAR, false);
        }
        // A foreign HIMC is not a reliable cross-process composition contract.
        // Only use documented default-IME window queries to prove English/closed
        // mode; an active unobservable IME requires the independent-search fallback.
        let ime = unsafe { ImmGetDefaultIMEWnd(self.control as _) };
        if !ime.is_null() {
            if let Ok(open) = message(ime as isize, WM_IME_CONTROL, 0x0005, 0) {
                if open == 0 {
                    return (IME_CLEAR, true);
                }
                if let Ok(conversion) = message(ime as isize, WM_IME_CONTROL, 0x0001, 0) {
                    if conversion & 1 == 0 {
                        return (IME_CLEAR, true);
                    }
                }
            }
        }
        (IME_UNKNOWN, true)
    }
    pub(super) fn anchor(&self) -> Option<PopupAnchor> {
        let fresh = FocusSnapshot::capture();
        if fresh.window_id != self.paste_target.window_id
            || fresh.process_id != self.paste_target.process_id
        {
            return None;
        }
        let element = self
            .automation
            .as_ref()
            .map(|a| &a.element)
            .or(self.native_element.as_ref());
        Some(crate::focus::anchor_for_element(&fresh, element))
    }
    pub(super) fn select(
        &self,
        span: Range<usize>,
        expected: &ComposerSnapshot,
        deadline: &Deadline,
    ) -> Result<(), String> {
        if !deadline.live() || !self.current() {
            return Err("Input changed before range selection".into());
        }
        if let Some(a) = &self.automation {
            unsafe {
                let (pattern, doc) = live_document(a)?;
                let range = if span.is_empty() {
                    let selection = single_selection(&pattern)?;
                    let snapshot = automation_snapshot(a)?;
                    if snapshot.selection != span || snapshot.text != expected.text {
                        return Err("Insertion point moved".into());
                    }
                    selection
                } else {
                    let query = BSTR::from_wide(&expected.text[span.clone()]);
                    let remaining = doc.Clone().map_err(|_| "Could not clone the text range")?;
                    let mut found = None;
                    for _ in 0..128 {
                        if !deadline.live() {
                            return Err("Range lookup timed out without replacing text".into());
                        }
                        let candidate = remaining
                            .FindText(&query, false, false)
                            .map_err(|_| "Query range could not be located exactly")?;
                        let prefix = doc
                            .Clone()
                            .map_err(|_| "Could not clone the prefix range")?;
                        prefix
                            .MoveEndpointByRange(
                                TextPatternRangeEndpoint_End,
                                &candidate,
                                TextPatternRangeEndpoint_Start,
                            )
                            .map_err(|_| "Could not identify query boundary")?;
                        let before = text(&prefix)?;
                        if before.len() == span.start
                            && text(&candidate)? == expected.text[span.clone()]
                        {
                            found = Some(candidate);
                            break;
                        }
                        if before.len() > span.start {
                            break;
                        }
                        remaining
                            .MoveEndpointByRange(
                                TextPatternRangeEndpoint_Start,
                                &candidate,
                                TextPatternRangeEndpoint_End,
                            )
                            .map_err(|_| "Could not advance range lookup")?;
                    }
                    found.ok_or("Query range is ambiguous; use independent search")?
                };
                if !deadline.live() || !self.current() {
                    return Err("Range selection cancelled".into());
                }
                // Keep the editable selection provider as the operation owner.
                // FindText can return a read-only descendant's text range.
                let editable = single_selection(&pattern)?
                    .Clone()
                    .map_err(|_| "Editable selection disappeared")?;
                editable
                    .MoveEndpointByRange(
                        TextPatternRangeEndpoint_End,
                        &range,
                        TextPatternRangeEndpoint_End,
                    )
                    .map_err(|_| "Cannot bind selection end to editor")?;
                editable
                    .MoveEndpointByRange(
                        TextPatternRangeEndpoint_Start,
                        &range,
                        TextPatternRangeEndpoint_Start,
                    )
                    .map_err(|_| "Cannot bind selection start to editor")?;
                editable.Select().map_err(|error| {
                    format!(
                        "Input refused exact range selection: {:#010x}",
                        error.code().0
                    )
                })?;
            }
        } else {
            message(self.control, EM_SETSEL, span.start, span.end as isize)?;
        }
        // UIA Select may acknowledge the action before Chromium's renderer has
        // published the new selection. Wait for that exact, non-mutating selection;
        // never paste merely because Select returned S_OK.
        let until = Instant::now() + Duration::from_millis(160);
        loop {
            if !deadline.live() || !self.current() {
                return Err("Range selection cancelled".into());
            }
            if let Ok(selected) = self.snapshot() {
                if selected.text != expected.text {
                    return Err("Input text changed during selection; nothing was replaced".into());
                }
                if selected.selection == span {
                    return Ok(());
                }
            }
            if Instant::now() >= until {
                return Err(
                    "The input did not acknowledge the exact selection; nothing was replaced"
                        .into(),
                );
            }
            std::thread::sleep(Duration::from_millis(8));
        }
    }
    pub(super) fn paste_selected(&self) -> Result<(), String> {
        if !self.current() {
            return Err("Input focus changed before paste".into());
        }
        native::modifiers_released()
            .map_err(|_| "ModifierKeysBusy: release held modifiers before inserting")?;
        if self.automation.is_some() {
            native::send_paste_shortcut()
                .map_err(|_| "Windows did not accept the paste shortcut".to_string())
        } else {
            message(self.control, WM_PASTE, 0, 0).map(|_| ())
        }
    }
    pub(super) fn normalize_inserted(&self, text: &str) -> Vec<u16> {
        if self.automation.is_none() {
            text.replace("\r\n", "\n")
                .replace('\n', "\r\n")
                .encode_utf16()
                .collect()
        } else {
            text.encode_utf16().collect()
        }
    }
}
fn message(hwnd: isize, message: u32, w: usize, l: isize) -> Result<usize, String> {
    let mut result = 0;
    if unsafe {
        SendMessageTimeoutW(
            hwnd as _,
            message,
            w,
            l,
            SMTO_ABORTIFHUNG | SMTO_BLOCK,
            80,
            &mut result,
        )
    } == 0
    {
        Err("Input control did not respond within the native message budget".into())
    } else {
        Ok(result)
    }
}
unsafe fn text(range: &IUIAutomationTextRange) -> Result<Vec<u16>, String> {
    let value = range
        .GetText((MAX_COMPOSER_UNITS + 1) as i32)
        .map_err(|_| "Input text is unavailable")?;
    if value.len() > MAX_COMPOSER_UNITS {
        return Err("Composer exceeds the bounded inline text budget".into());
    }
    Ok(value.to_vec())
}
unsafe fn single_selection(
    pattern: &IUIAutomationTextPattern,
) -> Result<IUIAutomationTextRange, String> {
    let selected = pattern
        .GetSelection()
        .map_err(|_| "Text selection is unavailable")?;
    if selected.Length().ok() != Some(1) {
        return Err("Inline completion requires one caret or text selection".into());
    }
    selected
        .GetElement(0)
        .map_err(|_| "Text selection disappeared".into())
}
// Refresh the provider from the SAME editor, not an arbitrary new focus.
// Chromium replaces empty paragraph providers when the first character arrives.
unsafe fn live_document(
    target: &AutomationTarget,
) -> Result<(IUIAutomationTextPattern, IUIAutomationTextRange), String> {
    let focused = target
        .uia
        .GetFocusedElement()
        .map_err(|_| "Focused editor disappeared")?;
    if !target
        .uia
        .CompareElements(&focused, &target.element)
        .is_ok_and(|v| v.as_bool())
    {
        return Err("Original editor identity changed".into());
    }
    let (pattern, from_child) = super::text_scope::resolve(&target.uia, &focused)?;
    let doc = super::text_scope::document(&pattern, &focused, from_child)?;
    Ok((pattern, doc))
}
unsafe fn automation_snapshot(target: &AutomationTarget) -> Result<ComposerSnapshot, String> {
    let (pattern, doc) = live_document(target)?;
    let selected = single_selection(&pattern)?;
    let selected =
        super::text_scope::bounded_selection(&target.uia, &target.element, &doc, &selected)?;
    if super::text_scope::empty_value_caret(&target.element, &selected)
        || super::text_scope::empty_paragraph_caret(&target.element, &doc, &selected)
    {
        return Ok(ComposerSnapshot {
            text: Vec::new(),
            selection: 0..0,
        });
    }
    let prefix = doc.Clone().map_err(|_| "Prefix range is unavailable")?;
    let suffix = doc.Clone().map_err(|_| "Suffix range is unavailable")?;
    prefix
        .MoveEndpointByRange(
            TextPatternRangeEndpoint_End,
            &selected,
            TextPatternRangeEndpoint_Start,
        )
        .map_err(|_| "Selection start is unavailable")?;
    suffix
        .MoveEndpointByRange(
            TextPatternRangeEndpoint_Start,
            &selected,
            TextPatternRangeEndpoint_End,
        )
        .map_err(|_| "Selection end is unavailable")?;
    let all = text(&doc)?;
    let before = text(&prefix)?;
    let selection = text(&selected)?;
    let after = text(&suffix)?;
    let start = before.len();
    let end = start + selection.len();
    if before
        .iter()
        .chain(selection.iter())
        .chain(after.iter())
        .copied()
        .collect::<Vec<_>>()
        != all
    {
        return Err("Input exposes inconsistent text and selection ranges".into());
    }
    Ok(ComposerSnapshot {
        text: all,
        selection: start..end,
    })
}
unsafe fn writable(element: &IUIAutomationElement) -> bool {
    if let Ok(value) = element.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
    {
        return value.CurrentIsReadOnly().is_ok_and(|v| !v.as_bool());
    }
    let Ok(pattern) = element.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
    else {
        return false;
    };
    let Ok(range) = pattern.DocumentRange() else {
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
