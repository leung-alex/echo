#[cfg(test)]
mod tests {
    use echo_engine::{
        fingerprint, ClipboardRepresentation, ContentType, NormalizedCapture, SourceContext,
    };
    use echo_storage::ClipboardStore;
    use rusqlite::Connection;
    use tempfile::{tempdir_in, TempDir};

    fn disk_tempdir() -> TempDir {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../.local/test-tmp/echo-storage");
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
