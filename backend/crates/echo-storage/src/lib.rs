use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use echo_clipboard::{CaptureSettings, ClipboardSink, NormalizedCapture, RecordResult};
use echo_platform::ClipboardRepresentation;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, Row, Transaction};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

const INLINE_LIMIT: usize = 64 * 1024;
const DEFAULT_MAX_ENTRIES: u32 = 5_000;
const DEFAULT_MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
const DEFAULT_MAX_ITEM_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("filesystem error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid storage data: {0}")]
    Invalid(String),
    #[error("blob is missing or invalid: {0}")]
    MissingBlob(String),
    #[error("legacy migration failed: {0}")]
    Migration(String),
}

pub type Result<T> = std::result::Result<T, StorageError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardSettings {
    pub history_enabled: bool,
    pub record_sensitive: bool,
    pub store_window_titles: bool,
    pub max_entries: u32,
    pub max_total_bytes: u64,
    pub max_item_bytes: u64,
}

impl Default for ClipboardSettings {
    fn default() -> Self {
        Self {
            history_enabled: true,
            record_sensitive: false,
            store_window_titles: false,
            max_entries: DEFAULT_MAX_ENTRIES,
            max_total_bytes: DEFAULT_MAX_TOTAL_BYTES,
            max_item_bytes: DEFAULT_MAX_ITEM_BYTES,
        }
    }
}

