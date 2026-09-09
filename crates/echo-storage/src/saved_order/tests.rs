use crate::ClipboardStore;
use echo_engine::{FavoriteDraft, SpaceAction, SpaceCommand, SpaceId};

fn content(text: &str) -> FavoriteDraft {
    FavoriteDraft {
        content: text.into(),
        name: Some(text.into()),
        tags: vec![],
        icon_key: None,
    }
}

#[test]
fn prepend_and_duplicate_preserve_existing_rows_payloads_and_pagination() {
    let mut store = ClipboardStore::open_in_memory_for_tests("saved-order-regression").unwrap();
    let first = store.create_favorite(content("first original")).unwrap();
    let second = store.create_favorite(content("second original")).unwrap();
    store.connection.execute_batch(
        "CREATE TEMP TRIGGER forbid_retained_order_rewrite BEFORE UPDATE OF favorite_order ON saved_items
         BEGIN SELECT RAISE(ABORT, 'retained order must not be rewritten by prepend'); END;"
    ).unwrap();
    let newest = store.create_favorite(content("newest original")).unwrap();
    let revision = store
        .list_spaces()
        .unwrap()
        .into_iter()
        .find(|space| space.id == SpaceId::FAVORITES)
        .unwrap()
        .revision;
    let duplicate = store
        .apply_space_command(SpaceCommand {
            space_id: Some(SpaceId::FAVORITES),
            expected_revision: Some(revision),
            request_id: uuid::Uuid::new_v4().to_string(),
            action: SpaceAction::DuplicateItem(first.id),
        })
        .unwrap()
        .created_item
        .unwrap();
    assert_eq!(
        store
            .saved_item(first.id)
            .unwrap()
            .unwrap()
            .item
            .favorite_order,
        first.favorite_order
    );
    assert_eq!(
        store
            .saved_item(second.id)
            .unwrap()
            .unwrap()
            .item
            .favorite_order,
        second.favorite_order
    );
    assert_eq!(
        store.saved_item_payload(duplicate).unwrap()[0].bytes,
        b"first original"
    );
    let page = store.list_saved_items_page("", 2, None).unwrap();
    assert_eq!(
        page.items.iter().map(|item| item.id).collect::<Vec<_>>(),
        [duplicate, newest.id]
    );
    let tail = store
        .list_saved_items_page("", 2, page.next_cursor)
        .unwrap();
    assert_eq!(
        tail.items.iter().map(|item| item.id).collect::<Vec<_>>(),
        [second.id, first.id]
    );
    assert!(tail.next_cursor.is_none());
    store
        .connection
        .execute_batch("DROP TRIGGER forbid_retained_order_rewrite")
        .unwrap();
    store
        .reorder_favorites(&[first.id, second.id, newest.id, duplicate])
        .unwrap();
    assert_eq!(
        store
            .list_saved_items("", 10)
            .unwrap()
            .iter()
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        [first.id, second.id, newest.id, duplicate]
    );
    let again = store
        .create_favorite(content("after explicit reorder"))
        .unwrap();
    assert_eq!(store.list_saved_items("", 10).unwrap()[0].id, again.id);
}

#[test]
fn exhausted_order_fails_without_changing_retained_content() {
    let mut store = ClipboardStore::open_in_memory_for_tests("saved-order-exhaustion").unwrap();
    let original = store.create_favorite(content("retained content")).unwrap();
    store
        .connection
        .execute(
            "UPDATE saved_items SET favorite_order=? WHERE id=?",
            rusqlite::params![i64::MIN, original.id],
        )
        .unwrap();
    assert!(store
        .create_favorite(content("must not be inserted"))
        .is_err());
    let items = store.list_saved_items("", 10).unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].favorite_order, i64::MIN);
    assert_eq!(
        store.saved_item_payload(original.id).unwrap()[0].bytes,
        b"retained content"
    );
}

#[test]
fn moving_history_to_favorites_does_not_rewrite_existing_order() {
    use echo_engine::{
        fingerprint, ClipboardRepresentation, ContentType, NormalizedCapture, SourceContext,
    };
    let mut store = ClipboardStore::open_in_memory_for_tests("saved-order-history").unwrap();
    let original = store.create_favorite(content("original favorite")).unwrap();
    let payload = ClipboardRepresentation {
        format: "text".into(),
        mime_type: "text/plain".into(),
        bytes: b"retained capture".to_vec(),
    };
    let recorded = store
        .record_capture(NormalizedCapture {
            sequence: 1,
            source: SourceContext::default(),
            content_type: ContentType::Text,
            preview_text: Some("retained capture".into()),
            searchable_text: Some("retained capture".into()),
            sanitized_html: None,
            fingerprint: fingerprint(std::slice::from_ref(&payload)),
            representations: vec![payload],
        })
        .unwrap();
    store.connection.execute_batch(
        "CREATE TEMP TRIGGER forbid_retained_order_rewrite BEFORE UPDATE OF favorite_order ON saved_items
         BEGIN SELECT RAISE(ABORT, 'retained order must not be rewritten by move'); END;"
    ).unwrap();
    let moved = store.move_history_to_favorite(recorded.id).unwrap();
    assert_eq!(
        store
            .saved_item(original.id)
            .unwrap()
            .unwrap()
            .item
            .favorite_order,
        original.favorite_order
    );
    assert_eq!(store.list_saved_items("", 10).unwrap()[0].id, moved.id);
    assert_eq!(
        store.saved_item_payload(moved.id).unwrap()[0].bytes,
        b"retained capture"
    );
    assert!(store.entry(recorded.id).unwrap().is_none());
}
