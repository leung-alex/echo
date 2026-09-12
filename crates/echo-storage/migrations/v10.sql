-- Space navigation always uses Tab / Shift+Tab; remove the retired preference.
UPDATE clipboard_settings
SET ui_settings_json = json_remove(ui_settings_json, '$.switch_shortcut');
