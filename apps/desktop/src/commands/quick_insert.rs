use tauri::State;

use echo_engine::{LibraryItemKind, QuickInsertRequest};

use crate::{
    composition::EchoState,
    preview_protocol,
    transport::{self, QuickInsertAction, QuickInsertSource},
};

#[tauri::command]
pub(crate) fn quick_insert_list(
    state: State<'_, EchoState>,
    view: transport::QuickInsertView,
    query: String,
    limit: u32,
) -> Result<Vec<transport::QuickInsertItem>, String> {
    state
        .quick_insert
        .list(&QuickInsertRequest {
            view: view.into(),
            query,
            limit,
        })
        .map(|items| items.into_iter().map(Into::into).collect())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn quick_insert_begin_session(
    state: State<'_, EchoState>,
) -> Result<transport::PasteSession, String> {
    state
        .quick_insert
        .begin_session()
        .map(|has_target| transport::PasteSession { has_target })
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn quick_insert_execute(
    state: State<'_, EchoState>,
    source: QuickInsertSource,
    id: i64,
    action: QuickInsertAction,
) -> Result<transport::QuickInsertOutcome, String> {
    state
        .quick_insert
        .execute(source.into(), id, action.into())
        .map(Into::into)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn quick_insert_set_favorite(
    state: State<'_, EchoState>,
    source: QuickInsertSource,
    id: i64,
    saved: bool,
) -> Result<bool, String> {
    state
        .quick_insert
        .set_favorite(source.into(), id, saved)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn saved_item_update(
    state: State<'_, EchoState>,
    id: i64,
    update: transport::SavedItemUpdate,
) -> Result<(), String> {
    state
        .quick_insert
        .update_saved_item(
            id,
            echo_engine::SavedItemUpdate {
                name: update.name,
                tags: update.tags,
                editable_text: update.editable_text,
            },
        )
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn saved_item_delete(state: State<'_, EchoState>, id: i64) -> Result<bool, String> {
    state
        .quick_insert
        .delete_saved_items(&[id])
        .map(|count| count == 1)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn saved_items_delete_many(
    state: State<'_, EchoState>,
    ids: Vec<i64>,
) -> Result<usize, String> {
    state
        .quick_insert
        .delete_saved_items(&ids)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn quick_insert_delete(
    state: State<'_, EchoState>,
    source: QuickInsertSource,
    id: i64,
) -> Result<bool, String> {
    state
        .quick_insert
        .delete(source.into(), id)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub(crate) fn quick_insert_get_image(
    state: State<'_, EchoState>,
    source: QuickInsertSource,
    id: i64,
) -> Result<Option<transport::ImagePreview>, String> {
    let kind = match source {
        QuickInsertSource::History => LibraryItemKind::History,
        QuickInsertSource::Favorite => LibraryItemKind::SavedItem,
    };
    state
        .library
        .payload(kind, id)
        .map(|representations| {
            representations
                .into_iter()
                .find(|representation| representation.mime_type.starts_with("image/"))
                .map(preview_protocol::image_preview)
        })
        .map_err(|error| error.to_string())
}
