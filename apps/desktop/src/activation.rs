use echo_activation::{quick_insert_payload, ActivationEnvelope};
use serde_json::json;
use tauri::{Emitter, Manager};

use crate::composition::{create_main_window, EchoState};

#[derive(Debug, Clone, serde::Serialize)]
pub(crate) struct PendingActivation {
    pub request_id: String,
    pub route: String,
    pub query: Option<String>,
}

pub(crate) fn show_main(
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

pub(crate) fn handle_activation(
    app: &tauri::AppHandle,
    envelope: ActivationEnvelope,
) -> Result<(), String> {
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
        action => Err(format!("unsupported activation action {action}")),
    }
}
