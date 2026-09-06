mod domain;
mod instrumentation;

mod history;
mod ingest;
mod preview;
mod quick_insert;
mod saved_items;
mod settings;
mod spaces;
mod ui_settings;
pub use spaces::{
    Space, SpaceAction, SpaceCommand, SpaceDraft, SpaceError, SpaceId, SpaceKind,
    SpaceMutationResult, SpacePage, SpaceStore,
};
pub use ui_settings::{
    Density, FrameRate, GraphicsMode, Motion, MotionSpeed, QueryOnSwitch, SettingsPatch,
    SettingsSnapshot, SideContent, SpaceViewMode, StartupSpace, SwitchShortcut, UiSettings,
};

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
    fingerprint, CaptureCommit, CaptureEvent, CaptureEventSubscription, CaptureOutcome,
    CapturePolicy, CaptureSettings, CapturedCapture, ClipboardError, ClipboardService,
    ClipboardSink, ContentIdentity, ContentType, NormalizedCapture, RecordResult,
    RepresentationIdentity, DEFAULT_CAPTURE_LIMIT_BYTES, INGESTION_QUEUE_CAPACITY,
};
pub use instrumentation::{OperationMetric, OperationMetrics};
pub use preview::{
    PreviewAsset, PreviewDisposition, Thumbnail, DEFAULT_THUMBNAIL_MAX_EDGE, THUMBNAIL_MIME_TYPE,
};
pub use quick_insert::{
    QuickInsertAction, QuickInsertError, QuickInsertItem, QuickInsertOutcome, QuickInsertPage,
    QuickInsertRequest, QuickInsertService, QuickInsertSource, QuickInsertView,
};
pub use saved_items::{
    is_text_like, normalize_icon_key, normalize_name, normalize_optional_name, normalize_tags,
    FavoriteDraft, FavoriteUpdate, SavedItem, SavedItemDraft, SavedItemValidationError,
};
pub use settings::{ClipboardSettings, ThemeMode};
