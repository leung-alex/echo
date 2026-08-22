use tauri::{Emitter, State};

use echo_engine::{FavoriteDraft, FavoriteUpdate, QuickInsertRequest};

use crate::{
    composition::EchoState,
    transport::{self, QuickInsertAction, QuickInsertSource},
};

#[tauri::command]
pub(crate) fn quick_insert_list(
    state: State<'_, EchoState>,
    view: transport::QuickInsertView,
    query: String,
    limit: u32,
    cursor: Option<transport::QuickInsertCursor>,
) -> Result<transport::QuickInsertPage, String> {
    state
        .quick_insert
        .list(&QuickInsertRequest {
            view: view.into(),
            query,
            limit,
            cursor: cursor.map(Into::into),
        })
        .map(Into::into)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn quick_insert_begin_session(
    state: State<'_, EchoState>,
) -> Result<transport::PasteSession, String> {
    state
        .begin_quick_insert_session()
        .map(|has_target| transport::PasteSession { has_target })
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn quick_insert_clear_session(state: State<'_, EchoState>) {
    state.clear_quick_insert_session();
}

#[tauri::command]
pub(crate) fn quick_insert_execute(
    state: State<'_, EchoState>,
    source: QuickInsertSource,
    id: i64,
    action: QuickInsertAction,
) -> Result<transport::QuickInsertOutcome, String> {
    let outcome = state
        .quick_insert
        .execute(source.into(), id, action.into())
        .map(Into::into)
        .map_err(|error| error.to_string())?;
    if matches!(
        outcome,
        transport::QuickInsertOutcome::Copied | transport::QuickInsertOutcome::Inserted
    ) {
        state.clear_quick_insert_session_marker();
    }
    Ok(outcome)
}

#[tauri::command]
pub(crate) fn quick_insert_move_history_to_favorite(
    state: State<'_, EchoState>,
    id: i64,
) -> Result<transport::QuickInsertItem, String> {
    state
        .quick_insert
        .move_history_to_favorite(id)
        .map(Into::into)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn quick_insert_move_history_many_to_favorites(
    state: State<'_, EchoState>,
    request: transport::HistoryIds,
) -> Result<Vec<transport::QuickInsertItem>, String> {
    state
        .quick_insert
        .move_history_many_to_favorites(&request.ids)
        .map(|items| items.into_iter().map(Into::into).collect())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn quick_insert_create_favorite(
    state: State<'_, EchoState>,
    draft: transport::FavoriteDraft,
) -> Result<transport::QuickInsertItem, String> {
    state
        .quick_insert
        .create_favorite(FavoriteDraft::from(draft))
        .map(Into::into)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn quick_insert_update_favorite(
    state: State<'_, EchoState>,
    id: i64,
    update: transport::FavoriteUpdate,
) -> Result<transport::QuickInsertItem, String> {
    state
        .quick_insert
        .update_favorite(id, FavoriteUpdate::from(update))
        .map(Into::into)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn quick_insert_pin_history(
    state: State<'_, EchoState>,
    id: i64,
) -> Result<bool, String> {
    state
        .quick_insert
        .pin_history(id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn quick_insert_unpin_history(
    state: State<'_, EchoState>,
    id: i64,
) -> Result<bool, String> {
    state
        .quick_insert
        .unpin_history(id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn quick_insert_pin_history_many(
    state: State<'_, EchoState>,
    request: transport::HistoryIds,
) -> Result<usize, String> {
    state
        .quick_insert
        .pin_history_many(&request.ids)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn quick_insert_delete_history_many(
    state: State<'_, EchoState>,
    request: transport::HistoryIds,
) -> Result<usize, String> {
    state
        .quick_insert
        .delete_history_many(&request.ids)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn quick_insert_clear_unpinned_history(
    state: State<'_, EchoState>,
) -> Result<usize, String> {
    state
        .quick_insert
        .clear_unpinned_history()
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn quick_insert_reorder_favorites(
    state: State<'_, EchoState>,
    request: transport::FavoriteReorderRequest,
) -> Result<(), String> {
    state
        .quick_insert
        .reorder_favorites(&request.ordered_ids)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn quick_insert_delete_favorite(
    state: State<'_, EchoState>,
    id: i64,
) -> Result<bool, String> {
    state
        .quick_insert
        .delete_favorite(id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn quick_insert_activate_panel(
    app: tauri::AppHandle,
    state: State<'_, EchoState>,
    panel: transport::QuickInsertView,
) -> Result<(), String> {
    state.set_active_panel(panel);
    app.emit(
        "echo-active-panel-changed",
        transport::ActivePanelChangedEvent { panel },
    )
    .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn quick_insert_active_panel(state: State<'_, EchoState>) -> transport::QuickInsertView {
    state.active_panel()
}