impl From<&ClipboardSettings> for CaptureSettings {
    fn from(settings: &ClipboardSettings) -> Self {
        Self {
            history_enabled: settings.history_enabled,
            record_sensitive: settings.record_sensitive,
            store_window_titles: settings.store_window_titles,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardEntry {
    pub id: i64,
    pub created_at: i64,
    pub updated_at: i64,
    pub source_app: Option<String>,
    pub source_executable: Option<String>,
    pub source_window_title: Option<String>,
    pub content_type: String,
    pub preview_text: Option<String>,
    pub searchable_text: Option<String>,
    pub sanitized_html: Option<String>,
    pub fingerprint: String,
    pub pinned: bool,
    pub byte_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRepresentation {
    pub id: i64,
    pub entry_id: i64,
    pub format: String,
    pub mime_type: String,
    pub inline_data: Option<Vec<u8>>,
    pub blob_hash: Option<String>,
    pub byte_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredClipboardEntry {
    pub entry: ClipboardEntry,
    pub representations: Vec<StoredRepresentation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedInsertItem {
    pub id: i64,
    pub source_entry_id: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
    pub source_app: Option<String>,
    pub source_executable: Option<String>,
    pub source_window_title: Option<String>,
    pub content_type: String,
    pub preview_text: Option<String>,
    pub searchable_text: Option<String>,
    pub sanitized_html: Option<String>,
    pub byte_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSavedInsertItem {
    pub item: SavedInsertItem,
    pub representations: Vec<StoredRepresentation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    pub id: i64,
    pub name: String,
    pub content: String,
    pub group_name: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    pub backup_dir: PathBuf,
    pub entries: usize,
    pub representations: usize,
    pub blobs: usize,
    pub favorites: usize,
    pub snippets: usize,
    pub already_migrated: bool,
}

#[derive(Debug, Clone)]
struct PreparedRepresentation {
    format: String,
    mime_type: String,
    inline_data: Option<Vec<u8>>,
    blob_hash: Option<String>,
    byte_size: u64,
}

pub struct ClipboardStore {
    connection: Connection,
    data_dir: PathBuf,
    blobs_dir: PathBuf,
}

impl ClipboardStore {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self> {
        let data_dir = data_dir.as_ref().to_path_buf();
        fs::create_dir_all(&data_dir)?;
        let blobs_dir = data_dir.join("blobs");
        fs::create_dir_all(&blobs_dir)?;
        let database = data_dir.join("echo.sqlite3");
        let connection = Connection::open(database)?;
        let mut store = Self {
            connection,
            data_dir,
            blobs_dir,
        };
        store.configure()?;
        store.ensure_schema()?;
        Ok(store)
    }

    pub fn open_in_memory(data_dir: impl AsRef<Path>) -> Result<Self> {
        let data_dir = data_dir.as_ref().to_path_buf();
        fs::create_dir_all(data_dir.join("blobs"))?;
        let connection = Connection::open_in_memory()?;
        let mut store = Self {
            connection,
            blobs_dir: data_dir.join("blobs"),
            data_dir,
        };
        store.configure()?;
        store.ensure_schema()?;
        Ok(store)
    }

    fn configure(&self) -> Result<()> {
        self.connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;
             PRAGMA journal_mode = WAL;",
        )?;
        Ok(())
    }

    fn ensure_schema(&mut self) -> Result<()> {
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS clipboard_settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                history_enabled INTEGER NOT NULL DEFAULT 1,
                record_sensitive INTEGER NOT NULL DEFAULT 0,
                store_window_titles INTEGER NOT NULL DEFAULT 0,
                max_entries INTEGER NOT NULL DEFAULT 5000,
                max_total_bytes INTEGER NOT NULL DEFAULT 536870912,
                max_item_bytes INTEGER NOT NULL DEFAULT 33554432
            );
            INSERT OR IGNORE INTO clipboard_settings (id) VALUES (1);
            CREATE TABLE IF NOT EXISTS clipboard_entries (
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
            CREATE TABLE IF NOT EXISTS clipboard_representations (
                id INTEGER PRIMARY KEY,
                entry_id INTEGER NOT NULL REFERENCES clipboard_entries(id) ON DELETE CASCADE,
                format TEXT NOT NULL,
                mime_type TEXT NOT NULL,
                inline_data BLOB,
                blob_hash TEXT,
                byte_size INTEGER NOT NULL,
                CHECK ((inline_data IS NULL) != (blob_hash IS NULL))
            );
            CREATE INDEX IF NOT EXISTS clipboard_representations_entry_idx
                ON clipboard_representations(entry_id);
            CREATE INDEX IF NOT EXISTS clipboard_representations_blob_idx
                ON clipboard_representations(blob_hash);
            CREATE TABLE IF NOT EXISTS clipboard_blobs (
                hash TEXT PRIMARY KEY,
                mime_type TEXT NOT NULL,
                byte_size INTEGER NOT NULL
            );
            CREATE VIRTUAL TABLE IF NOT EXISTS clipboard_fts USING fts5(
                entry_id UNINDEXED,
                searchable_text,
                source_app
            );
            CREATE TABLE IF NOT EXISTS saved_insert_items (
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
            CREATE UNIQUE INDEX IF NOT EXISTS saved_insert_items_source_idx
                ON saved_insert_items(source_entry_id) WHERE source_entry_id IS NOT NULL;
            CREATE TABLE IF NOT EXISTS saved_insert_representations (
                id INTEGER PRIMARY KEY,
                saved_item_id INTEGER NOT NULL REFERENCES saved_insert_items(id) ON DELETE CASCADE,
                format TEXT NOT NULL,
                mime_type TEXT NOT NULL,
                inline_data BLOB,
                blob_hash TEXT,
                byte_size INTEGER NOT NULL,
                CHECK ((inline_data IS NULL) != (blob_hash IS NULL))
            );
            CREATE INDEX IF NOT EXISTS saved_insert_representations_item_idx
                ON saved_insert_representations(saved_item_id);
            CREATE INDEX IF NOT EXISTS saved_insert_representations_blob_idx
                ON saved_insert_representations(blob_hash);
            CREATE TABLE IF NOT EXISTS snippets (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                content TEXT NOT NULL,
                group_name TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS migration_state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                completed_at INTEGER NOT NULL
            );",
        )?;
        Ok(())
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn settings(&self) -> Result<ClipboardSettings> {
        self.connection
            .query_row(
                "SELECT history_enabled, record_sensitive, store_window_titles,
                        max_entries, max_total_bytes, max_item_bytes
                 FROM clipboard_settings WHERE id = 1",
                [],
                |row| {
                    Ok(ClipboardSettings {
                        history_enabled: row.get::<_, i64>(0)? != 0,
                        record_sensitive: row.get::<_, i64>(1)? != 0,
                        store_window_titles: row.get::<_, i64>(2)? != 0,
                        max_entries: row.get::<_, i64>(3)?.try_into().unwrap_or(DEFAULT_MAX_ENTRIES),
                        max_total_bytes: row.get::<_, i64>(4)?.try_into().unwrap_or(DEFAULT_MAX_TOTAL_BYTES),
                        max_item_bytes: row.get::<_, i64>(5)?.try_into().unwrap_or(DEFAULT_MAX_ITEM_BYTES),
                    })
                },
            )
            .map_err(StorageError::from)
    }

    pub fn update_settings(&mut self, settings: &ClipboardSettings) -> Result<()> {
        self.connection.execute(
            "UPDATE clipboard_settings SET history_enabled = ?, record_sensitive = ?,
             store_window_titles = ?, max_entries = ?, max_total_bytes = ?, max_item_bytes = ?
             WHERE id = 1",
            params![
                settings.history_enabled as i64,
                settings.record_sensitive as i64,
                settings.store_window_titles as i64,
                i64::from(settings.max_entries),
                i64::try_from(settings.max_total_bytes).unwrap_or(i64::MAX),
                i64::try_from(settings.max_item_bytes).unwrap_or(i64::MAX),
            ],
        )?;
        Ok(())
    }

    pub fn record_capture(&mut self, capture: NormalizedCapture) -> Result<RecordResult> {
        let settings = self.settings()?;
        let byte_size = capture
            .representations
            .iter()
            .try_fold(0_u64, |total, representation| {
                total.checked_add(representation.bytes.len() as u64)
            })
            .ok_or_else(|| StorageError::Invalid("clipboard byte size overflow".to_owned()))?;
        if byte_size > settings.max_item_bytes {
            return Err(StorageError::Invalid(format!(
                "clipboard item exceeds max_item_bytes ({byte_size} > {})",
                settings.max_item_bytes
            )));
        }
        let prepared = capture
            .representations
            .iter()
            .map(|representation| self.prepare_representation(representation))
            .collect::<Result<Vec<_>>>()?;
        let now = now_millis();
        let tx = self.connection.transaction()?;
        let existing = tx
            .query_row(
                "SELECT id FROM clipboard_entries WHERE fingerprint = ?",
                [&capture.fingerprint],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        let (id, duplicate) = if let Some(id) = existing {
            tx.execute(
                "UPDATE clipboard_entries SET updated_at = ?, source_app = ?,
                 source_executable = ?, source_window_title = ?, content_type = ?,
                 preview_text = ?, searchable_text = ?, sanitized_html = ?, byte_size = ?
                 WHERE id = ?",
                params![
                    now,
                    capture.source.app_name,
                    capture.source.executable,
                    capture.source.window_title,
                    capture.content_type.as_str(),
                    capture.preview_text,
                    capture.searchable_text,
                    capture.sanitized_html,
                    i64::try_from(byte_size).unwrap_or(i64::MAX),
                    id,
                ],
            )?;
            Self::refresh_fts_tx(&tx, id, capture.searchable_text.as_deref(), capture.source.app_name.as_deref())?;
            (id, true)
        } else {
            tx.execute(
                "INSERT INTO clipboard_entries
                 (created_at, updated_at, source_app, source_executable, source_window_title,
                  content_type, preview_text, searchable_text, sanitized_html, fingerprint,
                  pinned, byte_size)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, ?)",
                params![
                    now,
                    now,
                    capture.source.app_name,
                    capture.source.executable,
                    capture.source.window_title,
                    capture.content_type.as_str(),
                    capture.preview_text,
                    capture.searchable_text,
                    capture.sanitized_html,
                    capture.fingerprint,
                    i64::try_from(byte_size).unwrap_or(i64::MAX),
                ],
            )?;
            let id = tx.last_insert_rowid();
            for representation in prepared {
                Self::insert_prepared_representation_tx(&tx, id, &representation)?;
            }
            Self::refresh_fts_tx(&tx, id, capture.searchable_text.as_deref(), capture.source.app_name.as_deref())?;
            (id, false)
        };
        Self::enforce_capacity_tx(&tx, &settings)?;
        tx.commit()?;
        self.reconcile_blob_store()?;
        Ok(RecordResult { id, duplicate })
    }

    fn prepare_representation(
        &self,
        representation: &ClipboardRepresentation,
    ) -> Result<PreparedRepresentation> {
        let byte_size = representation.bytes.len() as u64;
        if representation.bytes.len() <= INLINE_LIMIT {
            return Ok(PreparedRepresentation {
                format: representation.format.clone(),
                mime_type: representation.mime_type.clone(),
                inline_data: Some(representation.bytes.clone()),
                blob_hash: None,
                byte_size,
            });
        }
        let hash = hash_bytes(&representation.bytes);
        self.write_blob(&hash, &representation.mime_type, &representation.bytes)?;
        Ok(PreparedRepresentation {
            format: representation.format.clone(),
            mime_type: representation.mime_type.clone(),
            inline_data: None,
            blob_hash: Some(hash),
            byte_size,
        })
    }

    fn write_blob(&self, hash: &str, mime_type: &str, bytes: &[u8]) -> Result<()> {
        validate_hash(hash)?;
        let path = self.blobs_dir.join(hash);
        if path.exists() {
            let existing = fs::read(&path)?;
            if hash_bytes(&existing) != hash {
                return Err(StorageError::MissingBlob(hash.to_owned()));
            }
            return Ok(());
        }
        let temporary = self
            .blobs_dir
            .join(format!(".{hash}.{}.tmp", Uuid::new_v4()));
        {
            let mut file = File::create(&temporary)?;
            file.write_all(bytes)?;
            file.sync_all()?;
        }
        if let Err(error) = fs::rename(&temporary, &path) {
            if path.exists() {
                let existing = fs::read(&path)?;
                if hash_bytes(&existing) != hash {
                    return Err(StorageError::MissingBlob(hash.to_owned()));
                }
                let _ = fs::remove_file(&temporary);
            } else {
                return Err(StorageError::Io(error));
            }
        }
        let _ = mime_type;
        Ok(())
    }

    fn insert_prepared_representation_tx(
        tx: &Transaction<'_>,
        entry_id: i64,
        representation: &PreparedRepresentation,
    ) -> Result<()> {
        tx.execute(
            "INSERT INTO clipboard_representations
             (entry_id, format, mime_type, inline_data, blob_hash, byte_size)
             VALUES (?, ?, ?, ?, ?, ?)",
            params![
                entry_id,
                representation.format,
                representation.mime_type,
                representation.inline_data,
                representation.blob_hash,
                i64::try_from(representation.byte_size).unwrap_or(i64::MAX),
            ],
        )?;
        if let Some(hash) = &representation.blob_hash {
            tx.execute(
                "INSERT OR IGNORE INTO clipboard_blobs (hash, mime_type, byte_size) VALUES (?, ?, ?)",
                params![hash, representation.mime_type, i64::try_from(representation.byte_size).unwrap_or(i64::MAX)],
            )?;
        }
        Ok(())
    }

    fn refresh_fts_tx(
        tx: &Transaction<'_>,
        id: i64,
        searchable_text: Option<&str>,
        source_app: Option<&str>,
    ) -> Result<()> {
        tx.execute("DELETE FROM clipboard_fts WHERE entry_id = ?", [id.to_string()])?;
        tx.execute(
            "INSERT INTO clipboard_fts (entry_id, searchable_text, source_app) VALUES (?, ?, ?)",
            params![id.to_string(), searchable_text.unwrap_or_default(), source_app.unwrap_or_default()],
        )?;
        Ok(())
    }

    fn enforce_capacity_tx(tx: &Transaction<'_>, settings: &ClipboardSettings) -> Result<()> {
        loop {
            let (count, total): (i64, i64) = tx.query_row(
                "SELECT COUNT(*), COALESCE(SUM(byte_size), 0) FROM clipboard_entries WHERE pinned = 0",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;
            if count <= i64::from(settings.max_entries)
                && total <= i64::try_from(settings.max_total_bytes).unwrap_or(i64::MAX)
            {
                break;
            }
            let oldest: Option<i64> = tx
                .query_row(
                    "SELECT id FROM clipboard_entries WHERE pinned = 0 ORDER BY updated_at ASC, id ASC LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(oldest) = oldest else { break };
            tx.execute("DELETE FROM clipboard_fts WHERE entry_id = ?", [oldest.to_string()])?;
            tx.execute("DELETE FROM clipboard_entries WHERE id = ?", [oldest])?;
        }
        Ok(())
    }

    pub fn list_entries(&self, query: &str, limit: u32) -> Result<Vec<ClipboardEntry>> {
        let limit = i64::from(limit.clamp(1, 500));
        let mut entries = Vec::new();
        if query.trim().is_empty() {
            let mut statement = self.connection.prepare(
                "SELECT id, created_at, updated_at, source_app, source_executable,
                        source_window_title, content_type, preview_text, searchable_text,
                        sanitized_html, fingerprint, pinned, byte_size
                 FROM clipboard_entries ORDER BY pinned DESC, updated_at DESC, id DESC LIMIT ?",
            )?;
            let rows = statement.query_map([limit], map_entry)?;
            for row in rows { entries.push(row?); }
        } else {
            let pattern = format!("%{}%", query.trim());
            let mut statement = self.connection.prepare(
                "SELECT e.id, e.created_at, e.updated_at, e.source_app, e.source_executable,
                        e.source_window_title, e.content_type, e.preview_text, e.searchable_text,
                        e.sanitized_html, e.fingerprint, e.pinned, e.byte_size
                 FROM clipboard_entries e
                 JOIN clipboard_fts f ON CAST(f.entry_id AS INTEGER) = e.id
                 WHERE f.searchable_text LIKE ? OR f.source_app LIKE ?
                 ORDER BY e.pinned DESC, e.updated_at DESC, e.id DESC LIMIT ?",
            )?;
            let rows = statement.query_map(params![pattern, pattern, limit], map_entry)?;
            for row in rows { entries.push(row?); }
        }
        Ok(entries)
    }

    pub fn entry(&self, id: i64) -> Result<Option<StoredClipboardEntry>> {
        let entry = self
            .connection
            .query_row(
                "SELECT id, created_at, updated_at, source_app, source_executable,
                        source_window_title, content_type, preview_text, searchable_text,
                        sanitized_html, fingerprint, pinned, byte_size
                 FROM clipboard_entries WHERE id = ?",
                [id],
                map_entry,
            )
            .optional()?;
        let Some(entry) = entry else { return Ok(None) };
        Ok(Some(StoredClipboardEntry {
            representations: self.entry_representations(id)?,
            entry,
        }))
    }

    fn entry_representations(&self, id: i64) -> Result<Vec<StoredRepresentation>> {
        let mut statement = self.connection.prepare(
            "SELECT id, entry_id, format, mime_type, inline_data, blob_hash, byte_size
             FROM clipboard_representations WHERE entry_id = ? ORDER BY id",
        )?;
        let rows = statement.query_map([id], map_representation)?;
        rows.map(|row| row.map_err(StorageError::from)).collect()
    }

    pub fn entry_payload(&self, id: i64) -> Result<Vec<ClipboardRepresentation>> {
        let stored = self
            .entry(id)?
            .ok_or_else(|| StorageError::Invalid(format!("clipboard entry {id} does not exist")))?;
        stored
            .representations
            .iter()
            .map(|representation| self.resolve_representation(representation))
            .collect()
    }

    fn resolve_representation(
        &self,
        representation: &StoredRepresentation,
    ) -> Result<ClipboardRepresentation> {
        let bytes = if let Some(bytes) = &representation.inline_data {
            bytes.clone()
        } else if let Some(hash) = &representation.blob_hash {
            let path = self.blobs_dir.join(hash);
            let bytes = fs::read(&path).map_err(|_| StorageError::MissingBlob(hash.clone()))?;
            if hash_bytes(&bytes) != *hash {
                return Err(StorageError::MissingBlob(hash.clone()));
            }
            bytes
        } else {
            return Err(StorageError::Invalid("representation has no data".to_owned()));
        };
        Ok(ClipboardRepresentation {
            format: representation.format.clone(),
            mime_type: representation.mime_type.clone(),
            bytes,
        })
    }

    pub fn set_pinned(&mut self, entry_id: i64, pinned: bool) -> Result<bool> {
        let entry = self
            .entry(entry_id)?
            .ok_or_else(|| StorageError::Invalid(format!("clipboard entry {entry_id} does not exist")))?;
        let tx = self.connection.transaction()?;
        if pinned {
            tx.execute("DELETE FROM saved_insert_items WHERE source_entry_id = ?", [entry_id])?;
            tx.execute(
                "INSERT INTO saved_insert_items
                 (source_entry_id, created_at, updated_at, source_app, source_executable,
                  source_window_title, content_type, preview_text, searchable_text,
                  sanitized_html, byte_size)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                params![
                    entry_id,
                    entry.entry.created_at,
                    entry.entry.updated_at,
                    entry.entry.source_app,
                    entry.entry.source_executable,
                    entry.entry.source_window_title,
                    entry.entry.content_type,
                    entry.entry.preview_text,
                    entry.entry.searchable_text,
                    entry.entry.sanitized_html,
                    i64::try_from(entry.entry.byte_size).unwrap_or(i64::MAX),
                ],
            )?;
            let saved_id = tx.last_insert_rowid();
            for representation in entry.representations {
                tx.execute(
                    "INSERT INTO saved_insert_representations
                     (saved_item_id, format, mime_type, inline_data, blob_hash, byte_size)
                     VALUES (?, ?, ?, ?, ?, ?)",
                    params![
                        saved_id,
                        representation.format,
                        representation.mime_type,
                        representation.inline_data,
                        representation.blob_hash,
                        i64::try_from(representation.byte_size).unwrap_or(i64::MAX),
                    ],
                )?;
            }
        } else {
            tx.execute("DELETE FROM saved_insert_items WHERE source_entry_id = ?", [entry_id])?;
        }
        tx.execute(
            "UPDATE clipboard_entries SET pinned = ? WHERE id = ?",
            params![pinned as i64, entry_id],
        )?;
        tx.commit()?;
        self.reconcile_blob_store()?;
        Ok(true)
    }

    pub fn list_saved_items(&self, query: &str, limit: u32) -> Result<Vec<SavedInsertItem>> {
        let limit = i64::from(limit.clamp(1, 500));
        let pattern = format!("%{}%", query.trim());
        let mut statement = self.connection.prepare(
            "SELECT id, source_entry_id, created_at, updated_at, source_app, source_executable,
                    source_window_title, content_type, preview_text, searchable_text,
                    sanitized_html, byte_size
             FROM saved_insert_items
             WHERE ? = '' OR preview_text LIKE ? OR searchable_text LIKE ? OR source_app LIKE ?
             ORDER BY updated_at DESC, id DESC LIMIT ?",
        )?;
        let rows = statement.query_map(params![query.trim(), pattern, pattern, pattern, limit], |row| {
            Ok(SavedInsertItem {
                id: row.get(0)?,
                source_entry_id: row.get(1)?,
                created_at: row.get(2)?,
                updated_at: row.get(3)?,
                source_app: row.get(4)?,
                source_executable: row.get(5)?,
                source_window_title: row.get(6)?,
                content_type: row.get(7)?,
                preview_text: row.get(8)?,
                searchable_text: row.get(9)?,
                sanitized_html: row.get(10)?,
                byte_size: row.get::<_, i64>(11)?.try_into().unwrap_or(0),
            })
        })?;
        rows.map(|row| row.map_err(StorageError::from)).collect()
    }

    pub fn saved_insert_item(&self, id: i64) -> Result<Option<StoredSavedInsertItem>> {
        let item = self
            .connection
            .query_row(
                "SELECT id, source_entry_id, created_at, updated_at, source_app, source_executable,
                        source_window_title, content_type, preview_text, searchable_text,
                        sanitized_html, byte_size
                 FROM saved_insert_items WHERE id = ?",
                [id],
                |row| {
                    Ok(SavedInsertItem {
                        id: row.get(0)?,
                        source_entry_id: row.get(1)?,
                        created_at: row.get(2)?,
                        updated_at: row.get(3)?,
                        source_app: row.get(4)?,
                        source_executable: row.get(5)?,
                        source_window_title: row.get(6)?,
                        content_type: row.get(7)?,
                        preview_text: row.get(8)?,
                        searchable_text: row.get(9)?,
                        sanitized_html: row.get(10)?,
                        byte_size: row.get::<_, i64>(11)?.try_into().unwrap_or(0),
                    })
                },
            )
            .optional()?;
        let Some(item) = item else { return Ok(None) };
        let mut statement = self.connection.prepare(
            "SELECT id, saved_item_id, format, mime_type, inline_data, blob_hash, byte_size
             FROM saved_insert_representations WHERE saved_item_id = ? ORDER BY id",
        )?;
        let rows = statement.query_map([id], |row| {
            Ok(StoredRepresentation {
                id: row.get(0)?,
                entry_id: row.get(1)?,
                format: row.get(2)?,
                mime_type: row.get(3)?,
                inline_data: row.get(4)?,
                blob_hash: row.get(5)?,
                byte_size: row.get::<_, i64>(6)?.try_into().unwrap_or(0),
            })
        })?;
        let representations = rows.map(|row| row.map_err(StorageError::from)).collect::<Result<Vec<_>>>()?;
        Ok(Some(StoredSavedInsertItem { item, representations }))
    }

    pub fn saved_item_payload(&self, id: i64) -> Result<Vec<ClipboardRepresentation>> {
        let stored = self
            .saved_insert_item(id)?
            .ok_or_else(|| StorageError::Invalid(format!("saved item {id} does not exist")))?;
        stored
            .representations
            .iter()
            .map(|representation| self.resolve_representation(representation))
            .collect()
    }

    pub fn delete_entry(&mut self, id: i64) -> Result<bool> {
        let tx = self.connection.transaction()?;
        tx.execute("UPDATE saved_insert_items SET source_entry_id = NULL WHERE source_entry_id = ?", [id])?;
        let deleted = tx.execute("DELETE FROM clipboard_entries WHERE id = ?", [id])? > 0;
        tx.execute("DELETE FROM clipboard_fts WHERE entry_id = ?", [id.to_string()])?;
        tx.commit()?;
        self.reconcile_blob_store()?;
        Ok(deleted)
    }

    pub fn delete_saved_insert_item(&mut self, id: i64) -> Result<bool> {
        let tx = self.connection.transaction()?;
        let source_id: Option<i64> = tx
            .query_row("SELECT source_entry_id FROM saved_insert_items WHERE id = ?", [id], |row| row.get(0))
            .optional()?;
        let deleted = tx.execute("DELETE FROM saved_insert_items WHERE id = ?", [id])? > 0;
        if let Some(source_id) = source_id {
            tx.execute("UPDATE clipboard_entries SET pinned = 0 WHERE id = ?", [source_id])?;
        }
        tx.commit()?;
        self.reconcile_blob_store()?;
        Ok(deleted)
    }

    pub fn clear_history(&mut self) -> Result<()> {
        self.connection.execute("DELETE FROM clipboard_fts WHERE entry_id IN (SELECT CAST(entry_id AS INTEGER) FROM clipboard_fts)", [])?;
        self.connection.execute("DELETE FROM clipboard_entries WHERE pinned = 0", [])?;
        self.reconcile_blob_store()?;
        Ok(())
    }

    pub fn snippets(&self, query: &str) -> Result<Vec<Snippet>> {
        let pattern = format!("%{}%", query.trim());
        let mut statement = self.connection.prepare(
            "SELECT id, name, content, group_name, created_at, updated_at
             FROM snippets WHERE ? = '' OR name LIKE ? OR content LIKE ? OR group_name LIKE ?
             ORDER BY updated_at DESC, id DESC",
        )?;
        let rows = statement.query_map(params![query.trim(), pattern, pattern, pattern], |row| {
            Ok(Snippet {
                id: row.get(0)?,
                name: row.get(1)?,
                content: row.get(2)?,
                group_name: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
            })
        })?;
        rows.map(|row| row.map_err(StorageError::from)).collect()
    }

    pub fn save_snippet(
        &mut self,
        id: Option<i64>,
        name: &str,
        content: &str,
        group_name: Option<&str>,
    ) -> Result<i64> {
        if name.trim().is_empty() {
            return Err(StorageError::Invalid("snippet name cannot be empty".to_owned()));
        }
        let now = now_millis();
        if let Some(id) = id {
            let updated = self.connection.execute(
                "UPDATE snippets SET name = ?, content = ?, group_name = ?, updated_at = ? WHERE id = ?",
                params![name, content, group_name, now, id],
            )?;
            if updated == 0 {
                return Err(StorageError::Invalid(format!("snippet {id} does not exist")));
            }
            Ok(id)
        } else {
            self.connection.execute(
                "INSERT INTO snippets (name, content, group_name, created_at, updated_at) VALUES (?, ?, ?, ?, ?)",
                params![name, content, group_name, now, now],
            )?;
            Ok(self.connection.last_insert_rowid())
        }
    }

    pub fn delete_snippet(&mut self, id: i64) -> Result<bool> {
        Ok(self.connection.execute("DELETE FROM snippets WHERE id = ?", [id])? > 0)
    }

    pub fn reconcile_blob_store(&mut self) -> Result<()> {
        let mut referenced = HashSet::new();
        {
            let mut statement = self.connection.prepare(
                "SELECT blob_hash FROM clipboard_representations WHERE blob_hash IS NOT NULL
                 UNION SELECT blob_hash FROM saved_insert_representations WHERE blob_hash IS NOT NULL",
            )?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            for row in rows { referenced.insert(row?); }
        }
        for hash in &referenced {
            validate_hash(hash)?;
            let path = self.blobs_dir.join(hash);
            if !path.exists() {
                return Err(StorageError::MissingBlob(hash.clone()));
            }
            let bytes = fs::read(&path)?;
            if hash_bytes(&bytes) != *hash {
                return Err(StorageError::MissingBlob(hash.clone()));
            }
        }
        let mut statement = self.connection.prepare("SELECT hash FROM clipboard_blobs")?;
        let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        let database_hashes = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        for hash in database_hashes {
            if !referenced.contains(&hash) {
                self.connection.execute("DELETE FROM clipboard_blobs WHERE hash = ?", [&hash])?;
            }
        }
        if self.blobs_dir.exists() {
            for item in fs::read_dir(&self.blobs_dir)? {
                let item = item?;
                let path = item.path();
                let name = item.file_name().to_string_lossy().to_string();
                if name.starts_with('.') || name.ends_with(".tmp") || !referenced.contains(&name) {
                    if path.is_file() { let _ = fs::remove_file(path); }
                }
            }
        }
        Ok(())
    }

    pub fn migrate_legacy(&mut self, legacy_dir: impl AsRef<Path>) -> Result<MigrationReport> {
        let legacy_dir = legacy_dir.as_ref();
        let legacy_db = legacy_dir.join("culsans.sqlite3");
        if !legacy_db.is_file() {
            return Err(StorageError::Migration(format!("legacy database is missing: {}", legacy_db.display())));
        }
        let marker_key = legacy_dir.canonicalize()?.to_string_lossy().to_string();
        if self
            .connection
            .query_row("SELECT value FROM migration_state WHERE key = ?", [&marker_key], |row| row.get::<_, String>(0))
            .optional()?
            .is_some()
        {
            return Ok(MigrationReport {
                backup_dir: self.data_dir.join("migration-backups"),
                entries: self.count_table("clipboard_entries")?,
                representations: self.count_table("clipboard_representations")?,
                blobs: self.count_table("clipboard_blobs")?,
                favorites: self.count_table("saved_insert_items")?,
                snippets: self.count_table("snippets")?,
                already_migrated: true,
            });
        }
        let lock_path = legacy_dir.join(".echo-migration.lock");
        let _lock = OpenOptions::new().write(true).create_new(true).open(&lock_path)
            .map_err(|error| StorageError::Migration(format!("cannot acquire legacy migration lock: {error}")))?;
        let backup_dir = self.data_dir.join("migration-backups").join(Uuid::new_v4().to_string());
        let result = self.migrate_legacy_locked(legacy_dir, &legacy_db, &backup_dir, &marker_key);
        let _ = fs::remove_file(lock_path);
        result
    }

    fn migrate_legacy_locked(
        &mut self,
        legacy_dir: &Path,
        legacy_db: &Path,
        backup_dir: &Path,
        marker_key: &str,
    ) -> Result<MigrationReport> {
        let before = file_fingerprint(legacy_db)?;
        fs::create_dir_all(backup_dir.join("blobs"))?;
        fs::copy(legacy_db, backup_dir.join("culsans.sqlite3"))?;
        for suffix in ["-wal", "-shm"] {
            let source = legacy_dir.join(format!("culsans.sqlite3{suffix}"));
            if source.exists() { fs::copy(&source, backup_dir.join(format!("culsans.sqlite3{suffix}")))?; }
        }
        let legacy_blobs = legacy_dir.join("blobs");
        if legacy_blobs.exists() {
            for item in fs::read_dir(&legacy_blobs)? {
                let item = item?;
                if item.path().is_file() { fs::copy(item.path(), backup_dir.join("blobs").join(item.file_name()))?; }
            }
        }
        if file_fingerprint(legacy_db)? != before {
            return Err(StorageError::Migration("legacy database changed while snapshotting".to_owned()));
        }
        let snapshot_db = backup_dir.join("culsans.sqlite3");
        let source = Connection::open_with_flags(&snapshot_db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let entries_count = count_table_connection(&source, "clipboard_entries")?;
        let representations_count = count_table_connection(&source, "clipboard_representations")?;
        let snippets_count = count_table_connection(&source, "snippets")?;
        let has_saved = table_exists_connection(&source, "saved_insert_items")?;
        let saved_count = if has_saved { count_table_connection(&source, "saved_insert_items")? } else { 0 };
        let source_blobs = backup_dir.join("blobs");
        let tx = self.connection.transaction()?;
        if count_table_tx(&tx, "clipboard_entries")? != 0
            || count_table_tx(&tx, "snippets")? != 0
        {
            return Err(StorageError::Migration("destination already contains Echo data".to_owned()));
        }
        copy_settings(&source, &tx)?;
        copy_legacy_entries(&source, &source_blobs, &tx, &self.blobs_dir)?;
        let favorites = if has_saved {
            copy_legacy_saved_items(&source, &source_blobs, &tx, &self.blobs_dir)?
        } else {
            synthesize_pinned_items(&tx)?
        };
        let snippets = copy_legacy_snippets(&source, &tx)?;
        let actual_entries = count_table_tx(&tx, "clipboard_entries")?;
        let actual_representations = count_table_tx(&tx, "clipboard_representations")?;
        if actual_entries != entries_count || actual_representations != representations_count {
            return Err(StorageError::Migration(format!(
                "legacy row count changed during migration: entries {actual_entries}/{entries_count}, representations {actual_representations}/{representations_count}"
            )));
        }
        if has_saved && favorites != saved_count {
            return Err(StorageError::Migration(format!("legacy favorites changed during migration: {favorites}/{saved_count}")));
        }
        if snippets != snippets_count {
            return Err(StorageError::Migration(format!("legacy snippets changed during migration: {snippets}/{snippets_count}")));
        }
        tx.commit()?;
        self.rebuild_fts()?;
        self.reconcile_blob_store()?;
        let destination_hash = hash_file(&snapshot_db)?;
        self.connection.execute(
            "INSERT INTO migration_state (key, value, completed_at) VALUES (?, ?, ?)",
            params![marker_key, destination_hash, now_millis()],
        )?;
        Ok(MigrationReport {
            backup_dir: backup_dir.to_path_buf(),
            entries: entries_count,
            representations: representations_count,
            blobs: self.count_table("clipboard_blobs")?,
            favorites,
            snippets,
            already_migrated: false,
        })
    }

    fn rebuild_fts(&mut self) -> Result<()> {
        self.connection.execute("DELETE FROM clipboard_fts", [])?;
        let mut statement = self.connection.prepare("SELECT id, searchable_text, source_app FROM clipboard_entries")?;
        let rows = statement.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?, row.get::<_, Option<String>>(2)?))
        })?;
        let entries = rows.collect::<std::result::Result<Vec<_>, _>>()?;
        for (id, searchable, source) in entries {
            self.connection.execute(
                "INSERT INTO clipboard_fts (entry_id, searchable_text, source_app) VALUES (?, ?, ?)",
                params![id.to_string(), searchable.unwrap_or_default(), source.unwrap_or_default()],
            )?;
        }
        Ok(())
    }

    fn count_table(&self, table: &str) -> Result<usize> {
        Ok(self.connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get::<_, i64>(0))? as usize)
    }

}

#[derive(Clone)]
pub struct SharedClipboardStore {
    inner: Arc<Mutex<ClipboardStore>>,
}

impl SharedClipboardStore {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self> {
        Ok(Self {
            inner: Arc::new(Mutex::new(ClipboardStore::open(data_dir)?)),
        })
    }

    pub fn from_store(store: ClipboardStore) -> Self {
        Self { inner: Arc::new(Mutex::new(store)) }
    }

    pub fn with_store<T>(&self, operation: impl FnOnce(&mut ClipboardStore) -> Result<T>) -> Result<T> {
        let mut store = self.inner.lock().map_err(|_| StorageError::Invalid("storage lock poisoned".to_owned()))?;
        operation(&mut store)
    }

    pub fn settings(&self) -> Result<ClipboardSettings> {
        self.with_store(|store| store.settings())
    }

    pub fn update_settings(&self, settings: &ClipboardSettings) -> Result<()> {
        self.with_store(|store| store.update_settings(settings))
    }

    pub fn list_entries(&self, query: &str, limit: u32) -> Result<Vec<ClipboardEntry>> {
        let store = self.inner.lock().map_err(|_| StorageError::Invalid("storage lock poisoned".to_owned()))?;
        store.list_entries(query, limit)
    }

    pub fn entry(&self, id: i64) -> Result<Option<StoredClipboardEntry>> {
        let store = self.inner.lock().map_err(|_| StorageError::Invalid("storage lock poisoned".to_owned()))?;
        store.entry(id)
    }

    pub fn entry_payload(&self, id: i64) -> Result<Vec<ClipboardRepresentation>> {
        let store = self.inner.lock().map_err(|_| StorageError::Invalid("storage lock poisoned".to_owned()))?;
        store.entry_payload(id)
    }

    pub fn list_saved_items(&self, query: &str, limit: u32) -> Result<Vec<SavedInsertItem>> {
        let store = self.inner.lock().map_err(|_| StorageError::Invalid("storage lock poisoned".to_owned()))?;
        store.list_saved_items(query, limit)
    }

    pub fn saved_insert_item(&self, id: i64) -> Result<Option<StoredSavedInsertItem>> {
        let store = self.inner.lock().map_err(|_| StorageError::Invalid("storage lock poisoned".to_owned()))?;
        store.saved_insert_item(id)
    }

    pub fn saved_item_payload(&self, id: i64) -> Result<Vec<ClipboardRepresentation>> {
        let store = self.inner.lock().map_err(|_| StorageError::Invalid("storage lock poisoned".to_owned()))?;
        store.saved_item_payload(id)
    }

    pub fn snippets(&self, query: &str) -> Result<Vec<Snippet>> {
        let store = self.inner.lock().map_err(|_| StorageError::Invalid("storage lock poisoned".to_owned()))?;
        store.snippets(query)
    }

    pub fn save_snippet(&self, id: Option<i64>, name: &str, content: &str, group_name: Option<&str>) -> Result<i64> {
        self.with_store(|store| store.save_snippet(id, name, content, group_name))
    }

    pub fn delete_snippet(&self, id: i64) -> Result<bool> {
        self.with_store(|store| store.delete_snippet(id))
    }

    pub fn set_pinned(&self, id: i64, pinned: bool) -> Result<bool> {
        self.with_store(|store| store.set_pinned(id, pinned))
    }

    pub fn delete_entry(&self, id: i64) -> Result<bool> {
        self.with_store(|store| store.delete_entry(id))
    }

    pub fn delete_saved_insert_item(&self, id: i64) -> Result<bool> {
        self.with_store(|store| store.delete_saved_insert_item(id))
    }

    pub fn clear_history(&self) -> Result<()> {
        self.with_store(|store| store.clear_history())
    }

    pub fn reconcile_blob_store(&self) -> Result<()> {
        self.with_store(|store| store.reconcile_blob_store())
    }

    pub fn migrate_legacy(&self, legacy_dir: impl AsRef<Path>) -> Result<MigrationReport> {
        self.with_store(|store| store.migrate_legacy(legacy_dir))
    }
}

impl ClipboardSink for SharedClipboardStore {
    fn settings(&self) -> std::result::Result<CaptureSettings, String> {
        self.settings()
            .map(|settings| CaptureSettings::from(&settings))
            .map_err(|error| error.to_string())
    }

    fn record(&self, capture: NormalizedCapture) -> std::result::Result<RecordResult, String> {
        self.with_store(|store| store.record_capture(capture))
            .map_err(|error| error.to_string())
    }

    fn reconcile(&self) -> std::result::Result<(), String> {
        self.reconcile_blob_store().map_err(|error| error.to_string())
    }
}

fn map_entry(row: &Row<'_>) -> rusqlite::Result<ClipboardEntry> {
    Ok(ClipboardEntry {
        id: row.get(0)?,
        created_at: row.get(1)?,
        updated_at: row.get(2)?,
        source_app: row.get(3)?,
        source_executable: row.get(4)?,
        source_window_title: row.get(5)?,
        content_type: row.get(6)?,
        preview_text: row.get(7)?,
        searchable_text: row.get(8)?,
        sanitized_html: row.get(9)?,
        fingerprint: row.get(10)?,
        pinned: row.get::<_, i64>(11)? != 0,
        byte_size: row.get::<_, i64>(12)?.try_into().unwrap_or(0),
    })
}

fn map_representation(row: &Row<'_>) -> rusqlite::Result<StoredRepresentation> {
    Ok(StoredRepresentation {
        id: row.get(0)?,
        entry_id: row.get(1)?,
        format: row.get(2)?,
        mime_type: row.get(3)?,
        inline_data: row.get(4)?,
        blob_hash: row.get(5)?,
        byte_size: row.get::<_, i64>(6)?.try_into().unwrap_or(0),
    })
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(i64::MAX)
}

fn hash_bytes(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 { break; }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn file_fingerprint(path: &Path) -> Result<(u64, u128)> {
    let metadata = fs::metadata(path)?;
    let modified = metadata.modified().ok().and_then(|value| value.duration_since(UNIX_EPOCH).ok()).map(|value| value.as_nanos()).unwrap_or(0);
    Ok((metadata.len(), modified))
}

fn validate_hash(hash: &str) -> Result<()> {
    if hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(StorageError::Invalid(format!("invalid blob hash {hash}")));
    }
    Ok(())
}

fn table_exists_connection(connection: &Connection, table: &str) -> Result<bool> {
    Ok(connection.query_row(
        "SELECT EXISTS (SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?)",
        [table],
        |row| row.get::<_, i64>(0),
    )? != 0)
}

fn count_table_connection(connection: &Connection, table: &str) -> Result<usize> {
    if !table_exists_connection(connection, table)? { return Ok(0); }
    Ok(connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get::<_, i64>(0))? as usize)
}

fn count_table_tx(tx: &Transaction<'_>, table: &str) -> Result<usize> {
    Ok(tx.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row.get::<_, i64>(0))? as usize)
}

fn has_column(connection: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?.iter().any(|name| name == column))
}

fn copy_settings(source: &Connection, tx: &Transaction<'_>) -> Result<()> {
    if !table_exists_connection(source, "clipboard_settings")? { return Ok(()); }
    let record_sensitive = if has_column(source, "clipboard_settings", "record_sensitive")? { "record_sensitive" } else { "0" };
    let sql = format!(
        "SELECT history_enabled, {record_sensitive}, store_window_titles, max_entries, max_total_bytes, max_item_bytes FROM clipboard_settings WHERE id = 1"
    );
    let row: Option<(i64, i64, i64, i64, i64, i64)> = source.query_row(&sql, [], |row| {
        Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?))
    }).optional()?;
    if let Some((history, sensitive, titles, max_entries, max_total, max_item)) = row {
        tx.execute(
            "UPDATE clipboard_settings SET history_enabled = ?, record_sensitive = ?, store_window_titles = ?, max_entries = ?, max_total_bytes = ?, max_item_bytes = ? WHERE id = 1",
            params![history, sensitive, titles, max_entries, max_total, max_item],
        )?;
    }
    Ok(())
}

