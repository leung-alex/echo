PRAGMA user_version = 1;

CREATE TABLE clipboard_settings (
    id INTEGER PRIMARY KEY,
    history_enabled INTEGER NOT NULL,
    record_sensitive INTEGER NOT NULL,
    store_window_titles INTEGER NOT NULL,
    max_entries INTEGER NOT NULL,
    max_total_bytes INTEGER NOT NULL,
    max_item_bytes INTEGER NOT NULL
);
INSERT INTO clipboard_settings VALUES (1, 1, 0, 1, 5000, 536870912, 33554432);

CREATE TABLE clipboard_entries (
    id INTEGER PRIMARY KEY,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    source_app TEXT,
    source_executable TEXT,
    source_window_title TEXT,
    content_type TEXT NOT NULL,
    preview_text TEXT,
    searchable_text TEXT,
    sanitized_html TEXT,
    fingerprint TEXT NOT NULL UNIQUE,
    byte_size INTEGER NOT NULL
);
CREATE TABLE clipboard_representations (
    id INTEGER PRIMARY KEY,
    entry_id INTEGER NOT NULL,
    format TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    inline_data BLOB,
    blob_hash TEXT,
    byte_size INTEGER NOT NULL
);
CREATE TABLE clipboard_fts (
    entry_id TEXT NOT NULL,
    searchable_text TEXT NOT NULL,
    source_app TEXT NOT NULL
);

CREATE TABLE saved_items (
    id INTEGER PRIMARY KEY,
    source_history_id INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    name TEXT NOT NULL,
    content_type TEXT NOT NULL,
    editable_text TEXT,
    source_app TEXT,
    source_executable TEXT,
    source_window_title TEXT,
    preview_text TEXT,
    byte_size INTEGER NOT NULL,
    is_independent INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE saved_item_representations (
    id INTEGER PRIMARY KEY,
    saved_item_id INTEGER NOT NULL,
    format TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    inline_data BLOB,
    blob_hash TEXT,
    byte_size INTEGER NOT NULL
);
CREATE TABLE tags (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    normalized_name TEXT NOT NULL UNIQUE
);
CREATE TABLE saved_item_tags (
    saved_item_id INTEGER NOT NULL,
    tag_id INTEGER NOT NULL,
    PRIMARY KEY (saved_item_id, tag_id)
);
CREATE TABLE saved_items_fts (saved_item_id INTEGER PRIMARY KEY, document TEXT NOT NULL);

INSERT INTO clipboard_entries VALUES
    (12, 200, 201, 'Fixture Editor', NULL, NULL, 'text', 'pre-r1 text', 'pre-r1 text', NULL, 'fixture-r1', 12);
INSERT INTO clipboard_representations VALUES
    (22, 12, 'text', 'text/plain;charset=utf-8', X'7072652D72312074657874', NULL, 12);
INSERT INTO saved_items VALUES
    (31, 12, 200, 201, 'Fixture Favorite', 'text', 'pre-r1 text', 'Fixture Editor', NULL, NULL, 'pre-r1 text', 12, 0);
INSERT INTO saved_item_representations VALUES
    (32, 31, 'text', 'text/plain;charset=utf-8', X'7072652D72312074657874', NULL, 12);
