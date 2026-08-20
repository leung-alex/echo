use echo_activation::{quick_insert_payload, ActivationEnvelope};
use tauri::{Emitter, Manager};

use crate::{
    composition::{create_main_window, EchoState},
    transport::{ActivationPayload, ActivationRoute},
};

pub(crate) type PendingActivation = ActivationPayload;

pub(crate) fn show_main(
    app: &tauri::AppHandle,
    route: ActivationRoute,
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
            ActivationPayload {
                route,
                query: query.map(str::to_owned),
                request_id: request_id.to_owned(),
            },
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
                route: ActivationRoute::History,
                query: None,
            });
            show_main(app, ActivationRoute::History, None, &envelope.request_id)
        }
        "echo.quick_insert" => {
            let payload = quick_insert_payload(&envelope).map_err(|error| error.to_string())?;
            let _ = state.quick_insert.begin_session();
            *state
                .pending_activation
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = Some(PendingActivation {
                request_id: envelope.request_id.clone(),
                route: ActivationRoute::QuickInsert,
                query: payload.query.clone(),
            });
            show_main(
                app,
                ActivationRoute::QuickInsert,
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
                route: ActivationRoute::Settings,
                query: None,
            });
            show_main(app, ActivationRoute::Settings, None, &envelope.request_id)
        }
        action => Err(format!("unsupported activation action {action}")),
    }
}
