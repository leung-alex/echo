-- Retire captured window titles while retaining application identity and content.
ALTER TABLE clipboard_settings DROP COLUMN store_window_titles;
ALTER TABLE clipboard_entries DROP COLUMN source_window_title;
ALTER TABLE saved_items DROP COLUMN source_window_title;
