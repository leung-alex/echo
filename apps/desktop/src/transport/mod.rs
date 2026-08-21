use echo_engine::{
    ClipboardSettings as DomainSettings, FavoriteDraft as DomainFavoriteDraft,
    FavoriteUpdate as DomainFavoriteUpdate, PageCursor as DomainCursor,
    QuickInsertAction as DomainAction, QuickInsertItem as DomainItem,
    QuickInsertOutcome as DomainOutcome, QuickInsertPage as DomainPage,
    QuickInsertSource as DomainSource, QuickInsertView as DomainView, SavedItem as DomainSavedItem,
    ThemeMode as DomainThemeMode, Thumbnail,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
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
    pub pinned_at: Option<i64>,
    pub icon_key: Option<String>,
    pub favorite_order: Option<i64>,
    pub preview: Option<PreviewAsset>,
}

/// Cursor variants are deliberately view-specific.  A cursor is an
/// authoritative storage position, not a pair of fields that the UI may
/// reinterpret after pagination.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QuickInsertCursor {
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

impl From<QuickInsertCursor> for DomainCursor {
    fn from(cursor: QuickInsertCursor) -> Self {
        match cursor {
            QuickInsertCursor::History {
                pinned_at,
                updated_at,
                id,
            } => Self::History {
                pinned_at,
                updated_at,
                id,
            },
            QuickInsertCursor::HistorySearch {
                pinned_at,
                relevance,
                updated_at,
                id,
            } => Self::HistorySearch {
                pinned_at,
                relevance,
                updated_at,
                id,
            },
            QuickInsertCursor::Favorites { favorite_order, id } => {
                Self::Favorites { favorite_order, id }
            }
            QuickInsertCursor::FavoritesSearch {
                relevance,
                favorite_order,
                id,
            } => Self::FavoritesSearch {
                relevance,
                favorite_order,
                id,
            },
        }
    }
}

