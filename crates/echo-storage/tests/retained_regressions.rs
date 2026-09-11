// Behavior regressions migrated from the retired standalone verification harnesses.
#[cfg(test)]
mod r1_storage_harness {
    use echo_engine::{
        fingerprint, ClipboardRepresentation, ContentType, NormalizedCapture, SourceContext,
    };
    use echo_storage::ClipboardStore;

    #[test]
    fn saved_file_name_uses_the_file_name_not_the_source_path() {
        let path = r"C:\Users\Verifier\report.txt";
        let representation = ClipboardRepresentation {
            format: "files".to_owned(),
            mime_type: "text/uri-list".to_owned(),
            bytes: format!("{path}\n").into_bytes(),
        };
        let capture = NormalizedCapture {
            sequence: 1,
            source: SourceContext::default(),
            content_type: ContentType::Files,
            preview_text: Some(path.to_owned()),
            searchable_text: Some(path.to_owned()),
            sanitized_html: None,
            fingerprint: fingerprint(std::slice::from_ref(&representation)),
            representations: vec![representation],
        };

        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/test-tmp/echo-storage");
        std::fs::create_dir_all(&base).unwrap();
        let root = tempfile::tempdir_in(base).unwrap();
        let mut store = ClipboardStore::open(root.path()).unwrap();
        let history_id = store.record_capture(capture).unwrap().id;
        let saved = store.move_history_to_favorite(history_id).unwrap();

        assert_eq!(saved.name, "report.txt");
    }
}

#[cfg(test)]
mod r4_saved_item_fts {
    use echo_engine::{
        fingerprint, ClipboardRepresentation, ContentType, NormalizedCapture, SourceContext,
    };
    use echo_storage::ClipboardStore;
    use rusqlite::Connection;
    use tempfile::{tempdir_in, TempDir};

    fn disk_tempdir() -> TempDir {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/test-tmp/echo-storage");
        std::fs::create_dir_all(&root).unwrap();
        tempdir_in(root).unwrap()
    }

    #[test]
    fn deleting_a_saved_item_removes_its_saved_item_fts_document() {
        let root = disk_tempdir();
        let representation = ClipboardRepresentation {
            format: "text".to_owned(),
            mime_type: "text/plain;charset=utf-8".to_owned(),
            bytes: b"saved search document".to_vec(),
        };
        let capture = NormalizedCapture {
            sequence: 1,
            source: SourceContext::default(),
            content_type: ContentType::Text,
            preview_text: Some("saved search document".to_owned()),
            searchable_text: Some("saved search document".to_owned()),
            sanitized_html: None,
            fingerprint: fingerprint(std::slice::from_ref(&representation)),
            representations: vec![representation],
        };
        let mut store = ClipboardStore::open(root.path()).unwrap();
        let history_id = store.record_capture(capture).unwrap().id;
        let saved = store.move_history_to_favorite(history_id).unwrap();

        assert_eq!(store.list_saved_items("document", 20).unwrap().len(), 1);
        assert!(store.delete_favorite(saved.id).unwrap());
        drop(store);

        let connection = Connection::open(root.path().join("echo.sqlite3")).unwrap();
        let orphaned: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM saved_items_fts WHERE saved_item_id = ?",
                [saved.id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            orphaned, 0,
            "unsave left an orphaned SavedItem FTS document"
        );
    }
}

#[cfg(test)]
mod r5_storage_runtime {
    use echo_storage::SharedClipboardStore;
    use tempfile::{tempdir_in, TempDir};

    fn disk_tempdir() -> TempDir {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/test-tmp/echo-storage");
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
}
