PRAGMA user_version = 0;

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
    pinned INTEGER NOT NULL DEFAULT 0,
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
CREATE TABLE clipboard_blobs (
    hash TEXT PRIMARY KEY,
    mime_type TEXT NOT NULL,
    byte_size INTEGER NOT NULL
);
CREATE TABLE clipboard_fts (
    entry_id TEXT NOT NULL,
    searchable_text TEXT NOT NULL,
    source_app TEXT NOT NULL
);

CREATE TABLE saved_insert_items (
    id INTEGER PRIMARY KEY,
    source_entry_id INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    source_app TEXT,
    source_executable TEXT,
    source_window_title TEXT,
    content_type TEXT NOT NULL,
    preview_text TEXT,
    searchable_text TEXT,
    sanitized_html TEXT,
    byte_size INTEGER NOT NULL
);
CREATE TABLE saved_insert_representations (
    id INTEGER PRIMARY KEY,
    saved_item_id INTEGER NOT NULL,
    format TEXT NOT NULL,
    mime_type TEXT NOT NULL,
    inline_data BLOB,
    blob_hash TEXT,
    byte_size INTEGER NOT NULL
);

INSERT INTO clipboard_entries VALUES
    (41, 400, 401, 'Fixture Editor', NULL, NULL, 'text', 'pre-r0 saved text',
     'pre-r0 saved text', NULL, 'fixture-r0-saved', 1, 16);
INSERT INTO clipboard_representations VALUES
    (51, 41, 'text', 'text/plain;charset=utf-8',
     X'7072652D72302073617665642074657874', NULL, 16);
INSERT INTO saved_insert_items VALUES
    (61, 41, 400, 401, 'Fixture Editor', NULL, NULL, 'text',
     'pre-r0 saved text', 'pre-r0 saved text', NULL, 16);
INSERT INTO saved_insert_representations VALUES
    (71, 61, 'text', 'text/plain;charset=utf-8',
     X'7072652D72302073617665642074657874', NULL, 16);
