//! Opt-in, bounded lifecycle evidence for isolated synthetic runs. No user content.
use std::{
    io::{BufWriter, Write},
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex, OnceLock,
    },
};

static ROOT: OnceLock<Option<PathBuf>> = OnceLock::new();
static EVENTS: AtomicU64 = AtomicU64::new(0);
static INSTANCE: OnceLock<String> = OnceLock::new();
static WRITER: OnceLock<Mutex<Option<BufWriter<std::fs::File>>>> = OnceLock::new();

pub fn record(state: &'static str, details: serde_json::Value) {
    let root = ROOT.get_or_init(|| {
        if std::env::var("ECHO_WINDOWS_ACCEPTANCE").as_deref() != Ok("1") {
            return None;
        }
        let root = PathBuf::from(std::env::var_os("ECHO_MEMORY_TRACE_DIR")?);
        let data = PathBuf::from(std::env::var_os("ECHO_DATA_DIR")?);
        let marker: serde_json::Value =
            serde_json::from_slice(&std::fs::read(data.join("synthetic-fixture.json")).ok()?)
                .ok()?;
        (root.is_dir() && marker["synthetic"] == true).then_some(root)
    });
    let Some(root) = root else {
        return;
    };
    let index = EVENTS.fetch_add(1, Ordering::Relaxed);
    if index >= 10000 {
        return;
    }
    let instance = INSTANCE.get_or_init(|| {
        format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        )
    });
    let event = serde_json::json!({"schema":"echo.memory.lifecycle.v1", "pid":std::process::id(),
        "process_instance":instance,
        "utc_ns":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos().to_string(),
        "state":state, "details":details});
    // Reopening a file for every frame puts filesystem/antivirus latency on
    // the UI clock. Keep one bounded buffer; lifecycle barriers flush all prior
    // frame samples so a live collector can still verify ready/reclaimed states.
    let writer = WRITER.get_or_init(|| {
        Mutex::new(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(root.join(format!("lifecycle-{}.jsonl", std::process::id())))
                .ok()
                .map(|file| BufWriter::with_capacity(64 * 1024, file)),
        )
    });
    if let Some(file) = writer.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let _ = writeln!(file, "{event}");
        if !matches!(state, "frame_redraw" | "frame_presented") || index % 64 == 0 {
            let _ = file.flush();
        }
    }
}
