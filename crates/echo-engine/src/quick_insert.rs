use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::{
    ClipboardError, ClipboardPlatform, ClipboardService, FavoriteDraft, FavoriteUpdate, Library,
    LibraryError, LibraryItem, LibraryItemKind, LibraryPage, LibraryStore, LibraryView,
    OperationMetrics, PageCursor, PasteDelivery, PasteDeliveryFailure, PasteTarget, PlatformError,
    SavedItem, Thumbnail, DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE,
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
        if self
            .target
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_some()
        {
            return Ok(true);
        }
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
                match self.clipboard.copy_representations(&payload) {
                    Ok(_) => {}
                    Err(error) => {
                        let Some(preflight_error) =
                            target_preflight_error(&error).map(str::to_owned)
                        else {
                            return Err(error.into());
                        };
                        self.clear_session();
                        return match self.clipboard.copy_representations(&payload) {
                            // The preflight target was cleared, so report the existing
                            // target-free copy outcome rather than claiming a session remains.
                            Ok(_) => Ok(QuickInsertOutcome::ClipboardStaged),
                            Err(retry_error) => Err(QuickInsertError::Clipboard(
                                ClipboardError::Platform(PlatformError(format!(
                                    "{preflight_error}; retry failed: {retry_error}"
                                ))),
                            )),
                        };
                    }
                }
                Ok(QuickInsertOutcome::Copied)
            }
            QuickInsertAction::Insert => {
                let target = self
                    .target
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .clone()
                    .ok_or(QuickInsertError::NoTarget)?;
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

fn target_preflight_error(error: &ClipboardError) -> Option<&str> {
    matches!(
        error,
        ClipboardError::Platform(platform_error)
            if platform_error.0.starts_with("paste target was rejected before clipboard staging:")
    )
    .then(|| match error {
        ClipboardError::Platform(platform_error) => platform_error.0.as_str(),
        _ => unreachable!("target preflight errors are platform errors"),
    })
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
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

    use super::*;
    use crate::{
        CapturePolicy, CaptureSettings, ClipboardPlatform, ClipboardRepresentation,
        ClipboardService, ClipboardSettings, ClipboardSink, ClipboardSnapshot, HistoryEntry,
        InputTargetGeometry, NormalizedCapture, PhysicalRect, PlatformChangePublisher,
        PlatformChangeSubscription, PlatformError, RecordResult, ThemeMode,
    };

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum TargetState {
        Live,
        Elevated,
        Stale,
    }

    impl Default for TargetState {
        fn default() -> Self {
            Self::Live
        }
    }

    #[derive(Default)]
    struct StatefulPlatform {
        changes: PlatformChangePublisher,
        target_state: Mutex<TargetState>,
        captured_target: Mutex<Option<TargetState>>,
        sequence: AtomicU64,
        clipboard: Mutex<Vec<u8>>,
        writes: AtomicUsize,
        paste_calls: AtomicUsize,
        fail_writes: AtomicBool,
    }

    impl StatefulPlatform {
        fn set_target_state(&self, state: TargetState) {
            *self
                .target_state
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = state;
        }

        fn set_clipboard(&self, value: &str) {
            *self
                .clipboard
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = value.as_bytes().to_vec();
        }

        fn clipboard_text(&self) -> String {
            String::from_utf8(
                self.clipboard
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .clone(),
            )
            .expect("stateful clipboard stores UTF-8 test text")
        }

        fn writes(&self) -> usize {
            self.writes.load(Ordering::Acquire)
        }

        fn paste_calls(&self) -> usize {
            self.paste_calls.load(Ordering::Acquire)
        }

        fn set_write_failure(&self, fail: bool) {
            self.fail_writes.store(fail, Ordering::Release);
        }
    }

    impl ClipboardPlatform for StatefulPlatform {
        fn subscribe_changes(&self) -> PlatformChangeSubscription {
            self.changes.subscribe()
        }

        fn clipboard_sequence(&self) -> u64 {
            self.sequence.load(Ordering::Acquire)
        }

        fn read_clipboard(
            &self,
            _policy: &CapturePolicy,
        ) -> std::result::Result<Option<ClipboardSnapshot>, PlatformError> {
            Ok(None)
        }

        fn write_clipboard(
            &self,
            representations: &[ClipboardRepresentation],
        ) -> std::result::Result<u64, PlatformError> {
            match *self
                .captured_target
                .lock()
                .unwrap_or_else(|error| error.into_inner())
            {
                Some(TargetState::Elevated) => Err(PlatformError(
                    "paste target was rejected before clipboard staging: ElevatedTarget".to_owned(),
                )),
                Some(TargetState::Stale) => Err(PlatformError(
                    "paste target was rejected before clipboard staging: OriginalWindowUnavailable"
                        .to_owned(),
                )),
                Some(TargetState::Live) | None => {
                    if self.fail_writes.load(Ordering::Acquire) {
                        return Err(PlatformError("generic clipboard write failure".to_owned()));
                    }
                    let bytes = representations
                        .first()
                        .map(|representation| representation.bytes.clone())
                        .unwrap_or_default();
                    *self
                        .clipboard
                        .lock()
                        .unwrap_or_else(|error| error.into_inner()) = bytes;
                    self.writes.fetch_add(1, Ordering::AcqRel);
                    Ok(self.sequence.fetch_add(1, Ordering::AcqRel) + 1)
                }
            }
        }

        fn capture_target(&self) -> std::result::Result<Option<PasteTarget>, PlatformError> {
            *self
                .captured_target
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(
                *self
                    .target_state
                    .lock()
                    .unwrap_or_else(|error| error.into_inner()),
            );
            Ok(Some(test_target()))
        }

        fn paste_to_target(
            &self,
            _target: &PasteTarget,
        ) -> std::result::Result<PasteDelivery, PlatformError> {
            self.paste_calls.fetch_add(1, Ordering::AcqRel);
            Ok(PasteDelivery::Pasted)
        }

        fn reset_paste_window_session(&self) {
            *self
                .captured_target
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = None;
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
    fn target_validation_preserves_clipboard_and_copy_recovers_session_without_blocking_copy() {
        let platform = Arc::new(StatefulPlatform::default());
        let clipboard = Arc::new(ClipboardService::new(
            Arc::clone(&platform) as Arc<dyn ClipboardPlatform>,
            Arc::new(TestSink),
        ));
        let service = QuickInsertService::new(
            Library::new(Arc::new(TestStore)),
            Arc::clone(&clipboard),
            Arc::clone(&platform) as Arc<dyn ClipboardPlatform>,
        );

        platform.set_clipboard("clipboard sentinel");
        platform.set_target_state(TargetState::Elevated);
        assert!(service.begin_session().expect("target capture"));
        let error = service
            .execute(QuickInsertSource::Favorite, 1, QuickInsertAction::Insert)
            .expect_err("elevated target must be rejected");
        assert!(error.to_string().contains("ElevatedTarget"));
        assert_eq!(platform.clipboard_text(), "clipboard sentinel");
        assert_eq!(platform.writes(), 0);
        assert_eq!(platform.paste_calls(), 0);

        service.clear_session();
        platform.set_target_state(TargetState::Stale);
        assert!(service.begin_session().expect("stale target capture"));
        let error = service
            .execute(QuickInsertSource::Favorite, 1, QuickInsertAction::Insert)
            .expect_err("stale target must be rejected");
        assert!(error.to_string().contains("OriginalWindowUnavailable"));
        assert_eq!(platform.clipboard_text(), "clipboard sentinel");
        assert_eq!(platform.writes(), 0);
        assert_eq!(platform.paste_calls(), 0);

        let outcome = service
            .execute(QuickInsertSource::Favorite, 1, QuickInsertAction::Copy)
            .expect("copy must recover from stale insertion session");
        assert_eq!(outcome, QuickInsertOutcome::ClipboardStaged);
        assert_eq!(platform.clipboard_text(), "elevated target payload");
        assert_eq!(platform.writes(), 1);
        assert_eq!(platform.paste_calls(), 0);

        platform.set_target_state(TargetState::Live);
        assert!(service.begin_session().expect("fresh target capture"));
        let outcome = service
            .execute(QuickInsertSource::Favorite, 1, QuickInsertAction::Copy)
            .expect("copy with a live target");
        assert_eq!(outcome, QuickInsertOutcome::Copied);
        assert_eq!(platform.clipboard_text(), "elevated target payload");
        assert_eq!(platform.writes(), 2);

        let outcome = service
            .execute(QuickInsertSource::Favorite, 1, QuickInsertAction::Insert)
            .expect("insert after copy must retain the live target");
        assert_eq!(outcome, QuickInsertOutcome::Inserted);
        assert_eq!(platform.clipboard_text(), "elevated target payload");
        assert_eq!(platform.writes(), 3);
        assert_eq!(platform.paste_calls(), 1);
        clipboard.shutdown();
    }

    #[test]
    fn copy_failure_keeps_live_target_but_clears_preflight_target() {
        let platform = Arc::new(StatefulPlatform::default());
        let clipboard = Arc::new(ClipboardService::new(
            Arc::clone(&platform) as Arc<dyn ClipboardPlatform>,
            Arc::new(TestSink),
        ));
        let service = QuickInsertService::new(
            Library::new(Arc::new(TestStore)),
            Arc::clone(&clipboard),
            Arc::clone(&platform) as Arc<dyn ClipboardPlatform>,
        );

        platform.set_clipboard("clipboard sentinel");
        platform.set_write_failure(true);
        platform.set_target_state(TargetState::Live);
        assert!(service.begin_session().expect("live target capture"));
        let error = service
            .execute(QuickInsertSource::Favorite, 1, QuickInsertAction::Copy)
            .expect_err("generic live copy failure must be surfaced");
        assert_eq!(
            error.to_string(),
            "platform error: generic clipboard write failure"
        );
        assert_eq!(platform.clipboard_text(), "clipboard sentinel");
        assert_eq!(platform.writes(), 0);

        platform.set_write_failure(false);
        let outcome = service
            .execute(QuickInsertSource::Favorite, 1, QuickInsertAction::Insert)
            .expect("live target must remain available after generic copy failure");
        assert_eq!(outcome, QuickInsertOutcome::Inserted);
        assert_eq!(platform.writes(), 1);
        assert_eq!(platform.paste_calls(), 1);

        platform.set_clipboard("clipboard sentinel");
        platform.set_write_failure(true);
        platform.set_target_state(TargetState::Stale);
        assert!(service.begin_session().expect("stale target capture"));
        let error = service
            .execute(QuickInsertSource::Favorite, 1, QuickInsertAction::Copy)
            .expect_err("retry failure must be surfaced");
        assert!(error
            .to_string()
            .contains("paste target was rejected before clipboard staging:"));
        assert!(error
            .to_string()
            .contains("generic clipboard write failure"));
        assert_eq!(platform.clipboard_text(), "clipboard sentinel");
        assert_eq!(platform.writes(), 1);
        assert_eq!(platform.paste_calls(), 1);
        clipboard.shutdown();
    }
}
