use tauri::State;

use crate::{composition::EchoState, transport};

#[tauri::command]
pub(crate) fn settings_get(
    state: State<'_, EchoState>,
) -> Result<transport::ClipboardSettings, String> {
    state
        .library
        .settings()
        .map(|settings| transport::ClipboardSettings::with_theme(settings, state.theme()))
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn settings_update(
    app: tauri::AppHandle,
    state: State<'_, EchoState>,
    settings: transport::ClipboardSettings,
) -> Result<(), String> {
    let theme = settings.theme;
    state
        .library
        .update_settings(&settings.into())
        .map_err(|error| error.to_string())?;
    state.update_theme(&app, theme)?;
    state
        .quick_insert
        .refresh_capture_configuration()
        .map_err(|error| error.to_string())
}
