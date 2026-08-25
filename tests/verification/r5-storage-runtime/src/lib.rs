#[cfg(test)]
mod tests {
    use echo_storage::SharedClipboardStore;
    use tempfile::{tempdir_in, TempDir};

    const INGEST_SOURCE: &str =
        include_str!("../../../../crates/echo-engine/src/ingest.rs");

    fn disk_tempdir() -> TempDir {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.local/test-tmp/echo-storage");
        std::fs::create_dir_all(&root).unwrap();
        tempdir_in(root).unwrap()
    }

    #[test]
    fn storage_open_runs_startup_maintenance_once() {
        let root = disk_tempdir();
        let store = SharedClipboardStore::open(root.path()).unwrap();
        store.shutdown().unwrap();

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
