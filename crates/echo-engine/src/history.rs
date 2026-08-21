use std::fmt::Display;
use std::sync::Arc;

use crate::{
    ClipboardRepresentation, ClipboardSettings, FavoriteDraft, FavoriteUpdate, SavedItem, Thumbnail,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    pub id: i64,
    pub created_at: i64,
    pub updated_at: i64,
    /// Monotonic pin rank.  `Some` means the entry is pinned; larger values
    /// are newer pins and therefore appear first.
    pub pinned_at: Option<i64>,
    pub source_app: Option<String>,
    pub source_executable: Option<String>,
    pub source_window_title: Option<String>,
    pub content_type: String,
    pub preview_text: Option<String>,
    pub searchable_text: Option<String>,
    pub sanitized_html: Option<String>,
    pub fingerprint: String,
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
    fn move_history_to_favorite(
        &self,
        history_id: i64,
    ) -> std::result::Result<SavedItem, Self::Error>;
    fn move_history_many_to_favorites(
        &self,
        history_ids: &[i64],
    ) -> std::result::Result<Vec<SavedItem>, Self::Error>;
    fn create_favorite(&self, draft: FavoriteDraft) -> std::result::Result<SavedItem, Self::Error>;
    fn update_favorite(
        &self,
        id: i64,
        update: FavoriteUpdate,
    ) -> std::result::Result<SavedItem, Self::Error>;
    fn pin_history(&self, history_id: i64) -> std::result::Result<bool, Self::Error>;
    fn unpin_history(&self, history_id: i64) -> std::result::Result<bool, Self::Error>;
    fn pin_history_many(&self, history_ids: &[i64]) -> std::result::Result<usize, Self::Error>;
    fn delete_history_many(&self, history_ids: &[i64]) -> std::result::Result<usize, Self::Error>;
    fn delete_entry(&self, id: i64) -> std::result::Result<bool, Self::Error>;
    fn clear_unpinned_history(&self) -> std::result::Result<usize, Self::Error>;
    fn reorder_favorites(&self, ordered_ids: &[i64]) -> std::result::Result<(), Self::Error>;
    fn delete_favorite(&self, id: i64) -> std::result::Result<bool, Self::Error>;
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
pub enum PageCursor {
    History {
        pinned_at: Option<i64>,
        updated_at: i64,
        id: i64,
    },
    HistorySearch {
        pinned_at: Option<i64>,
        relevance: i64,
        updated_at: i64,
        id: i64,
    },
    Favorites {
        favorite_order: i64,
        id: i64,
    },
    FavoritesSearch {
        relevance: i64,
        favorite_order: i64,
        id: i64,
    },
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
    pub pinned_at: Option<i64>,
    pub icon_key: Option<String>,
    pub favorite_order: Option<i64>,
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

    pub fn move_history_to_favorite(&self, history_id: i64) -> Result<SavedItem> {
        self.store
            .move_history_to_favorite(history_id)
            .map_err(storage_error)
    }

    pub fn move_history_many_to_favorites(&self, history_ids: &[i64]) -> Result<Vec<SavedItem>> {
        self.store
            .move_history_many_to_favorites(history_ids)
            .map_err(storage_error)
    }

    pub fn create_favorite(&self, draft: FavoriteDraft) -> Result<SavedItem> {
        self.store.create_favorite(draft).map_err(storage_error)
    }

    pub fn update_favorite(&self, id: i64, update: FavoriteUpdate) -> Result<SavedItem> {
        self.store
            .update_favorite(id, update)
            .map_err(storage_error)
    }

    pub fn pin_history(&self, history_id: i64) -> Result<bool> {
        self.store.pin_history(history_id).map_err(storage_error)
    }

    pub fn unpin_history(&self, history_id: i64) -> Result<bool> {
        self.store.unpin_history(history_id).map_err(storage_error)
    }

    pub fn pin_history_many(&self, history_ids: &[i64]) -> Result<usize> {
        self.store
            .pin_history_many(history_ids)
            .map_err(storage_error)
    }

    pub fn delete_history_many(&self, history_ids: &[i64]) -> Result<usize> {
        self.store
            .delete_history_many(history_ids)
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

    pub fn clear_unpinned_history(&self) -> Result<usize> {
        self.store.clear_unpinned_history().map_err(storage_error)
    }

    pub fn reorder_favorites(&self, ordered_ids: &[i64]) -> Result<()> {
        self.store
            .reorder_favorites(ordered_ids)
            .map_err(storage_error)
    }

    pub fn delete_favorite(&self, id: i64) -> Result<bool> {
        self.store.delete_favorite(id).map_err(storage_error)
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
        pinned_at: entry.pinned_at,
        icon_key: None,
        favorite_order: None,
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
        pinned_at: None,
        icon_key: item.icon_key,
        favorite_order: Some(item.favorite_order),
        thumbnail: item.thumbnail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ThemeMode;
    use std::sync::Mutex;

    #[derive(Default)]
    struct SettingsStore(Mutex<ClipboardSettings>);

    fn unsupported<T>() -> std::result::Result<T, String> {
        Err("unsupported test operation".to_owned())
    }

    impl LibraryStore for SettingsStore {
        type Error = String;

        fn list_entries(
            &self,
            _query: &str,
            _limit: u32,
            _cursor: Option<PageCursor>,
        ) -> std::result::Result<LibraryPage<HistoryEntry>, Self::Error> {
            unsupported()
        }

        fn entry(&self, _id: i64) -> std::result::Result<Option<HistoryEntry>, Self::Error> {
            unsupported()
        }

        fn entry_payload(
            &self,
            _id: i64,
        ) -> std::result::Result<Vec<ClipboardRepresentation>, Self::Error> {
            unsupported()
        }

        fn move_history_to_favorite(
            &self,
            _history_id: i64,
        ) -> std::result::Result<SavedItem, Self::Error> {
            unsupported()
        }

        fn move_history_many_to_favorites(
            &self,
            _history_ids: &[i64],
        ) -> std::result::Result<Vec<SavedItem>, Self::Error> {
            unsupported()
        }

        fn create_favorite(
            &self,
            _draft: FavoriteDraft,
        ) -> std::result::Result<SavedItem, Self::Error> {
            unsupported()
        }

        fn update_favorite(
            &self,
            _id: i64,
            _update: FavoriteUpdate,
        ) -> std::result::Result<SavedItem, Self::Error> {
            unsupported()
        }

        fn pin_history(&self, _history_id: i64) -> std::result::Result<bool, Self::Error> {
            unsupported()
        }

        fn unpin_history(&self, _history_id: i64) -> std::result::Result<bool, Self::Error> {
            unsupported()
        }

        fn pin_history_many(
            &self,
            _history_ids: &[i64],
        ) -> std::result::Result<usize, Self::Error> {
            unsupported()
        }

        fn delete_history_many(
            &self,
            _history_ids: &[i64],
        ) -> std::result::Result<usize, Self::Error> {
            unsupported()
        }

        fn delete_entry(&self, _id: i64) -> std::result::Result<bool, Self::Error> {
            unsupported()
        }

        fn clear_unpinned_history(&self) -> std::result::Result<usize, Self::Error> {
            unsupported()
        }

        fn reorder_favorites(&self, _ordered_ids: &[i64]) -> std::result::Result<(), Self::Error> {
            unsupported()
        }

        fn delete_favorite(&self, _id: i64) -> std::result::Result<bool, Self::Error> {
            unsupported()
        }

        fn settings(&self) -> std::result::Result<ClipboardSettings, Self::Error> {
            Ok(self.0.lock().unwrap().clone())
        }

        fn update_settings(
            &self,
            settings: &ClipboardSettings,
        ) -> std::result::Result<(), Self::Error> {
            *self.0.lock().unwrap() = settings.clone();
            Ok(())
        }

        fn list_saved_items(
            &self,
            _query: &str,
            _limit: u32,
            _cursor: Option<PageCursor>,
        ) -> std::result::Result<LibraryPage<SavedItem>, Self::Error> {
            unsupported()
        }

        fn saved_item_payload(
            &self,
            _id: i64,
        ) -> std::result::Result<Vec<ClipboardRepresentation>, Self::Error> {
            unsupported()
        }

        fn delete_saved_items(&self, _ids: &[i64]) -> std::result::Result<usize, Self::Error> {
            unsupported()
        }
    }

    #[test]
    fn library_settings_interface_round_trips_theme() {
        let store = Arc::new(SettingsStore::default());
        let library = Library::new(store);
        assert_eq!(library.settings().unwrap().theme, ThemeMode::System);

        let mut settings = ClipboardSettings::default();
        settings.theme = ThemeMode::Dark;
        library.update_settings(&settings).unwrap();

        assert_eq!(library.settings().unwrap().theme, ThemeMode::Dark);
    }
}
