use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::{
    ClipboardError, ClipboardPlatform, ClipboardService, FavoriteDraft, FavoriteUpdate, Library,
    LibraryError, LibraryItem, LibraryItemKind, LibraryPage, LibraryStore, LibraryView,
    OperationMetrics, PageCursor, PasteDelivery, PasteDeliveryFailure, PasteTarget, SavedItem,
    Thumbnail, DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum QuickInsertError {
    #[error(transparent)]
    Clipboard(#[from] ClipboardError),
    #[error(transparent)]
    Library(#[from] LibraryError),
    #[error("no safe paste target is available")]
    NoTarget,
    #[error("paste target was no longer valid")]
    InvalidTarget,
    #[error("paste target was rejected for safe delivery: {0:?}")]
    DeliveryFailed(PasteDeliveryFailure),
}

pub type Result<T> = std::result::Result<T, QuickInsertError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuickInsertView {
    History,
    Favorites,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuickInsertSource {
    History,
    Favorite,
}

impl QuickInsertSource {
    fn kind(self) -> LibraryItemKind {
        match self {
            Self::History => LibraryItemKind::History,
            Self::Favorite => LibraryItemKind::SavedItem,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuickInsertAction {
    Copy,
    Insert,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickInsertItem {
    pub id: i64,
    pub source: QuickInsertSource,
    pub name: Option<String>,
    pub preview_text: Option<String>,
    pub content_type: String,
    pub editable_text: Option<String>,
    pub tags: Vec<String>,
    pub source_app: Option<String>,
    pub updated_at: i64,
    pub pinned_at: Option<i64>,
    pub icon_key: Option<String>,
    pub favorite_order: Option<i64>,
    pub thumbnail: Option<Thumbnail>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickInsertRequest {
    pub view: QuickInsertView,
    pub query: String,
    pub limit: u32,
    pub cursor: Option<PageCursor>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickInsertPage {
    pub items: Vec<QuickInsertItem>,
    pub next_cursor: Option<PageCursor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuickInsertOutcome {
    Copied,
    Inserted,
    ClipboardStaged,
}

pub struct QuickInsertService<S: LibraryStore> {
    library: Library<S>,
    clipboard: Arc<ClipboardService>,
    platform: Arc<dyn ClipboardPlatform>,
    target: Mutex<Option<PasteTarget>>,
    metrics: OperationMetrics,
}

impl<S: LibraryStore> QuickInsertService<S> {
    pub fn new(
        library: Library<S>,
        clipboard: Arc<ClipboardService>,
        platform: Arc<dyn ClipboardPlatform>,
    ) -> Self {
        Self {
            library,
            clipboard,
            platform,
            target: Mutex::new(None),
            metrics: OperationMetrics::default(),
        }
    }

    pub fn list(&self, request: &QuickInsertRequest) -> Result<QuickInsertPage> {
        let started = Instant::now();
        let view = match request.view {
            QuickInsertView::History => LibraryView::History,
            QuickInsertView::Favorites => LibraryView::Favorites,
        };
        let limit = if request.limit == 0 {
            DEFAULT_PAGE_SIZE
        } else {
            request.limit.clamp(1, MAX_PAGE_SIZE)
        };
        let LibraryPage { items, next_cursor } =
            self.library
                .list(view, &request.query, limit, request.cursor)?;
        self.metrics.record(
            "first_result_availability",
            started.elapsed(),
            u64::from(!items.is_empty()),
        );
        Ok(QuickInsertPage {
            items: items.into_iter().map(to_item).collect(),
            next_cursor,
        })
    }

    pub fn begin_session(&self) -> Result<bool> {
        let target = self
            .platform
            .capture_target()
            .map_err(ClipboardError::from)?;
        *self
            .target
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = target.clone();
        Ok(target.is_some())
    }

    pub fn clear_session(&self) {
        *self
            .target
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
        self.platform.reset_paste_window_session();
    }

    pub fn execute(
        &self,
        source: QuickInsertSource,
        id: i64,
        action: QuickInsertAction,
    ) -> Result<QuickInsertOutcome> {
        let payload = self.library.payload(source.kind(), id)?;
        match action {
            QuickInsertAction::Copy => {
                self.clipboard.copy_representations(&payload)?;
                Ok(QuickInsertOutcome::Copied)
            }
            QuickInsertAction::Insert => {
                let target = self
                    .target
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .clone()
                    .ok_or(QuickInsertError::NoTarget)?;
                self.platform
                    .validate_paste_target(&target)
                    .map_err(QuickInsertError::DeliveryFailed)?;
                self.clipboard.copy_representations(&payload)?;
                match self
                    .platform
                    .paste_to_target(&target)
                    .map_err(ClipboardError::from)?
                {
                    PasteDelivery::Pasted => {
                        self.clear_session();
                        Ok(QuickInsertOutcome::Inserted)
                    }
                    PasteDelivery::Failed(reason) => Err(QuickInsertError::DeliveryFailed(reason)),
                }
            }
        }
    }

    pub fn move_history_to_favorite(&self, id: i64) -> Result<SavedItem> {
        let item = self.library.move_history_to_favorite(id)?;
        self.invalidate_history(Some(id));
        Ok(item)
    }

    pub fn move_history_many_to_favorites(&self, ids: &[i64]) -> Result<Vec<SavedItem>> {
        let items = self.library.move_history_many_to_favorites(ids)?;
        if !items.is_empty() {
            self.invalidate_history(None);
        }
        Ok(items)
    }

    pub fn create_favorite(&self, draft: FavoriteDraft) -> Result<SavedItem> {
        let item = self.library.create_favorite(draft)?;
        self.invalidate_history(None);
        Ok(item)
    }

    pub fn update_favorite(&self, id: i64, update: FavoriteUpdate) -> Result<SavedItem> {
        let item = self.library.update_favorite(id, update)?;
        self.invalidate_history(Some(id));
        Ok(item)
    }

    pub fn pin_history(&self, id: i64) -> Result<bool> {
        let changed = self.library.pin_history(id)?;
        if changed {
            self.invalidate_history(Some(id));
        }
        Ok(changed)
    }

    pub fn unpin_history(&self, id: i64) -> Result<bool> {
        let changed = self.library.unpin_history(id)?;
        if changed {
            self.invalidate_history(Some(id));
        }
        Ok(changed)
    }

    pub fn pin_history_many(&self, ids: &[i64]) -> Result<usize> {
        let changed = self.library.pin_history_many(ids)?;
        if changed != 0 {
            self.invalidate_history(None);
        }
        Ok(changed)
    }

    pub fn delete_history_many(&self, ids: &[i64]) -> Result<usize> {
        let deleted = self.library.delete_history_many(ids)?;
        if deleted != 0 {
            self.invalidate_history(None);
        }
        Ok(deleted)
    }

    pub fn clear_unpinned_history(&self) -> Result<usize> {
        let deleted = self.library.clear_unpinned_history()?;
        if deleted != 0 {
            self.invalidate_history(None);
        }
        Ok(deleted)
    }

    pub fn reorder_favorites(&self, ordered_ids: &[i64]) -> Result<()> {
        self.library.reorder_favorites(ordered_ids)?;
        self.invalidate_history(None);
        Ok(())
    }

    pub fn delete_favorite(&self, id: i64) -> Result<bool> {
        let deleted = self.library.delete_favorite(id)?;
        if deleted {
            self.invalidate_history(Some(id));
        }
        Ok(deleted)
    }

    pub fn delete(&self, source: QuickInsertSource, id: i64) -> Result<bool> {
        let deleted = self.library.delete(source.kind(), id)?;
        if deleted {
            self.invalidate_history(Some(id));
        }
        Ok(deleted)
    }

    pub fn delete_saved_items(&self, ids: &[i64]) -> Result<usize> {
        let deleted = self.library.delete_saved_items(ids)?;
        if deleted != 0 {
            self.invalidate_history(None);
        }
        Ok(deleted)
    }

    pub fn invalidate_history(&self, id: Option<i64>) {
        self.clipboard.publish_history_invalidation(id);
    }

    pub fn request_maintenance(&self) {
        // Explicit repair only; storage mutators schedule their own maintenance.
        self.clipboard.request_maintenance();
    }

    pub fn refresh_capture_configuration(&self) -> Result<()> {
        self.clipboard.refresh_configuration()?;
        Ok(())
    }

    pub fn library(&self) -> &Library<S> {
        &self.library
    }

    pub fn metrics_snapshot(&self) -> Vec<crate::OperationMetric> {
        self.metrics.snapshot()
    }
}

fn to_item(item: LibraryItem) -> QuickInsertItem {
    QuickInsertItem {
        id: item.id,
        source: match item.kind {
            LibraryItemKind::History => QuickInsertSource::History,
            LibraryItemKind::SavedItem => QuickInsertSource::Favorite,
        },
        name: item.name,
        preview_text: item.preview_text,
        content_type: item.content_type,
        editable_text: item.editable_text,
        tags: item.tags,
        source_app: item.source_app,
        updated_at: item.updated_at,
        pinned_at: item.pinned_at,
        icon_key: item.icon_key,
        favorite_order: item.favorite_order,
        thumbnail: item.thumbnail,
    }
}

#[cfg(test)]
mod tests {
    use std::convert::Infallible;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;
    use crate::{
        CapturePolicy, CaptureSettings, ClipboardPlatform, ClipboardRepresentation,
        ClipboardService, ClipboardSettings, ClipboardSink, ClipboardSnapshot, HistoryEntry,
        InputTargetGeometry, NormalizedCapture, PhysicalRect, PlatformChangePublisher,
        PlatformChangeSubscription, PlatformError, RecordResult, ThemeMode,
    };

    #[derive(Default)]
    struct RejectingPlatform {
        changes: PlatformChangePublisher,
        writes: AtomicUsize,
    }

    impl RejectingPlatform {
        fn writes(&self) -> usize {
            self.writes.load(Ordering::Acquire)
        }
    }

    impl ClipboardPlatform for RejectingPlatform {
        fn subscribe_changes(&self) -> PlatformChangeSubscription {
            self.changes.subscribe()
        }

        fn clipboard_sequence(&self) -> u64 {
            1
        }

        fn read_clipboard(
            &self,
            _policy: &CapturePolicy,
        ) -> std::result::Result<Option<ClipboardSnapshot>, PlatformError> {
            Ok(None)
        }

        fn write_clipboard(
            &self,
            _representations: &[ClipboardRepresentation],
        ) -> std::result::Result<u64, PlatformError> {
            self.writes.fetch_add(1, Ordering::AcqRel);
            Ok(2)
        }

        fn capture_target(&self) -> std::result::Result<Option<PasteTarget>, PlatformError> {
            Ok(Some(test_target()))
        }

        fn validate_paste_target(
            &self,
            _target: &PasteTarget,
        ) -> std::result::Result<(), PasteDeliveryFailure> {
            Err(PasteDeliveryFailure::ElevatedTarget)
        }

        fn paste_to_target(
            &self,
            _target: &PasteTarget,
        ) -> std::result::Result<PasteDelivery, PlatformError> {
            panic!("paste must not be reached after target rejection")
        }
    }

    struct TestSink;

    impl ClipboardSink for TestSink {
        fn settings(&self) -> std::result::Result<CaptureSettings, String> {
            Ok(CaptureSettings::default())
        }

        fn record(&self, _capture: NormalizedCapture) -> std::result::Result<RecordResult, String> {
            Ok(RecordResult {
                id: 1,
                duplicate: false,
            })
        }
    }

    struct TestStore;

    fn unused<T>() -> std::result::Result<T, Infallible> {
        panic!("unused test store operation")
    }

    impl LibraryStore for TestStore {
        type Error = Infallible;

        fn list_entries(
            &self,
            _query: &str,
            _limit: u32,
            _cursor: Option<PageCursor>,
        ) -> std::result::Result<LibraryPage<HistoryEntry>, Self::Error> {
            unused()
        }

        fn entry(&self, _id: i64) -> std::result::Result<Option<HistoryEntry>, Self::Error> {
            unused()
        }

        fn entry_payload(
            &self,
            _id: i64,
        ) -> std::result::Result<Vec<ClipboardRepresentation>, Self::Error> {
            unused()
        }

        fn move_history_to_favorite(
            &self,
            _history_id: i64,
        ) -> std::result::Result<SavedItem, Self::Error> {
            unused()
        }

        fn move_history_many_to_favorites(
            &self,
            _history_ids: &[i64],
        ) -> std::result::Result<Vec<SavedItem>, Self::Error> {
            unused()
        }

        fn create_favorite(
            &self,
            _draft: FavoriteDraft,
        ) -> std::result::Result<SavedItem, Self::Error> {
            unused()
        }

        fn update_favorite(
            &self,
            _id: i64,
            _update: FavoriteUpdate,
        ) -> std::result::Result<SavedItem, Self::Error> {
            unused()
        }

        fn pin_history(&self, _history_id: i64) -> std::result::Result<bool, Self::Error> {
            unused()
        }

        fn unpin_history(&self, _history_id: i64) -> std::result::Result<bool, Self::Error> {
            unused()
        }

        fn pin_history_many(
            &self,
            _history_ids: &[i64],
        ) -> std::result::Result<usize, Self::Error> {
            unused()
        }

        fn delete_history_many(
            &self,
            _history_ids: &[i64],
        ) -> std::result::Result<usize, Self::Error> {
            unused()
        }

        fn delete_entry(&self, _id: i64) -> std::result::Result<bool, Self::Error> {
            unused()
        }

        fn clear_unpinned_history(&self) -> std::result::Result<usize, Self::Error> {
            unused()
        }

        fn reorder_favorites(&self, _ordered_ids: &[i64]) -> std::result::Result<(), Self::Error> {
            unused()
        }

        fn delete_favorite(&self, _id: i64) -> std::result::Result<bool, Self::Error> {
            unused()
        }

        fn settings(&self) -> std::result::Result<ClipboardSettings, Self::Error> {
            Ok(ClipboardSettings {
                history_enabled: true,
                record_sensitive: false,
                store_window_titles: false,
                max_entries: 100,
                max_total_bytes: 1024,
                max_item_bytes: 1024,
                theme: ThemeMode::System,
            })
        }

        fn update_settings(
            &self,
            _settings: &ClipboardSettings,
        ) -> std::result::Result<(), Self::Error> {
            Ok(())
        }

        fn list_saved_items(
            &self,
            _query: &str,
            _limit: u32,
            _cursor: Option<PageCursor>,
        ) -> std::result::Result<LibraryPage<SavedItem>, Self::Error> {
            unused()
        }

        fn saved_item_payload(
            &self,
            _id: i64,
        ) -> std::result::Result<Vec<ClipboardRepresentation>, Self::Error> {
            Ok(vec![ClipboardRepresentation {
                format: "text".to_owned(),
                mime_type: "text/plain".to_owned(),
                bytes: b"elevated target payload".to_vec(),
            }])
        }

        fn delete_saved_items(&self, _ids: &[i64]) -> std::result::Result<usize, Self::Error> {
            unused()
        }
    }

    fn test_target() -> PasteTarget {
        PasteTarget {
            window_id: 1,
            window_class: "Edit".to_owned(),
            process_id: 42,
            process_started_at: 1,
            focused_control: None,
            app_name: None,
            selected_text: None,
            is_single_line: Some(true),
            geometry: InputTargetGeometry {
                target: PhysicalRect {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                work_area: PhysicalRect {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                dpi: 96,
            },
        }
    }

    #[test]
    fn elevated_target_is_rejected_before_clipboard_staging() {
        let platform = Arc::new(RejectingPlatform::default());
        let clipboard = Arc::new(ClipboardService::new(
            Arc::clone(&platform) as Arc<dyn ClipboardPlatform>,
            Arc::new(TestSink),
        ));
        let service = QuickInsertService::new(
            Library::new(Arc::new(TestStore)),
            Arc::clone(&clipboard),
            Arc::clone(&platform) as Arc<dyn ClipboardPlatform>,
        );

        assert!(service.begin_session().expect("target capture"));
        let result = service.execute(QuickInsertSource::Favorite, 1, QuickInsertAction::Insert);
        match result {
            Err(QuickInsertError::DeliveryFailed(reason)) => {
                assert_eq!(reason, PasteDeliveryFailure::ElevatedTarget)
            }
            other => panic!("unexpected insertion result: {other:?}"),
        }
        assert_eq!(platform.writes(), 0);
        clipboard.shutdown();
    }
}
