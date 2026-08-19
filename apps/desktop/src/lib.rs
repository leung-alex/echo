use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use echo_clipboard::{ClipboardService, ClipboardSink};
use echo_library::{Library, LibraryItemKind};
use echo_platform::ClipboardPlatform;
#[cfg(not(windows))]
use echo_platform::{
    ClipboardRepresentation, ClipboardSnapshot, PasteDelivery, PasteTarget,
    PlatformChangePublisher, PlatformError,
};
use echo_protocol::{decode_args, quick_insert_payload, save_snippet_payload, ActivationEnvelope};
use echo_quick_insert::{
    QuickInsertAction, QuickInsertItem, QuickInsertRequest, QuickInsertService, QuickInsertSource,
    QuickInsertView,
};
use echo_storage::{ClipboardSettings, SharedClipboardStore};
use serde_json::json;
use tauri::{
    menu::{MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    Emitter, Manager, State, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};

pub struct EchoState {
    pub library: Library,
    pub quick_insert: QuickInsertService,
    pub store: Arc<SharedClipboardStore>,
    pub clipboard: Arc<ClipboardService>,
    pending_activation: Mutex<Option<PendingActivation>>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct PendingActivation {
    request_id: String,
    route: String,
    query: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
struct ImagePreview {
    mime_type: String,
    base64: String,
}

impl EchoState {
    fn build() -> Result<Self, String> {
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
            store,
            clipboard,
            pending_activation: Mutex::new(None),
        })
    }
}

#[tauri::command]
fn quick_insert_list(
    state: State<'_, EchoState>,
    view: QuickInsertView,
    query: String,
    limit: u32,
) -> Result<Vec<QuickInsertItem>, String> {
    state
        .quick_insert
        .list(&QuickInsertRequest { view, query, limit })
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn quick_insert_begin_session(state: State<'_, EchoState>) -> Result<bool, String> {
    state
        .quick_insert
        .begin_session()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn quick_insert_execute(
    state: State<'_, EchoState>,
    source: QuickInsertSource,
    id: i64,
    action: QuickInsertAction,
) -> Result<String, String> {
    state
        .quick_insert
        .execute(source, id, action)
        .map(|outcome| serde_json::to_string(&outcome).unwrap_or_else(|_| "unknown".to_owned()))
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn quick_insert_set_favorite(
    state: State<'_, EchoState>,
    source: QuickInsertSource,
    id: i64,
    pinned: bool,
) -> Result<bool, String> {
    state
        .quick_insert
        .set_favorite(source, id, pinned)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn quick_insert_delete(
    state: State<'_, EchoState>,
    source: QuickInsertSource,
    id: i64,
) -> Result<bool, String> {
    state
        .quick_insert
        .delete(source, id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn quick_insert_get_image(
    state: State<'_, EchoState>,
    source: QuickInsertSource,
    id: i64,
) -> Result<Option<ImagePreview>, String> {
    let kind = match source {
        QuickInsertSource::History => LibraryItemKind::History,
        QuickInsertSource::Favorite => LibraryItemKind::Favorite,
        QuickInsertSource::Snippet => LibraryItemKind::Snippet,
    };
    state
        .library
        .payload(kind, id)
        .map(|representations| {
            representations
                .into_iter()
                .find(|representation| representation.mime_type.starts_with("image/"))
                .map(|representation| ImagePreview {
                    mime_type: representation.mime_type,
                    base64: base64::engine::general_purpose::STANDARD.encode(representation.bytes),
                })
        })
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn settings_get(state: State<'_, EchoState>) -> Result<ClipboardSettings, String> {
    state.library.settings().map_err(|error| error.to_string())
}

#[tauri::command]
fn settings_update(state: State<'_, EchoState>, settings: ClipboardSettings) -> Result<(), String> {
    state
        .library
        .update_settings(&settings)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn history_clear(state: State<'_, EchoState>) -> Result<(), String> {
    state
        .library
        .clear_history()
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn snippets_list(
    state: State<'_, EchoState>,
    query: String,
) -> Result<Vec<echo_storage::Snippet>, String> {
    state
        .store
        .snippets(&query)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn snippet_save(
    state: State<'_, EchoState>,
    id: Option<i64>,
    name: String,
    content: String,
    group_name: Option<String>,
) -> Result<i64, String> {
    state
        .quick_insert
        .save_snippet(id, &name, &content, group_name.as_deref())
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn snippet_delete(state: State<'_, EchoState>, id: i64) -> Result<bool, String> {
    state
        .library
        .delete_snippet(id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
fn activation_state(state: State<'_, EchoState>) -> Option<PendingActivation> {
    state
        .pending_activation
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
}

#[tauri::command]
fn activation_ack(state: State<'_, EchoState>, request_id: String) {
    let mut pending = state
        .pending_activation
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if pending
        .as_ref()
        .is_some_and(|activation| activation.request_id == request_id)
    {
        *pending = None;
    }
}

fn echo_data_dir() -> PathBuf {
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
    Ok(Arc::new(echo_platform_windows::WindowsPlatform::new()))
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
    fn subscribe_changes(&self) -> echo_platform::PlatformChangeSubscription {
        self.changes.subscribe()
    }

    fn clipboard_sequence(&self) -> u64 {
        0
    }

    fn read_clipboard(&self) -> Result<Option<ClipboardSnapshot>, PlatformError> {
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
            echo_platform::PasteDeliveryFailure::InputUnavailable,
        ))
    }
}

fn create_main_window(app: &tauri::AppHandle) -> Result<(), String> {
    if app.get_webview_window("main").is_some() {
        return Ok(());
    }
    WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
        .title("Echo Recall")
        .inner_size(920.0, 680.0)
        .min_inner_size(640.0, 480.0)
        .build()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn show_main(
    app: &tauri::AppHandle,
    route: &str,
    query: Option<&str>,
    request_id: &str,
) -> Result<(), String> {
    create_main_window(app)?;
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "Echo main window is unavailable".to_owned())?;
    window.show().map_err(|error| error.to_string())?;
    window.set_focus().map_err(|error| error.to_string())?;
    window
        .emit(
            "echo-activation",
            json!({"route": route, "query": query, "request_id": request_id}),
        )
        .map_err(|error| error.to_string())
}

fn handle_activation(app: &tauri::AppHandle, envelope: ActivationEnvelope) -> Result<(), String> {
    let state = app
        .try_state::<EchoState>()
        .ok_or_else(|| "Echo state is not initialized".to_owned())?;
    match envelope.action.as_str() {
        "echo.open" => {
            *state
                .pending_activation
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(PendingActivation {
                request_id: envelope.request_id.clone(),
                route: "history".to_owned(),
                query: None,
            });
            show_main(app, "history", None, &envelope.request_id)
        }
        "echo.quick_insert" => {
            let payload = quick_insert_payload(&envelope).map_err(|error| error.to_string())?;
            let _ = state.quick_insert.begin_session();
            *state
                .pending_activation
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(PendingActivation {
                request_id: envelope.request_id.clone(),
                route: "quick_insert".to_owned(),
                query: payload.query.clone(),
            });
            show_main(
                app,
                "quick_insert",
                payload.query.as_deref(),
                &envelope.request_id,
            )
        }
        "echo.settings" => {
            *state
                .pending_activation
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(PendingActivation {
                request_id: envelope.request_id.clone(),
                route: "settings".to_owned(),
                query: None,
            });
            show_main(app, "settings", None, &envelope.request_id)
        }
        "echo.save_snippet" => {
            let payload = save_snippet_payload(&envelope).map_err(|error| error.to_string())?;
            state
                .quick_insert
                .save_snippet(
                    None,
                    &payload.name,
                    &payload.content,
                    payload.group_name.as_deref(),
                )
                .map(|_| ())
                .map_err(|error| error.to_string())
        }
        action => Err(format!("unsupported activation action {action}")),
    }
}

fn create_tray(app: &tauri::AppHandle) -> Result<(), String> {
    let open = MenuItemBuilder::with_id("open", "Open Echo")
        .build(app)
        .map_err(|error| error.to_string())?;
    let quit = MenuItemBuilder::with_id("quit", "Quit Echo")
        .build(app)
        .map_err(|error| error.to_string())?;
    let menu = MenuBuilder::new(app)
        .items(&[&open, &quit])
        .build()
        .map_err(|error| error.to_string())?;
    TrayIconBuilder::new()
        .menu(&menu)
        .tooltip("Echo Recall")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => {
                let _ = show_main(app, "history", None, "tray-open");
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub fn run() {
    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Some(Ok(envelope)) = decode_args(argv.iter().map(String::as_str)) {
                let _ = handle_activation(app, envelope);
            }
        }))
        .invoke_handler(tauri::generate_handler![
            quick_insert_list,
            quick_insert_begin_session,
            quick_insert_execute,
            quick_insert_set_favorite,
            quick_insert_delete,
            quick_insert_get_image,
            settings_get,
            settings_update,
            history_clear,
            snippets_list,
            snippet_save,
            snippet_delete,
            activation_state,
            activation_ack,
        ])
        .setup(|app| {
            let state = EchoState::build().map_err(std::io::Error::other)?;
            app.manage(state);
            create_tray(app.handle()).map_err(std::io::Error::other)?;
            create_main_window(app.handle()).map_err(std::io::Error::other)?;
            let args = std::env::args().collect::<Vec<_>>();
            if let Some(Ok(envelope)) = decode_args(args.iter().map(String::as_str)) {
                handle_activation(app.handle(), envelope).map_err(std::io::Error::other)?;
            } else if let Some(window) = app.get_webview_window("main") {
                window.show().map_err(std::io::Error::other)?;
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        });
    builder
        .run(tauri::generate_context!())
        .expect("error while running Echo Recall");
}
