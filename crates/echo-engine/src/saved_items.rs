use crate::{ClipboardRepresentation, HistoryEntry, Thumbnail};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedItem {
    pub id: i64,
    pub source_history_id: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub name: String,
    pub content_type: String,
    pub editable_text: Option<String>,
    pub source_app: Option<String>,
    pub source_executable: Option<String>,
    pub source_window_title: Option<String>,
    pub preview_text: Option<String>,
    pub byte_size: u64,
    pub tags: Vec<String>,
    /// Optional Apps SDK UI exported component key.  The engine treats this
    /// as opaque user data; catalog membership belongs to the UI layer.
    pub icon_key: Option<String>,
    /// Stable user-controlled display rank.  Lower values appear first.
    pub favorite_order: i64,
    pub thumbnail: Option<Thumbnail>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedItemDraft {
    pub source_history_id: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub name: String,
    pub content_type: String,
    pub editable_text: Option<String>,
    pub source_app: Option<String>,
    pub source_executable: Option<String>,
    pub source_window_title: Option<String>,
    pub preview_text: Option<String>,
    pub tags: Vec<String>,
    pub icon_key: Option<String>,
}

/// Input for the direct Favorites create flow.  Manual Favorites are
/// intentionally plain text in this release; history moves preserve the
/// complete original representation set through `SavedItemDraft`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FavoriteDraft {
    pub content: String,
    pub name: Option<String>,
    pub icon_key: Option<String>,
    pub tags: Vec<String>,
}

/// User-editable Favorite fields. `name: None` keeps the current generated or
/// user-provided name; `icon_key: None` explicitly means no icon. An empty
/// name is treated as omitted, while editable text must remain non-empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FavoriteUpdate {
    pub name: Option<String>,
    pub icon_key: Option<String>,
    pub tags: Vec<String>,
    pub editable_text: Option<String>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SavedItemValidationError {
    #[error("saved item name cannot be empty")]
    EmptyName,
    #[error("favorite content cannot be empty")]
    EmptyContent,
    #[error("saved item tag cannot be empty")]
    EmptyTag,
    #[error("saved item icon key is invalid")]
    InvalidIconKey,
}

impl SavedItemDraft {
    pub fn from_history(entry: &HistoryEntry) -> Self {
        let editable_text = crate::saved_items::is_text_like(&entry.content_type).then(|| {
            entry
                .searchable_text
                .clone()
                .or_else(|| entry.preview_text.clone())
                .unwrap_or_default()
        });
        let name = generated_name(entry, editable_text.as_deref());
        Self {
            source_history_id: Some(entry.id),
            created_at: entry.created_at,
            updated_at: entry.updated_at,
            name,
            content_type: entry.content_type.clone(),
            editable_text,
            source_app: entry.source_app.clone(),
            source_executable: entry.source_executable.clone(),
            source_window_title: entry.source_window_title.clone(),
            preview_text: entry.preview_text.clone(),
            tags: Vec::new(),
            icon_key: None,
        }
    }

    pub fn canonical_payload(
        &self,
        source_payload: &[ClipboardRepresentation],
    ) -> Vec<ClipboardRepresentation> {
        match &self.editable_text {
            Some(text) => vec![ClipboardRepresentation {
                format: "text".to_owned(),
                mime_type: "text/plain;charset=utf-8".to_owned(),
                bytes: text.as_bytes().to_vec(),
            }],
            None => source_payload.to_vec(),
        }
    }
}

impl FavoriteDraft {
    pub fn normalize(mut self) -> Result<Self, SavedItemValidationError> {
        if self.content.trim().is_empty() {
            return Err(SavedItemValidationError::EmptyContent);
        }
        self.name = normalize_optional_name(self.name.as_deref())?;
        self.icon_key = normalize_icon_key(self.icon_key.as_deref())?;
        self.tags = normalize_tags(&self.tags)?;
        Ok(self)
    }
}

impl FavoriteUpdate {
    pub fn normalize(mut self) -> Result<Self, SavedItemValidationError> {
        self.name = normalize_optional_name(self.name.as_deref())?;
        self.icon_key = normalize_icon_key(self.icon_key.as_deref())?;
        self.tags = normalize_tags(&self.tags)?;
        if self
            .editable_text
            .as_deref()
            .is_some_and(|text| text.trim().is_empty())
        {
            return Err(SavedItemValidationError::EmptyContent);
        }
        Ok(self)
    }
}

pub fn normalize_name(name: &str) -> Result<String, SavedItemValidationError> {
    let normalized = name.trim().to_owned();
    if normalized.is_empty() {
        Err(SavedItemValidationError::EmptyName)
    } else {
        Ok(normalized)
    }
}

