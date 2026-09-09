//! Opt-in fixture timing. Validate once before starting the internal clock;
//! buffer events in memory and write them only after dismissal.
use serde_json::{json, Value};
use std::{cell::RefCell, path::PathBuf, sync::OnceLock, time::Instant};

struct Trace {
    start: Instant,
    events: Vec<Value>,
}
thread_local! { static TRACE: RefCell<Option<Trace>> = const { RefCell::new(None) }; }
static ROOT: OnceLock<Option<PathBuf>> = OnceLock::new();
fn root() -> Option<&'static PathBuf> {
    ROOT.get_or_init(|| {
        if std::env::var("ECHO_WINDOWS_ACCEPTANCE").as_deref() != Ok("1") {
            return None;
        }
        let root = PathBuf::from(std::env::var_os("ECHO_POPUP_TIMING_DIR")?);
        let data = PathBuf::from(std::env::var_os("ECHO_DATA_DIR")?);
        let marker: Value =
            serde_json::from_slice(&std::fs::read(data.join("synthetic-fixture.json")).ok()?)
                .ok()?;
        (root.is_dir() && marker["synthetic"] == true && marker["capture_enabled"] == false)
            .then_some(root)
    })
    .as_ref()
}
pub fn begin() {
    if root().is_none() {
        return;
    }
    TRACE.with(|slot| {
        *slot.borrow_mut() = Some(Trace {
            start: Instant::now(),
            events: Vec::with_capacity(128),
        })
    });
    mark("hotkey_received");
}
pub fn enabled() -> bool {
    root().is_some()
}
pub fn mark(name: &'static str) {
    event(name, Value::Null);
}
pub struct Span {
    name: &'static str,
    start: Option<Instant>,
}
pub fn span(name: &'static str) -> Span {
    Span {
        name,
        start: TRACE.with(|slot| slot.borrow().as_ref().map(|_| Instant::now())),
    }
}
impl Drop for Span {
    fn drop(&mut self) {
        if let Some(start) = self.start {
            event(
                self.name,
                json!({"duration_us": start.elapsed().as_micros() as u64}),
            );
        }
    }
}
pub fn event(name: &'static str, detail: Value) {
    TRACE.with(|slot| {
        if let Some(trace) = slot.borrow_mut().as_mut() {
            if trace.events.len() < 256 { trace.events.push(json!({"event":name,"us":trace.start.elapsed().as_micros() as u64,"detail":detail})); }
        }
    });
}
pub fn finish() {
    TRACE.with(|slot| {
        if let Some(trace) = slot.borrow_mut().take() {
            if let Some(root) = root() {
                let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
                let value = json!({"pid":std::process::id(),"clock":"Instant; microseconds since desktop hotkey handler","events":trace.events});
                let _ = std::fs::write(root.join(format!("trace-{}-{stamp}.json", std::process::id())), value.to_string());
            }
        }
    });
}
