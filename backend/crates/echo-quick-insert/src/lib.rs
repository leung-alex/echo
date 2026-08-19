use std::sync::{Arc, Mutex};

use echo_clipboard::{ClipboardService, ClipboardError};
use echo_library::{Library, LibraryError, LibraryItem, LibraryItemKind, LibraryView};
use echo_platform::{ClipboardPlatform, PasteDelivery, PasteDeliveryFailure, PasteTarget};
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
    #[error("paste was staged in the clipboard but could not be delivered: {0:?}")]
    DeliveryFailed(PasteDeliveryFailure),
}

pub type Result<T> = std::result::Result<T, QuickInsertError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuickInsertView {
    History,
    Favorites,
    Snippets,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuickInsertSource {
    History,
    Favorite,
    Snippet,
}

impl QuickInsertSource {
    fn kind(self) -> LibraryItemKind {
        match self {
            Self::History => LibraryItemKind::History,
            Self::Favorite => LibraryItemKind::Favorite,
            Self::Snippet => LibraryItemKind::Snippet,
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
    pub title: Option<String>,
    pub preview_text: Option<String>,
    pub content_type: String,
    pub source_app: Option<String>,
    pub updated_at: i64,
    pub pinned: bool,
    pub group_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickInsertRequest {
    pub view: QuickInsertView,
    pub query: String,
    pub limit: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuickInsertOutcome {
    Copied,
    Inserted,
    ClipboardStaged,
}

pub struct QuickInsertService {
    library: Library,
    clipboard: Arc<ClipboardService>,
    platform: Arc<dyn ClipboardPlatform>,
    target: Mutex<Option<PasteTarget>>,
}

impl QuickInsertService {
    pub fn new(
        library: Library,
        clipboard: Arc<ClipboardService>,
        platform: Arc<dyn ClipboardPlatform>,
    ) -> Self {
        Self {
            library,
            clipboard,
            platform,
            target: Mutex::new(None),
        }
    }

    pub fn list(&self, request: &QuickInsertRequest) -> Result<Vec<QuickInsertItem>> {
        let view = match request.view {
            QuickInsertView::History => LibraryView::History,
            QuickInsertView::Favorites => LibraryView::Favorites,
            QuickInsertView::Snippets => LibraryView::Snippets,
        };
        let limit = request.limit.clamp(1, 200);
        Ok(self
            .library
            .list(view, &request.query, limit)?
            .into_iter()
            .map(to_item)
            .collect())
    }

    pub fn begin_session(&self) -> Result<bool> {
        let target = self
            .platform
            .capture_target()
            .map_err(|error| ClipboardError::from(error))?;
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
                self.clipboard.copy_representations(&payload)?;
                match self
                    .platform
                    .paste_to_target(&target)
                    .map_err(|error| ClipboardError::from(error))?
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

    pub fn set_favorite(&self, source: QuickInsertSource, id: i64, pinned: bool) -> Result<bool> {
        match source {
            QuickInsertSource::History => Ok(self.library.set_favorite(id, pinned)?),
            QuickInsertSource::Favorite if !pinned => Ok(self.library.delete(LibraryItemKind::Favorite, id)?),
            QuickInsertSource::Favorite => Ok(true),
            QuickInsertSource::Snippet => Ok(false),
        }
    }

    pub fn delete(&self, source: QuickInsertSource, id: i64) -> Result<bool> {
        Ok(self.library.delete(source.kind(), id)?)
    }

    pub fn save_snippet(
        &self,
        id: Option<i64>,
        name: &str,
        content: &str,
        group_name: Option<&str>,
    ) -> Result<i64> {
        Ok(self.library.save_snippet(id, name, content, group_name)?)
    }

    pub fn library(&self) -> &Library {
        &self.library
    }
}

fn to_item(item: LibraryItem) -> QuickInsertItem {
    QuickInsertItem {
        id: item.id,
        source: match item.kind {
            LibraryItemKind::History => QuickInsertSource::History,
            LibraryItemKind::Favorite => QuickInsertSource::Favorite,
            LibraryItemKind::Snippet => QuickInsertSource::Snippet,
        },
        title: item.title,
        preview_text: item.preview_text,
        content_type: item.content_type,
        source_app: item.source_app,
        updated_at: item.updated_at,
        pinned: item.pinned,
        group_name: item.group_name,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use echo_clipboard::{fingerprint, ContentType, MemorySink, NormalizedCapture};
    use echo_platform::{
        ClipboardRepresentation, ClipboardSnapshot, InputTargetGeometry,
        PasteControlIdentity, PhysicalRect, PlatformChangePublisher, PlatformError,
        SourceContext,
    };
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicU64, Ordering};
    use tempfile::TempDir;

    struct TestPlatform {
        changes: PlatformChangePublisher,
        sequence: AtomicU64,
        target: Mutex<Option<PasteTarget>>,
        paste_result: Mutex<PasteDelivery>,
        writes: Mutex<Vec<Vec<ClipboardRepresentation>>>,
        snapshots: Mutex<VecDeque<ClipboardSnapshot>>,
    }

    impl TestPlatform {
        fn new() -> Self {
            Self {
                changes: PlatformChangePublisher::default(),
                sequence: AtomicU64::new(1),
                target: Mutex::new(Some(target())),
                paste_result: Mutex::new(PasteDelivery::Pasted),
                writes: Mutex::new(Vec::new()),
                snapshots: Mutex::new(VecDeque::new()),
            }
        }
    }

    impl ClipboardPlatform for TestPlatform {
        fn subscribe_changes(&self) -> echo_platform::PlatformChangeSubscription {
            self.changes.subscribe()
        }

        fn clipboard_sequence(&self) -> u64 {
            self.sequence.load(Ordering::Acquire)
        }

        fn read_clipboard(&self) -> std::result::Result<Option<ClipboardSnapshot>, PlatformError> {
            Ok(self.snapshots.lock().unwrap().pop_front())
        }

        fn write_clipboard(
            &self,
            representations: &[ClipboardRepresentation],
        ) -> std::result::Result<u64, PlatformError> {
            self.writes.lock().unwrap().push(representations.to_vec());
            Ok(self.sequence.fetch_add(1, Ordering::AcqRel) + 1)
        }

        fn capture_target(&self) -> std::result::Result<Option<PasteTarget>, PlatformError> {
            Ok(self.target.lock().unwrap().clone())
        }

        fn paste_to_target(
            &self,
            _target: &PasteTarget,
        ) -> std::result::Result<PasteDelivery, PlatformError> {
            Ok(*self.paste_result.lock().unwrap())
        }
    }

    fn target() -> PasteTarget {
        PasteTarget {
            window_id: 10,
            window_class: "Edit".to_owned(),
            process_id: 20,
            process_started_at: 30,
            focused_control: Some(PasteControlIdentity::NativeWindow { handle: 11, class_name: "Edit".to_owned() }),
            app_name: Some("Test".to_owned()),
            selected_text: None,
            is_single_line: Some(true),
            geometry: InputTargetGeometry {
                target: PhysicalRect { x: 0, y: 0, width: 1, height: 1 },
                work_area: PhysicalRect { x: 0, y: 0, width: 1, height: 1 },
                dpi: 96,
            },
        }
    }

    fn setup() -> (TempDir, QuickInsertService, Arc<TestPlatform>) {
        let root = TempDir::new().unwrap();
        let store = Arc::new(echo_storage::SharedClipboardStore::open(root.path()).unwrap());
        let library = Library::new(store.clone());
        let sink = Arc::new(MemorySink::default());
        let platform = Arc::new(TestPlatform::new());
        let clipboard = Arc::new(ClipboardService::new(platform.clone(), sink));
        (root, QuickInsertService::new(library, clipboard, platform.clone()), platform)
    }

    fn record(service: &QuickInsertService, text: &str) -> i64 {
        let representation = ClipboardRepresentation {
            format: "text".to_owned(), mime_type: "text/plain".to_owned(), bytes: text.as_bytes().to_vec(),
        };
        service.library().store().with_store(|store| store.record_capture(NormalizedCapture {
            sequence: 1,
            source: SourceContext::default(),
            content_type: ContentType::Text,
            preview_text: Some(text.to_owned()),
            searchable_text: Some(text.to_owned()),
            sanitized_html: None,
            fingerprint: fingerprint(std::slice::from_ref(&representation)),
            representations: vec![representation],
        })).unwrap().id
    }

    #[test]
    fn all_sources_support_copy_and_insert() {
        let (_root, service, platform) = setup();
        let id = record(&service, "history");
        service.library().set_favorite(id, true).unwrap();
        let favorite = service.list(&QuickInsertRequest { view: QuickInsertView::Favorites, query: String::new(), limit: 20 }).unwrap()[0].id;
        let snippet = service.save_snippet(None, "Snippet", "snippet", None).unwrap();
        service.begin_session().unwrap();
        assert_eq!(service.execute(QuickInsertSource::History, id, QuickInsertAction::Copy).unwrap(), QuickInsertOutcome::Copied);
        assert_eq!(service.execute(QuickInsertSource::Favorite, favorite, QuickInsertAction::Insert).unwrap(), QuickInsertOutcome::Inserted);
        service.begin_session().unwrap();
        assert_eq!(service.execute(QuickInsertSource::Snippet, snippet, QuickInsertAction::Insert).unwrap(), QuickInsertOutcome::Inserted);
        assert_eq!(platform.writes.lock().unwrap().len(), 3);
    }

    #[test]
    fn insert_requires_a_target_and_stages_clipboard_on_delivery_failure() {
        let (_root, service, platform) = setup();
        let id = record(&service, "value");
        *platform.target.lock().unwrap() = None;
        service.begin_session().unwrap();
        assert!(matches!(service.execute(QuickInsertSource::History, id, QuickInsertAction::Insert), Err(QuickInsertError::NoTarget)));
        *platform.target.lock().unwrap() = Some(target());
        *platform.paste_result.lock().unwrap() = PasteDelivery::Failed(PasteDeliveryFailure::InputUnavailable);
        service.begin_session().unwrap();
        assert!(matches!(service.execute(QuickInsertSource::History, id, QuickInsertAction::Insert), Err(QuickInsertError::DeliveryFailed(PasteDeliveryFailure::InputUnavailable))));
        assert_eq!(platform.writes.lock().unwrap().len(), 1);
    }

    #[test]
    fn list_caps_the_view_limit_and_keeps_sources_explicit() {
        let (_root, service, _platform) = setup();
        record(&service, "one");
        let items = service.list(&QuickInsertRequest { view: QuickInsertView::History, query: String::new(), limit: 10_000 }).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source, QuickInsertSource::History);
    }
}
