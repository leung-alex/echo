use std::sync::Arc;

use echo_platform::ClipboardRepresentation;
use echo_storage::{
    ClipboardEntry, ClipboardSettings, SavedInsertItem, SharedClipboardStore,
    Snippet, StorageError,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum LibraryError {
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("library item {kind}:{id} does not exist")]
    ItemNotFound { kind: &'static str, id: i64 },
}

pub type Result<T> = std::result::Result<T, LibraryError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LibraryView {
    History,
    Favorites,
    Snippets,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LibraryItemKind {
    History,
    Favorite,
    Snippet,
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
    pub group_name: Option<String>,
    pub snippet_content: Option<String>,
}

#[derive(Clone)]
pub struct Library {
    store: Arc<SharedClipboardStore>,
}

impl Library {
    pub fn new(store: Arc<SharedClipboardStore>) -> Self {
        Self { store }
    }

    pub fn store(&self) -> &Arc<SharedClipboardStore> {
        &self.store
    }

    pub fn list(&self, view: LibraryView, query: &str, limit: u32) -> Result<Vec<LibraryItem>> {
        match view {
            LibraryView::History => Ok(self
                .store
                .list_entries(query, limit)?
                .into_iter()
                .map(history_item)
                .collect()),
            LibraryView::Favorites => Ok(self
                .store
                .list_saved_items(query, limit)?
                .into_iter()
                .map(favorite_item)
                .collect()),
            LibraryView::Snippets => Ok(self
                .store
                .snippets(query)?
                .into_iter()
                .map(snippet_item)
                .collect()),
        }
    }

    pub fn payload(&self, kind: LibraryItemKind, id: i64) -> Result<Vec<ClipboardRepresentation>> {
        match kind {
            LibraryItemKind::History => self
                .store
                .entry_payload(id)
                .map_err(LibraryError::from),
            LibraryItemKind::Favorite => self
                .store
                .saved_item_payload(id)
                .map_err(LibraryError::from),
            LibraryItemKind::Snippet => {
                let snippet = self
                    .store
                    .snippets("")?
                    .into_iter()
                    .find(|snippet| snippet.id == id)
                    .ok_or(LibraryError::ItemNotFound { kind: "snippet", id })?;
                Ok(vec![ClipboardRepresentation {
                    format: "text".to_owned(),
                    mime_type: "text/plain;charset=utf-8".to_owned(),
                    bytes: snippet.content.into_bytes(),
                }])
            }
        }
    }

    pub fn set_favorite(&self, history_id: i64, pinned: bool) -> Result<bool> {
        Ok(self.store.set_pinned(history_id, pinned)?)
    }

    pub fn delete(&self, kind: LibraryItemKind, id: i64) -> Result<bool> {
        match kind {
            LibraryItemKind::History => Ok(self.store.delete_entry(id)?),
            LibraryItemKind::Favorite => Ok(self.store.delete_saved_insert_item(id)?),
            LibraryItemKind::Snippet => Ok(self.store.delete_snippet(id)?),
        }
    }

    pub fn clear_history(&self) -> Result<()> {
        Ok(self.store.clear_history()?)
    }

    pub fn settings(&self) -> Result<ClipboardSettings> {
        Ok(self.store.settings()?)
    }

    pub fn update_settings(&self, settings: &ClipboardSettings) -> Result<()> {
        Ok(self.store.update_settings(settings)?)
    }

    pub fn save_snippet(
        &self,
        id: Option<i64>,
        name: &str,
        content: &str,
        group_name: Option<&str>,
    ) -> Result<i64> {
        Ok(self.store.save_snippet(id, name, content, group_name)?)
    }

    pub fn delete_snippet(&self, id: i64) -> Result<bool> {
        Ok(self.store.delete_snippet(id)?)
    }
}

fn history_item(entry: ClipboardEntry) -> LibraryItem {
    LibraryItem {
        id: entry.id,
        kind: LibraryItemKind::History,
        title: None,
        preview_text: entry.preview_text,
        content_type: entry.content_type,
        source_app: entry.source_app,
        updated_at: entry.updated_at,
        pinned: entry.pinned,
        group_name: None,
        snippet_content: None,
    }
}

fn favorite_item(item: SavedInsertItem) -> LibraryItem {
    LibraryItem {
        id: item.id,
        kind: LibraryItemKind::Favorite,
        title: None,
        preview_text: item.preview_text,
        content_type: item.content_type,
        source_app: item.source_app,
        updated_at: item.updated_at,
        pinned: true,
        group_name: None,
        snippet_content: None,
    }
}

fn snippet_item(snippet: Snippet) -> LibraryItem {
    LibraryItem {
        id: snippet.id,
        kind: LibraryItemKind::Snippet,
        title: Some(snippet.name),
        preview_text: Some(snippet.content.chars().take(600).collect()),
        content_type: "text".to_owned(),
        source_app: None,
        updated_at: snippet.updated_at,
        pinned: false,
        group_name: snippet.group_name,
        snippet_content: Some(snippet.content),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use echo_clipboard::{fingerprint, ContentType, NormalizedCapture};
    use echo_platform::SourceContext;
    use tempfile::TempDir;

    fn library() -> (TempDir, Library) {
        let root = TempDir::new().unwrap();
        let store = Arc::new(SharedClipboardStore::open(root.path()).unwrap());
        (root, Library::new(store))
    }

    fn record(library: &Library, text: &str) -> i64 {
        let representation = ClipboardRepresentation {
            format: "text".to_owned(),
            mime_type: "text/plain".to_owned(),
            bytes: text.as_bytes().to_vec(),
        };
        library
            .store()
            .with_store(|store| {
                store.record_capture(NormalizedCapture {
                    sequence: 1,
                    source: SourceContext::default(),
                    content_type: ContentType::Text,
                    preview_text: Some(text.to_owned()),
                    searchable_text: Some(text.to_owned()),
                    sanitized_html: None,
                    fingerprint: fingerprint(std::slice::from_ref(&representation)),
                    representations: vec![representation],
                })
            })
            .unwrap()
            .id
    }

    #[test]
    fn history_favorites_and_snippets_are_library_views() {
        let (_root, library) = library();
        let id = record(&library, "history value");
        assert_eq!(library.list(LibraryView::History, "value", 20).unwrap().len(), 1);
        library.set_favorite(id, true).unwrap();
        assert_eq!(library.list(LibraryView::Favorites, "value", 20).unwrap().len(), 1);
        let snippet_id = library.save_snippet(None, "Snippet", "snippet value", Some("Group")).unwrap();
        assert_eq!(library.list(LibraryView::Snippets, "value", 20).unwrap()[0].id, snippet_id);
        assert_eq!(library.payload(LibraryItemKind::Snippet, snippet_id).unwrap()[0].bytes, b"snippet value");
    }

    #[test]
    fn deleting_history_does_not_delete_favorite_snapshot() {
        let (_root, library) = library();
        let id = record(&library, "persisted favorite");
        library.set_favorite(id, true).unwrap();
        library.delete(LibraryItemKind::History, id).unwrap();
        assert_eq!(library.list(LibraryView::Favorites, "", 20).unwrap().len(), 1);
        assert_eq!(library.payload(LibraryItemKind::Favorite, library.list(LibraryView::Favorites, "", 20).unwrap()[0].id).unwrap()[0].bytes, b"persisted favorite");
    }
}
