#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::{Duration, Instant};

    use echo_storage::SharedClipboardStore;
    use tempfile::TempDir;

    const INGEST_SOURCE: &str =
        include_str!("../../../../crates/echo-engine/src/ingest.rs");

    #[test]
    fn storage_open_runs_startup_maintenance_once() {
        let root = TempDir::new().unwrap();
        let store = SharedClipboardStore::open(root.path()).unwrap();

        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            let samples = store
                .metrics_snapshot()
                .into_iter()
                .find(|metric| metric.operation == "maintenance_reconcile")
                .map(|metric| metric.samples)
                .unwrap_or(0);
            if samples >= 2 {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }

        let samples = store
            .metrics_snapshot()
            .into_iter()
            .find(|metric| metric.operation == "maintenance_reconcile")
            .map(|metric| metric.samples)
            .unwrap_or(0);
        assert_eq!(
            samples, 1,
            "storage startup scheduled {samples} maintenance passes"
        );
    }

    #[test]
    fn obsolete_clipboard_startup_hook_is_absent() {
        assert!(
            !INGEST_SOURCE.contains("start_maintenance"),
            "ClipboardService::start_maintenance remains a dead compatibility API"
        );
    }
}
