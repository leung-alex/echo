//! Spaces organize existing Saved Items; they never contain a second payload model.
use crate::{FavoriteDraft, LibraryPage, PageCursor, SavedItem, SettingsPatch, SettingsSnapshot};
use serde::{Deserialize, Serialize};
use std::fmt;
use unicode_normalization::UnicodeNormalization;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SpaceId(pub i64);
impl SpaceId {
    pub const HISTORY: Self = Self(1);
    pub const FAVORITES: Self = Self(2);
    pub fn parse(value: &str) -> Option<Self> {
        value.parse::<i64>().ok().filter(|n| *n > 0).map(Self)
    }
    pub fn is_system(self) -> bool {
        self == Self::HISTORY || self == Self::FAVORITES
    }
}
impl fmt::Display for SpaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpaceKind {
    History,
    Favorites,
    Collection,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Space {
    pub id: SpaceId,
    pub kind: SpaceKind,
    pub title: String,
    pub icon_key: Option<String>,
    pub accent_key: String,
    pub description: String,
    pub order_key: i64,
    pub revision: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub item_count: u64,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceDraft {
    pub title: String,
    pub icon_key: Option<String>,
    pub accent_key: String,
    pub description: String,
}
impl SpaceDraft {
    pub fn normalize(mut self) -> Result<Self, SpaceError> {
        self.title = self.title.trim().nfc().collect();
        if !(1..=32).contains(&self.title.chars().count())
            || self.title.chars().any(char::is_control)
        {
            return Err(SpaceError::Invalid(
                "Space names must contain 1–32 characters".into(),
            ));
        }
        self.description = self.description.trim().nfc().collect();
        if self.description.chars().count() > 120 {
            return Err(SpaceError::Invalid(
                "Description exceeds 120 characters".into(),
            ));
        }
        self.icon_key = crate::normalize_icon_key(self.icon_key.as_deref())
            .map_err(|e| SpaceError::Invalid(e.to_string()))?;
        if !["amber", "blue", "green", "violet", "rose", "slate"]
            .contains(&self.accent_key.as_str())
        {
            return Err(SpaceError::Invalid("Unknown space accent".into()));
        }
        Ok(self)
    }
    pub fn normalized_title(&self) -> String {
        self.title.to_lowercase().nfc().collect()
    }
}
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpaceError {
    #[error("{0}")]
    Invalid(String),
    #[error("This system space cannot be changed or deleted")]
    SystemSpace,
    #[error("The space changed; refresh and retry")]
    Conflict,
    #[error("This page is outdated; reload the space")]
    StaleCursor,
    #[error("The space or item no longer exists")]
    NotFound,
    #[error("A space with this name already exists")]
    DuplicateName,
    #[error("Invalid or reused operation identity")]
    InvalidRequest,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpaceAction {
    Create(SpaceDraft),
    Update(SpaceDraft),
    Delete,
    MoveSpace(i32),
    AddItems(Vec<i64>),
    RemoveItem(i64),
    ReorderItem {
        id: i64,
        before: Option<i64>,
        delta: i32,
    },
    CreateItem(FavoriteDraft),
    MoveHistory(Vec<i64>),
    DuplicateItem(i64),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpaceCommand {
    pub space_id: Option<SpaceId>,
    pub expected_revision: Option<i64>,
    pub request_id: String,
    pub action: SpaceAction,
}
impl SpaceCommand {
    pub fn validate(&self) -> Result<(), SpaceError> {
        if self.request_id.is_empty()
            || self.request_id.len() > 128
            || !self
                .request_id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_:./".contains(&b))
        {
            return Err(SpaceError::InvalidRequest);
        }
        if matches!(self.action, SpaceAction::Create(_)) {
            if self.space_id.is_some() || self.expected_revision.is_some() {
                return Err(SpaceError::InvalidRequest);
            }
        } else if self.space_id.is_none_or(|id| id.0 <= 0)
            || self.expected_revision.is_none_or(|r| r <= 0)
        {
            return Err(SpaceError::InvalidRequest);
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpaceMutationResult {
    pub affected_spaces: Vec<SpaceId>,
    pub created_space: Option<SpaceId>,
    pub created_item: Option<i64>,
    pub migrated_count: usize,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpacePage {
    pub space_id: SpaceId,
    pub revision: i64,
    pub total: u64,
    pub page: LibraryPage<SavedItem>,
}
pub trait SpaceStore: crate::LibraryStore {
    fn list_spaces(&self) -> Result<Vec<Space>, Self::Error>;
    fn list_space_items(
        &self,
        space: SpaceId,
        query: &str,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> Result<SpacePage, Self::Error>;
    fn spaces_for_item(&self, item: i64) -> Result<Vec<SpaceId>, Self::Error>;
    fn apply_space_command(
        &self,
        command: SpaceCommand,
    ) -> Result<SpaceMutationResult, Self::Error>;
    fn settings_snapshot(&self) -> Result<SettingsSnapshot, Self::Error>;
    fn save_settings_patch(&self, patch: SettingsPatch) -> Result<SettingsSnapshot, Self::Error>;
    fn save_resume_space(&self, space: SpaceId) -> Result<(), Self::Error>;
}
#[cfg(test)]
mod tests {
    use super::*;
    fn draft(title: &str) -> SpaceDraft {
        SpaceDraft {
            title: title.into(),
            icon_key: None,
            accent_key: "amber".into(),
            description: String::new(),
        }
    }
    #[test]
    fn names_are_nfc_normalized_and_case_insensitive() {
        assert_eq!(
            draft("  Cafe\u{301} ")
                .normalize()
                .unwrap()
                .normalized_title(),
            draft("CAFÉ").normalize().unwrap().normalized_title()
        );
        assert!(draft("").normalize().is_err());
        assert!(draft("a\nb").normalize().is_err());
        assert!(draft(&"中".repeat(32)).normalize().is_ok());
        assert!(draft(&"中".repeat(33)).normalize().is_err());
    }
    #[test]
    fn ids_preserve_the_full_storage_range() {
        assert_eq!(
            SpaceId::parse(&i64::MAX.to_string()),
            Some(SpaceId(i64::MAX))
        );
        for value in ["", "0", "-1", "abc", "9223372036854775808"] {
            assert!(SpaceId::parse(value).is_none());
        }
    }
    #[test]
    fn mutations_require_an_expected_revision() {
        let mut cmd = SpaceCommand {
            space_id: Some(SpaceId(3)),
            expected_revision: None,
            request_id: "test:1".into(),
            action: SpaceAction::Delete,
        };
        assert!(cmd.validate().is_err());
        cmd.expected_revision = Some(1);
        assert!(cmd.validate().is_ok());
        cmd.request_id.clear();
        assert!(cmd.validate().is_err());
    }
}
