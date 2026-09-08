//! Atomic settings patches share the clipboard settings table and writer actor.
use super::*;
use echo_engine::{SettingsPatch, SettingsSnapshot, SpaceError, SpaceId, UiSettings};

impl ClipboardStore {
    pub fn settings_snapshot(&self) -> Result<SettingsSnapshot> {
        let (mut clipboard, theme, json, revision): (ClipboardSettings, String, String, i64) =
            self.connection.query_row(
                "SELECT history_enabled,record_sensitive,store_window_titles,max_entries,
             max_total_bytes,max_item_bytes,theme,ui_settings_json,settings_revision
             FROM clipboard_settings WHERE id=1",
                [],
                |r| {
                    Ok((
                        ClipboardSettings {
                            history_enabled: r.get::<_, i64>(0)? != 0,
                            record_sensitive: r.get::<_, i64>(1)? != 0,
                            store_window_titles: r.get::<_, i64>(2)? != 0,
                            max_entries: r
                                .get::<_, i64>(3)?
                                .try_into()
                                .unwrap_or(DEFAULT_MAX_ENTRIES)
                                .min(DEFAULT_MAX_ENTRIES),
                            max_total_bytes: r
                                .get::<_, i64>(4)?
                                .try_into()
                                .unwrap_or(DEFAULT_MAX_TOTAL_BYTES),
                            max_item_bytes: r
                                .get::<_, i64>(5)?
                                .try_into()
                                .unwrap_or(DEFAULT_MAX_ITEM_BYTES),
                            theme: ThemeMode::System,
                        },
                        r.get(6)?,
                        r.get(7)?,
                        r.get(8)?,
                    ))
                },
            )?;
        clipboard.theme = ThemeMode::parse(&theme)
            .ok_or_else(|| StorageError::Invalid("Invalid theme setting".into()))?;
        let mut ui: UiSettings = serde_json::from_str(&json).map_err(|_| {
            StorageError::Invalid("UI settings are invalid or from an unsupported version".into())
        })?;
        // A stale resume hint must never prevent the app from opening.
        ui.resume_last_space_id = ui
            .resume_last_space_id
            .filter(|id| SpaceId::parse(id).is_some());
        ui.validate().map_err(StorageError::Invalid)?;
        if revision <= 0 {
            return Err(StorageError::Invalid("Invalid settings revision".into()));
        }
        Ok(SettingsSnapshot {
            clipboard,
            ui,
            revision,
        })
    }
    pub fn save_settings_patch(&mut self, mut patch: SettingsPatch) -> Result<SettingsSnapshot> {
        let c = &patch.clipboard;
        if c.max_entries > DEFAULT_MAX_ENTRIES {
            return Err(StorageError::Invalid(
                "Maximum history entries cannot exceed 2000".into(),
            ));
        }
        if c.max_entries == 0
            || c.max_total_bytes == 0
            || c.max_item_bytes == 0
            || c.max_item_bytes > c.max_total_bytes
            || c.max_total_bytes > i64::MAX as u64
        {
            return Err(StorageError::Invalid(
                "Storage limits must be positive; item limit cannot exceed total limit".into(),
            ));
        }
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let (revision, current_json): (i64, String) = tx.query_row(
            "SELECT settings_revision,ui_settings_json FROM clipboard_settings WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        if revision != patch.expected_revision {
            return Err(SpaceError::Conflict.into());
        }
        let current: UiSettings = serde_json::from_str(&current_json)
            .map_err(|_| StorageError::Invalid("Current UI settings cannot be decoded".into()))?;
        // Automatic resume writes own this field. A long-lived form must not overwrite it.
        patch.ui.resume_last_space_id = current
            .resume_last_space_id
            .filter(|id| SpaceId::parse(id).is_some());
        patch.ui.validate().map_err(StorageError::Invalid)?;
        let json = serde_json::to_string(&patch.ui)
            .map_err(|_| StorageError::Invalid("Could not encode UI settings".into()))?;
        let next = revision
            .checked_add(1)
            .ok_or_else(|| StorageError::Invalid("Settings revision overflow".into()))?;
        tx.execute("UPDATE clipboard_settings SET history_enabled=?,record_sensitive=?,store_window_titles=?,
            max_entries=?,max_total_bytes=?,max_item_bytes=?,theme=?,ui_settings_json=?,settings_revision=? WHERE id=1",
            params![c.history_enabled as i64,c.record_sensitive as i64,c.store_window_titles as i64,
                i64::from(c.max_entries),c.max_total_bytes as i64,c.max_item_bytes as i64,
                c.theme.as_str(),json,next])?;
        tx.commit()?;
        Ok(SettingsSnapshot {
            clipboard: patch.clipboard,
            ui: patch.ui,
            revision: next,
        })
    }
    pub fn save_resume_space(&mut self, id: SpaceId) -> Result<()> {
        let tx = self.connection.transaction()?;
        let json: String = tx.query_row(
            "SELECT ui_settings_json FROM clipboard_settings WHERE id=1",
            [],
            |r| r.get(0),
        )?;
        let mut ui: UiSettings = serde_json::from_str(&json)
            .map_err(|_| StorageError::Invalid("UI settings cannot be decoded".into()))?;
        let exists = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM spaces WHERE id=?)",
            [id.0],
            |r| r.get::<_, bool>(0),
        )?;
        let id = if exists { id } else { SpaceId::HISTORY };
        if ui.resume_last_space_id.as_deref() != Some(id.to_string().as_str()) {
            ui.resume_last_space_id = Some(id.to_string());
            let json = serde_json::to_string(&ui)
                .map_err(|_| StorageError::Invalid("Could not encode UI settings".into()))?;
            tx.execute(
                "UPDATE clipboard_settings SET ui_settings_json=? WHERE id=1",
                [json],
            )?;
        }
        tx.commit()?;
        Ok(())
    }
}
impl SharedClipboardStore {
    pub fn settings_snapshot(&self) -> Result<SettingsSnapshot> {
        let _admission = self.runtime.enter()?;
        self.runtime.reader.read(|store| store.settings_snapshot())
    }
    pub fn save_settings_patch(&self, patch: SettingsPatch) -> Result<SettingsSnapshot> {
        self.with_store(move |store| store.save_settings_patch(patch))
    }
    pub fn save_resume_space(&self, id: SpaceId) -> Result<()> {
        self.with_store(move |store| store.save_resume_space(id))
    }
}
