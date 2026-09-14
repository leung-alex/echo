use serde::{Deserialize, Serialize};

/// Absolute History capacity; Saved Items have a separate lifecycle.
pub const MAX_HISTORY_ENTRIES: u32 = 2_000;
pub const MAX_STORAGE_BYTES: u64 = 512 * 1024 * 1024;
pub const MAX_ITEM_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    Light,
    Dark,
}

impl Default for ThemeMode {
    fn default() -> Self {
        Self::Light
    }
}

impl ThemeMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClipboardSettings {
    pub history_enabled: bool,
    pub record_sensitive: bool,
    pub max_entries: u32,
    pub max_total_bytes: u64,
    pub max_item_bytes: u64,
    pub theme: ThemeMode,
}

impl Default for ClipboardSettings {
    fn default() -> Self {
        Self {
            history_enabled: true,
            record_sensitive: true,
            max_entries: MAX_HISTORY_ENTRIES,
            max_total_bytes: MAX_STORAGE_BYTES,
            max_item_bytes: MAX_ITEM_BYTES,
            theme: ThemeMode::Light,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn light_default_and_two_supported_themes() {
        assert_eq!(ClipboardSettings::default().theme, ThemeMode::Light);
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            assert_eq!(ThemeMode::parse(mode.as_str()), Some(mode));
            assert_eq!(
                serde_json::from_str::<ThemeMode>(&serde_json::to_string(&mode).unwrap()).unwrap(),
                mode
            );
        }
        assert_eq!(ThemeMode::parse("system"), None);
    }
}
