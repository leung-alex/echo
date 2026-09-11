//! Keep UIA reads and replacements inside the activated editor, not its page.
//! GetEnclosingElement may be a paragraph inside an editor, not the editor itself.
use windows::core::Interface;
use windows::Win32::UI::Accessibility::*;

/// Editor Kit exposes an empty composer as read-only hints followed by
/// three editable zero-width leaves. Typing removes the hints and puts the
/// caret BEFORE the retained leaves. Project only this verified empty shape;
/// literal zero-width characters in ordinary documents are never stripped.
pub(super) unsafe fn empty_editor_kit_snapshot(
    uia: &IUIAutomation,
    editor: &IUIAutomationElement,
    pattern: &IUIAutomationTextPattern,
    doc: &IUIAutomationTextRange,
    selected: &IUIAutomationTextRange,
    snapshot: &echo_engine::ComposerSnapshot,
) -> Option<echo_engine::ComposerSnapshot> {
    if !is_editor_kit(editor) {
        return None;
    }
    let walker = uia.RawViewWalker().ok()?;
    let paragraph = walker.GetFirstChildElement(editor).ok()?;
    if paragraph.CurrentControlType().ok()? != UIA_GroupControlTypeId
        || walker.GetNextSiblingElement(&paragraph).is_ok()
        || !uia
            .CompareElements(&doc.GetEnclosingElement().ok()?, &paragraph)
            .ok()?
            .as_bool()
    {
        return None;
    }
    let mut child = walker.GetFirstChildElement(&paragraph).ok();
    let mut leaves = Vec::new();
    while let Some(e) = child {
        if leaves.len() == 8 {
            return None;
        }
        child = walker.GetNextSiblingElement(&e).ok();
        leaves.push(e);
    }
    if leaves.len() < 4 {
        return None;
    }
    let first_caret = leaves.len() - 3;
    // Voice-input hints can be a readonly group beside the readonly text
    // placeholder. Only those verified siblings may disappear on typing.
    for hint in &leaves[..first_caret] {
        let role = hint.CurrentControlType().ok()?;
        if (role != UIA_TextControlTypeId && role != UIA_GroupControlTypeId)
            || !hint
                .CurrentAriaProperties()
                .ok()?
                .to_string()
                .split(';')
                .any(|p| p == "readonly=true")
        {
            return None;
        }
    }
    if !uia
        .CompareElements(&selected.GetEnclosingElement().ok()?, &leaves[first_caret])
        .ok()?
        .as_bool()
    {
        return None;
    }
    for leaf in &leaves[first_caret..] {
        if leaf.CurrentControlType().ok()? != UIA_TextControlTypeId
            || pattern.RangeFromChild(leaf).ok()?.GetText(2).ok()?.to_vec() != [0x200b]
        {
            return None;
        }
    }
    let prefix = doc.Clone().ok()?;
    let first = pattern.RangeFromChild(&leaves[first_caret]).ok()?;
    prefix
        .MoveEndpointByRange(
            TextPatternRangeEndpoint_End,
            &first,
            TextPatternRangeEndpoint_Start,
        )
        .ok()?;
    let prefix = prefix.GetText(1024).ok()?.to_vec();
    let placeholder = prefix.strip_suffix(&[10])?;
    project_empty_editor_kit(snapshot, placeholder)
}

pub(super) unsafe fn is_editor_kit(editor: &IUIAutomationElement) -> bool {
    editor.CurrentControlType().ok() == Some(UIA_GroupControlTypeId)
        && editor.CurrentClassName().is_ok_and(|class| {
            let class = class.to_string();
            ["editor-kit-container", "innerdocbody"]
                .iter()
                .all(|token| class.split_whitespace().any(|c| c == *token))
        })
}

/// Readback expectation only: the clipboard payload is never changed. Editor
/// Kit exposes a zero-width leaf before each paragraph separator in pasted text.
pub(super) fn editor_kit_inserted(text: &str) -> Vec<u16> {
    text.replace("\r\n", "\n")
        .replace('\n', "\u{200b}\n")
        .encode_utf16()
        .collect()
}

fn project_empty_editor_kit(
    snapshot: &echo_engine::ComposerSnapshot,
    placeholder: &[u16],
) -> Option<echo_engine::ComposerSnapshot> {
    const TAIL: [u16; 5] = [0x200b, 10, 0x200b, 10, 0x200b];
    let caret = placeholder.len() + 2;
    let mut raw = placeholder.to_vec();
    raw.push(10);
    raw.extend(TAIL);
    if placeholder.is_empty() || snapshot.text != raw || snapshot.selection != (caret..caret) {
        return None;
    }
    Some(echo_engine::ComposerSnapshot {
        text: TAIL.to_vec(),
        selection: 0..0,
    })
}

