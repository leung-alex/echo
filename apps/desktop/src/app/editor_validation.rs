//! Editor-local feedback. Page status is deliberately outside this boundary.
use crate::AppWindow;

pub(super) fn reset(window: &AppWindow) {
    window.set_editor_name_invalid(false);
    window.set_editor_content_invalid(false);
    window.set_editor_error("".into());
}

pub(super) fn edited(window: &AppWindow) {
    if echo_engine::normalize_name(window.get_draft_name().as_str()).is_ok() {
        window.set_editor_name_invalid(false);
    }
    if !window.get_content_editable() || !window.get_draft_content().trim().is_empty() {
        window.set_editor_content_invalid(false);
    }
}

pub(super) fn validate(window: &AppWindow) -> bool {
    window.set_editor_error("".into());
    let name_invalid = echo_engine::normalize_name(window.get_draft_name().as_str()).is_err();
    let content_invalid =
        window.get_content_editable() && window.get_draft_content().trim().is_empty();
    window.set_editor_name_invalid(name_invalid);
    window.set_editor_content_invalid(content_invalid);
    if name_invalid {
        window
            .set_editor_name_focus_request(window.get_editor_name_focus_request().wrapping_add(1));
    } else if content_invalid {
        window.set_editor_content_focus_request(
            window.get_editor_content_focus_request().wrapping_add(1),
        );
    }
    !name_invalid && !content_invalid
}

/// Returns whether the active editor owns this failed save.
pub(super) fn report_save_error(window: &AppWindow, error: &str) -> bool {
    if !window.get_editor_open() {
        return false;
    }
    window.set_editor_error(error.into());
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::platform::{
        software_renderer::MinimalSoftwareWindow, software_renderer::RepaintBufferType, Platform,
        WindowAdapter,
    };
    use std::rc::Rc;

    struct TestPlatform;
    impl Platform for TestPlatform {
        fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
            Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
        }
    }

    #[test]
    fn editor_validation_keeps_errors_local_and_preserves_retry_draft() {
        slint::platform::set_platform(Box::new(TestPlatform)).unwrap();
        let window = AppWindow::new().unwrap();
        window.set_editor_open(true);
        window.set_content_editable(true);
        window.set_status("Existing page notice".into());
        reset(&window);
        assert!(!window.get_editor_name_invalid());
        assert!(!window.get_editor_content_invalid());
        for empty in ["", " \t\n", "\u{3000}"] {
            window.set_draft_name(empty.into());
            window.set_draft_content(empty.into());
            let focus = window.get_editor_name_focus_request();
            assert!(!validate(&window));
            assert!(window.get_editor_name_invalid());
            assert!(window.get_editor_content_invalid());
            assert_eq!(window.get_editor_name_focus_request(), focus + 1);
            assert_eq!(window.get_status(), "Existing page notice");
            assert!(!window.get_status_error());
        }
        window.set_draft_name("New name".into());
        edited(&window);
        assert!(!window.get_editor_name_invalid());
        assert!(window.get_editor_content_invalid());
        let focus = window.get_editor_content_focus_request();
        assert!(!validate(&window));
        assert_eq!(window.get_editor_content_focus_request(), focus + 1);
        window.set_draft_content("Original content".into());
        edited(&window);
        assert!(!window.get_editor_content_invalid());
        assert!(validate(&window));
        assert!(report_save_error(&window, "Synthetic save failure"));
        assert_eq!(window.get_editor_error(), "Synthetic save failure");
        assert_eq!(window.get_status(), "Existing page notice");
        assert_eq!(window.get_draft_content(), "Original content");
        assert!(window.get_editor_open());
        assert!(validate(&window));
        assert!(window.get_editor_error().is_empty());
        window.set_draft_content("".into());
        window.set_content_editable(false);
        assert!(
            validate(&window),
            "retained non-text content is not required input"
        );
        window.set_draft_name("".into());
        assert!(!validate(&window));
        reset(&window);
        window.set_editor_open(false);
        assert!(!report_save_error(&window, "Unrelated failure"));
        window.set_editor_open(true);
        assert!(!window.get_editor_name_invalid());
        assert!(!window.get_editor_content_invalid());
        assert!(window.get_editor_error().is_empty());
    }
}
