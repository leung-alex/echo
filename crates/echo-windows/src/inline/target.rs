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
use windows_sys::Win32::UI::Controls::{EM_GETSEL, EM_REPLACESEL, EM_SETSEL};
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
    ime_observer: std::cell::RefCell<Option<super::ime_observer::Observer>>,
    ime_observer_eligible: bool,
    ime_observer_retry: std::cell::Cell<Instant>,
    preedit: std::cell::RefCell<Option<String>>,
    composition_source: std::cell::Cell<super::composition::CompositionSource>,
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
                if !text.SupportedTextSelection().is_ok_and(|s| {
                    s == SupportedTextSelection_Single || s == SupportedTextSelection_Multiple
                }) {
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
        let thread = unsafe { GetWindowThreadProcessId(control as _, null_mut()) };
        let ime_observer_eligible = matches!(&identity,
            PasteControlIdentity::NativeWindow { class_name, .. } if plain_edit_class(class_name));
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
            ime_observer: std::cell::RefCell::new(None),
            ime_observer_eligible,
            ime_observer_retry: std::cell::Cell::new(Instant::now()),
            preedit: std::cell::RefCell::new(None),
            composition_source: std::cell::Cell::new(
                super::composition::CompositionSource::TargetRead,
            ),
        })
    }
    pub(super) fn backend(&self) -> &'static str {
        if self.automation.is_some() {
            "uia-text-range"
        } else {
            "native-edit"
        }
    }
    #[cfg(feature = "native-test")]
    pub(super) fn inspect_synthetic_ranges(
        &self,
        expected: &str,
    ) -> Result<serde_json::Value, String> {
        let a = self
            .automation
            .as_ref()
            .ok_or("An UIA editor is required")?;
        unsafe {
            let value = a
                .element
                .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                .and_then(|p| p.CurrentValue())
                .map_err(|_| "Synthetic editor value cannot be verified")?;
            if value.to_string() != expected {
                let units = value.to_vec();
                let whitespace = units.iter().all(|u| matches!(*u, 9 | 10 | 13 | 32 | 160));
                return Err(format!(
                    "Editor value differs from the explicit synthetic value; no ranges inspected (units={}, whitespace={:?}, non_letter_units={:?})",
                    units.len(), whitespace.then_some(&units), units.iter().map(|u| if char::from_u32(u32::from(*u)).is_some_and(char::is_alphanumeric) { 0 } else { *u }).collect::<Vec<_>>()
                ));
            }
            let (pattern, doc) = live_document(a)?;
            let selected = single_selection(&pattern)?;
            let raw = text(&doc)?;
            let normalized = automation_snapshot(a)?;
            let selection_text = text(&selected)?;
            let find_observation = if selection_text.is_empty() {
                None
            } else {
                doc.FindText(&BSTR::from_wide(&selection_text), false, false)
                    .ok()
                    .and_then(|found| {
                        let before = doc.Clone().ok()?;
                        before.MoveEndpointByRange(TextPatternRangeEndpoint_End, &found, TextPatternRangeEndpoint_Start).ok()?;
                        Some(serde_json::json!({
                            "prefix_units": text(&before).ok()?.len(),
                            "candidate_units": text(&found).ok()?.len(),
                            "candidate_matches_selection": text(&found).ok()? == selection_text,
                            "start_relation": found.CompareEndpoints(TextPatternRangeEndpoint_Start, &selected, TextPatternRangeEndpoint_Start).ok(),
                            "end_relation": found.CompareEndpoints(TextPatternRangeEndpoint_End, &selected, TextPatternRangeEndpoint_End).ok(),
                        }))
                    })
            };
            let direct = a
                .uia
                .CreateTrueCondition()
                .and_then(|c| a.element.FindAll(TreeScope_Children, &c))
                .map_err(|_| "Direct child query failed")?;
            let mut direct_children = Vec::new();
            for i in 0..direct.Length().unwrap_or(0).min(12) {
                if let Ok(e) = direct.GetElement(i) {
                    direct_children.push(serde_json::json!({
                        "runtime_id": native::automation_runtime_id(&e),
                        "role": e.CurrentControlType().ok().map(|r| r.0),
                        "class": e.CurrentClassName().ok().map(|r| r.to_string()),
                        "name_matches_expected": e.CurrentName().is_ok_and(|s| s.to_string() == expected.trim()),
                    }));
                }
            }
            let walker = a
                .uia
                .RawViewWalker()
                .map_err(|_| "Editor tree unavailable")?;
            let mut child = walker.GetFirstChildElement(&a.element).ok();
            let mut children = Vec::new();
            while let Some(element) = child {
                if children.len() >= 12 {
                    break;
                }
                let child_range = pattern.RangeFromChild(&element).ok();
                let readonly = child_range.as_ref().and_then(|r| {
                    let mut v = r.GetAttributeValue(UIA_IsReadOnlyAttributeId).ok()?;
                    let result = (v.Anonymous.Anonymous.vt == VT_BOOL)
                        .then(|| v.Anonymous.Anonymous.Anonymous.boolVal.0 != 0);
                    let _ = VariantClear(&mut v);
                    result
                });
                children.push(serde_json::json!({
                    "runtime_id": native::automation_runtime_id(&element),
                    "encloses_document": doc.GetEnclosingElement().is_ok_and(|e| a.uia.CompareElements(&e, &element).is_ok_and(|same| same.as_bool())),
                    "role": element.CurrentControlType().ok().map(|r| r.0),
                    "aria": element.CurrentAriaRole().ok().map(|r| r.to_string()),
                    "aria_properties": element.CurrentAriaProperties().ok().map(|r| r.to_string()),
                    "class": element.CurrentClassName().ok().map(|r| r.to_string()),
                    "range_units": child_range.as_ref().and_then(|r| text(r).ok()).map(|s| s.len()),
                    "range_readonly": readonly,
                    "name_matches_expected": element.CurrentName().is_ok_and(|s| s.to_string() == expected.trim()),
                }));
                child = walker.GetNextSiblingElement(&element).ok();
            }
            let mut enclosing_chain = Vec::new();
            let mut ancestor = doc.GetEnclosingElement().ok();
            while let Some(element) = ancestor {
                if enclosing_chain.len() >= 8 {
                    break;
                }
                enclosing_chain.push(serde_json::json!({
                    "runtime_id": native::automation_runtime_id(&element),
                    "role": element.CurrentControlType().ok().map(|r| r.0),
                    "class": element.CurrentClassName().ok().map(|r| r.to_string()),
                    "name_matches_expected": element.CurrentName().is_ok_and(|s| s.to_string() == expected.trim()),
                    "first_child_id": walker.GetFirstChildElement(&element).ok().and_then(|e| native::automation_runtime_id(&e)),
                    "next_sibling_id": walker.GetNextSiblingElement(&element).ok().and_then(|e| native::automation_runtime_id(&e)),
                }));
                if a.uia
                    .CompareElements(&element, &a.element)
                    .is_ok_and(|same| same.as_bool())
                {
                    break;
                }
                ancestor = walker.GetParentElement(&element).ok();
            }
            Ok(serde_json::json!({
                "backend": self.backend(),
                "value_units": value.to_vec().len(),
                "raw_units": raw.len(),
                "raw_whitespace": raw.iter().all(|u| matches!(*u, 9 | 10 | 13 | 32 | 160)).then_some(&raw),
                "normalized_units": normalized.text.len(),
                "selection_start": normalized.selection.start,
                "selection_end": normalized.selection.end,
                "raw_selection_units": text(&selected)?.len(),
                "find_selection_observation": find_observation,
                "start_relation": selected.CompareEndpoints(TextPatternRangeEndpoint_Start, &doc, TextPatternRangeEndpoint_Start).ok(),
                "end_relation": selected.CompareEndpoints(TextPatternRangeEndpoint_End, &doc, TextPatternRangeEndpoint_End).ok(),
                "empty_value_caret": super::text_scope::empty_value_caret(&a.element, &selected),
                "empty_paragraph_caret": super::text_scope::empty_paragraph_caret(&a.element, &doc, &selected),
                "enclosing_role": doc.GetEnclosingElement().and_then(|e| e.CurrentControlType()).ok().map(|r| r.0),
                "aria_role": a.element.CurrentAriaRole().ok().map(|r| r.to_string()),
                "has_text_edit_pattern": a.edit.is_some(),
                "aria_properties": a.element.CurrentAriaProperties().ok().map(|r| r.to_string()),
                "name_matches_expected": a.element.CurrentName().is_ok_and(|s| s.to_string() == expected.trim()),
                "help_matches_expected": a.element.CurrentHelpText().is_ok_and(|s| s.to_string() == expected.trim()),
                "children": children,
                "enclosing_chain": enclosing_chain,
                "name_units": a.element.CurrentName().ok().map(|s| s.to_vec().len()),
                "direct_child_count": a.uia.CreateTrueCondition().and_then(|c| a.element.FindAll(TreeScope_Children, &c)).and_then(|c| c.Length()).ok(),
                "direct_children": direct_children,
                "enclosing_id": doc.GetEnclosingElement().ok().and_then(|e| native::automation_runtime_id(&e)),
                "empty_decorated": super::text_scope::empty_decorated_paragraph_caret(&a.uia, &a.element, &doc, &selected),
            }))
        }
    }
    // Called only after a coherent initial snapshot. Advertising selection is
    // intentionally distinct from observing successful exact selection.
    pub(super) fn capabilities(&self) -> TargetCapabilities {
        TargetCapabilities {
            backend: self.backend(),
            can_read_query: true,
            can_observe_selection: true,
            advertises_exact_selection: true,
            has_text_edit_pattern: self.automation.as_ref().is_some_and(|a| a.edit.is_some())
                || self.native_composition.is_some(),
            exact_selection_verified: false,
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
                focused_in_editor(a) == Some(true)
                    && a.element.CurrentIsEnabled().is_ok_and(|v| v.as_bool())
                    && a.element.CurrentIsPassword().is_ok_and(|v| !v.as_bool())
                    && writable(&a.element)
            }
        } else {
            native::focused_native_control_from_gui(hwnd, self.paste_target.process_id)
                == self.paste_target.focused_control
        }
    }
    /// A provider error is not proof that focus moved. Only a successful
    /// identity comparison can release protection within the original HWND.
    pub(super) fn definitely_left(&self) -> bool {
        let foreground = unsafe { GetForegroundWindow() };
        if foreground.is_null() {
            return false;
        }
        if foreground as isize != self.paste_target.window_id {
            return true;
        }
        if let Some(a) = &self.automation {
            unsafe { focused_in_editor(a) == Some(false) }
        } else {
            let identity = native::focused_native_control_from_gui(
                HWND(self.paste_target.window_id as _),
                self.paste_target.process_id,
            );
            identity.is_some() && identity != self.paste_target.focused_control
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
        *self.preedit.borrow_mut() = None;
        self.composition_source
            .set(super::composition::CompositionSource::TargetRead);
        #[cfg(feature = "native-test")]
        if super::diagnostics::COMPOSITION_UNAVAILABLE.load(std::sync::atomic::Ordering::Acquire) {
            return (IME_UNKNOWN, true);
        }
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
        // Standard Edit excludes its preedit from both WM_GETTEXT and UIA text.
        // Observe IMM on the actual editor thread; never treat a foreign HIMC
        // or an unrelated candidate window as a composition contract.
        if self.ime_observer_eligible && self.current() {
            let mut observer = self.ime_observer.borrow_mut();
            if observer.is_none() && Instant::now() >= self.ime_observer_retry.get() {
                self.ime_observer_retry
                    .set(Instant::now() + Duration::from_millis(500));
                *observer = super::ime_observer::Observer::new(
                    self.control,
                    self.paste_target.process_id,
                    self.paste_target.process_started_at,
                )
                .ok();
            }
            if let Some(sample) = observer.as_ref().and_then(|observer| observer.read()) {
                self.composition_source
                    .set(super::composition::CompositionSource::TargetThread);
                if sample.active {
                    *self.preedit.borrow_mut() = Some(sample.preedit);
                    return (IME_ACTIVE, true);
                }
                return (IME_CLEAR, true);
            }
            if observer.is_some() {
                return (IME_UNKNOWN, true);
            }
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
    pub(super) fn preview_query(
        &self,
        range: &QueryRange,
        snapshot: &ComposerSnapshot,
    ) -> Option<String> {
        let preedit = self.preedit.borrow();
        range
            .preview_composition(snapshot, preedit.as_deref()?)
            .ok()
    }
    pub(super) fn composition_source(&self) -> super::composition::CompositionSource {
        self.composition_source.get()
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
        #[cfg(feature = "native-test")]
        let selection_fault = super::diagnostics::selection_fault();
        #[cfg(not(feature = "native-test"))]
        let selection_fault = 0;
        if selection_fault == 1 {
            return Err(
                "Input refused exact range selection: 0x80004005 (native-test provider fault)"
                    .into(),
            );
        }
        if selection_fault != 2 {
            if let Some(a) = &self.automation {
                unsafe {
                    let (pattern, doc) = live_document(a)?;
                    let range = if span == expected.selection {
                        let selection = single_selection(&pattern)?;
                        let selection = super::text_scope::bounded_selection(
                            &a.uia, &a.element, &doc, &selection,
                        )?;
                        let snapshot = automation_snapshot(a)?;
                        if snapshot.selection != span || snapshot.text != expected.text {
                            return Err("Insertion point moved".into());
                        }
                        selection
                    } else {
                        let query = BSTR::from_wide(&expected.text[span.clone()]);
                        let remaining =
                            doc.Clone().map_err(|_| "Could not clone the text range")?;
                        let mut found = None;
                        for _ in 0..128 {
                            if !deadline.live() {
                                return Err("Range lookup timed out without replacing text".into());
                            }
                            let Ok(candidate) = remaining.FindText(&query, false, false) else {
                                break;
                            };
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
                        match found {
                            Some(range) => range,
                            None => range_from_offsets(&doc, &span, &expected.text, deadline)?,
                        }
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
        }
        #[cfg(feature = "native-test")]
        if selection_fault == 3 {
            // The real selection has changed, but its readback misses the
            // original request deadline. No replacement is performed here.
            std::thread::sleep(Duration::from_millis(950));
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
    pub(super) fn uses_native_range_replace(&self) -> bool {
        self.automation.is_none()
            && matches!(&self.paste_target.focused_control,
                Some(PasteControlIdentity::NativeWindow { class_name, .. })
                    if plain_edit_class(class_name))
    }
    pub(super) fn paste_selected(&self, retained_text: &str) -> Result<(), String> {
        if !self.current() {
            return Err("Input focus changed before paste".into());
        }
        native::modifiers_released()
            .map_err(|_| "ModifierKeysBusy: release held modifiers before inserting")?;
        if self.uses_native_range_replace() {
            // Standard Edit's WM_PASTE can remove the selection even when its
            // clipboard read fails. Use the frozen original text in the native
            // range operation instead; the clipboard may be read concurrently.
            // EM_REPLACESEL is a system-marshalled message below WM_USER. TRUE
            // records one Undo operation, with no separate delete or rewrite.
            let mut replacement = self.normalize_inserted(retained_text);
            replacement.push(0);
            message(
                self.control,
                EM_REPLACESEL,
                1,
                replacement.as_ptr() as isize,
            )
            .map(|_| ())
        } else if self.automation.is_some() {
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
fn plain_edit_class(class_name: &str) -> bool {
    let class_name = class_name.to_ascii_lowercase();
    class_name == "edit" || class_name.starts_with("windowsforms10.edit.")
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

/// Chromium FindText can return shifted endpoints after paragraph separators.
/// Locate boundaries through provider units, measuring each candidate against
/// the frozen UTF-16 prefix. Never assume a Character unit is one UTF-16 unit.
unsafe fn point_at_offset(
    doc: &IUIAutomationTextRange,
    expected: &[u16],
    offset: usize,
    deadline: &Deadline,
) -> Result<IUIAutomationTextRange, String> {
    let mut low = 0i32;
    let mut high = expected.len() as i32;
    let mut units = offset as i32;
    for _ in 0..24 {
        if !deadline.live() || low > high {
            break;
        }
        let point = doc.Clone().map_err(|_| "Cannot clone a query endpoint")?;
        point
            .MoveEndpointByRange(
                TextPatternRangeEndpoint_End,
                doc,
                TextPatternRangeEndpoint_Start,
            )
            .map_err(|_| "Cannot initialize a query endpoint")?;
        point
            .MoveEndpointByUnit(TextPatternRangeEndpoint_Start, TextUnit_Character, units)
            .map_err(|_| "Provider cannot map query character boundaries")?;
        if point
            .CompareEndpoints(
                TextPatternRangeEndpoint_Start,
                doc,
                TextPatternRangeEndpoint_End,
            )
            .map_err(|_| "Cannot bound a query endpoint")?
            > 0
        {
            high = units - 1;
        } else {
            let prefix = doc.Clone().map_err(|_| "Cannot clone a query prefix")?;
            prefix
                .MoveEndpointByRange(
                    TextPatternRangeEndpoint_End,
                    &point,
                    TextPatternRangeEndpoint_Start,
                )
                .map_err(|_| "Cannot measure a query endpoint")?;
            let before = text(&prefix)?;
            if !expected.starts_with(&before) {
                return Err("Input changed while mapping a query endpoint".into());
            }
            match before.len().cmp(&offset) {
                std::cmp::Ordering::Equal => return Ok(point),
                std::cmp::Ordering::Less => low = units + 1,
                std::cmp::Ordering::Greater => high = units - 1,
            }
        }
        units = low + (high - low) / 2;
    }
    Err("Query boundary is not a verified provider character boundary".into())
}

unsafe fn range_from_offsets(
    doc: &IUIAutomationTextRange,
    span: &Range<usize>,
    expected: &[u16],
    deadline: &Deadline,
) -> Result<IUIAutomationTextRange, String> {
    if span.start > span.end || span.end > expected.len() || text(doc)? != expected {
        return Err("Input changed before query range mapping".into());
    }
    let start = point_at_offset(doc, expected, span.start, deadline)?;
    let end = point_at_offset(doc, expected, span.end, deadline)?;
    start
        .MoveEndpointByRange(
            TextPatternRangeEndpoint_End,
            &end,
            TextPatternRangeEndpoint_Start,
        )
        .map_err(|_| "Cannot join query endpoints")?;
    let suffix = doc.Clone().map_err(|_| "Cannot clone query suffix")?;
    suffix
        .MoveEndpointByRange(
            TextPatternRangeEndpoint_Start,
            &start,
            TextPatternRangeEndpoint_End,
        )
        .map_err(|_| "Cannot verify query suffix")?;
    if text(&start)? != expected[span.clone()] || text(&suffix)? != expected[span.end..] {
        return Err("Mapped query range does not preserve the frozen text".into());
    }
    Ok(start)
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
    if focused_in_editor(target) != Some(true) {
        return Err("Original editor identity changed".into());
    }
    let (pattern, from_child) = super::text_scope::resolve(&target.uia, &target.element)?;
    let doc = super::text_scope::document(&pattern, &target.element, from_child)?;
    Ok((pattern, doc))
}
/// A provider child may be reconstructed while the verified editor root lives.
/// Unknown ancestry is a suspended capability, never proof of a different input.
unsafe fn focused_in_editor(target: &AutomationTarget) -> Option<bool> {
    let pid = target.element.CurrentProcessId().ok()?;
    let mut current = target.uia.GetFocusedElement().ok()?;
    let walker = target.uia.RawViewWalker().ok()?;
    for _ in 0..32 {
        if target
            .uia
            .CompareElements(&current, &target.element)
            .ok()?
            .as_bool()
        {
            return Some(true);
        }
        if current.CurrentProcessId().ok()? != pid {
            return Some(false);
        }
        // Another editable control is not a replacement paragraph/provider.
        let role = current.CurrentControlType().ok()?;
        if role == UIA_EditControlTypeId
            || role == UIA_ComboBoxControlTypeId
            || role == UIA_WindowControlTypeId
            || (role == UIA_DocumentControlTypeId
                && writable(&current)
                && current.CurrentIsKeyboardFocusable().ok()?.as_bool())
        {
            return Some(false);
        }
        current = walker.GetParentElement(&current).ok()?;
    }
    None
}
unsafe fn automation_snapshot(target: &AutomationTarget) -> Result<ComposerSnapshot, String> {
    let (pattern, doc) = live_document(target)?;
    let selected = single_selection(&pattern)?;
    let selected =
        super::text_scope::bounded_selection(&target.uia, &target.element, &doc, &selected)?;
    if super::text_scope::empty_value_caret(&target.element, &selected)
        || super::text_scope::empty_paragraph_caret(&target.element, &doc, &selected)
        || super::text_scope::empty_decorated_paragraph_caret(
            &target.uia,
            &target.element,
            &doc,
            &selected,
        )
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
    if selection.is_empty()
        && all.get(start).is_some_and(|u| matches!(*u, 10 | 13))
        && super::text_scope::ambiguous_interior_paragraph_caret(
            &target.uia,
            &target.element,
            &pattern,
            &doc,
            &selected,
        )
    {
        return Err("Empty paragraph boundary is ambiguous; use independent search".into());
    }
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
