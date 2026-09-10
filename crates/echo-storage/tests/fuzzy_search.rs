//! Real SQLite corpus, pagination, invalidation and retained payload regression.
use echo_engine::*;
use echo_storage::SharedClipboardStore;
use std::sync::Arc;
struct NoDesktop {
    changes: PlatformChangePublisher,
}
impl ClipboardPlatform for NoDesktop {
    fn subscribe_changes(&self) -> PlatformChangeSubscription {
        self.changes.subscribe()
    }
    fn clipboard_sequence(&self) -> u64 {
        0
    }
    fn read_clipboard(
        &self,
        _: &CapturePolicy,
    ) -> Result<Option<ClipboardSnapshot>, PlatformError> {
        Ok(None)
    }
    fn write_clipboard(&self, _: &[ClipboardRepresentation]) -> Result<u64, PlatformError> {
        Err(PlatformError("Test forbids OS clipboard".into()))
    }
    fn capture_target(&self) -> Result<Option<PasteTarget>, PlatformError> {
        Ok(None)
    }
    fn paste_to_target(&self, _: &PasteTarget) -> Result<PasteDelivery, PlatformError> {
        Err(PlatformError("Test forbids OS input".into()))
    }
}
#[test]
fn fuzzy_search_is_complete_paginated_and_revision_checked() {
    let parent = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/test-tmp/echo-storage");
    std::fs::create_dir_all(&parent).unwrap();
    let root = tempfile::tempdir_in(parent).unwrap();
    let store = Arc::new(SharedClipboardStore::open(root.path()).unwrap());
    let platform = Arc::new(NoDesktop {
        changes: Default::default(),
    });
    let clipboard = Arc::new(ClipboardService::new(platform.clone(), store.clone()));
    let quick = QuickInsertService::new(Library::new(store.clone()), clipboard.clone(), platform);
    for i in 0..135 {
        quick
            .create_favorite(FavoriteDraft {
                content: format!("Amber worktree note {i:04}"),
                name: Some(format!("Record {i:04}")),
                icon_key: None,
                tags: Vec::new(),
            })
            .unwrap();
    }
    let special = quick
        .create_favorite(FavoriteDraft {
            content: r"D:\Worktrees\echo\ui".into(),
            name: Some("Deep result".into()),
            icon_key: None,
            tags: Vec::new(),
        })
        .unwrap();
    let (one, _, count) = quick
        .fuzzy_list_space(SpaceId::FAVORITES, "wrk echo ui", 50, None)
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(one.items[0].id, special.id);
    assert!(one.items[0].editable_text.is_none());
    assert_eq!(
        store.saved_item_payload(special.id).unwrap()[0].bytes,
        r"D:\Worktrees\echo\ui".as_bytes()
    );
    let mut cursor = None;
    let mut ids = Vec::new();
    let mut first_cursor = None;
    loop {
        let (page, _, total) = quick
            .fuzzy_list_space(SpaceId::FAVORITES, "ambr nt", 17, cursor)
            .unwrap();
        assert_eq!(total, 135);
        assert!(page.items.len() <= 17);
        ids.extend(page.items.into_iter().map(|i| i.id));
        cursor = page.next_cursor;
        if first_cursor.is_none() {
            first_cursor = cursor;
        }
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(ids.len(), 135);
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 135);
    assert!(quick
        .fuzzy_list_space(SpaceId::FAVORITES, "unrelated", 17, first_cursor)
        .is_err());
    assert!(quick
        .fuzzy_list_space(SpaceId::HISTORY, "ambr nt", 17, first_cursor)
        .is_err());
    quick
        .create_favorite(FavoriteDraft {
            content: "Amber worktree note newest".into(),
            name: None,
            icon_key: None,
            tags: Vec::new(),
        })
        .unwrap();
    assert!(quick
        .fuzzy_list_space(SpaceId::FAVORITES, "ambr nt", 17, first_cursor)
        .is_err());
    assert_eq!(
        quick
            .fuzzy_list_space(SpaceId::FAVORITES, "ambr nt", 17, None)
            .unwrap()
            .2,
        136
    );
    quick.release_search_cache();
    assert_eq!(
        quick
            .fuzzy_list_space(SpaceId::FAVORITES, "ambr   nt  ", 17, None)
            .unwrap()
            .2,
        136
    );
    // Cancellation is exercised inside a warm scan and a cold paginated scan;
    // neither may publish a truncated page as success, and the next query works.
    for cold in [false, true] {
        if cold {
            quick.release_search_cache();
        }
        let checks = std::cell::Cell::new(0);
        let cancelled = || {
            checks.set(checks.get() + 1);
            checks.get() >= 4
        };
        assert!(quick
            .fuzzy_list_space_cancellable(SpaceId::FAVORITES, "nt ambr", 17, None, &cancelled)
            .is_err());
        assert!(checks.get() >= 4);
        assert_eq!(
            quick
                .fuzzy_list_space(SpaceId::FAVORITES, "ambr nt", 17, None)
                .unwrap()
                .2,
            136
        );
    }
    let original = "中文 👨‍👩‍👧‍👦 Cafe\u{301} <script> ** [x](url) & \\";
    let unicode = quick
        .create_favorite(FavoriteDraft {
            content: original.into(),
            name: None,
            icon_key: None,
            tags: Vec::new(),
        })
        .unwrap();
    let (page, _, count) = quick
        .fuzzy_list_space(SpaceId::FAVORITES, "👨‍👩‍👧‍👦 cafe 中文", 17, None)
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(page.items[0].id, unicode.id);
    assert_eq!(
        store.saved_item_payload(unicode.id).unwrap()[0].bytes,
        original.as_bytes()
    );
    let full_body = format!("{} memory50-tail-recall", "前缀正文 ".repeat(4096));
    let long_item = quick
        .create_favorite(FavoriteDraft {
            content: full_body.clone(),
            name: Some("Long original".into()),
            icon_key: None,
            tags: Vec::new(),
        })
        .unwrap();
    for cold in [true, false] {
        if cold {
            quick.release_search_cache();
        }
        let (page, _, count) = quick
            .fuzzy_list_space(SpaceId::FAVORITES, "memory50-tail-recall", 17, None)
            .unwrap();
        assert_eq!(count, 1);
        assert_eq!(page.items[0].id, long_item.id);
        assert!(page.items[0].editable_text.is_none());
    }
    assert_eq!(
        store
            .saved_item(long_item.id)
            .unwrap()
            .unwrap()
            .item
            .editable_text
            .as_deref(),
        Some(full_body.as_str())
    );
    assert_eq!(
        store.saved_item_payload(long_item.id).unwrap()[0].bytes,
        full_body.as_bytes()
    );
    clipboard.shutdown();
    drop(quick);
    drop(clipboard);
    store.shutdown().unwrap();
}

