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
    clipboard.shutdown();
    drop(quick);
    drop(clipboard);
    store.shutdown().unwrap();
}
