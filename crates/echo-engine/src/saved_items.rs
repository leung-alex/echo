use crate::{ClipboardRepresentation, HistoryEntry};
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
    pub is_independent: bool,
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
    pub is_independent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedItemUpdate {
    pub name: String,
    pub tags: Vec<String>,
    pub editable_text: Option<String>,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SavedItemValidationError {
    #[error("saved item name cannot be empty")]
    EmptyName,
    #[error("saved item tag cannot be empty")]
    EmptyTag,
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
            is_independent: false,
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

pub fn normalize_name(name: &str) -> Result<String, SavedItemValidationError> {
    let normalized = name.trim().to_owned();
    if normalized.is_empty() {
        Err(SavedItemValidationError::EmptyName)
    } else {
        Ok(normalized)
    }
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
        .map(|line| truncate_name(line, 120))
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

fn truncate_name(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

fn casefold_key(value: &str) -> String {
    value.to_lowercase()
}