#[cfg(test)]
mod editor_kit_tests {
    use super::*;
    use echo_engine::{ComposerSnapshot, QueryRange};

    #[test]
    fn empty_placeholder_transition_preserves_query_and_replacement_boundaries() {
        let placeholder: Vec<u16> = "Synthetic hint".encode_utf16().collect();
        let mut raw = placeholder.clone();
        raw.extend([10, 0x200b, 10, 0x200b, 10, 0x200b]);
        let caret = placeholder.len() + 2;
        let empty = ComposerSnapshot {
            text: raw,
            selection: caret..caret,
        };
        let projected = project_empty_editor_kit(&empty, &placeholder).unwrap();
        let mut range = QueryRange::begin(&projected).unwrap();
        for query in ["s", "select", "selec", "", "中文"] {
            let mut text: Vec<u16> = query.encode_utf16().collect();
            let end = text.len();
            text.extend_from_slice(&projected.text);
            let next = if query.is_empty() {
                projected.clone()
            } else {
                ComposerSnapshot {
                    text,
                    selection: end..end,
                }
            };
            range.observe(&next).unwrap();
            assert_eq!(range.query(), query);
            assert_eq!(range.seal(&next, range.revision()).unwrap(), 0..end);
        }
        let mut changed = empty.clone();
        changed.selection = 0..0;
        assert!(project_empty_editor_kit(&changed, &placeholder).is_none());
        changed = empty.clone();
        changed.text.push(65);
        assert!(project_empty_editor_kit(&changed, &placeholder).is_none());
        changed = empty;
        changed.selection.end += 1;
        assert!(project_empty_editor_kit(&changed, &placeholder).is_none());
    }

