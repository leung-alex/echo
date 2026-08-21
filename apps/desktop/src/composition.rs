use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;

use echo_engine::{
    CaptureEvent, CaptureEventSubscription, ClipboardPlatform, ClipboardService, ClipboardSink,
    Library, QuickInsertService,
};
#[cfg(not(windows))]
use echo_engine::{
    CapturePolicy, ClipboardRepresentation, ClipboardSnapshot, PasteDelivery, PasteTarget,
    PlatformChangePublisher, PlatformError,
};
use echo_storage::SharedClipboardStore;
use serde::de::DeserializeOwned;
use tauri::window::{Effect, EffectsBuilder};
use tauri::{Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::activation::PendingActivation;
use crate::transport::{
    ActivationRoute, ActivePanelChangedEvent, HistoryChangedEvent, LibraryChangeKind,
    LibraryChangedEvent, QuickInsertView, ThemeChangedEvent, ThemeMode,
};

const MAIN_LABEL: &str = "main";
const FAVORITES_LABEL: &str = "favorites";
const THEME_FILE_NAME: &str = "theme.json";

pub(crate) struct EchoState {
    pub library: Library<SharedClipboardStore>,
    pub quick_insert: QuickInsertService<SharedClipboardStore>,
    pub clipboard: Arc<ClipboardService>,
    pub pending_activation: Mutex<Option<PendingActivation>>,
    active_panel: Mutex<QuickInsertView>,
    quick_insert_session: Mutex<Option<bool>>,
    theme: Mutex<ThemeMode>,
    theme_path: PathBuf,
}

impl EchoState {
    pub(crate) fn build() -> Result<Self, String> {
        let data_dir = echo_data_dir();
        let store =
            Arc::new(SharedClipboardStore::open(&data_dir).map_err(|error| error.to_string())?);
        let platform = make_platform()?;
        let sink: Arc<dyn ClipboardSink> = store.clone();
        let clipboard = Arc::new(ClipboardService::new(platform.clone(), sink));
        let library = Library::new(store.clone());
        let quick_insert = QuickInsertService::new(library.clone(), clipboard.clone(), platform);
        let theme_path = data_dir.join(THEME_FILE_NAME);
        let theme = load_theme(&theme_path);
        Ok(Self {
            library,
            quick_insert,
            clipboard,
            pending_activation: Mutex::new(None),
            active_panel: Mutex::new(QuickInsertView::History),
            quick_insert_session: Mutex::new(None),
            theme: Mutex::new(theme),
            theme_path,
        })
    }

    pub(crate) fn start_history_event_bridge(&self, app: &tauri::AppHandle) {
        let events = self.clipboard.subscribe_events();
        let app = app.clone();
        let _ = thread::Builder::new()
            .name("echo-library-events".to_owned())
            .spawn(move || forward_history_events(app, events));
    }

    pub(crate) fn begin_quick_insert_session(&self) -> Result<bool, echo_engine::QuickInsertError> {
        let mut session = self
            .quick_insert_session
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some(has_target) = *session {
            return Ok(has_target);
        }
        let has_target = self.quick_insert.begin_session()?;
        *session = Some(has_target);
        Ok(has_target)
    }

    pub(crate) fn begin_quick_insert_activation(
        &self,
    ) -> Result<bool, echo_engine::QuickInsertError> {
        self.clear_quick_insert_session();
        self.begin_quick_insert_session()
    }

    pub(crate) fn clear_quick_insert_session(&self) {
        self.quick_insert.clear_session();
        self.clear_quick_insert_session_marker();
    }

    pub(crate) fn clear_quick_insert_session_marker(&self) {
        *self
            .quick_insert_session
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
    }

    pub(crate) fn set_active_panel(&self, panel: QuickInsertView) {
        *self
            .active_panel
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = panel;
    }

    pub(crate) fn active_panel(&self) -> QuickInsertView {
        *self
            .active_panel
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    pub(crate) fn theme(&self) -> ThemeMode {
        *self.theme.lock().unwrap_or_else(|error| error.into_inner())
    }

    pub(crate) fn update_theme(
        &self,
        app: &tauri::AppHandle,
        mode: ThemeMode,
    ) -> Result<(), String> {
        persist_theme(&self.theme_path, mode)?;
        *self.theme.lock().unwrap_or_else(|error| error.into_inner()) = mode;
        let native_mica = apply_theme_to_windows(app, mode);
        app.emit(
            "echo-theme-changed",
            ThemeChangedEvent { mode, native_mica },
        )
        .map_err(|error| error.to_string())
    }
}

fn forward_history_events(app: tauri::AppHandle, events: CaptureEventSubscription) {
    let mut version = 0_u64;
    while let Ok(event) = events.recv() {
        version = version.saturating_add(1);
        let kind = match event {
            CaptureEvent::HistoryChanged { .. } => LibraryChangeKind::History,
            // Explicit History/Favorite mutations use the shared invalidation
            // seam. Both surfaces reload so a committed move is visible in
            // both windows without polling or UI-side choreography.
            CaptureEvent::HistoryInvalidated { .. } => LibraryChangeKind::HistoryAndFavorites,
        };
        let _ = app.emit(
            "echo-library-changed",
            LibraryChangedEvent { version, kind },
        );
        // Keep this small legacy event during the transport transition. It is
        // still typed and emitted from the same event-driven source; no caller
        // relies on a timer or a second data source.
        let _ = app.emit("echo-history-changed", HistoryChangedEvent { version });
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

fn load_theme(path: &Path) -> ThemeMode {
    read_json(path).unwrap_or(ThemeMode::System)
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    fs::read_to_string(path)
        .ok()
        .and_then(|contents| serde_json::from_str(&contents).ok())
}

fn persist_theme(path: &Path, mode: ThemeMode) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let contents = serde_json::to_vec(&mode).map_err(|error| error.to_string())?;
    fs::write(path, contents).map_err(|error| error.to_string())
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
    if app.get_webview_window(MAIN_LABEL).is_none() {
        WebviewWindowBuilder::new(app, MAIN_LABEL, WebviewUrl::App("index.html".into()))
            .title("Echo Recall")
            .inner_size(824.0, 814.0)
            .min_inner_size(640.0, 560.0)
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
            .map_err(|error| error.to_string())?;
    }
    create_favorites_window(app)?;
    if let Some(state) = app.try_state::<EchoState>() {
        let _ = apply_theme_to_windows(app, state.theme());
    }
    Ok(())
}

fn create_favorites_window(app: &tauri::AppHandle) -> Result<(), String> {
    if app.get_webview_window(FAVORITES_LABEL).is_some() {
        return Ok(());
    }
    let main = app
        .get_webview_window(MAIN_LABEL)
        .ok_or_else(|| "Echo main window is unavailable".to_owned())?;
    let builder =
        WebviewWindowBuilder::new(app, FAVORITES_LABEL, WebviewUrl::App("index.html".into()))
            .title("Echo Favorites")
            .inner_size(310.0, 575.0)
            .min_inner_size(250.0, 420.0)
            .visible(false)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .always_on_top(false)
            .skip_taskbar(true)
            .resizable(true)
            .focusable(true);
    builder
        .parent(&main)
        .map_err(|error| error.to_string())?
        .build()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub(crate) fn show_composition(app: &tauri::AppHandle) -> Result<(), String> {
    create_main_window(app)?;
    let main = app
        .get_webview_window(MAIN_LABEL)
        .ok_or_else(|| "Echo main window is unavailable".to_owned())?;
    let favorites = app
        .get_webview_window(FAVORITES_LABEL)
        .ok_or_else(|| "Echo Favorites window is unavailable".to_owned())?;
    if let Some(state) = app.try_state::<EchoState>() {
        let _ = apply_theme_to_windows(app, state.theme());
    }
    main.show().map_err(|error| error.to_string())?;
    reposition_favorites(app);
    favorites.show().map_err(|error| error.to_string())?;
    main.set_focus().map_err(|error| error.to_string())
}

pub(crate) fn hide_composition(app: &tauri::AppHandle) {
    if let Some(favorites) = app.get_webview_window(FAVORITES_LABEL) {
        let _ = favorites.hide();
    }
    if let Some(main) = app.get_webview_window(MAIN_LABEL) {
        let _ = main.hide();
    }
    if let Some(state) = app.try_state::<EchoState>() {
        state.clear_quick_insert_session();
    }
}

pub(crate) fn reposition_favorites(app: &tauri::AppHandle) {
    let Some(main) = app.get_webview_window(MAIN_LABEL) else {
        return;
    };
    let Some(favorites) = app.get_webview_window(FAVORITES_LABEL) else {
        return;
    };
    let Ok(main_position) = main.outer_position() else {
        return;
    };
    let Ok(main_monitor) = main.current_monitor().or_else(|_| main.primary_monitor()) else {
        return;
    };
    let Some(monitor) = main_monitor else {
        return;
    };
    let Ok(favorites_size) = favorites.outer_size() else {
        return;
    };
    let scale = monitor.scale_factor();
    let offset_x = (-251.0 * scale).round() as i32;
    let offset_y = (93.0 * scale).round() as i32;
    let work_area = monitor.work_area();
    let desired_x = main_position.x.saturating_add(offset_x);
    let desired_y = main_position.y.saturating_add(offset_y);
    let min_x = work_area.position.x;
    let min_y = work_area.position.y;
    let max_x = min_x
        .saturating_add(work_area.size.width as i32)
        .saturating_sub(favorites_size.width as i32);
    let max_y = min_y
        .saturating_add(work_area.size.height as i32)
        .saturating_sub(favorites_size.height as i32);
    let x = desired_x.clamp(min_x, max_x.max(min_x));
    let y = desired_y.clamp(min_y, max_y.max(min_y));
    let _ = favorites.set_position(PhysicalPosition::new(x, y));
}

pub(crate) fn apply_theme_to_windows(app: &tauri::AppHandle, mode: ThemeMode) -> bool {
    let mut native_mica = true;
    let effect = match mode {
        ThemeMode::System => Effect::Mica,
        ThemeMode::Light => Effect::MicaLight,
        ThemeMode::Dark => Effect::MicaDark,
    };
    let windows: Vec<WebviewWindow> = [MAIN_LABEL, FAVORITES_LABEL]
        .into_iter()
        .filter_map(|label| app.get_webview_window(label))
        .collect();
    if windows.len() != 2 {
        native_mica = false;
    }
    for window in windows {
        let theme = match mode {
            ThemeMode::System => None,
            ThemeMode::Light => Some(tauri::Theme::Light),
            ThemeMode::Dark => Some(tauri::Theme::Dark),
        };
        if window.set_theme(theme).is_err()
            || window
                .set_effects(EffectsBuilder::new().effect(effect).build())
                .is_err()
        {
            native_mica = false;
        }
    }
    #[cfg(not(windows))]
    {
        native_mica = false;
    }
    native_mica
}

pub(crate) fn active_panel_event(
    app: &tauri::AppHandle,
    state: &EchoState,
    panel: QuickInsertView,
) -> Result<(), String> {
    state.set_active_panel(panel);
    app.emit(
        "echo-active-panel-changed",
        ActivePanelChangedEvent { panel },
    )
    .map_err(|error| error.to_string())
}

pub(crate) fn route_panel(route: ActivationRoute) -> Option<QuickInsertView> {
    match route {
        ActivationRoute::History | ActivationRoute::QuickInsert => Some(QuickInsertView::History),
        ActivationRoute::Settings => None,
    }
}
