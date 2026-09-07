//! Keep UIA reads and replacements inside the activated editor, not its page.
//! GetEnclosingElement may be a paragraph inside an editor, not the editor itself.
use windows::Win32::UI::Accessibility::*;

unsafe fn inside(
    uia: &IUIAutomation,
    child: &IUIAutomationElement,
    parent: &IUIAutomationElement,
) -> bool {
    let Ok(walker) = uia.RawViewWalker() else {
        return false;
    };
    let mut current = child.clone();
    for _ in 0..32 {
        if uia
            .CompareElements(&current, parent)
            .is_ok_and(|v| v.as_bool())
        {
            return true;
        }
        let Ok(next) = walker.GetParentElement(&current) else {
            return false;
        };
        current = next;
    }
    false
}
unsafe fn encloses_only_editor(
    uia: &IUIAutomation,
    range: &IUIAutomationTextRange,
    editor: &IUIAutomationElement,
) -> bool {
    range
        .GetEnclosingElement()
        .is_ok_and(|enclosing| inside(uia, &enclosing, editor))
}
/// Resolve a scoped provider once; the range itself is fetched fresh on each read.
pub(super) unsafe fn resolve(
    uia: &IUIAutomation,
    editor: &IUIAutomationElement,
) -> Result<(IUIAutomationTextPattern, bool), String> {
    if let Ok(pattern) = editor.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId) {
        if pattern
            .DocumentRange()
            .is_ok_and(|r| encloses_only_editor(uia, &r, editor))
        {
            return Ok((pattern, false));
        }
        if pattern
            .RangeFromChild(editor)
            .is_ok_and(|r| encloses_only_editor(uia, &r, editor))
        {
            return Ok((pattern, true));
        }
    }
    // Some web editors expose the TextPattern on an ancestor, but support the
    // documented RangeFromChild contract for the focused editable descendant.
    let walker = uia
        .RawViewWalker()
        .map_err(|_| "Text provider tree is unavailable")?;
    let mut parent = editor.clone();
    for _ in 0..16 {
        let Ok(next) = walker.GetParentElement(&parent) else {
            break;
        };
        parent = next;
        if parent.CurrentProcessId().ok() != editor.CurrentProcessId().ok() {
            break;
        }
        if let Ok(pattern) =
            parent.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
        {
            if pattern
                .RangeFromChild(editor)
                .is_ok_and(|r| encloses_only_editor(uia, &r, editor))
            {
                return Ok((pattern, true));
            }
        }
        if parent.CurrentControlType().ok() == Some(UIA_WindowControlTypeId) {
            break;
        }
    }
    Err("This editor does not expose an independently scoped text range".into())
}
pub(super) unsafe fn document(
    pattern: &IUIAutomationTextPattern,
    editor: &IUIAutomationElement,
    from_child: bool,
) -> Result<IUIAutomationTextRange, String> {
    if from_child {
        pattern.RangeFromChild(editor)
    } else {
        pattern.DocumentRange()
    }
    .map_err(|_| "The editor-scoped text range is unavailable".into())
}
pub(super) unsafe fn validate_selection(
    doc: &IUIAutomationTextRange,
    selected: &IUIAutomationTextRange,
) -> Result<(), String> {
    let start = selected
        .CompareEndpoints(
            TextPatternRangeEndpoint_Start,
            doc,
            TextPatternRangeEndpoint_Start,
        )
        .map_err(|_| "Selection start cannot be related to the editor")?;
    let end = selected
        .CompareEndpoints(
            TextPatternRangeEndpoint_End,
            doc,
            TextPatternRangeEndpoint_End,
        )
        .map_err(|_| "Selection end cannot be related to the editor")?;
    if start < 0 || end > 0 {
        return Err(format!(
            "The selection is outside the activated editor (start={start}, end={end})"
        ));
    }
    Ok(())
}
pub(super) unsafe fn empty_value_caret(
    editor: &IUIAutomationElement,
    selected: &IUIAutomationTextRange,
) -> bool {
    if !editor
        .CurrentControlType()
        .is_ok_and(|t| t == UIA_EditControlTypeId || t == UIA_ComboBoxControlTypeId)
    {
        return false;
    }
    if selected
        .CompareEndpoints(
            TextPatternRangeEndpoint_Start,
            selected,
            TextPatternRangeEndpoint_End,
        )
        .ok()
        != Some(0)
    {
        return false;
    }
    editor
        .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
        .ok()
        .filter(|v| {
            v.CurrentIsReadOnly()
                .is_ok_and(|readonly| !readonly.as_bool())
        })
        .and_then(|v| v.CurrentValue().ok())
        .is_some_and(|v| v.is_empty())
}

