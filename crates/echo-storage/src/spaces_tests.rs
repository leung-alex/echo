use super::*;
use echo_engine::*;
fn store() -> ClipboardStore {
    ClipboardStore::open_in_memory_for_tests(tempfile::tempdir().unwrap().keep()).unwrap()
}
fn draft(name: &str) -> SpaceDraft {
    SpaceDraft {
        title: name.into(),
        icon_key: None,
        accent_key: "amber".into(),
        description: String::new(),
    }
}
fn content(text: &str) -> FavoriteDraft {
    FavoriteDraft {
        name: Some(text.into()),
        content: text.into(),
        tags: vec![],
        icon_key: None,
    }
}
fn count(s: &ClipboardStore) -> i64 {
    s.connection
        .query_row("SELECT COUNT(*) FROM saved_items", [], |r| r.get(0))
        .unwrap()
}
fn revision(s: &ClipboardStore, id: SpaceId) -> i64 {
    s.list_spaces()
        .unwrap()
        .into_iter()
        .find(|x| x.id == id)
        .unwrap()
        .revision
}
fn command(s: &mut ClipboardStore, id: SpaceId, action: SpaceAction) -> SpaceMutationResult {
    let revision = revision(s, id);
    s.apply_space_command(SpaceCommand {
        space_id: Some(id),
        expected_revision: Some(revision),
        request_id: Uuid::new_v4().to_string(),
        action,
    })
    .unwrap()
}
fn create(s: &mut ClipboardStore, name: &str) -> SpaceId {
    s.apply_space_command(SpaceCommand {
        space_id: None,
        expected_revision: None,
        request_id: Uuid::new_v4().to_string(),
        action: SpaceAction::Create(draft(name)),
    })
    .unwrap()
    .created_space
    .unwrap()
}
#[test]
fn clear_favorites_covers_unloaded_items_but_preserves_shared_content_and_history() {
    let mut s = store();
    let work = create(&mut s, "Work");
    let shared = s
        .create_favorite(content("shared searchable content"))
        .unwrap();
    command(&mut s, work, SpaceAction::AddItems(vec![shared.id]));
    let only_work = command(&mut s, work, SpaceAction::CreateItem(content("work only")))
        .created_item
        .unwrap();
    let mut exclusive = Vec::new();
    for text in [
        "matching exclusive",
        "unloaded exclusive",
        "hidden exclusive",
    ] {
        exclusive.push(s.create_favorite(content(text)).unwrap().id);
    }
    let rep = ClipboardRepresentation {
        format: "text".into(),
        mime_type: "text/plain;charset=utf-8".into(),
        bytes: b"history survives".to_vec(),
    };
    s.record_capture(NormalizedCapture {
        sequence: 1,
        source: SourceContext::default(),
        content_type: ContentType::Text,
        preview_text: Some("history survives".into()),
        searchable_text: Some("history survives".into()),
        sanitized_html: None,
        fingerprint: fingerprint(std::slice::from_ref(&rep)),
        representations: vec![rep],
    })
    .unwrap();
    let before_work = s.list_space_items(work, "", 100, None).unwrap();
    let filtered = s
        .list_space_items(SpaceId::FAVORITES, "matching", 1, None)
        .unwrap();
    assert_eq!(filtered.page.items.len(), 1);
    let result = command(&mut s, SpaceId::FAVORITES, SpaceAction::ClearFavorites);
    assert_eq!(result.affected_spaces, [SpaceId::FAVORITES]);
    assert_eq!(
        s.list_space_items(SpaceId::FAVORITES, "", 1, None)
            .unwrap()
            .total,
        0
    );
    for id in exclusive {
        assert!(s.saved_item(id).unwrap().is_none());
        assert_eq!(
            s.connection
                .query_row(
                    "SELECT COUNT(*) FROM saved_items_fts WHERE saved_item_id=?",
                    [id],
                    |r| r.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }
    assert_eq!(s.spaces_for_item(shared.id).unwrap(), [work]);
    assert_eq!(
        s.saved_item_payload(shared.id).unwrap()[0].bytes,
        b"shared searchable content"
    );
    assert!(s.saved_item(only_work).unwrap().is_some());
    let after_work = s.list_space_items(work, "", 100, None).unwrap();
    assert_eq!(after_work.revision, before_work.revision);
    assert_eq!(
        after_work
            .page
            .items
            .iter()
            .map(|i| i.id)
            .collect::<Vec<_>>(),
        before_work
            .page
            .items
            .iter()
            .map(|i| i.id)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        s.list_space_items(work, "shared", 100, None).unwrap().total,
        1
    );
    assert_eq!(
        s.connection
            .query_row("SELECT COUNT(*) FROM clipboard_entries", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert!(s.maintenance_pending().unwrap());
}

#[test]
fn clear_favorites_rejects_stale_confirmation_and_replay_never_deletes_new_content() {
    let mut s = store();
    let work = create(&mut s, "Work");
    let old_revision = revision(&s, SpaceId::FAVORITES);
    s.create_favorite(content("added after confirmation"))
        .unwrap();
    let mut clear = SpaceCommand {
        space_id: Some(SpaceId::FAVORITES),
        expected_revision: Some(old_revision),
        request_id: "clear-favorites".into(),
        action: SpaceAction::ClearFavorites,
    };
    assert!(matches!(
        s.apply_space_command(clear.clone()),
        Err(StorageError::Space(SpaceError::Conflict))
    ));
    assert_eq!(count(&s), 1);
    clear.expected_revision = Some(revision(&s, SpaceId::FAVORITES));
    let result = s.apply_space_command(clear.clone()).unwrap();
    let added = s.create_favorite(content("created after clear")).unwrap();
    assert_eq!(s.apply_space_command(clear).unwrap(), result);
    assert!(s.saved_item(added.id).unwrap().is_some());
    for id in [SpaceId::HISTORY, work] {
        let request = SpaceCommand {
            space_id: Some(id),
            expected_revision: Some(revision(&s, id)),
            request_id: format!("wrong-clear-{}", id.0),
            action: SpaceAction::ClearFavorites,
        };
        assert!(s.apply_space_command(request).is_err());
    }
    command(&mut s, SpaceId::FAVORITES, SpaceAction::ClearFavorites);
    let empty_revision = revision(&s, SpaceId::FAVORITES);
    assert!(
        command(&mut s, SpaceId::FAVORITES, SpaceAction::ClearFavorites)
            .affected_spaces
            .is_empty()
    );
    assert_eq!(revision(&s, SpaceId::FAVORITES), empty_revision);
}

#[test]
fn clear_favorites_rolls_back_payload_search_and_memberships_on_failure() {
    let mut s = store();
    let item = s.create_favorite(content("rollback searchable")).unwrap();
    let old_revision = revision(&s, SpaceId::FAVORITES);
    s.connection
        .execute_batch(
            "CREATE TRIGGER fail_favorites_clear BEFORE DELETE ON space_memberships
         WHEN OLD.space_id=2 BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;",
        )
        .unwrap();
    assert!(s
        .apply_space_command(SpaceCommand {
            space_id: Some(SpaceId::FAVORITES),
            expected_revision: Some(old_revision),
            request_id: "rollback-favorites-clear".into(),
            action: SpaceAction::ClearFavorites,
        })
        .is_err());
    assert_eq!(
        s.saved_item_payload(item.id).unwrap()[0].bytes,
        b"rollback searchable"
    );
    assert_eq!(s.spaces_for_item(item.id).unwrap(), [SpaceId::FAVORITES]);
    assert_eq!(
        s.list_space_items(SpaceId::FAVORITES, "rollback", 10, None)
            .unwrap()
            .total,
        1
    );
    assert_eq!(revision(&s, SpaceId::FAVORITES), old_revision);
}
#[test]
fn custom_spaces_are_not_automatically_favorites() {
    let mut s = store();
    let id = create(&mut s, "Work");
    let item = command(
        &mut s,
        id,
        SpaceAction::CreateItem(content("work@example.test")),
    )
    .created_item
    .unwrap();
    assert_eq!(s.spaces_for_item(item).unwrap(), [id]);
    assert!(s
        .list_space_items(SpaceId::FAVORITES, "", 50, None)
        .unwrap()
        .page
        .items
        .is_empty());
    assert_eq!(
        s.list_space_items(id, "", 50, None).unwrap().page.items[0].id,
        item
    );
}
#[test]
fn membership_and_shared_edits_have_no_duplicate_payloads() {
    let mut s = store();
    let work = create(&mut s, "Work");
    let personal = create(&mut s, "Personal");
    let item = s.create_favorite(content("shared value")).unwrap();
    command(&mut s, work, SpaceAction::AddItems(vec![item.id, item.id]));
    command(&mut s, personal, SpaceAction::AddItems(vec![item.id]));
    assert_eq!(s.spaces_for_item(item.id).unwrap().len(), 3);
    assert_eq!(count(&s), 1);
    let before = revision(&s, work);
    s.update_favorite(
        item.id,
        FavoriteUpdate {
            name: Some("Email".into()),
            editable_text: Some("new@example.test".into()),
            tags: vec![],
            icon_key: None,
        },
    )
    .unwrap();
    assert!(revision(&s, work) > before);
    assert_eq!(
        s.saved_item_payload(item.id).unwrap()[0].bytes,
        b"new@example.test"
    );
}
#[test]
fn deleting_space_rehomes_only_exclusive_items_atomically() {
    let mut s = store();
    let work = create(&mut s, "Work");
    let other = create(&mut s, "Other");
    let exclusive = command(&mut s, work, SpaceAction::CreateItem(content("exclusive")))
        .created_item
        .unwrap();
    let shared = command(&mut s, work, SpaceAction::CreateItem(content("shared")))
        .created_item
        .unwrap();
    command(&mut s, other, SpaceAction::AddItems(vec![shared]));
    let result = command(&mut s, work, SpaceAction::Delete);
    assert_eq!(result.migrated_count, 1);
    assert_eq!(s.spaces_for_item(exclusive).unwrap(), [SpaceId::FAVORITES]);
    assert_eq!(s.spaces_for_item(shared).unwrap(), [other]);
    assert_eq!(
        s.saved_item_payload(exclusive).unwrap()[0].bytes,
        b"exclusive"
    );
    let new = create(&mut s, "Work");
    assert!(new.0 > other.0, "deleted IDs must never be reused");
}
#[test]
fn failed_space_deletion_rolls_back_rehoming() {
    let mut s = store();
    let work = create(&mut s, "Work");
    let item = command(&mut s, work, SpaceAction::CreateItem(content("retained")))
        .created_item
        .unwrap();
    s.connection.execute_batch("CREATE TRIGGER fail_space_delete BEFORE DELETE ON spaces BEGIN SELECT RAISE(ABORT,'test'); END;").unwrap();
    let r = revision(&s, work);
    assert!(s
        .apply_space_command(SpaceCommand {
            space_id: Some(work),
            expected_revision: Some(r),
            request_id: "delete-test".into(),
            action: SpaceAction::Delete
        })
        .is_err());
    assert_eq!(s.spaces_for_item(item).unwrap(), [work]);
    assert_eq!(revision(&s, work), r);
}
#[test]
fn stale_command_and_stale_cursor_cannot_mutate_or_mix_results() {
    let mut s = store();
    let work = create(&mut s, "Work");
    let old = revision(&s, work);
    for i in 0..4 {
        command(
            &mut s,
            work,
            SpaceAction::CreateItem(content(&format!("value {i}"))),
        );
    }
    let page = s.list_space_items(work, "", 2, None).unwrap();
    assert_eq!(page.page.items.len(), 2);
    let cursor = page.page.next_cursor.unwrap();
    command(&mut s, work, SpaceAction::CreateItem(content("new")));
    assert!(matches!(
        s.list_space_items(work, "", 2, Some(cursor)),
        Err(StorageError::Space(SpaceError::StaleCursor))
    ));
    assert!(matches!(
        s.apply_space_command(SpaceCommand {
            space_id: Some(work),
            expected_revision: Some(old),
            request_id: "stale".into(),
            action: SpaceAction::Delete
        }),
        Err(StorageError::Space(SpaceError::Conflict))
    ));
}
#[test]
fn space_reorder_operates_on_unloaded_rows_without_losing_them() {
    let mut s = store();
    let work = create(&mut s, "Work");
    let mut ids = Vec::new();
    for i in 0..160 {
        ids.push(
            command(
                &mut s,
                work,
                SpaceAction::CreateItem(content(&format!("saved content {i}"))),
            )
            .created_item
            .unwrap(),
        );
    }
    command(
        &mut s,
        work,
        SpaceAction::ReorderItem {
            id: ids[0],
            before: Some(ids[159]),
            delta: 0,
        },
    );
    let first = s.list_space_items(work, "", 50, None).unwrap();
    assert_eq!(first.page.items[0].id, ids[0]);
    let mut cursor = None;
    let mut all = Vec::new();
    loop {
        let page = s.list_space_items(work, "", 50, cursor).unwrap();
        all.extend(page.page.items.iter().map(|i| i.id));
        cursor = page.page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    all.sort_unstable();
    ids.sort_unstable();
    assert_eq!(all, ids);
}
#[test]
fn search_is_scoped_for_fts_short_unicode_and_special_input() {
    let mut s = store();
    let a = create(&mut s, "A");
    let b = create(&mut s, "B");
    command(
        &mut s,
        a,
        SpaceAction::CreateItem(content("Hello 中文 item 100%")),
    );
    command(&mut s, b, SpaceAction::CreateItem(content("Hello hidden")));
    for query in ["Hello", "He", "中文", "%"] {
        let p = s.list_space_items(a, query, 50, None).unwrap();
        assert_eq!(p.page.items.len(), 1, "{query}");
        assert!(!p.page.items[0]
            .editable_text
            .as_deref()
            .unwrap()
            .contains("hidden"));
    }
    assert!(s.list_space_items(a, "\" OR * -", 50, None).is_ok());
}
#[test]
fn replay_is_idempotent_and_reuse_with_different_payload_is_rejected() {
    let mut s = store();
    let work = create(&mut s, "Work");
    let mut cmd = SpaceCommand {
        space_id: Some(work),
        expected_revision: Some(revision(&s, work)),
        request_id: "create:1".into(),
        action: SpaceAction::CreateItem(content("one")),
    };
    let a = s.apply_space_command(cmd.clone()).unwrap();
    let b = s.apply_space_command(cmd.clone()).unwrap();
    assert_eq!(a, b);
    assert_eq!(count(&s), 1);
    cmd.action = SpaceAction::CreateItem(content("two"));
    assert!(matches!(
        s.apply_space_command(cmd),
        Err(StorageError::Space(SpaceError::InvalidRequest))
    ));
}
#[test]
fn removing_last_collection_membership_is_not_content_deletion() {
    let mut s = store();
    let work = create(&mut s, "Work");
    let item = command(&mut s, work, SpaceAction::CreateItem(content("keep")))
        .created_item
        .unwrap();
    command(&mut s, work, SpaceAction::RemoveItem(item));
    assert_eq!(s.spaces_for_item(item).unwrap(), [SpaceId::FAVORITES]);
    let r = revision(&s, SpaceId::FAVORITES);
    assert!(s
        .apply_space_command(SpaceCommand {
            space_id: Some(SpaceId::FAVORITES),
            expected_revision: Some(r),
            request_id: "remove-last".into(),
            action: SpaceAction::RemoveItem(item)
        })
        .is_err());
}
#[test]
fn duplicate_copies_original_representations_not_preview() {
    let mut s = store();
    let work = create(&mut s, "Work");
    let item = s
        .create_favorite(content(&"long original ".repeat(80)))
        .unwrap();
    let copied = command(&mut s, work, SpaceAction::DuplicateItem(item.id))
        .created_item
        .unwrap();
    assert_eq!(
        s.saved_item_payload(copied).unwrap(),
        s.saved_item_payload(item.id).unwrap()
    );
    assert_ne!(copied, item.id);
    assert_eq!(s.spaces_for_item(copied).unwrap(), [work]);
}
#[test]
fn default_spaces_are_protected_and_names_are_canonical() {
    let mut s = store();
    let _ = create(&mut s, "Café");
    assert!(s
        .apply_space_command(SpaceCommand {
            space_id: None,
            expected_revision: None,
            request_id: "duplicate".into(),
            action: SpaceAction::Create(draft("Cafe\u{301}"))
        })
        .is_err());
    for id in [SpaceId::HISTORY, SpaceId::FAVORITES] {
        assert!(s
            .apply_space_command(SpaceCommand {
                space_id: Some(id),
                expected_revision: Some(revision(&s, id)),
                request_id: format!("delete{id}"),
                action: SpaceAction::Delete
            })
            .is_err());
    }
}
#[test]
fn settings_patch_is_atomic_and_does_not_overwrite_automatic_resume() {
    let mut s = store();
    let snapshot = s.settings_snapshot().unwrap();
    let work = create(&mut s, "Work");
    s.save_resume_space(work).unwrap();
    let mut ui = snapshot.ui;
    ui.graphics = GraphicsMode::Software;
    ui.motion = Motion::Reduced;
    let mut clipboard = snapshot.clipboard;
    clipboard.theme = ThemeMode::Dark;
    let patch = SettingsPatch {
        expected_revision: snapshot.revision,
        clipboard,
        ui,
    };
    let saved = s.save_settings_patch(patch.clone()).unwrap();
    assert_eq!(saved.ui.resume_last_space_id, Some(work.to_string()));
    assert_eq!(saved.clipboard.theme, ThemeMode::Dark);
    assert!(matches!(
        s.save_settings_patch(patch),
        Err(StorageError::Space(SpaceError::Conflict))
    ));
    let mut invalid = SettingsPatch {
        expected_revision: saved.revision,
        clipboard: saved.clipboard.clone(),
        ui: saved.ui.clone(),
    };
    invalid.clipboard.max_entries = 2001;
    assert!(s.save_settings_patch(invalid.clone()).is_err());
    assert_eq!(s.settings_snapshot().unwrap(), saved);
    invalid.clipboard.max_entries = 2000;
    invalid.clipboard.max_item_bytes = invalid.clipboard.max_total_bytes + 1;
    assert!(s.save_settings_patch(invalid).is_err());
    assert_eq!(s.settings_snapshot().unwrap(), saved);
}
#[test]
fn shared_runtime_space_reads_and_writes_use_existing_actors() {
    let root = tempfile::tempdir().unwrap();
    let s = SharedClipboardStore::open(root.path()).unwrap();
    let created = s
        .apply_space_command(SpaceCommand {
            space_id: None,
            expected_revision: None,
            request_id: "shared:1".into(),
            action: SpaceAction::Create(draft("Work")),
        })
        .unwrap()
        .created_space
        .unwrap();
    assert!(s.list_spaces().unwrap().iter().any(|x| x.id == created));
    let snap = s.settings_snapshot().unwrap();
    s.save_resume_space(created).unwrap();
    assert_eq!(s.settings_snapshot().unwrap().revision, snap.revision);
    s.shutdown().unwrap();
    assert!(s.list_spaces().is_err());
}

#[test]
fn indexed_space_cursor_keeps_tied_negative_keys_and_filtered_totals() {
    let mut s = store();
    let mut ids = Vec::new();
    for n in 0..6 {
        ids.push(
            s.create_favorite(content(&format!(
                "{} {n}",
                if n % 2 == 0 { "match" } else { "other" }
            )))
            .unwrap()
            .id,
        );
    }
    s.connection
        .execute(
            "UPDATE space_memberships SET sort_key=-1024 WHERE space_id=2",
            [],
        )
        .unwrap();
    let mut cursor = None;
    let mut seen = Vec::new();
    loop {
        let page = s
            .list_space_items(SpaceId::FAVORITES, "", 2, cursor)
            .unwrap();
        assert_eq!(page.total, 6);
        seen.extend(page.page.items.iter().map(|item| item.id));
        cursor = page.page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(seen, ids);
    let filtered = s
        .list_space_items(SpaceId::FAVORITES, "match", 2, None)
        .unwrap();
    assert_eq!(filtered.total, 3);
    assert_eq!(
        filtered
            .page
            .items
            .iter()
            .map(|item| item.id)
            .collect::<Vec<_>>(),
        [ids[0], ids[2]]
    );
    let tail = s
        .list_space_items(SpaceId::FAVORITES, "match", 2, filtered.page.next_cursor)
        .unwrap();
    assert_eq!(tail.total, 3);
    assert_eq!(tail.page.items[0].id, ids[4]);
    assert!(tail.page.next_cursor.is_none());
    let mut plan = s.connection.prepare(
        "EXPLAIN QUERY PLAN SELECT s.id FROM space_memberships m JOIN saved_items s ON s.id=m.saved_item_id
         WHERE m.space_id=? AND (m.sort_key,m.saved_item_id)>(?,?)
         ORDER BY m.sort_key,m.saved_item_id LIMIT ?"
    ).unwrap();
    let details = plan
        .query_map(params![2, -1024, ids[1], 2], |row| row.get::<_, String>(3))
        .unwrap()
        .collect::<std::result::Result<Vec<_>, _>>()
        .unwrap();
    assert!(
        details
            .iter()
            .any(|line| line.contains("space_memberships_page_idx")),
        "{details:?}"
    );
    assert!(
        !details.iter().any(|line| line.contains("USE TEMP B-TREE")),
        "{details:?}"
    );
}