impl From<DomainCursor> for QuickInsertCursor {
    fn from(cursor: DomainCursor) -> Self {
        match cursor {
            DomainCursor::History {
                pinned_at,
                updated_at,
                id,
            } => Self::History {
                pinned_at,
                updated_at,
                id,
            },
            DomainCursor::HistorySearch {
                pinned_at,
                relevance,
                updated_at,
                id,
            } => Self::HistorySearch {
                pinned_at,
                relevance,
                updated_at,
                id,
            },
            DomainCursor::Favorites { favorite_order, id } => {
                Self::Favorites { favorite_order, id }
            }
            DomainCursor::FavoritesSearch {
                relevance,
                favorite_order,
                id,
            } => Self::FavoritesSearch {
                relevance,
                favorite_order,
                id,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct QuickInsertPage {
    pub items: Vec<QuickInsertItem>,
    pub next_cursor: Option<QuickInsertCursor>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct HistoryChangedEvent {
    pub version: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum LibraryChangeKind {
    History,
    Favorites,
    HistoryAndFavorites,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryChangedEvent {
    pub version: u64,
    pub kind: LibraryChangeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivePanelChangedEvent {
    pub panel: QuickInsertView,
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
pub struct FavoriteDraft {
    pub content: String,
    pub name: Option<String>,
    pub icon_key: Option<String>,
    pub tags: Vec<String>,
}

impl From<FavoriteDraft> for DomainFavoriteDraft {
    fn from(draft: FavoriteDraft) -> Self {
        Self {
            content: draft.content,
            name: draft.name,
            icon_key: draft.icon_key,
            tags: draft.tags,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FavoriteUpdate {
    pub name: Option<String>,
    pub icon_key: Option<String>,
    pub tags: Vec<String>,
    pub editable_text: Option<String>,
}

impl From<FavoriteUpdate> for DomainFavoriteUpdate {
    fn from(update: FavoriteUpdate) -> Self {
        Self {
            name: update.name,
            icon_key: update.icon_key,
            tags: update.tags,
            editable_text: update.editable_text,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct HistoryIds {
    pub ids: Vec<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct FavoriteReorderRequest {
    pub ordered_ids: Vec<i64>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

impl From<DomainThemeMode> for ThemeMode {
    fn from(mode: DomainThemeMode) -> Self {
        match mode {
            DomainThemeMode::System => Self::System,
            DomainThemeMode::Light => Self::Light,
            DomainThemeMode::Dark => Self::Dark,
        }
    }
}

impl From<ThemeMode> for DomainThemeMode {
    fn from(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::System => Self::System,
            ThemeMode::Light => Self::Light,
            ThemeMode::Dark => Self::Dark,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThemeChangedEvent {
    pub mode: ThemeMode,
    pub native_mica: bool,
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
            pinned_at: item.pinned_at,
            icon_key: item.icon_key,
            favorite_order: item.favorite_order,
            preview: item.thumbnail.map(Into::into),
        }
    }
}

impl From<DomainSavedItem> for QuickInsertItem {
    fn from(item: DomainSavedItem) -> Self {
        Self {
            id: item.id,
            source: QuickInsertSource::Favorite,
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
            preview: item.thumbnail.map(Into::into),
        }
    }
}

impl From<DomainPage> for QuickInsertPage {
    fn from(page: DomainPage) -> Self {
        Self {
            items: page.items.into_iter().map(Into::into).collect(),
            next_cursor: page.next_cursor.map(Into::into),
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
    pub theme: ThemeMode,
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
            theme: settings.theme.into(),
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
            theme: settings.theme.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ClipboardSettings, QuickInsertCursor, ThemeMode};
    use echo_engine::{PageCursor, ThemeMode as DomainThemeMode};

    #[test]
    fn cursor_transport_round_trips_each_authoritative_order() {
        let cursors = [
            QuickInsertCursor::History {
                pinned_at: Some(7),
                updated_at: 8,
                id: 9,
            },
            QuickInsertCursor::HistorySearch {
                pinned_at: None,
                relevance: 10,
                updated_at: 11,
                id: 12,
            },
            QuickInsertCursor::Favorites {
                favorite_order: 13,
                id: 14,
            },
            QuickInsertCursor::FavoritesSearch {
                relevance: 15,
                favorite_order: 16,
                id: 17,
            },
        ];
        for cursor in cursors {
            let json = serde_json::to_string(&cursor).unwrap();
            let decoded: QuickInsertCursor = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, cursor);
        }
    }

    #[test]
    fn cursor_mapping_preserves_domain_variant() {
        let domain = PageCursor::FavoritesSearch {
            relevance: 3,
            favorite_order: 4,
            id: 5,
        };
        let transport = QuickInsertCursor::from(domain);
        assert_eq!(
            transport,
            QuickInsertCursor::FavoritesSearch {
                relevance: 3,
                favorite_order: 4,
                id: 5,
            }
        );
    }

    #[test]
    fn theme_mode_uses_stable_lowercase_wire_values() {
        assert_eq!(
            serde_json::to_string(&ThemeMode::System).unwrap(),
            "\"system\""
        );
        assert_eq!(
            serde_json::to_string(&ThemeMode::Light).unwrap(),
            "\"light\""
        );
        assert_eq!(serde_json::to_string(&ThemeMode::Dark).unwrap(), "\"dark\"");
    }

    #[test]
    fn settings_transport_preserves_persisted_domain_theme() {
        for mode in [
            DomainThemeMode::System,
            DomainThemeMode::Light,
            DomainThemeMode::Dark,
        ] {
            let mut domain = echo_engine::ClipboardSettings::default();
            domain.theme = mode;
            let transport = ClipboardSettings::from(domain.clone());
            assert_eq!(transport.theme, ThemeMode::from(mode));
            assert_eq!(echo_engine::ClipboardSettings::from(transport).theme, mode);
        }
    }
}
