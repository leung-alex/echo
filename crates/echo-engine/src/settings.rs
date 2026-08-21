use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

impl Default for ThemeMode {
    fn default() -> Self {
        Self::System
    }
}

impl ThemeMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "system" => Some(Self::System),
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
    pub store_window_titles: bool,
    pub max_entries: u32,
    pub max_total_bytes: u64,
    pub max_item_bytes: u64,
    pub theme: ThemeMode,
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
            theme: ThemeMode::System,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_mode_has_a_system_default_and_stable_wire_values() {
        assert_eq!(ThemeMode::default(), ThemeMode::System);
        assert_eq!(ThemeMode::System.as_str(), "system");
        assert_eq!(ThemeMode::Light.as_str(), "light");
        assert_eq!(ThemeMode::Dark.as_str(), "dark");
        assert_eq!(
            serde_json::to_string(&ThemeMode::System).unwrap(),
            "\"system\""
        );
        assert_eq!(
            serde_json::to_string(&ThemeMode::Light).unwrap(),
            "\"light\""
        );
        assert_eq!(serde_json::to_string(&ThemeMode::Dark).unwrap(), "\"dark\"");
        assert_eq!(ClipboardSettings::default().theme, ThemeMode::System);
    }

    #[test]
    fn theme_mode_rejects_unknown_or_mixed_case_values() {
        assert_eq!(ThemeMode::parse("system"), Some(ThemeMode::System));
        assert_eq!(ThemeMode::parse("light"), Some(ThemeMode::Light));
        assert_eq!(ThemeMode::parse("dark"), Some(ThemeMode::Dark));
        assert_eq!(ThemeMode::parse("System"), None);
        assert_eq!(ThemeMode::parse("sepia"), None);
        assert!(serde_json::from_str::<ThemeMode>("\"sepia\"").is_err());
    }
}
