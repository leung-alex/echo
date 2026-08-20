use std::fmt::Display;
use std::sync::Arc;

use crate::{
    ClipboardRepresentation, ClipboardSettings, SavedItem, SavedItemDraft, SavedItemUpdate,
    Thumbnail,
};
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
    pub saved_item_id: Option<i64>,
    pub byte_size: u64,
    pub thumbnail: Option<Thumbnail>,
}

pub trait LibraryStore: Send + Sync {
    type Error: Display + 'static;

    fn list_entries(
        &self,
        query: &str,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> std::result::Result<LibraryPage<HistoryEntry>, Self::Error>;
    fn entry(&self, id: i64) -> std::result::Result<Option<HistoryEntry>, Self::Error>;
    fn entry_payload(
        &self,
        id: i64,
    ) -> std::result::Result<Vec<ClipboardRepresentation>, Self::Error>;
    fn save_history_item(
        &self,
        draft: SavedItemDraft,
        payload: Vec<ClipboardRepresentation>,
    ) -> std::result::Result<SavedItem, Self::Error>;
    fn unsave_history_item(&self, history_id: i64) -> std::result::Result<bool, Self::Error>;
    fn update_saved_item(
        &self,
        id: i64,
        update: SavedItemUpdate,
    ) -> std::result::Result<SavedItem, Self::Error>;
    fn delete_entry(&self, id: i64) -> std::result::Result<bool, Self::Error>;
    fn clear_history(&self) -> std::result::Result<(), Self::Error>;
    fn settings(&self) -> std::result::Result<ClipboardSettings, Self::Error>;
    fn update_settings(&self, settings: &ClipboardSettings)
        -> std::result::Result<(), Self::Error>;
    fn list_saved_items(
        &self,
        query: &str,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> std::result::Result<LibraryPage<SavedItem>, Self::Error>;
    fn saved_item_payload(
        &self,
        id: i64,
    ) -> std::result::Result<Vec<ClipboardRepresentation>, Self::Error>;
    fn delete_saved_items(&self, ids: &[i64]) -> std::result::Result<usize, Self::Error>;
}

pub const DEFAULT_PAGE_SIZE: u32 = 50;
pub const MAX_PAGE_SIZE: u32 = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageCursor {
    pub updated_at: i64,
    pub id: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryPage<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<PageCursor>,
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
#[serde(rename_all = "snake_case")]
pub enum LibraryItemKind {
    History,
    SavedItem,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibraryItem {
    pub id: i64,
    pub kind: LibraryItemKind,
    pub name: Option<String>,
    pub preview_text: Option<String>,
    pub content_type: String,
    pub editable_text: Option<String>,
    pub tags: Vec<String>,
    pub source_app: Option<String>,
    pub updated_at: i64,
    pub saved_item_id: Option<i64>,
    pub is_independent: bool,
    pub thumbnail: Option<Thumbnail>,
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

    pub fn list(
        &self,
        view: LibraryView,
        query: &str,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> Result<LibraryPage<LibraryItem>> {
        match view {
            LibraryView::History => {
                let page = self
                    .store
                    .list_entries(query, limit, cursor)
                    .map_err(storage_error)?;
                Ok(LibraryPage {
                    items: page.items.into_iter().map(history_item).collect(),
                    next_cursor: page.next_cursor,
                })
            }
            LibraryView::Favorites => {
                let page = self
                    .store
                    .list_saved_items(query, limit, cursor)
                    .map_err(storage_error)?;
                Ok(LibraryPage {
                    items: page.items.into_iter().map(saved_item).collect(),
                    next_cursor: page.next_cursor,
                })
            }
        }
    }

    pub fn payload(&self, kind: LibraryItemKind, id: i64) -> Result<Vec<ClipboardRepresentation>> {
        match kind {
            LibraryItemKind::History => self.store.entry_payload(id).map_err(storage_error),
            LibraryItemKind::SavedItem => self.store.saved_item_payload(id).map_err(storage_error),
        }
    }

    pub fn save_history_item(&self, history_id: i64) -> Result<SavedItem> {
        let entry = self.store.entry(history_id).map_err(storage_error)?.ok_or(
            LibraryError::ItemNotFound {
                kind: "history",
                id: history_id,
            },
        )?;
        let payload = self
            .store
            .entry_payload(history_id)
            .map_err(storage_error)?;
        self.store
            .save_history_item(SavedItemDraft::from_history(&entry), payload)
            .map_err(storage_error)
    }

    pub fn unsave_history_item(&self, history_id: i64) -> Result<bool> {
        self.store
            .unsave_history_item(history_id)
            .map_err(storage_error)
    }

    pub fn set_favorite(&self, history_id: i64, saved: bool) -> Result<bool> {
        if saved {
            self.save_history_item(history_id).map(|_| true)
        } else {
            self.unsave_history_item(history_id)
        }
    }

    pub fn update_saved_item(&self, id: i64, update: SavedItemUpdate) -> Result<SavedItem> {
        self.store
            .update_saved_item(id, update)
            .map_err(storage_error)
    }

    pub fn delete(&self, kind: LibraryItemKind, id: i64) -> Result<bool> {
        match kind {
            LibraryItemKind::History => self.store.delete_entry(id).map_err(storage_error),
            LibraryItemKind::SavedItem => self
                .store
                .delete_saved_items(&[id])
                .map(|count| count == 1)
                .map_err(storage_error),
        }
    }

    pub fn delete_saved_items(&self, ids: &[i64]) -> Result<usize> {
        self.store.delete_saved_items(ids).map_err(storage_error)
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
        name: None,
        preview_text: entry.preview_text,
        content_type: entry.content_type,
        editable_text: None,
        tags: Vec::new(),
        source_app: entry.source_app,
        updated_at: entry.updated_at,
        saved_item_id: entry.saved_item_id,
        is_independent: false,
        thumbnail: entry.thumbnail,
    }
}

fn saved_item(item: SavedItem) -> LibraryItem {
    LibraryItem {
        id: item.id,
        kind: LibraryItemKind::SavedItem,
        name: Some(item.name),
        preview_text: item.preview_text,
        content_type: item.content_type,
        editable_text: item.editable_text,
        tags: item.tags,
        source_app: item.source_app,
        updated_at: item.updated_at,
        saved_item_id: Some(item.id),
        is_independent: item.is_independent,
        thumbnail: item.thumbnail,
    }
}
