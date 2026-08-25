#[cfg(test)]
mod tests {
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

        let mut store = ClipboardStore::open_in_memory_for_tests("r1-storage-memory").unwrap();
        let history_id = store.record_capture(capture).unwrap().id;
        let saved = store.move_history_to_favorite(history_id).unwrap();

        assert_eq!(saved.name, "report.txt");
    }
}
