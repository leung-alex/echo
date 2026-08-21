use tauri::{Emitter, State};

use crate::{composition::EchoState, transport};

#[tauri::command]
pub(crate) fn settings_get(
    state: State<'_, EchoState>,
) -> Result<transport::ClipboardSettings, String> {
    state
        .library
        .settings()
        .map(Into::into)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn settings_update(
    app: tauri::AppHandle,
    state: State<'_, EchoState>,
    settings: transport::ClipboardSettings,
) -> Result<(), String> {
    state
        .library
        .update_settings(&settings.into())
        .map_err(|error| error.to_string())?;
    state
        .quick_insert
        .refresh_capture_configuration()
        .map_err(|error| error.to_string())?;
    let theme_event = state.apply_persisted_theme(&app)?;
    app.emit("echo-theme-changed", theme_event)
        .map_err(|error| error.to_string())
}