pub fn normalize_optional_name(
    name: Option<&str>,
) -> Result<Option<String>, SavedItemValidationError> {
    let Some(name) = name else {
        return Ok(None);
    };
    let normalized = name.trim().to_owned();
    Ok((!normalized.is_empty()).then_some(normalized))
}

/// Validate and normalize a persisted Apps SDK UI icon export key without
/// importing the frontend package into the engine.
pub fn normalize_icon_key(
    icon_key: Option<&str>,
) -> Result<Option<String>, SavedItemValidationError> {
    let Some(icon_key) = icon_key else {
        return Ok(None);
    };
    let normalized = icon_key.trim();
    if normalized.is_empty() {
        return Ok(None);
    }
    if normalized.len() > 128
        || !normalized
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
    {
        return Err(SavedItemValidationError::InvalidIconKey);
    }
    Ok(Some(normalized.to_owned()))
}

pub fn normalize_tags(tags: &[String]) -> Result<Vec<String>, SavedItemValidationError> {
    let mut normalized = Vec::with_capacity(tags.len());
    for tag in tags {
        let display = tag.trim().to_owned();
        if display.is_empty() {
            return Err(SavedItemValidationError::EmptyTag);
        }
        let key = casefold_key(&display);
        if !normalized
            .iter()
            .any(|existing: &String| casefold_key(existing) == key)
        {
            normalized.push(display);
        }
    }
    Ok(normalized)
}

pub fn is_text_like(content_type: &str) -> bool {
    matches!(
        content_type.to_ascii_lowercase().as_str(),
        "text" | "html" | "rtf"
    )
}

fn generated_name(entry: &HistoryEntry, body: Option<&str>) -> String {
    body.or(entry.preview_text.as_deref())
        .and_then(first_non_empty_line)
        .map(|line| {
            let line = if entry.content_type == "files" {
                final_file_name(line).unwrap_or(line)
            } else {
                line
            };
            truncate_name(line, 120)
        })
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| match entry.content_type.as_str() {
            "image" => entry
                .source_app
                .as_deref()
                .map(|source| format!("Image from {source}"))
                .unwrap_or_else(|| "Saved image".to_owned()),
            "files" => "Files".to_owned(),
            "html" => "HTML content".to_owned(),
            "rtf" => "Rich text".to_owned(),
            _ => "Saved item".to_owned(),
        })
}

fn first_non_empty_line(value: &str) -> Option<&str> {
    value.lines().map(str::trim).find(|line| !line.is_empty())
}

fn final_file_name(value: &str) -> Option<&str> {
    let trimmed = value.trim_end_matches(|character| character == '\\' || character == '/');
    let name = trimmed
        .rsplit(|character| character == '\\' || character == '/')
        .next()
        .unwrap_or(trimmed);
    (!name.is_empty()).then_some(name)
}

fn truncate_name(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn casefold_key(value: &str) -> String {
    value.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{final_file_name, FavoriteDraft, FavoriteUpdate, SavedItemValidationError};

    #[test]
    fn final_file_name_handles_windows_paths_and_plain_names() {
        assert_eq!(
            final_file_name(r"C:\Users\Verifier\report.txt"),
            Some("report.txt")
        );
        assert_eq!(final_file_name("report.txt"), Some("report.txt"));
        assert_eq!(final_file_name("/tmp/report.txt"), Some("report.txt"));
    }

    #[test]
    fn manual_favorite_draft_requires_content_and_normalizes_optional_fields() {
        let draft = FavoriteDraft {
            content: "  reusable text  ".to_owned(),
            name: Some("  Greeting  ".to_owned()),
            icon_key: Some(" Star ".to_owned()),
            tags: vec![" Work ".to_owned(), "work".to_owned()],
        }
        .normalize()
        .unwrap();
        assert_eq!(draft.content, "  reusable text  ");
        assert_eq!(draft.name.as_deref(), Some("Greeting"));
        assert_eq!(draft.icon_key.as_deref(), Some("Star"));
        assert_eq!(draft.tags, vec!["Work"]);

        let error = FavoriteDraft {
            content: " \n".to_owned(),
            name: None,
            icon_key: None,
            tags: Vec::new(),
        }
        .normalize()
        .unwrap_err();
        assert_eq!(error, SavedItemValidationError::EmptyContent);

        let error = FavoriteUpdate {
            name: None,
            icon_key: None,
            tags: Vec::new(),
            editable_text: Some(" \n".to_owned()),
        }
        .normalize()
        .unwrap_err();
        assert_eq!(error, SavedItemValidationError::EmptyContent);
    }
}