fn copy_legacy_entries(
    source: &Connection,
    source_blobs: &Path,
    tx: &Transaction<'_>,
    destination_blobs: &Path,
) -> Result<()> {
    if !table_exists_connection(source, "clipboard_entries")? { return Ok(()); }
    let mut statement = source.prepare(
        "SELECT id, created_at, updated_at, source_app, source_executable, source_window_title,
                content_type, preview_text, searchable_text, sanitized_html, fingerprint, pinned, byte_size
         FROM clipboard_entries ORDER BY id",
    )?;
    let entries = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?,
            row.get::<_, Option<String>>(3)?, row.get::<_, Option<String>>(4)?, row.get::<_, Option<String>>(5)?,
            row.get::<_, String>(6)?, row.get::<_, Option<String>>(7)?, row.get::<_, Option<String>>(8)?,
            row.get::<_, Option<String>>(9)?, row.get::<_, String>(10)?, row.get::<_, i64>(11)?, row.get::<_, i64>(12)?,
        ))
    })?;
    let entries = entries.collect::<std::result::Result<Vec<_>, _>>()?;
    for (id, created, updated, app, executable, title, content_type, preview, searchable, html, fingerprint, pinned, byte_size) in entries {
        tx.execute(
            "INSERT INTO clipboard_entries (id, created_at, updated_at, source_app, source_executable, source_window_title, content_type, preview_text, searchable_text, sanitized_html, fingerprint, pinned, byte_size) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![id, created, updated, app, executable, title, content_type, preview, searchable, html, fingerprint, pinned, byte_size],
        )?;
    }
    if !table_exists_connection(source, "clipboard_representations")? { return Ok(()) }
    let mut statement = source.prepare(
        "SELECT id, entry_id, format, mime_type, inline_data, blob_hash, byte_size FROM clipboard_representations ORDER BY id",
    )?;
    let representations = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?,
            row.get::<_, Option<Vec<u8>>>(4)?, row.get::<_, Option<String>>(5)?, row.get::<_, i64>(6)?,
        ))
    })?;
    for (id, entry_id, format, mime, inline_data, blob_hash, byte_size) in representations.collect::<std::result::Result<Vec<_>, _>>()? {
        let data = if let Some(bytes) = inline_data {
            (Some(bytes), None)
        } else if let Some(hash) = blob_hash {
            validate_hash(&hash)?;
            let bytes = fs::read(source_blobs.join(&hash)).map_err(|_| StorageError::MissingBlob(hash.clone()))?;
            if hash_bytes(&bytes) != hash { return Err(StorageError::MissingBlob(hash)); }
            let destination = destination_blobs.join(&hash);
            if !destination.exists() { fs::copy(source_blobs.join(&hash), destination)?; }
            (None, Some(hash))
        } else {
            return Err(StorageError::Migration(format!("representation {id} has no inline or blob data")));
        };
        tx.execute(
            "INSERT INTO clipboard_representations (id, entry_id, format, mime_type, inline_data, blob_hash, byte_size) VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![id, entry_id, format, mime, data.0, data.1, byte_size],
        )?;
        if let Some(hash) = data.1 {
            tx.execute("INSERT OR IGNORE INTO clipboard_blobs (hash, mime_type, byte_size) VALUES (?, ?, ?)", params![hash, mime, byte_size])?;
        }
    }
    Ok(())
}

