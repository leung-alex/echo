#[cfg(test)]
mod tests {
    use echo_engine::{
        fingerprint, ClipboardRepresentation, ContentType, NormalizedCapture, SavedItemDraft,
        SourceContext,
    };
    use echo_storage::ClipboardStore;
    use rusqlite::Connection;
    use tempfile::TempDir;

    #[test]
    fn unsaving_a_history_item_removes_its_saved_item_fts_document() {
        let root = TempDir::new().unwrap();
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
        let entry = store.entry(history_id).unwrap().unwrap().entry;
        let payload = store.entry_payload(history_id).unwrap();
        let saved = store
            .save_history_item(SavedItemDraft::from_history(&entry), payload)
            .unwrap();

        assert_eq!(store.list_saved_items("document", 20).unwrap().len(), 1);
        assert!(store.unsave_history_item(history_id).unwrap());
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
