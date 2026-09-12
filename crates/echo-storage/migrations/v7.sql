-- Retire configurable card appearance without touching content or other settings.
ALTER TABLE clipboard_settings RENAME COLUMN theme TO retired_theme;
ALTER TABLE clipboard_settings ADD COLUMN theme TEXT NOT NULL DEFAULT 'light'
    CHECK (theme IN ('light', 'dark'));
UPDATE clipboard_settings SET theme = CASE WHEN retired_theme = 'dark' THEN 'dark' ELSE 'light' END,
    ui_settings_json = json_set(
        json_remove(ui_settings_json, '$.view_mode', '$.motion', '$.motion_speed', '$.density'),
        '$.language', COALESCE(json_extract(ui_settings_json, '$.language'), 'zh-CN'));
ALTER TABLE clipboard_settings DROP COLUMN retired_theme;
