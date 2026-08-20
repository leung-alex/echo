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
    HistoryEntry, Library, LibraryError, LibraryItem, LibraryItemKind, LibraryStore, LibraryView,
};
pub use ingest::{
    fingerprint, CaptureOutcome, CaptureSettings, ClipboardError, ClipboardService, ClipboardSink,
    ContentType, MemorySink, NormalizedCapture, RecordResult,
};
pub use quick_insert::{
    QuickInsertAction, QuickInsertError, QuickInsertItem, QuickInsertOutcome, QuickInsertRequest,
    QuickInsertService, QuickInsertSource, QuickInsertView,
};
pub use saved_items::SavedItem;
pub use settings::ClipboardSettings;
