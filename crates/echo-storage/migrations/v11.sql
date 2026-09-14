-- Recording is always enabled in production, including sensitive content.
UPDATE clipboard_settings SET history_enabled=1, record_sensitive=1;
