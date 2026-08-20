mod domain;

pub mod history;
pub mod ingest;
pub mod preview;
pub mod quick_insert;
pub mod saved_items;
pub mod settings;

pub use domain::{
    ClipboardPlatform, ClipboardRepresentation, ClipboardSnapshot, FocusSafety, FocusedTarget,
    InputTargetGeometry, PasteControlIdentity, PasteDelivery, PasteDeliveryFailure, PasteTarget,
    PhysicalRect, PlatformChange, PlatformChangePublisher, PlatformChangeSubscription,
    PlatformError, SourceContext,
};
pub use history::{
    HistoryEntry, Library, LibraryError, LibraryItem, LibraryItemKind, LibraryPage, LibraryStore,
    LibraryView, PageCursor, DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE,
};
pub use ingest::{
    fingerprint, CaptureCommit, CaptureEvent, CaptureEventPublisher, CaptureEventSubscription,
    CaptureOutcome, CapturePolicy, CaptureSettings, CapturedCapture, ClipboardError,
    ClipboardService, ClipboardSink, ContentIdentity, ContentType, MemorySink, NormalizedCapture,
    RecordResult, RepresentationIdentity, DEFAULT_CAPTURE_LIMIT_BYTES, INGESTION_QUEUE_CAPACITY,
};
pub use preview::{PreviewAsset, Thumbnail, DEFAULT_THUMBNAIL_MAX_EDGE, THUMBNAIL_MIME_TYPE};
pub use quick_insert::{
    QuickInsertAction, QuickInsertError, QuickInsertItem, QuickInsertOutcome, QuickInsertPage,
    QuickInsertRequest, QuickInsertService, QuickInsertSource, QuickInsertView,
};
pub use saved_items::{
    is_text_like, normalize_name, normalize_tags, SavedItem, SavedItemDraft, SavedItemUpdate,
    SavedItemValidationError,
};
pub use settings::ClipboardSettings;