fn copy_legacy_saved_items(
    source: &Connection,
    source_blobs: &Path,
    tx: &Transaction<'_>,
    destination_blobs: &Path,
) -> Result<usize> {
    let mut statement = source.prepare(
        "SELECT id, source_entry_id, created_at, updated_at, source_app, source_executable,
                source_window_title, content_type, preview_text, searchable_text, sanitized_html, byte_size
         FROM saved_insert_items ORDER BY id",
    )?;
    let items = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?,
            row.get::<_, Option<String>>(4)?, row.get::<_, Option<String>>(5)?, row.get::<_, Option<String>>(6)?,
            row.get::<_, String>(7)?, row.get::<_, Option<String>>(8)?, row.get::<_, Option<String>>(9)?,
            row.get::<_, Option<String>>(10)?, row.get::<_, i64>(11)?,
        ))
    })?;
    let items = items.collect::<std::result::Result<Vec<_>, _>>()?;
    for (id, source_id, created, updated, app, executable, title, content_type, preview, searchable, html, byte_size) in &items {
        tx.execute(
            "INSERT INTO saved_insert_items (id, source_entry_id, created_at, updated_at, source_app, source_executable, source_window_title, content_type, preview_text, searchable_text, sanitized_html, byte_size) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![id, source_id, created, updated, app, executable, title, content_type, preview, searchable, html, byte_size],
        )?;
    }
    let mut statement = source.prepare(
        "SELECT id, saved_item_id, format, mime_type, inline_data, blob_hash, byte_size FROM saved_insert_representations ORDER BY id",
    )?;
    let representations = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?,
            row.get::<_, Option<Vec<u8>>>(4)?, row.get::<_, Option<String>>(5)?, row.get::<_, i64>(6)?,
        ))
    })?;
    for (id, saved_id, format, mime, inline_data, blob_hash, byte_size) in representations.collect::<std::result::Result<Vec<_>, _>>()? {
        let (inline, blob) = if let Some(bytes) = inline_data { (Some(bytes), None) } else if let Some(hash) = blob_hash {
            validate_hash(&hash)?;
            let bytes = fs::read(source_blobs.join(&hash)).map_err(|_| StorageError::MissingBlob(hash.clone()))?;
            if hash_bytes(&bytes) != hash { return Err(StorageError::MissingBlob(hash)); }
            let destination = destination_blobs.join(&hash);
            if !destination.exists() { fs::copy(source_blobs.join(&hash), destination)?; }
            (None, Some(hash))
        } else { return Err(StorageError::Migration(format!("saved representation {id} has no data"))); };
        tx.execute(
            "INSERT INTO saved_insert_representations (id, saved_item_id, format, mime_type, inline_data, blob_hash, byte_size) VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![id, saved_id, format, mime, inline, blob, byte_size],
        )?;
        if let Some(hash) = blob {
            tx.execute("INSERT OR IGNORE INTO clipboard_blobs (hash, mime_type, byte_size) VALUES (?, ?, ?)", params![hash, mime, byte_size])?;
        }
    }
    Ok(items.len())
}