    #[test]
    fn editor_kit_readback_keeps_literal_zero_width_characters() {
        assert_eq!(
            String::from_utf16(&editor_kit_inserted("a\u{200b}\r\nb\nc")).unwrap(),
            "a\u{200b}\u{200b}\nb\u{200b}\nc"
        );
    }
}

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
pub(super) unsafe fn encloses_only_editor(
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

/// Some Chromium providers collapse an interior empty p/br into the previous
/// paragraph's separator. Its first input then introduces a new separator that
/// looks like query text. Refuse that unproven mapping before any replacement.
pub(super) unsafe fn ambiguous_interior_paragraph_caret(
    uia: &IUIAutomation,
    editor: &IUIAutomationElement,
    pattern: &IUIAutomationTextPattern,
    doc: &IUIAutomationTextRange,
    selected: &IUIAutomationTextRange,
) -> bool {
    if editor
        .CurrentClassName()
        .ok()
        .is_none_or(|s| s.to_string() != "ProseMirror")
        || selected
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
            .is_none_or(|v| v <= 0)
        || selected
            .CompareEndpoints(
                TextPatternRangeEndpoint_End,
                doc,
                TextPatternRangeEndpoint_End,
            )
            .ok()
            .is_none_or(|v| v >= 0)
    {
        return false;
    }
    let Ok(walker) = uia.RawViewWalker() else {
        return false;
    };
    let mut child = walker.GetFirstChildElement(editor).ok();
    for _ in 0..64 {
        let Some(element) = child else {
            break;
        };
        if element.CurrentControlType().ok() == Some(UIA_GroupControlTypeId) {
            if let Ok(range) = pattern.RangeFromChild(&element) {
                if range
                    .GetText(3)
                    .is_ok_and(|v| matches!(v.to_string().as_str(), "\n" | "\r\n"))
                    && range
                        .CompareEndpoints(
                            TextPatternRangeEndpoint_Start,
                            selected,
                            TextPatternRangeEndpoint_Start,
                        )
                        .ok()
                        == Some(0)
                {
                    return true;
                }
            }
        }
        child = walker.GetNextSiblingElement(&element).ok();
    }
    false
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

/// ProseMirror's empty-paragraph decoration can expose generated CSS text through
/// Chromium UIA. Recognize the complete paragraph/range structure, including the
/// sole generated text leaf used when the empty <br> is hidden. A label, class name,
/// newline, or empty selection alone never authorizes trimming.
pub(super) unsafe fn empty_decorated_paragraph_caret(
    uia: &IUIAutomation,
    editor: &IUIAutomationElement,
    doc: &IUIAutomationTextRange,
    selected: &IUIAutomationTextRange,
) -> bool {
    if editor.CurrentAriaRole().map(|r| r.to_string()).as_deref() != Ok("textbox")
        || selected
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
    let Ok(name) = editor.CurrentName() else {
        return false;
    };
    let name = name.to_vec();
    if name.is_empty() || name.len() > 512 {
        return false;
    }
    let Ok(raw) = doc.GetText(516) else {
        return false;
    };
    let raw = raw.to_vec();
    if raw.is_empty() || raw.len() > 515 {
        return false;
    }
    if !editor.CurrentClassName().is_ok_and(|c| {
        c.to_string()
            .split_whitespace()
            .any(|token| token == "ProseMirror")
    }) {
        return false;
    }
    // Chromium's FindAll children can flatten ignored paragraph groups into
    // their text leaves. The raw tree preserves the paragraph identity.
    let Ok(walker) = uia.RawViewWalker() else {
        return false;
    };
    let Ok(paragraph) = walker.GetFirstChildElement(editor) else {
        return false;
    };
    let mut sibling = std::ptr::null_mut();
    let sibling_result =
        (walker.vtable().GetNextSiblingElement)(walker.as_raw(), paragraph.as_raw(), &mut sibling);
    if !sibling.is_null() {
        drop(IUIAutomationElement::from_raw(sibling));
        return false;
    }
    if sibling_result.is_err() {
        return false;
    }
    if paragraph.CurrentControlType().ok() != Some(UIA_GroupControlTypeId)
        || !paragraph.CurrentClassName().is_ok_and(|c| {
            c.to_string()
                .split_whitespace()
                .any(|token| token == "placeholder")
        })
    {
        return false;
    }
    if !editor
        .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
        .and_then(|p| p.CurrentValue())
        .is_ok_and(|value| value.to_vec() == raw)
    {
        return false;
    }
    let Ok(enclosing) = doc.GetEnclosingElement() else {
        return false;
    };
    if uia
        .CompareElements(&enclosing, &paragraph)
        .is_ok_and(|same| same.as_bool())
    {
        return raw.strip_prefix(&[10]) == Some(name.as_slice())
            || raw.strip_prefix(&[13, 10]) == Some(name.as_slice());
    }
    // A decoration with a hidden <br> can be the entire document range. Chromium
    // retains its anonymous CSS-generated group between the paragraph and text.
    // Require that exact single-child chain; real text alongside the decoration
    // introduces another sibling or a wider enclosing range and is retained.
    if raw.len() > 512
        || raw.iter().any(|unit| matches!(*unit, 10 | 13))
        || enclosing.CurrentControlType().ok() != Some(UIA_TextControlTypeId)
        || !enclosing
            .CurrentName()
            .is_ok_and(|label| label.to_vec() == raw)
    {
        return false;
    }
    let Ok(decoration) = walker.GetFirstChildElement(&paragraph) else {
        return false;
    };
    let mut next_decoration = std::ptr::null_mut();
    let result = (walker.vtable().GetNextSiblingElement)(
        walker.as_raw(),
        decoration.as_raw(),
        &mut next_decoration,
    );
    if !next_decoration.is_null() {
        drop(IUIAutomationElement::from_raw(next_decoration));
        return false;
    }
    if result.is_err() {
        return false;
    }
    let leaf = match decoration.CurrentControlType().ok() {
        Some(role)
            if role == UIA_GroupControlTypeId
                && decoration.CurrentName().is_ok_and(|name| name.is_empty())
                && decoration
                    .CurrentClassName()
                    .is_ok_and(|class| class.is_empty()) =>
        {
            let Ok(leaf) = walker.GetFirstChildElement(&decoration) else {
                return false;
            };
            leaf
        }
        // The observed ChatGPT provider flattens the CSS group itself. Keep
        // this less informative shape scoped to its verified editor identity;
        // arbitrary ProseMirror paragraphs with a literal text leaf must not
        // become empty just because they happen to use a placeholder class.
        Some(role)
            if role == UIA_TextControlTypeId
                && editor
                    .CurrentAutomationId()
                    .is_ok_and(|id| id.to_string() == "prompt-textarea")
                && name == "Chat with ChatGPT".encode_utf16().collect::<Vec<_>>() =>
        {
            decoration
        }
        _ => return false,
    };
    if !uia
        .CompareElements(&leaf, &enclosing)
        .is_ok_and(|same| same.as_bool())
    {
        return false;
    }
    let mut next_leaf = std::ptr::null_mut();
    let result =
        (walker.vtable().GetNextSiblingElement)(walker.as_raw(), leaf.as_raw(), &mut next_leaf);
    if !next_leaf.is_null() {
        drop(IUIAutomationElement::from_raw(next_leaf));
        return false;
    }
    result.is_ok()
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
