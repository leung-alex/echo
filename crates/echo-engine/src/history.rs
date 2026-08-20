use std::fmt::Display;
use std::sync::Arc;

use crate::{ClipboardRepresentation, ClipboardSettings, SavedItem};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub id: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub source_app: Option<String>,
    pub source_executable: Option<String>,
    pub source_window_title: Option<String>,
    pub content_type: String,
    pub preview_text: Option<String>,
    pub searchable_text: Option<String>,
    pub sanitized_html: Option<String>,
    pub fingerprint: String,
    pub pinned: bool,
    pub byte_size: u64,
}

pub trait LibraryStore: Send + Sync {
    type Error: Display + 'static;

    fn list_entries(
        &self,
        query: &str,
        limit: u32,
    ) -> std::result::Result<Vec<HistoryEntry>, Self::Error>;
    fn entry_payload(
        &self,
        id: i64,
    ) -> std::result::Result<Vec<ClipboardRepresentation>, Self::Error>;
    fn set_pinned(&self, entry_id: i64, pinned: bool) -> std::result::Result<bool, Self::Error>;
    fn delete_entry(&self, id: i64) -> std::result::Result<bool, Self::Error>;
    fn clear_history(&self) -> std::result::Result<(), Self::Error>;
    fn settings(&self) -> std::result::Result<ClipboardSettings, Self::Error>;
    fn update_settings(&self, settings: &ClipboardSettings)
        -> std::result::Result<(), Self::Error>;
    fn list_saved_items(
        &self,
        query: &str,
        limit: u32,
    ) -> std::result::Result<Vec<SavedItem>, Self::Error>;
    fn saved_item_payload(
        &self,
        id: i64,
    ) -> std::result::Result<Vec<ClipboardRepresentation>, Self::Error>;
    fn delete_saved_item(&self, id: i64) -> std::result::Result<bool, Self::Error>;
}

#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("storage error: {0}")]
    Storage(String),
    #[error("library item {kind}:{id} does not exist")]
    ItemNotFound { kind: &'static str, id: i64 },
}

pub type Result<T> = std::result::Result<T, LibraryError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LibraryView {
    History,
    Favorites,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LibraryItemKind {
    History,
    Favorite,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryItem {
    pub id: i64,
    pub kind: LibraryItemKind,
    pub title: Option<String>,
    pub preview_text: Option<String>,
    pub content_type: String,
    pub source_app: Option<String>,
    pub updated_at: i64,
    pub pinned: bool,
}

#[derive(Clone)]
pub struct Library<S: LibraryStore> {
    store: Arc<S>,
}

impl<S: LibraryStore> Library<S> {
    pub fn new(store: Arc<S>) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &Arc<S> {
        &self.store
    }

    pub fn list(&self, view: LibraryView, query: &str, limit: u32) -> Result<Vec<LibraryItem>> {
        let items = match view {
            LibraryView::History => self
                .store
                .list_entries(query, limit)
                .map_err(storage_error)?
                .into_iter()
                .map(history_item)
                .collect(),
            LibraryView::Favorites => self
                .store
                .list_saved_items(query, limit)
                .map_err(storage_error)?
                .into_iter()
                .map(favorite_item)
                .collect(),
        };
        Ok(items)
    }

    pub fn payload(&self, kind: LibraryItemKind, id: i64) -> Result<Vec<ClipboardRepresentation>> {
        match kind {
            LibraryItemKind::History => self.store.entry_payload(id).map_err(storage_error),
            LibraryItemKind::Favorite => self.store.saved_item_payload(id).map_err(storage_error),
        }
    }

    pub fn set_favorite(&self, history_id: i64, pinned: bool) -> Result<bool> {
        self.store
            .set_pinned(history_id, pinned)
            .map_err(storage_error)
    }

    pub fn delete(&self, kind: LibraryItemKind, id: i64) -> Result<bool> {
        match kind {
            LibraryItemKind::History => self.store.delete_entry(id).map_err(storage_error),
            LibraryItemKind::Favorite => self.store.delete_saved_item(id).map_err(storage_error),
        }
    }

    pub fn clear_history(&self) -> Result<()> {
        self.store.clear_history().map_err(storage_error)
    }

    pub fn settings(&self) -> Result<ClipboardSettings> {
        self.store.settings().map_err(storage_error)
    }

    pub fn update_settings(&self, settings: &ClipboardSettings) -> Result<()> {
        self.store.update_settings(settings).map_err(storage_error)
    }
}

fn storage_error<E: Display>(error: E) -> LibraryError {
    LibraryError::Storage(error.to_string())
}

fn history_item(entry: HistoryEntry) -> LibraryItem {
    LibraryItem {
        id: entry.id,
        kind: LibraryItemKind::History,
        title: None,
        preview_text: entry.preview_text,
        content_type: entry.content_type,
        source_app: entry.source_app,
        updated_at: entry.updated_at,
        pinned: entry.pinned,
    }
}

fn favorite_item(item: SavedItem) -> LibraryItem {
    LibraryItem {
        id: item.id,
        kind: LibraryItemKind::Favorite,
        title: None,
        preview_text: item.preview_text,
        content_type: item.content_type,
        source_app: item.source_app,
        updated_at: item.updated_at,
        pinned: true,
    }
}
