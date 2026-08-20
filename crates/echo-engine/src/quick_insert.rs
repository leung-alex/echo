use std::sync::{Arc, Mutex};

use crate::{
    ClipboardError, ClipboardPlatform, ClipboardService, Library, LibraryError, LibraryItem,
    LibraryItemKind, LibraryStore, LibraryView, PasteDelivery, PasteDeliveryFailure, PasteTarget,
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
            Self::Favorite => LibraryItemKind::Favorite,
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
    pub title: Option<String>,
    pub preview_text: Option<String>,
    pub content_type: String,
    pub source_app: Option<String>,
    pub updated_at: i64,
    pub pinned: bool,
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

    pub fn set_favorite(&self, source: QuickInsertSource, id: i64, pinned: bool) -> Result<bool> {
        match source {
            QuickInsertSource::History => Ok(self.library.set_favorite(id, pinned)?),
            QuickInsertSource::Favorite if !pinned => {
                Ok(self.library.delete(LibraryItemKind::Favorite, id)?)
            }
            QuickInsertSource::Favorite => Ok(true),
        }
    }

    pub fn delete(&self, source: QuickInsertSource, id: i64) -> Result<bool> {
        Ok(self.library.delete(source.kind(), id)?)
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
            LibraryItemKind::Favorite => QuickInsertSource::Favorite,
        },
        title: item.title,
        preview_text: item.preview_text,
        content_type: item.content_type,
        source_app: item.source_app,
        updated_at: item.updated_at,
        pinned: item.pinned,
    }
}
