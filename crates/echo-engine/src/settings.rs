use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardSettings {
    pub history_enabled: bool,
    pub record_sensitive: bool,
    pub store_window_titles: bool,
    pub max_entries: u32,
    pub max_total_bytes: u64,
    pub max_item_bytes: u64,
}

impl Default for ClipboardSettings {
    fn default() -> Self {
        Self {
            history_enabled: true,
            record_sensitive: false,
            store_window_titles: false,
            max_entries: 5_000,
            max_total_bytes: 512 * 1024 * 1024,
            max_item_bytes: 32 * 1024 * 1024,
        }
    }
}