/// A sole empty HTML paragraph has a terminal newline at caret zero.
/// Never trim real whitespace, selected newlines, or native text controls.
pub(super) unsafe fn empty_paragraph_caret(
    editor: &IUIAutomationElement,
    doc: &IUIAutomationTextRange,
    selected: &IUIAutomationTextRange,
) -> bool {
    let role = editor
        .CurrentAriaRole()
        .map(|v| v.to_string())
        .unwrap_or_default();
    if role != "textbox" && role != "searchbox" {
        return false;
    }
    if selected
        .CompareEndpoints(
            TextPatternRangeEndpoint_Start,
            selected,
            TextPatternRangeEndpoint_End,
        )
        .ok()
        != Some(0)
        || selected
            .CompareEndpoints(
                TextPatternRangeEndpoint_Start,
                doc,
                TextPatternRangeEndpoint_Start,
            )
            .ok()
            != Some(0)
    {
        return false;
    }
    let Ok(raw) = doc.GetText(3) else {
        return false;
    };
    if raw.to_vec() != [10] && raw.to_vec() != [13, 10] {
        return false;
    }
    let Ok(enclosing) = doc.GetEnclosingElement() else {
        return false;
    };
    if enclosing.CurrentControlType().ok() == editor.CurrentControlType().ok() {
        return false;
    }
    true
}

/// Compare semantic text positions when a provider uses parent/leaf boundary
/// anchors interchangeably. The selection must remain inside the same editor,
/// and all text pieces must reconstruct that editor exactly before normalization.
pub(super) unsafe fn bounded_selection(
    uia: &IUIAutomation,
    editor: &IUIAutomationElement,
    doc: &IUIAutomationTextRange,
    selected: &IUIAutomationTextRange,
) -> Result<IUIAutomationTextRange, String> {
    if validate_selection(doc, selected).is_ok() {
        return Ok(selected.clone());
    }
    if !encloses_only_editor(uia, selected, editor) {
        return Err("Selection left the activated editor".into());
    }
    let before = doc
        .Clone()
        .map_err(|_| "Cannot inspect selection boundary")?;
    let after = doc
        .Clone()
        .map_err(|_| "Cannot inspect selection boundary")?;
    before
        .MoveEndpointByRange(
            TextPatternRangeEndpoint_End,
            selected,
            TextPatternRangeEndpoint_Start,
        )
        .map_err(|_| "Invalid selection boundary")?;
    after
        .MoveEndpointByRange(
            TextPatternRangeEndpoint_Start,
            selected,
            TextPatternRangeEndpoint_End,
        )
        .map_err(|_| "Invalid selection boundary")?;
    let get = |r: &IUIAutomationTextRange| {
        r.GetText((echo_engine::MAX_COMPOSER_UNITS + 1) as i32)
            .map(|s| s.to_vec())
            .map_err(|_| "Selection boundary text unavailable")
    };
    let all = get(doc)?;
    let left = get(&before)?;
    let middle = get(selected)?;
    let right = get(&after)?;
    if all.len() > echo_engine::MAX_COMPOSER_UNITS
        || left
            .iter()
            .chain(&middle)
            .chain(&right)
            .copied()
            .collect::<Vec<_>>()
            != all
    {
        return Err("Selection boundary does not reconstruct this editor".into());
    }
    let normalized = doc.Clone().map_err(|_| "Cannot normalize editor range")?;
    if left.is_empty() {
        normalized.MoveEndpointByRange(
            TextPatternRangeEndpoint_Start,
            doc,
            TextPatternRangeEndpoint_Start,
        )
    } else {
        normalized.MoveEndpointByRange(
            TextPatternRangeEndpoint_Start,
            selected,
            TextPatternRangeEndpoint_Start,
        )
    }
    .map_err(|_| "Cannot normalize range start")?;
    if right.is_empty() {
        normalized.MoveEndpointByRange(
            TextPatternRangeEndpoint_End,
            doc,
            TextPatternRangeEndpoint_End,
        )
    } else {
        normalized.MoveEndpointByRange(
            TextPatternRangeEndpoint_End,
            selected,
            TextPatternRangeEndpoint_End,
        )
    }
    .map_err(|_| "Cannot normalize range end")?;
    if get(&normalized)? != middle {
        return Err("Normalized selection differs; nothing was changed".into());
    }
    Ok(normalized)
}
