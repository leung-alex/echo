#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedItem {
    pub id: i64,
    pub source_entry_id: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub source_app: Option<String>,
    pub source_executable: Option<String>,
    pub source_window_title: Option<String>,
    pub content_type: String,
    pub preview_text: Option<String>,
    pub searchable_text: Option<String>,
    pub sanitized_html: Option<String>,
    pub byte_size: u64,
}
