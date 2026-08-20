use std::path::PathBuf;
use std::sync::Arc;
use std::thread;

use echo_engine::{
    CaptureEventSubscription, ClipboardPlatform, ClipboardService, ClipboardSink, Library,
    QuickInsertService,
};
#[cfg(not(windows))]
use echo_engine::{
    CapturePolicy, ClipboardRepresentation, ClipboardSnapshot, PasteDelivery, PasteTarget,
    PlatformChangePublisher, PlatformError,
};
use echo_storage::SharedClipboardStore;
use tauri::{Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::activation::PendingActivation;

pub(crate) struct EchoState {
    pub library: Library<SharedClipboardStore>,
    pub quick_insert: QuickInsertService<SharedClipboardStore>,
    pub clipboard: Arc<ClipboardService>,
    pub pending_activation: std::sync::Mutex<Option<PendingActivation>>,
}

impl EchoState {
    pub(crate) fn build() -> Result<Self, String> {
        let data_dir = echo_data_dir();
        let store =
            Arc::new(SharedClipboardStore::open(&data_dir).map_err(|error| error.to_string())?);
        if let Some(legacy_dir) = legacy_data_dir() {
            if legacy_dir.join("culsans.sqlite3").is_file() {
                store
                    .migrate_legacy(&legacy_dir)
                    .map_err(|error| error.to_string())?;
            }
        }
        let platform = make_platform()?;
        let sink: Arc<dyn ClipboardSink> = store.clone();
        let clipboard = Arc::new(ClipboardService::new(platform.clone(), sink));
        clipboard.start_maintenance();
        let library = Library::new(store.clone());
        let quick_insert = QuickInsertService::new(library.clone(), clipboard.clone(), platform);
        Ok(Self {
            library,
            quick_insert,
            clipboard,
            pending_activation: std::sync::Mutex::new(None),
        })
    }

    pub(crate) fn start_history_event_bridge(&self, app: &tauri::AppHandle) {
        let events = self.clipboard.subscribe_events();
        let app = app.clone();
        let _ = thread::Builder::new()
            .name("echo-history-events".to_owned())
            .spawn(move || forward_history_events(app, events));
    }
}

fn forward_history_events(app: tauri::AppHandle, events: CaptureEventSubscription) {
    let mut version = 0_u64;
    while events.recv().is_ok() {
        version = version.saturating_add(1);
        let _ = app.emit(
            "echo-history-changed",
            crate::transport::HistoryChangedEvent { version },
        );
    }
}

pub(crate) fn echo_data_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("ECHO_DATA_DIR") {
        return PathBuf::from(path);
    }
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|path| path.join("Echo"))
        .unwrap_or_else(|| PathBuf::from(".echo"))
}

fn legacy_data_dir() -> Option<PathBuf> {
    std::env::var_os("ECHO_LEGACY_DATA_DIR")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("LOCALAPPDATA").map(|path| PathBuf::from(path).join("Culsans"))
        })
}

#[cfg(windows)]
fn make_platform() -> Result<Arc<dyn ClipboardPlatform>, String> {
    Ok(Arc::new(echo_windows::WindowsPlatform::new()))
}

#[cfg(not(windows))]
fn make_platform() -> Result<Arc<dyn ClipboardPlatform>, String> {
    Ok(Arc::new(UnsupportedPlatform::default()))
}

#[cfg(not(windows))]
#[derive(Default)]
struct UnsupportedPlatform {
    changes: PlatformChangePublisher,
}

#[cfg(not(windows))]
impl ClipboardPlatform for UnsupportedPlatform {
    fn subscribe_changes(&self) -> echo_engine::PlatformChangeSubscription {
        self.changes.subscribe()
    }

    fn clipboard_sequence(&self) -> u64 {
        0
    }

    fn read_clipboard(
        &self,
        _policy: &CapturePolicy,
    ) -> Result<Option<ClipboardSnapshot>, PlatformError> {
        Err(PlatformError(
            "native clipboard is only available on Windows".to_owned(),
        ))
    }

    fn write_clipboard(
        &self,
        _representations: &[ClipboardRepresentation],
    ) -> Result<u64, PlatformError> {
        Err(PlatformError(
            "native clipboard is only available on Windows".to_owned(),
        ))
    }

    fn capture_target(&self) -> Result<Option<PasteTarget>, PlatformError> {
        Ok(None)
    }

    fn paste_to_target(&self, _target: &PasteTarget) -> Result<PasteDelivery, PlatformError> {
        Ok(PasteDelivery::Failed(
            echo_engine::PasteDeliveryFailure::InputUnavailable,
        ))
    }
}

pub(crate) fn create_main_window(app: &tauri::AppHandle) -> Result<(), String> {
    if app.get_webview_window("main").is_some() {
        return Ok(());
    }
    WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("Echo Recall")
        .inner_size(920.0, 680.0)
        .min_inner_size(640.0, 480.0)
        .visible(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(false)
        .skip_taskbar(false)
        .resizable(true)
        .focusable(true)
        .focused(true)
        .center()
        .build()
        .map(|_| ())
        .map_err(|error| error.to_string())
}
