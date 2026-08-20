#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use echo_engine::{
        fingerprint, ClipboardRepresentation, ContentType, NormalizedCapture, SavedItemDraft,
        SourceContext,
    };
    use echo_storage::ClipboardStore;

    #[test]
    fn saved_file_name_uses_the_file_name_not_the_source_path() {
        let root = std::env::temp_dir().join(format!(
            "echo-r1-verifier-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();

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

        let mut store = ClipboardStore::open(&root).unwrap();
        let history_id = store.record_capture(capture).unwrap().id;
        let entry = store.entry(history_id).unwrap().unwrap().entry;
        let payload = store.entry_payload(history_id).unwrap();
        let saved = store
            .save_history_item(SavedItemDraft::from_history(&entry), payload)
            .unwrap();

        assert_eq!(saved.name, "report.txt");
        let _ = std::fs::remove_dir_all(root);
    }
}
