//! SQLite and Win32 are not one transaction. Preserve truthful persisted/UI state
//! if the native commit fails after a successful optimistic storage transaction.
use echo_engine::{SettingsPatch, SettingsSnapshot};
pub(super) fn reconcile(
    previous: &SettingsSnapshot,
    saved: SettingsSnapshot,
    error: &str,
    save: impl FnOnce(SettingsPatch) -> Result<SettingsSnapshot, String>,
    reload: impl FnOnce() -> Result<SettingsSnapshot, String>,
) -> (SettingsSnapshot, String) {
    let mut ui = saved.ui.clone();
    ui.global_hotkey = previous.ui.global_hotkey.clone();
    ui.global_hotkey_enabled = previous.ui.global_hotkey_enabled;
    let rollback = SettingsPatch {
        expected_revision: saved.revision,
        clipboard: saved.clipboard.clone(),
        ui,
    };
    match save(rollback) {
        Ok(snapshot) => (snapshot, format!("Other settings saved. The shortcut could not be applied; its previous preference was restored. {error}")),
        Err(rollback_error) => {
            let snapshot = reload().unwrap_or(saved);
            (snapshot, format!("Settings are stored, but the global shortcut is not synchronized with Windows. The runtime binding was not switched. Review its status and use Retry saved shortcut. Commit: {error}; preference rollback: {rollback_error}"))
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failed_native_commit_rolls_back_only_shortcut_fields_at_saved_revision() {
        let previous = SettingsSnapshot::default();
        let mut saved = previous.clone();
        saved.revision = 7;
        saved.ui.global_hotkey = "Ctrl+Alt+J".into();
        saved.ui.global_hotkey_enabled = false;
        saved.ui.remember_position = false;
        let (result, warning) = reconcile(
            &previous,
            saved,
            "fixture failure",
            |patch| {
                assert_eq!(patch.expected_revision, 7);
                assert_eq!(patch.ui.global_hotkey, "Alt+V");
                assert!(patch.ui.global_hotkey_enabled);
                assert!(!patch.ui.remember_position);
                Ok(SettingsSnapshot {
                    revision: 8,
                    clipboard: patch.clipboard,
                    ui: patch.ui,
                })
            },
            || panic!("No reload after successful rollback"),
        );
        assert_eq!(result.revision, 8);
        assert!(warning.contains("previous preference was restored"));
    }
    #[test]
    fn rollback_conflict_keeps_newer_revision_and_reports_unsynchronized_runtime() {
        let previous = SettingsSnapshot::default();
        let mut saved = previous.clone();
        saved.revision = 7;
        let mut newer = saved.clone();
        newer.revision = 9;
        newer.ui.global_hotkey = "Ctrl+Alt+K".into();
        let (result, warning) = reconcile(
            &previous,
            saved,
            "fixture failure",
            |_| Err("stale revision".into()),
            || Ok(newer),
        );
        assert_eq!(result.revision, 9);
        assert_eq!(result.ui.global_hotkey, "Ctrl+Alt+K");
        assert!(warning.contains("not synchronized"));
    }
}
