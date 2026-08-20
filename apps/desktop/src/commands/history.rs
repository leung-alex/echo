use tauri::State;

use crate::{activation::PendingActivation, composition::EchoState};

#[tauri::command]
pub(crate) fn history_clear(state: State<'_, EchoState>) -> Result<(), String> {
    state
        .library
        .clear_history()
        .map_err(|error| error.to_string())?;
    state.quick_insert.request_maintenance();
    Ok(())
}

#[tauri::command]
pub(crate) fn activation_state(state: State<'_, EchoState>) -> Option<PendingActivation> {
    state
        .pending_activation
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
}

#[tauri::command]
pub(crate) fn activation_ack(state: State<'_, EchoState>, request_id: String) {
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