fn synthesize_pinned_items(tx: &Transaction<'_>) -> Result<usize> {
    let mut statement = tx.prepare("SELECT id, created_at, updated_at, source_app, source_executable, source_window_title, content_type, preview_text, searchable_text, sanitized_html, byte_size FROM clipboard_entries WHERE pinned = 1 ORDER BY id")?;
    let entries = statement.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?, row.get::<_, Option<String>>(3)?, row.get::<_, Option<String>>(4)?, row.get::<_, Option<String>>(5)?, row.get::<_, String>(6)?, row.get::<_, Option<String>>(7)?, row.get::<_, Option<String>>(8)?, row.get::<_, Option<String>>(9)?, row.get::<_, i64>(10)?))
    })?;
    let entries = entries.collect::<std::result::Result<Vec<_>, _>>()?;
    for (entry_id, created, updated, app, executable, title, content_type, preview, searchable, html, byte_size) in &entries {
        tx.execute("INSERT INTO saved_insert_items (source_entry_id, created_at, updated_at, source_app, source_executable, source_window_title, content_type, preview_text, searchable_text, sanitized_html, byte_size) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)", params![entry_id, created, updated, app, executable, title, content_type, preview, searchable, html, byte_size])?;
        let saved_id = tx.last_insert_rowid();
        let mut reps = tx.prepare("SELECT format, mime_type, inline_data, blob_hash, byte_size FROM clipboard_representations WHERE entry_id = ? ORDER BY id")?;
        let rows = reps.query_map([entry_id], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, Option<Vec<u8>>>(2)?, row.get::<_, Option<String>>(3)?, row.get::<_, i64>(4)?)))?;
        for (format, mime, inline, blob, size) in rows.collect::<std::result::Result<Vec<_>, _>>()? {
            tx.execute("INSERT INTO saved_insert_representations (saved_item_id, format, mime_type, inline_data, blob_hash, byte_size) VALUES (?, ?, ?, ?, ?, ?)", params![saved_id, format, mime, inline, blob, size])?;
        }
    }
    Ok(entries.len())
}

