//! Process-local Settings/About placement, independent of History and caret popups.
use super::App;

type Geometry = (slint::PhysicalPosition, slint::PhysicalSize);

#[derive(Default)]
pub(super) struct SettingsGeometry {
    active: bool,
    saved: Option<Geometry>,
    history: Option<Geometry>,
}

impl SettingsGeometry {
    pub(super) fn active(&self) -> bool {
        self.active
    }

    pub(super) fn enter(&mut self, history: Option<Geometry>) -> Option<Geometry> {
        self.active = true;
        self.history = history;
        self.saved
    }

    fn remember(&mut self, current: Geometry) {
        if self.active {
            self.saved = Some(current);
        }
    }

    pub(super) fn leave(&mut self, current: Geometry) -> Option<Geometry> {
        self.remember(current);
        self.active = false;
        self.history.take()
    }
}

impl App {
    pub(super) fn current_geometry(&self) -> Geometry {
        (self.window.window().position(), self.window.window().size())
    }

    pub(super) fn apply_geometry(&self, geometry: Geometry) {
        self.window.window().set_position(geometry.0);
        self.window.window().set_size(geometry.1);
    }

    pub(super) fn remember_settings_geometry(&mut self) {
        if self.hwnd.is_some() {
            self.settings_geometry.remember(self.current_geometry());
        }
    }
}

use slint::ComponentHandle;

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry(x: i32) -> Geometry {
        (
            slint::PhysicalPosition::new(x, 100),
            slint::PhysicalSize::new(1100, 760),
        )
    }

    #[test]
    fn first_entry_centers_and_restart_discards_position() {
        let mut state = SettingsGeometry::default();
        assert_eq!(state.enter(Some(geometry(10))), None);
        state.remember(geometry(250));
        assert_eq!(state.leave(geometry(250)), Some(geometry(10)));
        assert_eq!(SettingsGeometry::default().enter(None), None);
    }

    #[test]
    fn hidden_and_about_stay_in_settings_session() {
        let mut state = SettingsGeometry::default();
        state.enter(Some(geometry(10)));
        state.remember(geometry(250));
        assert!(state.active());
        // Hide/show and About do not leave this geometry owner.
        state.remember(geometry(300));
        assert_eq!(state.leave(geometry(300)), Some(geometry(10)));
        assert_eq!(state.enter(Some(geometry(20))), Some(geometry(300)));
    }

    #[test]
    fn history_and_popup_roundtrip_cannot_overwrite_settings() {
        let mut state = SettingsGeometry::default();
        state.enter(Some(geometry(10)));
        assert_eq!(state.leave(geometry(250)), Some(geometry(10)));
        state.remember(geometry(900)); // Inactive: caret popup coordinates are ignored.
        assert_eq!(state.enter(Some(geometry(40))), Some(geometry(250)));
        assert_eq!(state.leave(geometry(260)), Some(geometry(40)));
    }
}
