use echo_engine::{
    ClipboardSettings as DomainSettings, QuickInsertAction as DomainAction,
    QuickInsertItem as DomainItem, QuickInsertOutcome as DomainOutcome,
    QuickInsertSource as DomainSource, QuickInsertView as DomainView, Thumbnail,
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
    pub name: Option<String>,
    pub preview_text: Option<String>,
    pub content_type: String,
    pub editable_text: Option<String>,
    pub tags: Vec<String>,
    pub source_app: Option<String>,
    pub updated_at: i64,
    pub saved_item_id: Option<i64>,
    pub is_independent: bool,
    pub preview: Option<PreviewAsset>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PreviewAsset {
    pub url: String,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
    pub byte_size: u64,
    pub content_hash: String,
}

impl From<Thumbnail> for PreviewAsset {
    fn from(thumbnail: Thumbnail) -> Self {
        let content_hash = thumbnail.content_hash.clone();
        Self {
            url: preview_url(&content_hash),
            mime_type: thumbnail.mime_type,
            width: thumbnail.width,
            height: thumbnail.height,
            byte_size: thumbnail.byte_size,
            content_hash,
        }
    }
}

fn preview_url(content_hash: &str) -> String {
    if cfg!(windows) {
        format!("http://echo-preview.localhost/thumbnail/{content_hash}.png")
    } else {
        format!("echo-preview://localhost/thumbnail/{content_hash}.png")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SavedItemUpdate {
    pub name: String,
    pub tags: Vec<String>,
    pub editable_text: Option<String>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActivationRoute {
    History,
    QuickInsert,
    Settings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ActivationPayload {
    pub route: ActivationRoute,
    pub query: Option<String>,
    pub request_id: String,
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
            name: item.name,
            preview_text: item.preview_text,
            content_type: item.content_type,
            editable_text: item.editable_text,
            tags: item.tags,
            source_app: item.source_app,
            updated_at: item.updated_at,
            saved_item_id: item.saved_item_id,
            is_independent: item.is_independent,
            preview: item.thumbnail.map(Into::into),
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
