use echo_engine::{
    ClipboardSettings as DomainSettings, QuickInsertAction as DomainAction,
    QuickInsertItem as DomainItem, QuickInsertOutcome as DomainOutcome,
    QuickInsertSource as DomainSource, QuickInsertView as DomainView,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuickInsertView {
    History,
    Favorites,
}

impl From<QuickInsertView> for DomainView {
    fn from(view: QuickInsertView) -> Self {
        match view {
            QuickInsertView::History => Self::History,
            QuickInsertView::Favorites => Self::Favorites,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum QuickInsertSource {
    History,
    Favorite,
}

impl From<QuickInsertSource> for DomainSource {
    fn from(source: QuickInsertSource) -> Self {
        match source {
            QuickInsertSource::History => Self::History,
            QuickInsertSource::Favorite => Self::Favorite,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum QuickInsertAction {
    Copy,
    Insert,
}

impl From<QuickInsertAction> for DomainAction {
    fn from(action: QuickInsertAction) -> Self {
        match action {
            QuickInsertAction::Copy => Self::Copy,
            QuickInsertAction::Insert => Self::Insert,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

impl From<DomainSource> for QuickInsertSource {
    fn from(source: DomainSource) -> Self {
        match source {
            DomainSource::History => Self::History,
            DomainSource::Favorite => Self::Favorite,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum QuickInsertOutcome {
    Copied,
    Inserted,
    ClipboardStaged,
}

impl From<DomainOutcome> for QuickInsertOutcome {
    fn from(outcome: DomainOutcome) -> Self {
        match outcome {
            DomainOutcome::Copied => Self::Copied,
            DomainOutcome::Inserted => Self::Inserted,
            DomainOutcome::ClipboardStaged => Self::ClipboardStaged,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PasteSession {
    pub has_target: bool,
}

impl From<DomainItem> for QuickInsertItem {
    fn from(item: DomainItem) -> Self {
        Self {
            id: item.id,
            source: item.source.into(),
            title: item.title,
            preview_text: item.preview_text,
            content_type: item.content_type,
            source_app: item.source_app,
            updated_at: item.updated_at,
            pinned: item.pinned,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardSettings {
    pub history_enabled: bool,
    pub record_sensitive: bool,
    pub store_window_titles: bool,
    pub max_entries: u32,
    pub max_total_bytes: u64,
    pub max_item_bytes: u64,
}

impl From<DomainSettings> for ClipboardSettings {
    fn from(settings: DomainSettings) -> Self {
        Self {
            history_enabled: settings.history_enabled,
            record_sensitive: settings.record_sensitive,
            store_window_titles: settings.store_window_titles,
            max_entries: settings.max_entries,
            max_total_bytes: settings.max_total_bytes,
            max_item_bytes: settings.max_item_bytes,
        }
    }
}

impl From<ClipboardSettings> for DomainSettings {
    fn from(settings: ClipboardSettings) -> Self {
        Self {
            history_enabled: settings.history_enabled,
            record_sensitive: settings.record_sensitive,
            store_window_titles: settings.store_window_titles,
            max_entries: settings.max_entries,
            max_total_bytes: settings.max_total_bytes,
            max_item_bytes: settings.max_item_bytes,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImagePreview {
    pub mime_type: String,
    pub base64: String,
}