/// Explicit scale gate; real SQLite and the production search path, no OS input.
#[test]
#[ignore = "explicit 1k/10k search scale gate"]
fn fuzzy_search_large_corpus_keeps_tail_results_and_cancels() {
    let parent = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.local/test-tmp/echo-storage");
    std::fs::create_dir_all(&parent).unwrap();
    let root = tempfile::tempdir_in(parent).unwrap();
    let store = Arc::new(SharedClipboardStore::open(root.path()).unwrap());
    let platform = Arc::new(NoDesktop {
        changes: Default::default(),
    });
    let clipboard = Arc::new(ClipboardService::new(platform.clone(), store.clone()));
    let quick = QuickInsertService::new(Library::new(store.clone()), clipboard.clone(), platform);
    // At 10k, the searchable bodies alone exceed the 16 MiB corpus budget.
    // The unique oldest item must still be found after falling back to streaming.
    let padding = "x".repeat(2048);
    let mut oldest = 0;
    for i in 0..10_000 {
        let item = quick
            .create_favorite(FavoriteDraft {
                content: format!(
                    "scale-item-{i:05} {} {padding}",
                    if i == 0 { "uniquetailneedle" } else { "common" }
                ),
                name: Some(format!("Scale {i:05}")),
                icon_key: None,
                tags: Vec::new(),
            })
            .unwrap();
        if i == 0 {
            oldest = item.id;
        }
        if i + 1 != 1000 && i + 1 != 10_000 {
            continue;
        }
        for round in 0..3 {
            quick.release_search_cache();
            let start = std::time::Instant::now();
            let (page, _, total) = quick
                .fuzzy_list_space(SpaceId::FAVORITES, "uniquetailneedle", 50, None)
                .unwrap();
            assert_eq!(total, 1);
            assert_eq!(page.items[0].id, oldest);
            let cold_us = start.elapsed().as_micros();
            let start = std::time::Instant::now();
            assert_eq!(
                quick
                    .fuzzy_list_space(SpaceId::FAVORITES, "  uniquetailneedle  ", 50, None)
                    .unwrap()
                    .0
                    .items[0]
                    .id,
                oldest
            );
            let normalized_us = start.elapsed().as_micros();
            // Closing a session retires its query/page, not the bounded corpus.
            // Verify the reopened production path still reaches the oldest item.
            quick.release_search_results();
            let start = std::time::Instant::now();
            let (reopened, _, reopened_total) = quick
                .fuzzy_list_space(SpaceId::FAVORITES, "uniquetailneedle", 50, None)
                .unwrap();
            assert_eq!(reopened_total, 1);
            assert_eq!(reopened.items[0].id, oldest);
            let reopen_us = start.elapsed().as_micros();
            let checks = std::cell::Cell::new(0);
            let start = std::time::Instant::now();
            let result =
                quick.fuzzy_list_space_cancellable(SpaceId::FAVORITES, "common", 50, None, &|| {
                    checks.set(checks.get() + 1);
                    checks.get() >= 8
                });
            assert!(result.is_err());
            assert_eq!(checks.get(), 8);
            println!("SEARCH_SCALE count={} round={} body_bytes_min={} cold_us={} normalized_us={} reopen_us={} cancel_us={}", i + 1, round + 1, (i + 1) * padding.len(), cold_us, normalized_us, reopen_us, start.elapsed().as_micros());
            assert_eq!(
                quick
                    .fuzzy_list_space(SpaceId::FAVORITES, "common", 50, None)
                    .unwrap()
                    .2,
                i as u64
            );
        }
    }
    clipboard.shutdown();
    drop(quick);
    drop(clipboard);
    store.shutdown().unwrap();
}