fn copy_legacy_snippets(source: &Connection, tx: &Transaction<'_>) -> Result<usize> {
    if !table_exists_connection(source, "snippets")? { return Ok(0); }
    let mut statement = source.prepare("SELECT id, name, content, group_name, created_at, updated_at FROM snippets ORDER BY id")?;
    let rows = statement.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?, row.get::<_, i64>(4)?, row.get::<_, i64>(5)?)))?;
    let rows = rows.collect::<std::result::Result<Vec<_>, _>>()?;
    for (id, name, content, group, created, updated) in &rows {
        tx.execute("INSERT INTO snippets (id, name, content, group_name, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?)", params![id, name, content, group, created, updated])?;
    }
    Ok(rows.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use echo_clipboard::{fingerprint, ContentType};
    use echo_platform::SourceContext;
    use tempfile::TempDir;

    fn text_capture(text: &str, sequence: u64) -> NormalizedCapture {
        let representation = ClipboardRepresentation {
            format: "text".to_owned(),
            mime_type: "text/plain;charset=utf-8".to_owned(),
            bytes: text.as_bytes().to_vec(),
        };
        NormalizedCapture {
            sequence,
            source: SourceContext::default(),
            content_type: ContentType::Text,
            preview_text: Some(text.to_owned()),
            searchable_text: Some(text.to_owned()),
            sanitized_html: None,
            fingerprint: fingerprint(std::slice::from_ref(&representation)),
            representations: vec![representation],
        }
    }

    #[test]
    fn record_deduplicates_and_refreshes_the_timestamp() {
        let root = TempDir::new().unwrap();
        let mut store = ClipboardStore::open(root.path()).unwrap();
        let first = store.record_capture(text_capture("hello", 1)).unwrap();
        let before = store.entry(first.id).unwrap().unwrap().entry.updated_at;
        std::thread::sleep(std::time::Duration::from_millis(2));
        let second = store.record_capture(text_capture("hello", 2)).unwrap();
        let after = store.entry(second.id).unwrap().unwrap().entry.updated_at;
        assert!(!first.duplicate);
        assert!(second.duplicate);
        assert_eq!(first.id, second.id);
        assert!(after >= before);
    }

    #[test]
    fn large_representation_is_stored_and_reconciled_as_a_blob() {
        let root = TempDir::new().unwrap();
        let mut store = ClipboardStore::open(root.path()).unwrap();
        let mut capture = text_capture("large", 1);
        capture.representations[0].bytes = vec![7; INLINE_LIMIT + 5];
        capture.fingerprint = fingerprint(&capture.representations);
        store.record_capture(capture).unwrap();
        let blobs = fs::read_dir(root.path().join("blobs")).unwrap().count();
        assert_eq!(blobs, 1);
        store.reconcile_blob_store().unwrap();
        fs::write(root.path().join("blobs").join("orphan"), b"orphan").unwrap();
        store.reconcile_blob_store().unwrap();
        assert!(!root.path().join("blobs").join("orphan").exists());
    }

    #[test]
    fn favorites_survive_history_clear_and_source_delete() {
        let root = TempDir::new().unwrap();
        let mut store = ClipboardStore::open(root.path()).unwrap();
        let id = store.record_capture(text_capture("favorite", 1)).unwrap().id;
        store.set_pinned(id, true).unwrap();
        store.clear_history().unwrap();
        assert!(store.entry(id).unwrap().is_some());
        assert_eq!(store.list_saved_items("", 20).unwrap().len(), 1);
        store.delete_entry(id).unwrap();
        let saved = store.list_saved_items("", 20).unwrap();
        assert_eq!(saved.len(), 1);
        assert!(saved[0].source_entry_id.is_none());
    }

    #[test]
    fn snippets_support_create_update_search_and_delete() {
        let root = TempDir::new().unwrap();
        let mut store = ClipboardStore::open(root.path()).unwrap();
        let id = store.save_snippet(None, "Greeting", "Hello world", Some("Common")).unwrap();
        store.save_snippet(Some(id), "Greeting", "Updated", Some("Common")).unwrap();
        assert_eq!(store.snippets("Updated").unwrap()[0].id, id);
        assert!(store.delete_snippet(id).unwrap());
        assert!(store.snippets("").unwrap().is_empty());
    }

    #[test]
    fn migrates_pinned_only_legacy_schema_and_is_idempotent() {
        let legacy = TempDir::new().unwrap();
        fs::create_dir_all(legacy.path().join("blobs")).unwrap();
        let legacy_db = legacy.path().join("culsans.sqlite3");
        let connection = Connection::open(&legacy_db).unwrap();
        connection.execute_batch(
            "CREATE TABLE clipboard_settings (id INTEGER PRIMARY KEY, history_enabled INTEGER, record_sensitive INTEGER, store_window_titles INTEGER, max_entries INTEGER, max_total_bytes INTEGER, max_item_bytes INTEGER);
             INSERT INTO clipboard_settings VALUES (1, 1, 0, 1, 50, 1000, 1000);
             CREATE TABLE clipboard_entries (id INTEGER PRIMARY KEY, created_at INTEGER, updated_at INTEGER, source_app TEXT, source_executable TEXT, source_window_title TEXT, content_type TEXT, preview_text TEXT, searchable_text TEXT, sanitized_html TEXT, fingerprint TEXT UNIQUE, pinned INTEGER, byte_size INTEGER);
             CREATE TABLE clipboard_representations (id INTEGER PRIMARY KEY, entry_id INTEGER, format TEXT, mime_type TEXT, inline_data BLOB, blob_hash TEXT, byte_size INTEGER);
             CREATE TABLE snippets (id INTEGER PRIMARY KEY, name TEXT, content TEXT, group_name TEXT, created_at INTEGER, updated_at INTEGER);",
        ).unwrap();
        let bytes = vec![4; INLINE_LIMIT + 1];
        let hash = hash_bytes(&bytes);
        fs::write(legacy.path().join("blobs").join(&hash), &bytes).unwrap();
        connection.execute("INSERT INTO clipboard_entries VALUES (1, 1, 2, 'Editor', NULL, NULL, 'image', 'preview', 'search', NULL, ?, 1, ?)", params![hash, bytes.len() as i64]).unwrap();
        connection.execute("INSERT INTO clipboard_representations VALUES (1, 1, 'image', 'image/bmp', NULL, ?, ?)", params![hash, bytes.len() as i64]).unwrap();
        connection.close().unwrap();
        let echo = TempDir::new().unwrap();
        let mut store = ClipboardStore::open(echo.path()).unwrap();
        let report = store.migrate_legacy(legacy.path()).unwrap();
        assert_eq!(report.entries, 1);
        assert_eq!(report.favorites, 1);
        assert_eq!(store.list_saved_items("", 10).unwrap().len(), 1);
        let again = store.migrate_legacy(legacy.path()).unwrap();
        assert!(again.already_migrated);
        assert_eq!(store.list_entries("", 10).unwrap().len(), 1);
        assert!(report.backup_dir.join("blobs").join(&hash).exists());
    }
}
