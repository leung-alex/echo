use std::sync::{Arc, Mutex};

use crate::{
    ClipboardError, ClipboardPlatform, ClipboardService, Library, LibraryError, LibraryItem,
    LibraryItemKind, LibraryStore, LibraryView, PasteDelivery, PasteDeliveryFailure, PasteTarget,
    SavedItem, SavedItemUpdate, Thumbnail,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum QuickInsertError {
    #[error(transparent)]
    Clipboard(#[from] ClipboardError),
    #[error(transparent)]
    Library(#[from] LibraryError),
    #[error("no safe paste target is available")]
    NoTarget,
    #[error("paste target was no longer valid")]
    InvalidTarget,
    #[error("paste was staged in the clipboard but could not be delivered: {0:?}")]
    DeliveryFailed(PasteDeliveryFailure),
}

pub type Result<T> = std::result::Result<T, QuickInsertError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuickInsertView {
    History,
    Favorites,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuickInsertSource {
    History,
    Favorite,
}

impl QuickInsertSource {
    fn kind(self) -> LibraryItemKind {
        match self {
            Self::History => LibraryItemKind::History,
            Self::Favorite => LibraryItemKind::SavedItem,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuickInsertAction {
    Copy,
    Insert,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickInsertItem {
    pub id: i64,
    pub source: QuickInsertSource,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickInsertRequest {
    pub view: QuickInsertView,
    pub query: String,
    pub limit: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuickInsertOutcome {
    Copied,
    Inserted,
    ClipboardStaged,
}

pub struct QuickInsertService<S: LibraryStore> {
    library: Library<S>,
    clipboard: Arc<ClipboardService>,
    platform: Arc<dyn ClipboardPlatform>,
    target: Mutex<Option<PasteTarget>>,
}

impl<S: LibraryStore> QuickInsertService<S> {
    pub fn new(
        library: Library<S>,
        clipboard: Arc<ClipboardService>,
        platform: Arc<dyn ClipboardPlatform>,
    ) -> Self {
        Self {
            library,
            clipboard,
            platform,
            target: Mutex::new(None),
        }
    }

    pub fn list(&self, request: &QuickInsertRequest) -> Result<Vec<QuickInsertItem>> {
        let view = match request.view {
            QuickInsertView::History => LibraryView::History,
            QuickInsertView::Favorites => LibraryView::Favorites,
        };
        let limit = request.limit.clamp(1, 200);
        Ok(self
            .library
            .list(view, &request.query, limit)?
            .into_iter()
            .map(to_item)
            .collect())
    }

    pub fn begin_session(&self) -> Result<bool> {
        let target = self
            .platform
            .capture_target()
            .map_err(ClipboardError::from)?;
        *self
            .target
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = target.clone();
        Ok(target.is_some())
    }

    pub fn clear_session(&self) {
        *self
            .target
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = None;
        self.platform.reset_paste_window_session();
    }

    pub fn execute(
        &self,
        source: QuickInsertSource,
        id: i64,
        action: QuickInsertAction,
    ) -> Result<QuickInsertOutcome> {
        let payload = self.library.payload(source.kind(), id)?;
        match action {
            QuickInsertAction::Copy => {
                self.clipboard.copy_representations(&payload)?;
                Ok(QuickInsertOutcome::Copied)
            }
            QuickInsertAction::Insert => {
                let target = self
                    .target
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .clone()
                    .ok_or(QuickInsertError::NoTarget)?;
                self.clipboard.copy_representations(&payload)?;
                match self
                    .platform
                    .paste_to_target(&target)
                    .map_err(ClipboardError::from)?
                {
                    PasteDelivery::Pasted => {
                        self.clear_session();
                        Ok(QuickInsertOutcome::Inserted)
                    }
                    PasteDelivery::Failed(reason) => Err(QuickInsertError::DeliveryFailed(reason)),
                }
            }
        }
    }

    pub fn set_favorite(&self, source: QuickInsertSource, id: i64, saved: bool) -> Result<bool> {
        match source {
            QuickInsertSource::History => {
                let changed = self.library.set_favorite(id, saved)?;
                if changed {
                    self.request_maintenance();
                }
                Ok(changed)
            }
            QuickInsertSource::Favorite if !saved => {
                let deleted = self.library.delete(LibraryItemKind::SavedItem, id)?;
                if deleted {
                    self.request_maintenance();
                }
                Ok(deleted)
            }
            QuickInsertSource::Favorite => Ok(true),
        }
    }

    pub fn delete(&self, source: QuickInsertSource, id: i64) -> Result<bool> {
        let deleted = self.library.delete(source.kind(), id)?;
        if deleted {
            self.request_maintenance();
        }
        Ok(deleted)
    }

    pub fn update_saved_item(&self, id: i64, update: SavedItemUpdate) -> Result<SavedItem> {
        let item = self.library.update_saved_item(id, update)?;
        self.request_maintenance();
        Ok(item)
    }

    pub fn delete_saved_items(&self, ids: &[i64]) -> Result<usize> {
        let deleted = self.library.delete_saved_items(ids)?;
        if deleted != 0 {
            self.request_maintenance();
        }
        Ok(deleted)
    }

    pub fn request_maintenance(&self) {
        self.clipboard.request_maintenance();
    }

    pub fn refresh_capture_configuration(&self) -> Result<()> {
        self.clipboard.refresh_configuration()?;
        Ok(())
    }

    pub fn library(&self) -> &Library<S> {
        &self.library
    }
}

fn to_item(item: LibraryItem) -> QuickInsertItem {
    QuickInsertItem {
        id: item.id,
        source: match item.kind {
            LibraryItemKind::History => QuickInsertSource::History,
            LibraryItemKind::SavedItem => QuickInsertSource::Favorite,
        },
        name: item.name,
        preview_text: item.preview_text,
        content_type: item.content_type,
        editable_text: item.editable_text,
        tags: item.tags,
        source_app: item.source_app,
        updated_at: item.updated_at,
        saved_item_id: item.saved_item_id,
        is_independent: item.is_independent,
        thumbnail: item.thumbnail,
    }
}
