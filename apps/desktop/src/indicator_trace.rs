//! Bounded, content-free indicator diagnostics on the existing domain worker.
use std::{
    collections::VecDeque,
    io::Write,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::SyncSender,
        Mutex, OnceLock,
    },
};

struct Sink {
    sender: SyncSender<(u64, crate::service::Work)>,
    queue: Mutex<VecDeque<serde_json::Value>>,
    wake_pending: AtomicBool,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn logs_append_across_sessions_and_rotate_with_a_fixed_disk_budget() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs");
        let payload = "x".repeat(280_000);
        for session in 0..2 {
            let mut writer = Writer::new(path.clone());
            for index in 0..12 {
                writer
                    .append(&serde_json::json!({"session":session,"index":index,"test":payload}))
                    .unwrap();
            }
        }
        let files: Vec<_> = std::fs::read_dir(&path)
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(files.len(), 4);
        for file in files {
            assert!(file.metadata().unwrap().len() <= 1024 * 1024);
            for line in std::fs::read_to_string(file.path()).unwrap().lines() {
                serde_json::from_str::<serde_json::Value>(line).unwrap();
            }
        }
        let latest = std::fs::read_to_string(path.join("input-indicator.jsonl")).unwrap();
        let last: serde_json::Value = serde_json::from_str(latest.lines().last().unwrap()).unwrap();
        assert_eq!(last["session"], 1);
        assert_eq!(last["index"], 11);
    }
    #[test]
    fn unavailable_log_directory_does_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("not-a-directory");
        std::fs::write(&path, "fixture").unwrap();
        Writer::new(path).write(serde_json::json!({"event":"fixture"}));
    }
}
static SINK: OnceLock<Sink> = OnceLock::new();
static SEQUENCE: AtomicU64 = AtomicU64::new(0);
static DROPPED: AtomicU64 = AtomicU64::new(0);

pub fn initialize(sender: SyncSender<(u64, crate::service::Work)>) {
    let _ = SINK.set(Sink {
        sender,
        queue: Mutex::new(VecDeque::new()),
        wake_pending: AtomicBool::new(false),
    });
    record(
        "session-start",
        serde_json::json!({"version":env!("CARGO_PKG_VERSION"),"flip_ms":80}),
    );
}
pub fn record(event: &str, details: serde_json::Value) {
    let Some(sink) = SINK.get() else {
        return;
    };
    let dropped = DROPPED.swap(0, Ordering::Relaxed);
    let entry = serde_json::json!({"utc":chrono::Utc::now().to_rfc3339(),"utc_ms":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis(),
        "pid":std::process::id(),"sequence":SEQUENCE.fetch_add(1,Ordering::Relaxed),"dropped":dropped,"event":event,"details":details});
    {
        let mut queue = sink.queue.lock().unwrap_or_else(|e| e.into_inner());
        if queue.len() >= 128 {
            DROPPED.fetch_add(dropped + 1, Ordering::Relaxed);
            return;
        }
        queue.push_back(entry);
    }
    if !sink.wake_pending.swap(true, Ordering::AcqRel)
        && sink
            .sender
            .try_send((0, crate::service::Work::Wake))
            .is_err()
    {
        sink.wake_pending.store(false, Ordering::Release);
    }
}

pub struct Writer {
    root: PathBuf,
    file: Option<std::fs::File>,
    bytes: u64,
}
impl Writer {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            file: None,
            bytes: 0,
        }
    }
    pub fn drain(&mut self) {
        let Some(sink) = SINK.get() else {
            return;
        };
        let events = {
            let mut queue = sink.queue.lock().unwrap_or_else(|e| e.into_inner());
            sink.wake_pending.store(false, Ordering::Release);
            std::mem::take(&mut *queue)
        };
        for event in events {
            self.write(event);
        }
    }
    pub fn write(&mut self, event: serde_json::Value) {
        if let Err(error) = self.append(&event) {
            // Diagnostics failure must never interrupt input or the application.
            self.file = None;
            eprintln!("Input indicator log unavailable: {error}");
        }
    }
    fn append(&mut self, event: &serde_json::Value) -> std::io::Result<()> {
        const LIMIT: u64 = 1024 * 1024;
        std::fs::create_dir_all(&self.root)?;
        let path = self.root.join("input-indicator.jsonl");
        if self.file.is_none() {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?;
            self.bytes = file.metadata()?.len();
            self.file = Some(file);
        }
        let line = format!("{event}\n");
        if self.bytes + line.len() as u64 > LIMIT {
            self.file.take();
            let oldest = self.root.join("input-indicator.3.jsonl");
            if oldest.exists() {
                std::fs::remove_file(oldest)?;
            }
            for n in (1..3).rev() {
                let old = self.root.join(format!("input-indicator.{n}.jsonl"));
                if old.exists() {
                    std::fs::rename(
                        old,
                        self.root.join(format!("input-indicator.{}.jsonl", n + 1)),
                    )?;
                }
            }
            std::fs::rename(&path, self.root.join("input-indicator.1.jsonl"))?;
            self.file = Some(
                std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)?,
            );
            self.bytes = 0;
        }
        self.file.as_mut().unwrap().write_all(line.as_bytes())?;
        self.bytes += line.len() as u64;
        Ok(())
    }
}
