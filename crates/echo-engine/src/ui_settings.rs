//! Persisted UI policy. Renderer resources and platform types do not belong here.
use crate::ClipboardSettings;
use serde::{Deserialize, Serialize};

macro_rules! setting_enum {
    ($name:ident, $default:ident, {$($variant:ident => $value:literal),+ $(,)?}) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
        pub enum $name { $(#[serde(rename = $value)] $variant),+ }
        impl Default for $name { fn default() -> Self { Self::$default } }
        impl $name {
            pub const fn as_str(self) -> &'static str { match self { $(Self::$variant => $value),+ } }
            pub fn parse(value: &str) -> Option<Self> {
                match value { $($value => Some(Self::$variant),)+ _ => None }
            }
        }
    };
}
setting_enum!(SpaceViewMode, CoverFlow, { CoverFlow => "cover_flow", Flat => "flat" });
setting_enum!(Motion, System, { System => "system", Reduced => "reduced", Off => "off" });
setting_enum!(MotionSpeed, Standard, { Snappy => "snappy", Standard => "standard", Relaxed => "relaxed" });
setting_enum!(SwitchShortcut, Tab, { Tab => "tab", CtrlTab => "ctrl_tab" });
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum StartupSpace {
    #[default]
    History,
    Last,
    Existing(crate::SpaceId),
}
impl StartupSpace {
    pub fn key(self) -> String {
        match self {
            Self::History => "history".into(),
            Self::Last => "last".into(),
            Self::Existing(id) => id.to_string(),
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "history" => Some(Self::History),
            "last" => Some(Self::Last),
            _ => crate::SpaceId::parse(value).map(Self::Existing),
        }
    }
    pub fn resolve(
        self,
        last: Option<&str>,
        existing: impl IntoIterator<Item = crate::SpaceId>,
    ) -> crate::SpaceId {
        let requested = match self {
            Self::History => crate::SpaceId::HISTORY,
            Self::Last => last
                .and_then(crate::SpaceId::parse)
                .unwrap_or(crate::SpaceId::HISTORY),
            Self::Existing(id) => id,
        };
        if existing.into_iter().any(|id| id == requested) {
            requested
        } else {
            crate::SpaceId::HISTORY
        }
    }
}
impl TryFrom<String> for StartupSpace {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(&value).ok_or_else(|| "Invalid startup space".into())
    }
}
impl From<StartupSpace> for String {
    fn from(value: StartupSpace) -> Self {
        value.key()
    }
}
setting_enum!(QueryOnSwitch, Preserve, { Preserve => "preserve", Clear => "clear" });
setting_enum!(Density, Comfortable, { Comfortable => "comfortable", Compact => "compact" });
setting_enum!(SideContent, Visible, { Visible => "visible", TitlesOnly => "titles_only" });
setting_enum!(GraphicsMode, Auto, { Auto => "auto", Software => "software" });
setting_enum!(FrameRate, Auto, { Auto => "auto", Fps60 => "60" });
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UiSettings {
    pub version: u32,
    pub view_mode: SpaceViewMode,
    pub motion: Motion,
    pub motion_speed: MotionSpeed,
    pub loop_spaces: bool,
    pub switch_shortcut: SwitchShortcut,
    pub global_hotkey_enabled: bool,
    pub global_hotkey: String,
    pub caret_anchor: bool,
    pub inline_completion: bool,
    pub startup_space: StartupSpace,
    pub remember_position: bool,
    pub query_on_switch: QueryOnSwitch,
    pub density: Density,
    pub reflections: bool,
    pub side_content: SideContent,
    pub graphics: GraphicsMode,
    pub frame_rate: FrameRate,
    pub trim_when_hidden: bool,
    pub reduce_on_battery: bool,
    pub resume_last_space_id: Option<String>,
}
impl Default for UiSettings {
    fn default() -> Self {
        Self {
            version: 1,
            view_mode: Default::default(),
            motion: Default::default(),
            motion_speed: Default::default(),
            loop_spaces: true,
            switch_shortcut: Default::default(),
            global_hotkey_enabled: true,
            global_hotkey: "Alt+V".into(),
            caret_anchor: true,
            inline_completion: true,
            startup_space: Default::default(),
            remember_position: true,
            query_on_switch: Default::default(),
            density: Default::default(),
            reflections: false,
            side_content: Default::default(),
            graphics: Default::default(),
            frame_rate: Default::default(),
            trim_when_hidden: true,
            reduce_on_battery: true,
            resume_last_space_id: None,
        }
    }
}
impl UiSettings {
    pub fn validate(&self) -> Result<(), String> {
        if let StartupSpace::Existing(id) = self.startup_space {
            if id.0 <= 0 {
                return Err("Invalid startup space identity".into());
            }
        }
        crate::GlobalShortcut::parse(&self.global_hotkey)?;
        if self.version != 1 {
            return Err("Unsupported UI settings version".into());
        }
        if let Some(id) = &self.resume_last_space_id {
            if id.parse::<i64>().ok().filter(|id| *id > 0).is_none() {
                return Err("Invalid resume space identity".into());
            }
        }
        Ok(())
    }
    pub fn restore_appearance_defaults(&mut self) {
        let defaults = Self::default();
        self.view_mode = defaults.view_mode;
        self.motion = defaults.motion;
        self.motion_speed = defaults.motion_speed;
        self.density = defaults.density;
        self.reflections = defaults.reflections;
        self.side_content = defaults.side_content;
    }
}
impl MotionSpeed {
    pub const fn omega(self) -> f64 {
        match self {
            Self::Snappy => 50.0,
            Self::Standard => 40.0,
            Self::Relaxed => 30.0,
        }
    }
    pub const fn deadline_ms(self) -> u64 {
        match self {
            Self::Snappy => 220,
            Self::Standard => 260,
            Self::Relaxed => 340,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsSnapshot {
    pub clipboard: ClipboardSettings,
    pub ui: UiSettings,
    pub revision: i64,
}
impl Default for SettingsSnapshot {
    fn default() -> Self {
        Self {
            clipboard: Default::default(),
            ui: Default::default(),
            revision: 1,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsPatch {
    pub expected_revision: i64,
    pub clipboard: ClipboardSettings,
    pub ui: UiSettings,
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_space_accepts_existing_space_identity_and_legacy_choices() {
        for value in ["history", "last", "2", "37", "9223372036854775807"] {
            let source = serde_json::json!({"startup_space": value});
            let settings: UiSettings = serde_json::from_value(source).unwrap();
            settings.validate().unwrap();
            assert_eq!(
                serde_json::to_value(&settings).unwrap()["startup_space"],
                value
            );
        }
        for value in ["0", "-1", "invalid", "9223372036854775808"] {
            assert!(serde_json::from_value::<UiSettings>(
                serde_json::json!({"startup_space": value})
            )
            .is_err());
        }
    }
    #[test]
    fn startup_space_resolves_saved_choice_and_falls_back_after_deletion() {
        use crate::SpaceId;
        let spaces = [SpaceId::HISTORY, SpaceId::FAVORITES, SpaceId(37)];
        assert_eq!(
            StartupSpace::Existing(SpaceId(37)).resolve(None, spaces),
            SpaceId(37)
        );
        assert_eq!(
            StartupSpace::Existing(SpaceId::FAVORITES).resolve(None, spaces),
            SpaceId::FAVORITES
        );
        assert_eq!(StartupSpace::Last.resolve(Some("37"), spaces), SpaceId(37));
        assert_eq!(
            StartupSpace::Existing(SpaceId(38)).resolve(None, spaces),
            SpaceId::HISTORY
        );
        assert_eq!(
            StartupSpace::Last.resolve(Some("38"), spaces),
            SpaceId::HISTORY
        );
    }
    #[test]
    fn retired_graphics_preferences_round_trip_without_resetting_settings() {
        for view in ["cover_flow", "flat"] {
            for graphics in ["auto", "software"] {
                let source = serde_json::json!({"version":1,"view_mode":view,"graphics":graphics,"reflections":true,"side_content":"titles_only","global_hotkey":"Ctrl+Alt+J","caret_anchor":false});
                let settings: UiSettings = serde_json::from_value(source).unwrap();
                settings.validate().unwrap();
                let encoded = serde_json::to_value(&settings).unwrap();
                assert_eq!(encoded["view_mode"], view);
                assert_eq!(encoded["graphics"], graphics);
                assert_eq!(encoded["reflections"], true);
                assert_eq!(encoded["global_hotkey"], "Ctrl+Alt+J");
                assert_eq!(encoded["caret_anchor"], false);
                let restored: UiSettings = serde_json::from_value(encoded).unwrap();
                assert_eq!(settings, restored);
            }
        }
    }
    #[test]
    fn old_settings_get_complete_defaults() {
        let settings: UiSettings = serde_json::from_str(r#"{"version":1}"#).unwrap();
        assert_eq!(settings, UiSettings::default());
        settings.validate().unwrap();
    }
    #[test]
    fn unsupported_settings_are_not_silently_accepted() {
        assert!(serde_json::from_str::<UiSettings>(r#"{"motion":"random"}"#).is_err());
        assert!(serde_json::from_str::<UiSettings>(r#"{"unknown":true}"#).is_err());
        let mut settings = UiSettings::default();
        settings.version = 2;
        assert!(settings.validate().is_err());
    }
    #[test]
    fn hotkey_and_anchor_settings_round_trip_and_survive_appearance_reset() {
        let mut s = UiSettings::default();
        assert_eq!(s.global_hotkey, "Alt+V");
        assert!(s.global_hotkey_enabled && s.caret_anchor);
        s.global_hotkey = "Ctrl+Alt+J".into();
        s.global_hotkey_enabled = false;
        s.caret_anchor = false;
        s.restore_appearance_defaults();
        let decoded: UiSettings =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(s, decoded);
        decoded.validate().unwrap();
        s.global_hotkey = "V".into();
        assert!(s.validate().is_err());
    }
    #[test]
    fn appearance_reset_does_not_reset_other_policy() {
        let mut s = UiSettings::default();
        s.graphics = GraphicsMode::Software;
        s.loop_spaces = false;
        s.startup_space = StartupSpace::Last;
        s.motion = Motion::Off;
        s.restore_appearance_defaults();
        assert_eq!(s.graphics, GraphicsMode::Software);
        assert!(!s.loop_spaces);
        assert_eq!(s.startup_space, StartupSpace::Last);
        assert_eq!(s.motion, Motion::System);
    }
    #[test]
    fn identity_is_not_narrowed_to_a_ui_integer() {
        let mut s = UiSettings::default();
        s.resume_last_space_id = Some(i64::MAX.to_string());
        s.validate().unwrap();
        s.resume_last_space_id = Some("0".into());
        assert!(s.validate().is_err());
    }
}
