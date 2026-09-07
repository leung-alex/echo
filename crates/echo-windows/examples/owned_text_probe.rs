//! Read-only UIA probe restricted to a recorded, capture-disabled test fixture.
use windows::{
    core::Interface,
    Win32::{System::Com::*, UI::Accessibility::*},
};
fn run() -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    if std::env::var("ECHO_WINDOWS_ACCEPTANCE").as_deref() != Ok("1") {
        return Err("Test authorization required".into());
    }
    let mut args = std::env::args().skip(1);
    let root = std::path::PathBuf::from(args.next().ok_or("Missing evidence root")?);
    let pid: u32 = args.next().ok_or("Missing owned pid")?.parse()?;
    let marker: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("data/synthetic-fixture.json"))?)?;
    if marker["synthetic"] != true || marker["capture_enabled"] != false {
        return Err("Not an isolated fixture".into());
    }
    let records: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(root.join("owned-processes.json"))?)?;
    let snapshot = echo_windows::focus::FocusSnapshot::capture();
    if snapshot.process_id != pid {
        return Err("Owned fixture is not focused".into());
    }
    if !records.iter().any(|r| {
        r["pid"].as_u64() == Some(pid as u64)
            && r["started_filetime"].as_u64() == Some(snapshot.process_started_at)
    }) {
        return Err("Process identity does not match the recorded fixture".into());
    }
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
        let u: IUIAutomation = CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER)?;
        if let Ok(settings) = u.cast::<IUIAutomation2>() {
            settings.SetAutoSetFocus(false)?;
            settings.SetTransactionTimeout(200)?;
        }
        let e = u.GetFocusedElement()?;
        if e.CurrentProcessId()? as u32 != pid || e.CurrentIsPassword()?.as_bool() {
            return Err("Unexpected or protected target".into());
        }
        if args.next().as_deref() == Some("hold") {
            std::fs::write(root.join("probe.ready"), "ready")?;
            let clock = std::time::Instant::now();
            while !root.join("probe.go").exists()
                && clock.elapsed() < std::time::Duration::from_secs(20)
            {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            let f = u.GetFocusedElement()?;
            return Ok(
                serde_json::json!({"same":u.CompareElements(&e,&f).ok().map(|v|v.as_bool()),"old_focus":e.CurrentHasKeyboardFocus().ok().map(|v|v.as_bool()),"new_focus":f.CurrentHasKeyboardFocus().ok().map(|v|v.as_bool()),"old_value":e.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId).ok().and_then(|v|v.CurrentValue().ok()).map(|v|v.to_string()),"new_value":f.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId).ok().and_then(|v|v.CurrentValue().ok()).map(|v|v.to_string()),"old_text":e.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId).and_then(|t|t.DocumentRange()).and_then(|r|r.GetText(200)).map(|s|s.to_string()).map_err(|e|e.to_string()),"new_text":f.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId).and_then(|t|t.DocumentRange()).and_then(|r|r.GetText(200)).map(|s|s.to_string()).map_err(|e|e.to_string())}),
            );
        }
        let value = e
            .GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
            .ok()
            .and_then(|v| v.CurrentValue().ok())
            .map(|s| s.to_string());
        let tp = e.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)?;
        let doc = tp.DocumentRange()?;
        let sel = tp.GetSelection()?;
        if sel.Length()? != 1 {
            return Err("Not one selection".into());
        }
        let selected = sel.GetElement(0)?;
        let prefix = doc.Clone()?;
        let suffix = doc.Clone()?;
        prefix.MoveEndpointByRange(
            TextPatternRangeEndpoint_End,
            &selected,
            TextPatternRangeEndpoint_Start,
        )?;
        suffix.MoveEndpointByRange(
            TextPatternRangeEndpoint_Start,
            &selected,
            TextPatternRangeEndpoint_End,
        )?;
        let info = |range: &IUIAutomationTextRange| -> serde_json::Value {
            let enclosing = range.GetEnclosingElement().ok();
            serde_json::json!({"text":range.GetText(4096).ok().map(|s|s.to_string()),"enclosing_name":enclosing.as_ref().and_then(|n|n.CurrentName().ok()).map(|s|s.to_string()),"same_element":enclosing.as_ref().is_some_and(|n|u.CompareElements(n,&e).is_ok_and(|v|v.as_bool()))})
        };
        let child = tp.RangeFromChild(&e);
        let mut parents = Vec::new();
        let walker = u.RawViewWalker()?;
        let mut parent = e.clone();
        for _ in 0..5 {
            let Ok(next) = walker.GetParentElement(&parent) else {
                break;
            };
            parent = next;
            if let Ok(pattern) =
                parent.GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
            {
                let range = pattern.RangeFromChild(&e);
                parents.push(serde_json::json!({"name":parent.CurrentName().ok().map(|s|s.to_string()),"range":range.as_ref().ok().map(&info),"error":range.err().map(|e|e.to_string())}));
            }
        }
        let offsets = serde_json::json!({
            "start":selected.CompareEndpoints(TextPatternRangeEndpoint_Start,&doc,TextPatternRangeEndpoint_Start).ok(),
            "end":selected.CompareEndpoints(TextPatternRangeEndpoint_End,&doc,TextPatternRangeEndpoint_End).ok(),
            "collapsed":selected.CompareEndpoints(TextPatternRangeEndpoint_Start,&selected,TextPatternRangeEndpoint_End).ok(),
            "aria":e.CurrentAriaRole().ok().map(|s|s.to_string()),
        });
        let result = serde_json::json!({"offsets":offsets,"value":value,"document":info(&doc),"prefix":prefix.GetText(4096)?.to_string(),"selected":selected.GetText(4096)?.to_string(),"suffix":suffix.GetText(4096)?.to_string(),"focused":e.CurrentHasKeyboardFocus()?.as_bool(),"control_type":e.CurrentControlType()?.0,"from_self":child.as_ref().ok().map(&info),"from_self_error":child.err().map(|e|e.to_string()),"parents":parents});
        if !snapshot.still_current() {
            return Err("Focus moved during the test read".into());
        }
        Ok(result)
    }
}
fn main() {
    match run() {
        Ok(value) => println!("{}", serde_json::json!({"status":"PASS","value":value})),
        Err(e) => {
            eprintln!(
                "{}",
                serde_json::json!({"status":"FAIL","error":e.to_string()})
            );
            std::process::exit(1);
        }
    }
}
