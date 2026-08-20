use std::any::Any;
use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use echo_engine::{
    is_text_like, normalize_name, normalize_tags, CapturePolicy, CaptureSettings, CapturedCapture,
    ClipboardRepresentation, ClipboardSink, ContentIdentity, LibraryPage, LibraryStore,
    NormalizedCapture, OperationMetric, OperationMetrics, PageCursor, PreviewAsset, RecordResult,
    RepresentationIdentity, SavedItemDraft, SavedItemUpdate, Thumbnail, DEFAULT_PAGE_SIZE,
    MAX_PAGE_SIZE, THUMBNAIL_MIME_TYPE,
};
use rusqlite::types::Value;
use rusqlite::{
    params, params_from_iter, Connection, OpenFlags, OptionalExtension, Row, Transaction,
};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

const INLINE_LIMIT: usize = 64 * 1024;
const DEFAULT_MAX_ENTRIES: u32 = 5_000;
const DEFAULT_MAX_TOTAL_BYTES: u64 = 512 * 1024 * 1024;
const DEFAULT_MAX_ITEM_BYTES: u64 = 32 * 1024 * 1024;
const SEARCH_FTS_SCHEMA_KEY: &str = "search_fts_schema";
const CURRENT_SCHEMA_VERSION: i32 = 4;

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

pub use echo_engine::{ClipboardSettings, HistoryEntry as ClipboardEntry, SavedItem};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRepresentation {
    pub id: i64,
    pub entry_id: i64,
    pub format: String,
    pub mime_type: String,
    pub inline_data: Option<Vec<u8>>,
    pub blob_hash: Option<String>,
    pub content_hash: Option<String>,
    pub byte_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredThumbnail {
    pub metadata: Thumbnail,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredClipboardEntry {
    pub entry: ClipboardEntry,
    pub representations: Vec<StoredRepresentation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredSavedItem {
    pub item: SavedItem,
    pub representations: Vec<StoredRepresentation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationReport {
    pub backup_dir: PathBuf,
    pub entries: usize,
    pub representations: usize,
    pub blobs: usize,
    pub favorites: usize,
    pub already_migrated: bool,
}

#[derive(Debug, Clone)]
struct PreparedRepresentation {
    format: String,
    mime_type: String,
    inline_data: Option<Vec<u8>>,
    blob_hash: Option<String>,
    content_hash: String,
    byte_size: u64,
}

pub struct ClipboardStore {
    connection: Connection,
    data_dir: PathBuf,
    blobs_dir: PathBuf,
    thumbnails_dir: PathBuf,
    metrics: Arc<OperationMetrics>,
}

impl ClipboardStore {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_metrics(data_dir.as_ref(), Arc::new(OperationMetrics::default()))
    }

    fn open_with_metrics(data_dir: &Path, metrics: Arc<OperationMetrics>) -> Result<Self> {
        let data_dir = data_dir.to_path_buf();
        fs::create_dir_all(&data_dir)?;
        let blobs_dir = data_dir.join("blobs");
        fs::create_dir_all(&blobs_dir)?;
        let thumbnails_dir = data_dir.join("thumbnails");
        fs::create_dir_all(&thumbnails_dir)?;
        let database = data_dir.join("echo.sqlite3");
        let connection = Connection::open(database)?;
        let mut store = Self {
            connection,
            data_dir,
            blobs_dir,
            thumbnails_dir,
            metrics,
        };
        store.configure()?;
        store.ensure_schema()?;
        Ok(store)
    }

    pub fn open_in_memory(data_dir: impl AsRef<Path>) -> Result<Self> {
        Self::open_in_memory_with_metrics(data_dir.as_ref(), Arc::new(OperationMetrics::default()))
    }

    fn open_in_memory_with_metrics(
        data_dir: &Path,
        metrics: Arc<OperationMetrics>,
    ) -> Result<Self> {
        let data_dir = data_dir.to_path_buf();
        fs::create_dir_all(data_dir.join("blobs"))?;
        fs::create_dir_all(data_dir.join("thumbnails"))?;
        let connection = Connection::open_in_memory()?;
        let mut store = Self {
            connection,
            blobs_dir: data_dir.join("blobs"),
            thumbnails_dir: data_dir.join("thumbnails"),
            data_dir,
            metrics,
        };
        store.configure()?;
        store.ensure_schema()?;
        Ok(store)
    }

    fn open_read_only(data_dir: &Path, metrics: Arc<OperationMetrics>) -> Result<Self> {
        let data_dir = data_dir.to_path_buf();
        let connection = Connection::open_with_flags(
            data_dir.join("echo.sqlite3"),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )?;
        let store = Self {
            connection,
            blobs_dir: data_dir.join("blobs"),
            thumbnails_dir: data_dir.join("thumbnails"),
            data_dir,
            metrics,
        };
        store.configure_read_only()?;
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

    fn configure_read_only(&self) -> Result<()> {
        self.connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;
             PRAGMA query_only = ON;",
        )?;
        Ok(())
    }

    fn ensure_column(&self, table: &str, column: &str, definition: &str) -> Result<()> {
        if !has_column(&self.connection, table, column)? {
            self.connection.execute_batch(&format!(
                "ALTER TABLE {table} ADD COLUMN {column} {definition}"
            ))?;
        }
        Ok(())
    }

    fn ensure_schema(&mut self) -> Result<()> {
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS migration_state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                completed_at INTEGER NOT NULL
            );",
        )?;
        let version: i32 = self
            .connection
            .query_row("PRAGMA user_version", [], |row| row.get(0))?;
        if version > CURRENT_SCHEMA_VERSION {
            return Err(StorageError::Migration(format!(
                "database schema version {version} is newer than Echo supports"
            )));
        }
        for next in (version + 1)..=CURRENT_SCHEMA_VERSION {
            match next {
                1 => self.migrate_schema_v1()?,
                2 => self.migrate_schema_v2()?,
                3 => self.migrate_schema_v3()?,
                4 => self.ensure_search_schema()?,
                _ => unreachable!("schema version is bounded above"),
            }
            self.connection
                .execute_batch(&format!("PRAGMA user_version = {next};"))?;
        }
        Ok(())
    }

    fn migrate_schema_v1(&mut self) -> Result<()> {
        self.ensure_base_objects()?;
        if table_exists_connection(&self.connection, "saved_insert_items")? {
            self.migrate_legacy_saved_tables()?;
        } else if has_column(&self.connection, "clipboard_entries", "pinned")?
            && self.count_table("saved_items")? == 0
        {
            self.migrate_pinned_saved_items()?;
        }
        Ok(())
    }

    fn migrate_legacy_saved_tables(&mut self) -> Result<()> {
        let items = {
            let mut statement = self.connection.prepare(
                "SELECT id, source_entry_id, created_at, updated_at, source_app,
                        source_executable, source_window_title, content_type, preview_text,
                        searchable_text, byte_size
                 FROM saved_insert_items ORDER BY id",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, i64>(10)?,
                ))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        let representations = {
            let mut statement = self.connection.prepare(
                "SELECT id, saved_item_id, format, mime_type, inline_data, blob_hash, byte_size
                 FROM saved_insert_representations ORDER BY id",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<Vec<u8>>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        let tx = self.connection.transaction()?;
        for (
            id,
            source_id,
            created,
            updated,
            app,
            executable,
            title,
            content_type,
            preview,
            searchable,
            byte_size,
        ) in &items
        {
            tx.execute(
                "INSERT OR IGNORE INTO saved_items
                 (id, source_history_id, created_at, updated_at, name, content_type, editable_text,
                  source_app, source_executable, source_window_title, preview_text, byte_size, is_independent)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0)",
                params![
                    id,
                    source_id,
                    created,
                    updated,
                    legacy_saved_name(content_type, preview.as_deref()),
                    content_type,
                    is_text_like(content_type)
                        .then(|| searchable.clone().or_else(|| preview.clone()).unwrap_or_default()),
                    app,
                    executable,
                    title,
                    preview,
                    byte_size,
                ],
            )?;
            replace_tags_tx(&tx, *id, &[])?;
        }
        for (id, saved_id, format, mime, inline_data, blob_hash, byte_size) in representations {
            tx.execute(
                "INSERT OR IGNORE INTO saved_item_representations
                 (id, saved_item_id, format, mime_type, inline_data, blob_hash, byte_size)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
                params![
                    id,
                    saved_id,
                    format,
                    mime,
                    inline_data,
                    blob_hash,
                    byte_size
                ],
            )?;
            if let Some(hash) = blob_hash {
                tx.execute(
                    "INSERT OR IGNORE INTO clipboard_blobs (hash, mime_type, byte_size)
                     VALUES (?, ?, ?)",
                    params![hash, mime, byte_size],
                )?;
            }
            refresh_saved_search_tx(&tx, saved_id)?;
        }
        for (id, ..) in &items {
            refresh_saved_search_tx(&tx, *id)?;
        }
        tx.execute_batch(
            "DROP TABLE saved_insert_representations;
             DROP TABLE saved_insert_items;",
        )?;
        tx.commit()?;
        Ok(())
    }

    fn migrate_pinned_saved_items(&mut self) -> Result<()> {
        let entries = {
            let mut statement = self.connection.prepare(
                "SELECT id, created_at, updated_at, source_app, source_executable,
                        source_window_title, content_type, preview_text, searchable_text, byte_size
                 FROM clipboard_entries WHERE pinned = 1 ORDER BY id",
            )?;
            let rows = statement.query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, i64>(9)?,
                ))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        let tx = self.connection.transaction()?;
        for (
            entry_id,
            created,
            updated,
            app,
            executable,
            title,
            content_type,
            preview,
            searchable,
            byte_size,
        ) in entries
        {
            tx.execute(
                "INSERT OR IGNORE INTO saved_items
                 (source_history_id, created_at, updated_at, name, content_type, editable_text,
                  source_app, source_executable, source_window_title, preview_text, byte_size, is_independent)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0)",
                params![
                    entry_id,
                    created,
                    updated,
                    legacy_saved_name(&content_type, preview.as_deref()),
                    content_type,
                    is_text_like(&content_type)
                        .then(|| searchable.clone().or_else(|| preview.clone()).unwrap_or_default()),
                    app,
                    executable,
                    title,
                    preview,
                    byte_size,
                ],
            )?;
            let saved_id = tx.last_insert_rowid();
            let representations = {
                let mut statement = tx.prepare(
                    "SELECT format, mime_type, inline_data, blob_hash, byte_size
                     FROM clipboard_representations WHERE entry_id = ? ORDER BY id",
                )?;
                let rows = statement.query_map([entry_id], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<Vec<u8>>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                })?;
                rows.collect::<std::result::Result<Vec<_>, _>>()?
            };
            for (format, mime, inline_data, blob_hash, size) in representations {
                tx.execute(
                    "INSERT INTO saved_item_representations
                     (saved_item_id, format, mime_type, inline_data, blob_hash, byte_size)
                     VALUES (?, ?, ?, ?, ?, ?)",
                    params![saved_id, format, mime, inline_data, blob_hash, size],
                )?;
                if let Some(hash) = blob_hash {
                    tx.execute(
                        "INSERT OR IGNORE INTO clipboard_blobs (hash, mime_type, byte_size)
                         VALUES (?, ?, ?)",
                        params![hash, mime, size],
                    )?;
                }
            }
            replace_tags_tx(&tx, saved_id, &[])?;
            refresh_saved_search_tx(&tx, saved_id)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn migrate_schema_v2(&self) -> Result<()> {
        self.ensure_column("clipboard_representations", "content_hash", "TEXT")?;
        self.ensure_column("saved_item_representations", "content_hash", "TEXT")?;
        Ok(())
    }

    fn migrate_schema_v3(&self) -> Result<()> {
        self.connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS preview_assets (
                source_hash TEXT PRIMARY KEY,
                thumbnail_hash TEXT NOT NULL,
                mime_type TEXT NOT NULL,
                width INTEGER NOT NULL,
                height INTEGER NOT NULL,
                byte_size INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS preview_assets_thumbnail_idx
                ON preview_assets(thumbnail_hash);",
        )?;
        Ok(())
    }

    fn ensure_base_objects(&mut self) -> Result<()> {
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
            CREATE TABLE IF NOT EXISTS clipboard_fts (
                entry_id TEXT NOT NULL,
                searchable_text TEXT NOT NULL,
                source_app TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS saved_items (
                id INTEGER PRIMARY KEY,
                source_history_id INTEGER REFERENCES clipboard_entries(id) ON DELETE SET NULL,
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
            CREATE UNIQUE INDEX IF NOT EXISTS saved_items_source_history_idx
                ON saved_items(source_history_id) WHERE source_history_id IS NOT NULL;
            CREATE TABLE IF NOT EXISTS saved_item_representations (
                id INTEGER PRIMARY KEY,
                saved_item_id INTEGER NOT NULL REFERENCES saved_items(id) ON DELETE CASCADE,
                format TEXT NOT NULL,
                mime_type TEXT NOT NULL,
                inline_data BLOB,
                blob_hash TEXT,
                byte_size INTEGER NOT NULL,
                CHECK ((inline_data IS NULL) != (blob_hash IS NULL))
            );
            CREATE INDEX IF NOT EXISTS saved_item_representations_item_idx
                ON saved_item_representations(saved_item_id);
            CREATE INDEX IF NOT EXISTS saved_item_representations_blob_idx
                ON saved_item_representations(blob_hash);
            CREATE TABLE IF NOT EXISTS tags (
                id INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                normalized_name TEXT NOT NULL UNIQUE
            );
            CREATE TABLE IF NOT EXISTS saved_item_tags (
                saved_item_id INTEGER NOT NULL REFERENCES saved_items(id) ON DELETE CASCADE,
                tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE RESTRICT,
                PRIMARY KEY (saved_item_id, tag_id)
            );
            CREATE INDEX IF NOT EXISTS saved_item_tags_tag_idx ON saved_item_tags(tag_id);
            CREATE TABLE IF NOT EXISTS saved_items_fts (
                saved_item_id INTEGER PRIMARY KEY,
                document TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS migration_state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL,
                completed_at INTEGER NOT NULL
            );",
        )?;
        Ok(())
    }

    fn ensure_search_schema(&mut self) -> Result<()> {
        let version = self
            .connection
            .query_row(
                "SELECT value FROM migration_state WHERE key = ?",
                [SEARCH_FTS_SCHEMA_KEY],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        let ready = version
            .as_deref()
            .is_some_and(|value| value.starts_with("2:"))
            && is_fts5_table(&self.connection, "clipboard_fts")?
            && is_fts5_table(&self.connection, "saved_items_fts")?;
        if ready {
            return Ok(());
        }

        self.connection.execute_batch(
            "DROP TABLE IF EXISTS clipboard_fts;
             DROP TABLE IF EXISTS saved_items_fts;",
        )?;
        let tokenizer = if self.create_search_tables("trigram").is_ok() {
            "trigram"
        } else {
            self.connection.execute_batch(
                "DROP TABLE IF EXISTS clipboard_fts;
                 DROP TABLE IF EXISTS saved_items_fts;",
            )?;
            self.create_search_tables("unicode61")?;
            "unicode61"
        };
        self.rebuild_fts()?;
        self.rebuild_saved_search()?;
        self.connection.execute(
            "INSERT INTO migration_state (key, value, completed_at) VALUES (?, ?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, completed_at = excluded.completed_at",
            params![SEARCH_FTS_SCHEMA_KEY, format!("2:{tokenizer}"), now_millis()],
        )?;
        Ok(())
    }

    fn create_search_tables(&self, tokenizer: &str) -> Result<()> {
        self.connection.execute_batch(&format!(
            "CREATE VIRTUAL TABLE clipboard_fts USING fts5(
                entry_id UNINDEXED,
                searchable_text,
                source_app,
                tokenize = '{tokenizer}'
            );
            CREATE VIRTUAL TABLE saved_items_fts USING fts5(
                saved_item_id UNINDEXED,
                document,
                tokenize = '{tokenizer}'
            );"
        ))?;
        Ok(())
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn metrics_snapshot(&self) -> Vec<OperationMetric> {
        self.metrics.snapshot()
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
                        max_entries: row
                            .get::<_, i64>(3)?
                            .try_into()
                            .unwrap_or(DEFAULT_MAX_ENTRIES),
                        max_total_bytes: row
                            .get::<_, i64>(4)?
                            .try_into()
                            .unwrap_or(DEFAULT_MAX_TOTAL_BYTES),
                        max_item_bytes: row
                            .get::<_, i64>(5)?
                            .try_into()
                            .unwrap_or(DEFAULT_MAX_ITEM_BYTES),
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
        let started = Instant::now();
        let captured = CapturedCapture::from_capture(capture).prepare_preview();
        self.metrics.record(
            "thumbnail_generation",
            started.elapsed(),
            u64::from(captured.preview.is_some()),
        );
        self.record_captured(captured)
    }

    pub fn record_captured(&mut self, captured: CapturedCapture) -> Result<RecordResult> {
        self.record_capture_with_identity_and_preview(
            captured.capture,
            captured.identity,
            captured.preview,
        )
    }

    pub fn record_capture_with_identity(
        &mut self,
        capture: NormalizedCapture,
        identity: ContentIdentity,
    ) -> Result<RecordResult> {
        self.record_capture_with_identity_and_preview(capture, identity, None)
    }

    fn record_capture_with_identity_and_preview(
        &mut self,
        mut capture: NormalizedCapture,
        identity: ContentIdentity,
        preview: Option<PreviewAsset>,
    ) -> Result<RecordResult> {
        let settings = self.settings()?;
        if identity.representations.len() != capture.representations.len() {
            return Err(StorageError::Invalid(
                "content identity does not match clipboard representations".to_owned(),
            ));
        }
        capture.fingerprint = identity.fingerprint.clone();
        let byte_size = identity.total_byte_size;
        if byte_size > settings.max_item_bytes {
            return Err(StorageError::Invalid(format!(
                "clipboard item exceeds max_item_bytes ({byte_size} > {})",
                settings.max_item_bytes
            )));
        }
        let now = now_millis();
        let dedupe_started = Instant::now();
        let existing = self
            .connection
            .query_row(
                "SELECT id FROM clipboard_entries WHERE fingerprint = ?",
                [&capture.fingerprint],
                |row| row.get::<_, i64>(0),
            )
            .optional()?;
        self.metrics.record(
            "dedupe",
            dedupe_started.elapsed(),
            u64::from(existing.is_some()),
        );
        let prepared = if existing.is_none() {
            capture
                .representations
                .iter()
                .zip(identity.representations.iter())
                .map(|(representation, identity)| {
                    self.prepare_representation(representation, Some(identity))
                })
                .collect::<Result<Vec<_>>>()?
        } else {
            Vec::new()
        };
        let preview = preview.and_then(|asset| {
            (asset.thumbnail.mime_type == THUMBNAIL_MIME_TYPE
                && capture
                    .representations
                    .iter()
                    .zip(identity.representations.iter())
                    .any(|(representation, identity)| {
                        (representation.format == "image"
                            || representation.mime_type.starts_with("image/"))
                            && identity.hash == asset.thumbnail.source_hash
                    }))
            .then(|| self.persist_thumbnail(&asset).ok().map(|_| asset.thumbnail))
            .flatten()
        });
        let tx = self.connection.transaction()?;
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
            Self::refresh_fts_tx(
                &tx,
                id,
                capture.searchable_text.as_deref(),
                capture.source.app_name.as_deref(),
            )?;
            (id, true)
        } else {
            tx.execute(
                "INSERT INTO clipboard_entries
                 (created_at, updated_at, source_app, source_executable, source_window_title,
                   content_type, preview_text, searchable_text, sanitized_html, fingerprint,
                   byte_size)
                  VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
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
            Self::refresh_fts_tx(
                &tx,
                id,
                capture.searchable_text.as_deref(),
                capture.source.app_name.as_deref(),
            )?;
            (id, false)
        };
        if let Some(thumbnail) = &preview {
            let exists: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM preview_assets WHERE source_hash = ?)",
                [&thumbnail.source_hash],
                |row| row.get::<_, i64>(0),
            )? != 0;
            if !exists {
                Self::insert_thumbnail_tx(&tx, thumbnail)?;
            }
        }
        let evicted = Self::enforce_capacity_tx(&tx, &settings)?;
        let commit_started = Instant::now();
        tx.commit()?;
        self.metrics
            .record("db_commit", commit_started.elapsed(), 1);
        if evicted {
            self.schedule_blob_gc()?;
        }
        Ok(RecordResult { id, duplicate })
    }

    fn prepare_representation(
        &self,
        representation: &ClipboardRepresentation,
        identity: Option<&RepresentationIdentity>,
    ) -> Result<PreparedRepresentation> {
        let byte_size = representation.bytes.len() as u64;
        let content_hash = identity
            .map(|identity| identity.hash.clone())
            .unwrap_or_else(|| hash_bytes(&representation.bytes));
        if representation.bytes.len() <= INLINE_LIMIT {
            return Ok(PreparedRepresentation {
                format: representation.format.clone(),
                mime_type: representation.mime_type.clone(),
                inline_data: Some(representation.bytes.clone()),
                blob_hash: None,
                content_hash,
                byte_size,
            });
        }
        let hash = content_hash.clone();
        self.write_blob(&hash, &representation.mime_type, &representation.bytes)?;
        Ok(PreparedRepresentation {
            format: representation.format.clone(),
            mime_type: representation.mime_type.clone(),
            inline_data: None,
            blob_hash: Some(hash),
            content_hash,
            byte_size,
        })
    }

    fn write_blob(&self, hash: &str, mime_type: &str, bytes: &[u8]) -> Result<()> {
        let started = Instant::now();
        let result = self.write_blob_inner(hash, mime_type, bytes);
        self.metrics
            .record("blob_write", started.elapsed(), u64::from(result.is_ok()));
        result
    }

    fn write_blob_inner(&self, hash: &str, mime_type: &str, bytes: &[u8]) -> Result<()> {
        validate_hash(hash)?;
        let path = self.blobs_dir.join(hash);
        if path.exists() {
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
                let _ = fs::remove_file(&temporary);
            } else {
                return Err(StorageError::Io(error));
            }
        }
        let _ = mime_type;
        Ok(())
    }

    fn persist_thumbnail(&self, asset: &PreviewAsset) -> Result<()> {
        let started = Instant::now();
        let result = self.persist_thumbnail_inner(asset);
        self.metrics.record(
            "thumbnail_write",
            started.elapsed(),
            u64::from(result.is_ok()),
        );
        result
    }

    fn persist_thumbnail_inner(&self, asset: &PreviewAsset) -> Result<()> {
        let metadata = &asset.thumbnail;
        validate_hash(&metadata.source_hash)?;
        validate_hash(&metadata.content_hash)?;
        if metadata.mime_type != THUMBNAIL_MIME_TYPE
            || metadata.width == 0
            || metadata.height == 0
            || metadata.width > 256
            || metadata.height > 256
            || metadata.byte_size != asset.bytes.len() as u64
            || hash_bytes(&asset.bytes) != metadata.content_hash
        {
            return Err(StorageError::Invalid(
                "thumbnail metadata does not match its bytes".to_owned(),
            ));
        }
        let path = self.thumbnails_dir.join(&metadata.content_hash);
        if path.exists() {
            if fs::read(&path)
                .ok()
                .is_some_and(|bytes| hash_bytes(&bytes) == metadata.content_hash)
            {
                return Ok(());
            }
            let _ = fs::remove_file(&path);
        }
        let temporary =
            self.thumbnails_dir
                .join(format!(".{}.{}.tmp", metadata.content_hash, Uuid::new_v4()));
        {
            let mut file = File::create(&temporary)?;
            file.write_all(&asset.bytes)?;
            file.sync_all()?;
        }
        if let Err(error) = fs::rename(&temporary, &path) {
            if path.exists() {
                let _ = fs::remove_file(&temporary);
            } else {
                return Err(StorageError::Io(error));
            }
        }
        Ok(())
    }

    fn insert_thumbnail_tx(tx: &Transaction<'_>, metadata: &Thumbnail) -> Result<()> {
        tx.execute(
            "INSERT OR IGNORE INTO preview_assets
             (source_hash, thumbnail_hash, mime_type, width, height, byte_size)
             VALUES (?, ?, ?, ?, ?, ?)",
            params![
                metadata.source_hash,
                metadata.content_hash,
                metadata.mime_type,
                i64::from(metadata.width),
                i64::from(metadata.height),
                i64::try_from(metadata.byte_size).unwrap_or(i64::MAX),
            ],
        )?;
        Ok(())
    }

    fn insert_prepared_representation_tx(
        tx: &Transaction<'_>,
        entry_id: i64,
        representation: &PreparedRepresentation,
    ) -> Result<()> {
        tx.execute(
            "INSERT INTO clipboard_representations
             (entry_id, format, mime_type, inline_data, blob_hash, content_hash, byte_size)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![
                entry_id,
                representation.format,
                representation.mime_type,
                representation.inline_data,
                representation.blob_hash,
                representation.content_hash,
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

    fn insert_saved_representation_tx(
        tx: &Transaction<'_>,
        saved_item_id: i64,
        representation: &PreparedRepresentation,
    ) -> Result<()> {
        tx.execute(
            "INSERT INTO saved_item_representations
             (saved_item_id, format, mime_type, inline_data, blob_hash, content_hash, byte_size)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![
                saved_item_id,
                representation.format,
                representation.mime_type,
                representation.inline_data,
                representation.blob_hash,
                representation.content_hash,
                i64::try_from(representation.byte_size).unwrap_or(i64::MAX),
            ],
        )?;
        if let Some(hash) = &representation.blob_hash {
            tx.execute(
                "INSERT OR IGNORE INTO clipboard_blobs (hash, mime_type, byte_size) VALUES (?, ?, ?)",
                params![
                    hash,
                    representation.mime_type,
                    i64::try_from(representation.byte_size).unwrap_or(i64::MAX)
                ],
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
        tx.execute(
            "DELETE FROM clipboard_fts WHERE entry_id = ?",
            [id.to_string()],
        )?;
        tx.execute(
            "INSERT INTO clipboard_fts (entry_id, searchable_text, source_app) VALUES (?, ?, ?)",
            params![
                id.to_string(),
                searchable_text.unwrap_or_default(),
                source_app.unwrap_or_default()
            ],
        )?;
        Ok(())
    }

    fn enforce_capacity_tx(tx: &Transaction<'_>, settings: &ClipboardSettings) -> Result<bool> {
        let mut evicted = false;
        loop {
            let (count, total): (i64, i64) = tx.query_row(
                "SELECT COUNT(*), COALESCE(SUM(byte_size), 0) FROM clipboard_entries",
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
                    "SELECT id FROM clipboard_entries ORDER BY updated_at ASC, id ASC LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(oldest) = oldest else { break };
            tx.execute(
                "DELETE FROM clipboard_fts WHERE entry_id = ?",
                [oldest.to_string()],
            )?;
            tx.execute("DELETE FROM clipboard_entries WHERE id = ?", [oldest])?;
            evicted = true;
        }
        Ok(evicted)
    }

    pub fn list_entries(&self, query: &str, limit: u32) -> Result<Vec<ClipboardEntry>> {
        Ok(self.list_entries_page(query, limit, None)?.items)
    }

    pub fn list_entries_page(
        &self,
        query: &str,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> Result<LibraryPage<ClipboardEntry>> {
        let query_started = Instant::now();
        let page_size = if limit == 0 {
            DEFAULT_PAGE_SIZE
        } else {
            limit.clamp(1, MAX_PAGE_SIZE)
        };
        let fetch_limit = i64::from(page_size) + 1;
        let mut entries = Vec::new();
        let trimmed = query.trim();
        let (cursor_clause, cursor_values) = cursor_filter("e", cursor);
        if trimmed.is_empty() {
            let sql = format!(
                "SELECT e.id, e.created_at, e.updated_at, e.source_app, e.source_executable,
                        e.source_window_title, e.content_type, e.preview_text, e.searchable_text,
                        e.sanitized_html, e.fingerprint,
                        (SELECT id FROM saved_items WHERE source_history_id = e.id),
                        e.byte_size
                 FROM clipboard_entries e
                 WHERE 1 = 1 {cursor_clause}
                 ORDER BY e.updated_at DESC, e.id DESC LIMIT ?"
            );
            let mut values = cursor_values;
            values.push(Value::Integer(fetch_limit));
            let mut statement = self.connection.prepare(&sql)?;
            let rows = statement.query_map(params_from_iter(values), map_entry)?;
            for row in rows {
                entries.push(row?);
            }
        } else if trimmed.chars().count() < 3 {
            let pattern = format!("%{}%", escape_like_pattern(&trimmed.to_lowercase()));
            let sql = format!(
                "SELECT e.id, e.created_at, e.updated_at, e.source_app, e.source_executable,
                        e.source_window_title, e.content_type, e.preview_text, e.searchable_text,
                        e.sanitized_html, e.fingerprint, s.id, e.byte_size
                 FROM clipboard_entries e
                 LEFT JOIN saved_items s ON s.source_history_id = e.id
                 WHERE (lower(COALESCE(e.searchable_text, '')) LIKE ? ESCAPE '\\'
                    OR lower(COALESCE(e.source_app, '')) LIKE ? ESCAPE '\\'
                    OR lower(COALESCE(e.preview_text, '')) LIKE ? ESCAPE '\\')
                   {cursor_clause}
                 ORDER BY e.updated_at DESC, e.id DESC LIMIT ?"
            );
            let mut values = vec![
                Value::Text(pattern.clone()),
                Value::Text(pattern.clone()),
                Value::Text(pattern),
            ];
            values.extend(cursor_values);
            values.push(Value::Integer(fetch_limit));
            let mut statement = self.connection.prepare(&sql)?;
            let rows = statement.query_map(params_from_iter(values), map_entry)?;
            for row in rows {
                entries.push(row?);
            }
        } else {
            let sql = format!(
                "SELECT e.id, e.created_at, e.updated_at, e.source_app, e.source_executable,
                        e.source_window_title, e.content_type, e.preview_text, e.searchable_text,
                        e.sanitized_html, e.fingerprint, s.id, e.byte_size
                 FROM clipboard_entries e
                 JOIN clipboard_fts f ON CAST(f.entry_id AS INTEGER) = e.id
                 LEFT JOIN saved_items s ON s.source_history_id = e.id
                 WHERE clipboard_fts MATCH ? {cursor_clause}
                 ORDER BY e.updated_at DESC, e.id DESC LIMIT ?"
            );
            let mut values = vec![Value::Text(fts_match_query(trimmed))];
            values.extend(cursor_values);
            values.push(Value::Integer(fetch_limit));
            let mut statement = self.connection.prepare(&sql)?;
            let rows = statement.query_map(params_from_iter(values), map_entry)?;
            for row in rows {
                entries.push(row?);
            }
        }
        for entry in &mut entries {
            entry.thumbnail = self.thumbnail_for_entry(entry.id)?;
        }
        let has_more = entries.len() > usize::try_from(page_size).unwrap_or(usize::MAX);
        entries.truncate(usize::try_from(page_size).unwrap_or(usize::MAX));
        let next_cursor = has_more
            .then(|| {
                entries.last().map(|entry| PageCursor {
                    updated_at: entry.updated_at,
                    id: entry.id,
                })
            })
            .flatten();
        self.metrics.record(
            "history_query",
            query_started.elapsed(),
            entries.len() as u64,
        );
        Ok(LibraryPage {
            items: entries,
            next_cursor,
        })
    }

    pub fn entry(&self, id: i64) -> Result<Option<StoredClipboardEntry>> {
        let entry = self
            .connection
            .query_row(
                "SELECT id, created_at, updated_at, source_app, source_executable,
                        source_window_title, content_type, preview_text, searchable_text,
                        sanitized_html, fingerprint,
                        (SELECT id FROM saved_items WHERE source_history_id = clipboard_entries.id),
                        byte_size
                 FROM clipboard_entries WHERE id = ?",
                [id],
                map_entry,
            )
            .optional()?;
        let Some(mut entry) = entry else {
            return Ok(None);
        };
        entry.thumbnail = self.thumbnail_for_entry(id)?;
        Ok(Some(StoredClipboardEntry {
            representations: self.entry_representations(id)?,
            entry,
        }))
    }

    fn entry_representations(&self, id: i64) -> Result<Vec<StoredRepresentation>> {
        let mut statement = self.connection.prepare(
            "SELECT id, entry_id, format, mime_type, inline_data, blob_hash, content_hash, byte_size
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
            return Err(StorageError::Invalid(
                "representation has no data".to_owned(),
            ));
        };
        Ok(ClipboardRepresentation {
            format: representation.format.clone(),
            mime_type: representation.mime_type.clone(),
            bytes,
        })
    }

    fn thumbnail_for_entry(&self, entry_id: i64) -> Result<Option<Thumbnail>> {
        self.thumbnail_for_representation("clipboard_representations", "entry_id", entry_id)
    }

    fn thumbnail_for_saved_item(&self, saved_item_id: i64) -> Result<Option<Thumbnail>> {
        self.thumbnail_for_representation(
            "saved_item_representations",
            "saved_item_id",
            saved_item_id,
        )
    }

    fn thumbnail_for_representation(
        &self,
        table: &str,
        owner_column: &str,
        owner_id: i64,
    ) -> Result<Option<Thumbnail>> {
        let sql = format!(
            "SELECT p.source_hash, p.thumbnail_hash, p.mime_type, p.width, p.height, p.byte_size
             FROM {table} r
             JOIN preview_assets p ON p.source_hash = COALESCE(r.content_hash, r.blob_hash)
             WHERE r.{owner_column} = ?
               AND (r.format = 'image' OR r.mime_type LIKE 'image/%')
             LIMIT 1"
        );
        self.connection
            .query_row(&sql, [owner_id], map_thumbnail)
            .optional()
            .map_err(StorageError::from)
    }

    pub fn read_thumbnail(&self, content_hash: &str) -> Result<Option<StoredThumbnail>> {
        let started = Instant::now();
        let result = self.read_thumbnail_inner(content_hash);
        let count = result
            .as_ref()
            .ok()
            .and_then(Option::as_ref)
            .map(|thumbnail| thumbnail.bytes.len() as u64)
            .unwrap_or(0);
        self.metrics
            .record("preview_open", started.elapsed(), count);
        result
    }

    fn read_thumbnail_inner(&self, content_hash: &str) -> Result<Option<StoredThumbnail>> {
        validate_hash(content_hash)?;
        let metadata = self
            .connection
            .query_row(
                "SELECT source_hash, thumbnail_hash, mime_type, width, height, byte_size
                 FROM preview_assets WHERE thumbnail_hash = ?",
                [content_hash],
                map_thumbnail,
            )
            .optional()?;
        let Some(metadata) = metadata else {
            return Ok(None);
        };
        let path = self.thumbnails_dir.join(content_hash);
        let bytes = match fs::read(path) {
            Ok(bytes) => bytes,
            Err(_) => return Ok(None),
        };
        if metadata.mime_type != THUMBNAIL_MIME_TYPE
            || metadata.width == 0
            || metadata.height == 0
            || metadata.width > 256
            || metadata.height > 256
            || metadata.byte_size != bytes.len() as u64
            || hash_bytes(&bytes) != content_hash
        {
            return Ok(None);
        }
        Ok(Some(StoredThumbnail { metadata, bytes }))
    }

    pub fn save_history_item(
        &mut self,
        mut draft: SavedItemDraft,
        source_payload: Vec<ClipboardRepresentation>,
    ) -> Result<SavedItem> {
        draft.name = normalize_name(&draft.name)
            .map_err(|error| StorageError::Invalid(error.to_string()))?;
        draft.tags = normalize_tags(&draft.tags)
            .map_err(|error| StorageError::Invalid(error.to_string()))?;
        let payload = draft.canonical_payload(&source_payload);
        let prepared = payload
            .iter()
            .map(|representation| self.prepare_representation(representation, None))
            .collect::<Result<Vec<_>>>()?;
        let byte_size = payload
            .iter()
            .try_fold(0_u64, |total, representation| {
                total.checked_add(representation.bytes.len() as u64)
            })
            .ok_or_else(|| StorageError::Invalid("saved item byte size overflow".to_owned()))?;
        let tx = self.connection.transaction()?;
        let existing: Option<(i64, bool)> = tx
            .query_row(
                "SELECT id, is_independent FROM saved_items WHERE source_history_id = ?",
                [draft.source_history_id],
                |row| Ok((row.get(0)?, row.get::<_, i64>(1)? != 0)),
            )
            .optional()?;
        let saved_id = if let Some((id, true)) = existing {
            tx.commit()?;
            self.schedule_blob_gc()?;
            return self
                .saved_item(id)?
                .map(|stored| stored.item)
                .ok_or_else(|| StorageError::Invalid(format!("saved item {id} disappeared")));
        } else if let Some((id, false)) = existing {
            tx.execute(
                "UPDATE saved_items SET updated_at = ?, name = ?, content_type = ?, editable_text = ?,
                 source_app = ?, source_executable = ?, source_window_title = ?, preview_text = ?,
                 byte_size = ?, is_independent = 0 WHERE id = ?",
                params![
                    draft.updated_at,
                    draft.name,
                    draft.content_type,
                    draft.editable_text,
                    draft.source_app,
                    draft.source_executable,
                    draft.source_window_title,
                    draft.preview_text,
                    i64::try_from(byte_size).unwrap_or(i64::MAX),
                    id,
                ],
            )?;
            tx.execute(
                "DELETE FROM saved_item_representations WHERE saved_item_id = ?",
                [id],
            )?;
            for representation in &prepared {
                Self::insert_saved_representation_tx(&tx, id, representation)?;
            }
            replace_tags_tx(&tx, id, &draft.tags)?;
            refresh_saved_search_tx(&tx, id)?;
            id
        } else {
            tx.execute(
                "INSERT INTO saved_items
                 (source_history_id, created_at, updated_at, name, content_type, editable_text,
                  source_app, source_executable, source_window_title, preview_text, byte_size, is_independent)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0)",
                params![
                    draft.source_history_id,
                    draft.created_at,
                    draft.updated_at,
                    draft.name,
                    draft.content_type,
                    draft.editable_text,
                    draft.source_app,
                    draft.source_executable,
                    draft.source_window_title,
                    draft.preview_text,
                    i64::try_from(byte_size).unwrap_or(i64::MAX),
                ],
            )?;
            let id = tx.last_insert_rowid();
            for representation in &prepared {
                Self::insert_saved_representation_tx(&tx, id, representation)?;
            }
            replace_tags_tx(&tx, id, &draft.tags)?;
            refresh_saved_search_tx(&tx, id)?;
            id
        };
        tx.commit()?;
        self.schedule_blob_gc()?;
        self.saved_item(saved_id)?
            .map(|stored| stored.item)
            .ok_or_else(|| StorageError::Invalid(format!("saved item {saved_id} disappeared")))
    }

    pub fn unsave_history_item(&mut self, history_id: i64) -> Result<bool> {
        let tx = self.connection.transaction()?;
        let saved: Option<(i64, bool)> = tx
            .query_row(
                "SELECT id, is_independent FROM saved_items WHERE source_history_id = ?",
                [history_id],
                |row| Ok((row.get(0)?, row.get::<_, i64>(1)? != 0)),
            )
            .optional()?;
        let Some((id, independent)) = saved else {
            tx.commit()?;
            return Ok(false);
        };
        if independent {
            tx.execute(
                "UPDATE saved_items SET source_history_id = NULL, updated_at = ? WHERE id = ?",
                params![now_millis(), id],
            )?;
        } else {
            tx.execute("DELETE FROM saved_items_fts WHERE saved_item_id = ?", [id])?;
            tx.execute("DELETE FROM saved_items WHERE id = ?", [id])?;
        }
        tx.commit()?;
        self.schedule_blob_gc()?;
        Ok(true)
    }

    pub fn update_saved_item(&mut self, id: i64, mut update: SavedItemUpdate) -> Result<SavedItem> {
        update.name = normalize_name(&update.name)
            .map_err(|error| StorageError::Invalid(error.to_string()))?;
        update.tags = normalize_tags(&update.tags)
            .map_err(|error| StorageError::Invalid(error.to_string()))?;
        let current = self
            .saved_item(id)?
            .ok_or_else(|| StorageError::Invalid(format!("saved item {id} does not exist")))?;
        let text_payload = if is_text_like(&current.item.content_type) {
            update
                .editable_text
                .as_ref()
                .map(|text| ClipboardRepresentation {
                    format: "text".to_owned(),
                    mime_type: "text/plain;charset=utf-8".to_owned(),
                    bytes: text.as_bytes().to_vec(),
                })
        } else {
            None
        };
        let prepared = text_payload
            .as_ref()
            .map(|representation| self.prepare_representation(representation, None))
            .transpose()?;
        let byte_size = text_payload
            .as_ref()
            .map(|representation| representation.bytes.len() as u64)
            .unwrap_or(current.item.byte_size);
        let preview = update.editable_text.as_deref().map(preview_text);
        let tx = self.connection.transaction()?;
        let changed = tx.execute(
            "UPDATE saved_items SET updated_at = ?, name = ?, editable_text = CASE WHEN ? IS NULL THEN editable_text ELSE ? END,
             preview_text = CASE WHEN ? IS NULL THEN preview_text ELSE ? END, byte_size = ?, is_independent = 1
             WHERE id = ?",
            params![
                now_millis(),
                update.name,
                update.editable_text,
                update.editable_text,
                preview,
                preview,
                i64::try_from(byte_size).unwrap_or(i64::MAX),
                id,
            ],
        )?;
        if changed == 0 {
            return Err(StorageError::Invalid(format!(
                "saved item {id} does not exist"
            )));
        }
        if let Some(representation) = prepared {
            tx.execute(
                "DELETE FROM saved_item_representations WHERE saved_item_id = ?",
                [id],
            )?;
            Self::insert_saved_representation_tx(&tx, id, &representation)?;
        }
        replace_tags_tx(&tx, id, &update.tags)?;
        refresh_saved_search_tx(&tx, id)?;
        tx.commit()?;
        self.schedule_blob_gc()?;
        self.saved_item(id)?
            .map(|stored| stored.item)
            .ok_or_else(|| StorageError::Invalid(format!("saved item {id} disappeared")))
    }

    pub fn list_saved_items(&self, query: &str, limit: u32) -> Result<Vec<SavedItem>> {
        Ok(self.list_saved_items_page(query, limit, None)?.items)
    }

    pub fn list_saved_items_page(
        &self,
        query: &str,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> Result<LibraryPage<SavedItem>> {
        let query_started = Instant::now();
        let page_size = if limit == 0 {
            DEFAULT_PAGE_SIZE
        } else {
            limit.clamp(1, MAX_PAGE_SIZE)
        };
        let fetch_limit = i64::from(page_size) + 1;
        let trimmed = query.trim();
        let (cursor_clause, cursor_values) = cursor_filter("s", cursor);
        let select = "SELECT s.id, s.source_history_id, s.created_at, s.updated_at, s.name,
                    s.content_type, s.editable_text, s.source_app, s.source_executable,
                    s.source_window_title, s.preview_text, s.byte_size, s.is_independent
             FROM saved_items s";
        let mut items = Vec::new();
        if trimmed.is_empty() {
            let sql = format!(
                "{select}
                 WHERE 1 = 1 {cursor_clause}
                 ORDER BY s.updated_at DESC, s.id DESC LIMIT ?"
            );
            let mut values = cursor_values;
            values.push(Value::Integer(fetch_limit));
            let mut statement = self.connection.prepare(&sql)?;
            let rows = statement.query_map(params_from_iter(values), map_saved_item)?;
            for row in rows {
                items.push(row?);
            }
        } else if trimmed.chars().count() < 3 {
            let pattern = format!("%{}%", escape_like_pattern(&trimmed.to_lowercase()));
            let sql = format!(
                "{select}
                 JOIN saved_items_fts f ON CAST(f.saved_item_id AS INTEGER) = s.id
                 WHERE lower(f.document) LIKE ? ESCAPE '\\' {cursor_clause}
                 ORDER BY s.updated_at DESC, s.id DESC LIMIT ?"
            );
            let mut values = vec![Value::Text(pattern)];
            values.extend(cursor_values);
            values.push(Value::Integer(fetch_limit));
            let mut statement = self.connection.prepare(&sql)?;
            let rows = statement.query_map(params_from_iter(values), map_saved_item)?;
            for row in rows {
                items.push(row?);
            }
        } else {
            let sql = format!(
                "{select}
                 JOIN saved_items_fts f ON CAST(f.saved_item_id AS INTEGER) = s.id
                 WHERE saved_items_fts MATCH ? {cursor_clause}
                 ORDER BY s.updated_at DESC, s.id DESC LIMIT ?"
            );
            let mut values = vec![Value::Text(fts_match_query(trimmed))];
            values.extend(cursor_values);
            values.push(Value::Integer(fetch_limit));
            let mut statement = self.connection.prepare(&sql)?;
            let rows = statement.query_map(params_from_iter(values), map_saved_item)?;
            for row in rows {
                items.push(row?);
            }
        }
        for item in &mut items {
            item.tags = self.tags_for_item(item.id)?;
            item.thumbnail = self.thumbnail_for_saved_item(item.id)?;
        }
        let has_more = items.len() > usize::try_from(page_size).unwrap_or(usize::MAX);
        items.truncate(usize::try_from(page_size).unwrap_or(usize::MAX));
        let next_cursor = has_more
            .then(|| {
                items.last().map(|item| PageCursor {
                    updated_at: item.updated_at,
                    id: item.id,
                })
            })
            .flatten();
        self.metrics.record(
            "saved_item_query",
            query_started.elapsed(),
            items.len() as u64,
        );
        Ok(LibraryPage { items, next_cursor })
    }

    pub fn saved_item(&self, id: i64) -> Result<Option<StoredSavedItem>> {
        let item = self
            .connection
            .query_row(
                "SELECT id, source_history_id, created_at, updated_at, name, content_type,
                        editable_text, source_app, source_executable, source_window_title,
                        preview_text, byte_size, is_independent
                 FROM saved_items WHERE id = ?",
                [id],
                |row| {
                    Ok(SavedItem {
                        id: row.get(0)?,
                        source_history_id: row.get(1)?,
                        created_at: row.get(2)?,
                        updated_at: row.get(3)?,
                        name: row.get(4)?,
                        content_type: row.get(5)?,
                        editable_text: row.get(6)?,
                        source_app: row.get(7)?,
                        source_executable: row.get(8)?,
                        source_window_title: row.get(9)?,
                        preview_text: row.get(10)?,
                        byte_size: row.get::<_, i64>(11)?.try_into().unwrap_or(0),
                        tags: Vec::new(),
                        is_independent: row.get::<_, i64>(12)? != 0,
                        thumbnail: None,
                    })
                },
            )
            .optional()?;
        let Some(mut item) = item else {
            return Ok(None);
        };
        item.tags = self.tags_for_item(id)?;
        item.thumbnail = self.thumbnail_for_saved_item(id)?;
        let mut statement = self.connection.prepare(
            "SELECT id, saved_item_id, format, mime_type, inline_data, blob_hash, content_hash, byte_size
             FROM saved_item_representations WHERE saved_item_id = ? ORDER BY id",
        )?;
        let rows = statement.query_map([id], |row| {
            Ok(StoredRepresentation {
                id: row.get(0)?,
                entry_id: row.get(1)?,
                format: row.get(2)?,
                mime_type: row.get(3)?,
                inline_data: row.get(4)?,
                blob_hash: row.get(5)?,
                content_hash: row.get(6)?,
                byte_size: row.get::<_, i64>(7)?.try_into().unwrap_or(0),
            })
        })?;
        let representations = rows
            .map(|row| row.map_err(StorageError::from))
            .collect::<Result<Vec<_>>>()?;
        Ok(Some(StoredSavedItem {
            item,
            representations,
        }))
    }

    pub fn saved_item_payload(&self, id: i64) -> Result<Vec<ClipboardRepresentation>> {
        let stored = self
            .saved_item(id)?
            .ok_or_else(|| StorageError::Invalid(format!("saved item {id} does not exist")))?;
        stored
            .representations
            .iter()
            .map(|representation| self.resolve_representation(representation))
            .collect()
    }

    pub fn delete_entry(&mut self, id: i64) -> Result<bool> {
        let tx = self.connection.transaction()?;
        let deleted = tx.execute("DELETE FROM clipboard_entries WHERE id = ?", [id])? > 0;
        tx.execute(
            "DELETE FROM clipboard_fts WHERE entry_id = ?",
            [id.to_string()],
        )?;
        tx.commit()?;
        self.schedule_blob_gc()?;
        Ok(deleted)
    }

    pub fn delete_saved_items(&mut self, ids: &[i64]) -> Result<usize> {
        let ids = ids.iter().copied().filter(|id| *id > 0).collect::<Vec<_>>();
        if ids.is_empty() {
            return Ok(0);
        }
        let placeholders = std::iter::repeat("?")
            .take(ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("DELETE FROM saved_items WHERE id IN ({placeholders})");
        let tx = self.connection.transaction()?;
        let fts_sql =
            format!("DELETE FROM saved_items_fts WHERE saved_item_id IN ({placeholders})");
        tx.execute(&fts_sql, rusqlite::params_from_iter(ids.iter()))?;
        let deleted = tx.execute(&sql, rusqlite::params_from_iter(ids.iter()))?;
        tx.commit()?;
        self.schedule_blob_gc()?;
        Ok(deleted)
    }

    pub fn clear_history(&mut self) -> Result<()> {
        let tx = self.connection.transaction()?;
        tx.execute("DELETE FROM clipboard_fts", [])?;
        tx.execute("DELETE FROM clipboard_entries", [])?;
        tx.commit()?;
        self.schedule_blob_gc()?;
        Ok(())
    }

    fn tags_for_item(&self, id: i64) -> Result<Vec<String>> {
        let mut statement = self.connection.prepare(
            "SELECT t.name FROM tags t
             JOIN saved_item_tags sit ON sit.tag_id = t.id
             WHERE sit.saved_item_id = ? ORDER BY sit.rowid",
        )?;
        let rows = statement.query_map([id], |row| row.get::<_, String>(0))?;
        rows.map(|row| row.map_err(StorageError::from)).collect()
    }

    fn schedule_blob_gc(&self) -> Result<()> {
        self.connection.execute(
            "INSERT INTO migration_state (key, value, completed_at) VALUES ('blob_gc_pending', '1', ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value, completed_at = excluded.completed_at",
            [now_millis()],
        )?;
        Ok(())
    }

    fn maintenance_pending(&self) -> Result<bool> {
        Ok(self.connection.query_row(
            "SELECT EXISTS (SELECT 1 FROM migration_state WHERE key = 'blob_gc_pending')",
            [],
            |row| row.get::<_, i64>(0),
        )? != 0)
    }

    pub fn reconcile_blob_store(&mut self) -> Result<()> {
        let mut referenced = HashSet::new();
        {
            let mut statement = self.connection.prepare(
                "SELECT blob_hash FROM clipboard_representations WHERE blob_hash IS NOT NULL
                 UNION SELECT blob_hash FROM saved_item_representations WHERE blob_hash IS NOT NULL",
            )?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            for row in rows {
                referenced.insert(row?);
            }
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
        let database_hashes = {
            let mut statement = self
                .connection
                .prepare("SELECT hash FROM clipboard_blobs")?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        for hash in database_hashes {
            if !referenced.contains(&hash) {
                self.connection
                    .execute("DELETE FROM clipboard_blobs WHERE hash = ?", [&hash])?;
            }
        }
        if self.blobs_dir.exists() {
            for item in fs::read_dir(&self.blobs_dir)? {
                let item = item?;
                let path = item.path();
                let name = item.file_name().to_string_lossy().to_string();
                if name.starts_with('.') || name.ends_with(".tmp") || !referenced.contains(&name) {
                    if path.is_file() {
                        let _ = fs::remove_file(path);
                    }
                }
            }
        }
        self.reconcile_thumbnail_store()?;
        self.connection.execute(
            "DELETE FROM migration_state WHERE key = 'blob_gc_pending'",
            [],
        )?;
        Ok(())
    }

    fn reconcile_thumbnail_store(&mut self) -> Result<()> {
        let mut referenced_sources = HashSet::new();
        for table in ["clipboard_representations", "saved_item_representations"] {
            let sql = format!(
                "SELECT COALESCE(content_hash, blob_hash) FROM {table} WHERE COALESCE(content_hash, blob_hash) IS NOT NULL"
            );
            let mut statement = self.connection.prepare(&sql)?;
            let rows = statement.query_map([], |row| row.get::<_, String>(0))?;
            for row in rows {
                referenced_sources.insert(row?);
            }
        }
        let preview_rows = {
            let mut statement = self
                .connection
                .prepare("SELECT source_hash, thumbnail_hash FROM preview_assets")?;
            let rows = statement.query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        let mut referenced_thumbnails = HashSet::new();
        for (source_hash, thumbnail_hash) in preview_rows {
            if referenced_sources.contains(&source_hash) {
                referenced_thumbnails.insert(thumbnail_hash);
            } else {
                self.connection.execute(
                    "DELETE FROM preview_assets WHERE source_hash = ?",
                    [source_hash],
                )?;
            }
        }
        if self.thumbnails_dir.exists() {
            for item in fs::read_dir(&self.thumbnails_dir)? {
                let item = item?;
                let path = item.path();
                let name = item.file_name().to_string_lossy().to_string();
                if name.starts_with('.')
                    || name.ends_with(".tmp")
                    || !referenced_thumbnails.contains(&name)
                {
                    if path.is_file() {
                        let _ = fs::remove_file(path);
                    }
                }
            }
        }
        Ok(())
    }

    pub fn migrate_legacy(&mut self, legacy_dir: impl AsRef<Path>) -> Result<MigrationReport> {
        let legacy_dir = legacy_dir.as_ref();
        let legacy_db = legacy_dir.join("culsans.sqlite3");
        if !legacy_db.is_file() {
            return Err(StorageError::Migration(format!(
                "legacy database is missing: {}",
                legacy_db.display()
            )));
        }
        let marker_key = legacy_dir.canonicalize()?.to_string_lossy().to_string();
        if self
            .connection
            .query_row(
                "SELECT value FROM migration_state WHERE key = ?",
                [&marker_key],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .is_some()
        {
            return Ok(MigrationReport {
                backup_dir: self.data_dir.join("migration-backups"),
                entries: self.count_table("clipboard_entries")?,
                representations: self.count_table("clipboard_representations")?,
                blobs: self.count_table("clipboard_blobs")?,
                favorites: self.count_table("saved_items")?,
                already_migrated: true,
            });
        }
        let lock_path = legacy_dir.join(".echo-migration.lock");
        let _lock = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&lock_path)
            .map_err(|error| {
                StorageError::Migration(format!("cannot acquire legacy migration lock: {error}"))
            })?;
        let backup_dir = self
            .data_dir
            .join("migration-backups")
            .join(Uuid::new_v4().to_string());
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
            if source.exists() {
                fs::copy(&source, backup_dir.join(format!("culsans.sqlite3{suffix}")))?;
            }
        }
        let legacy_blobs = legacy_dir.join("blobs");
        if legacy_blobs.exists() {
            for item in fs::read_dir(&legacy_blobs)? {
                let item = item?;
                if item.path().is_file() {
                    fs::copy(item.path(), backup_dir.join("blobs").join(item.file_name()))?;
                }
            }
        }
        if file_fingerprint(legacy_db)? != before {
            return Err(StorageError::Migration(
                "legacy database changed while snapshotting".to_owned(),
            ));
        }
        let snapshot_db = backup_dir.join("culsans.sqlite3");
        let source = Connection::open_with_flags(&snapshot_db, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let entries_count = count_table_connection(&source, "clipboard_entries")?;
        let representations_count = count_table_connection(&source, "clipboard_representations")?;
        let has_saved = table_exists_connection(&source, "saved_insert_items")?;
        let saved_count = if has_saved {
            count_table_connection(&source, "saved_insert_items")?
        } else {
            0
        };
        let source_blobs = backup_dir.join("blobs");
        let tx = self.connection.transaction()?;
        if count_table_tx(&tx, "clipboard_entries")? != 0 {
            return Err(StorageError::Migration(
                "destination already contains Echo data".to_owned(),
            ));
        }
        copy_settings(&source, &tx)?;
        copy_legacy_entries(&source, &source_blobs, &tx, &self.blobs_dir)?;
        let favorites = if has_saved {
            copy_legacy_saved_items(&source, &source_blobs, &tx, &self.blobs_dir)?
        } else {
            synthesize_pinned_items(&source, &tx)?
        };
        let actual_entries = count_table_tx(&tx, "clipboard_entries")?;
        let actual_representations = count_table_tx(&tx, "clipboard_representations")?;
        if actual_entries != entries_count || actual_representations != representations_count {
            return Err(StorageError::Migration(format!(
                "legacy row count changed during migration: entries {actual_entries}/{entries_count}, representations {actual_representations}/{representations_count}"
            )));
        }
        if has_saved && favorites != saved_count {
            return Err(StorageError::Migration(format!(
                "legacy favorites changed during migration: {favorites}/{saved_count}"
            )));
        }
        tx.commit()?;
        self.rebuild_fts()?;
        self.rebuild_saved_search()?;
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
            already_migrated: false,
        })
    }

    fn rebuild_fts(&mut self) -> Result<()> {
        self.connection.execute("DELETE FROM clipboard_fts", [])?;
        let mut statement = self
            .connection
            .prepare("SELECT id, searchable_text, source_app FROM clipboard_entries")?;
        let rows = statement.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
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

    fn rebuild_saved_search(&mut self) -> Result<()> {
        let ids = {
            let mut statement = self.connection.prepare("SELECT id FROM saved_items")?;
            let rows = statement.query_map([], |row| row.get::<_, i64>(0))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        let tx = self.connection.transaction()?;
        tx.execute("DELETE FROM saved_items_fts", [])?;
        for id in ids {
            refresh_saved_search_tx(&tx, id)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn count_table(&self, table: &str) -> Result<usize> {
        Ok(self
            .connection
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get::<_, i64>(0)
            })? as usize)
    }
}

fn replace_tags_tx(tx: &Transaction<'_>, saved_item_id: i64, tags: &[String]) -> Result<()> {
    tx.execute(
        "DELETE FROM saved_item_tags WHERE saved_item_id = ?",
        [saved_item_id],
    )?;
    for tag in tags {
        let display = tag.trim();
        let normalized = display.to_lowercase();
        tx.execute(
            "INSERT OR IGNORE INTO tags (name, normalized_name) VALUES (?, ?)",
            params![display, normalized],
        )?;
        let tag_id: i64 = tx.query_row(
            "SELECT id FROM tags WHERE normalized_name = ?",
            [normalized],
            |row| row.get(0),
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO saved_item_tags (saved_item_id, tag_id) VALUES (?, ?)",
            params![saved_item_id, tag_id],
        )?;
    }
    Ok(())
}

fn refresh_saved_search_tx(tx: &Transaction<'_>, saved_item_id: i64) -> Result<()> {
    let (name, body, source_app, preview): (
        String,
        Option<String>,
        Option<String>,
        Option<String>,
    ) = tx.query_row(
        "SELECT name, editable_text, source_app, preview_text FROM saved_items WHERE id = ?",
        [saved_item_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let mut tags = tx.prepare(
        "SELECT t.name FROM tags t
         JOIN saved_item_tags sit ON sit.tag_id = t.id
         WHERE sit.saved_item_id = ? ORDER BY sit.rowid",
    )?;
    let tag_rows = tags.query_map([saved_item_id], |row| row.get::<_, String>(0))?;
    let tag_values = tag_rows.collect::<std::result::Result<Vec<_>, _>>()?;
    let document = [
        Some(name),
        body,
        preview,
        source_app,
        (!tag_values.is_empty()).then(|| tag_values.join(" ")),
    ]
    .into_iter()
    .flatten()
    .map(|value| value.to_lowercase())
    .collect::<Vec<_>>()
    .join("\n");
    tx.execute(
        "DELETE FROM saved_items_fts WHERE saved_item_id = ?",
        [saved_item_id],
    )?;
    tx.execute(
        "INSERT INTO saved_items_fts (saved_item_id, document) VALUES (?, ?)",
        params![saved_item_id, document],
    )?;
    Ok(())
}

fn preview_text(value: &str) -> String {
    value.chars().take(600).collect()
}

type BoxedWriteResult = Result<Box<dyn Any + Send>>;
type WriteOperation = Box<dyn FnOnce(&mut ClipboardStore) -> BoxedWriteResult + Send>;

struct WriteRequest {
    operation: WriteOperation,
    response: SyncSender<BoxedWriteResult>,
}

struct WriterRuntime {
    sender: SyncSender<WriteRequest>,
}

impl WriterRuntime {
    fn open(data_dir: &Path, metrics: Arc<OperationMetrics>) -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(32);
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let data_dir = data_dir.to_path_buf();
        thread::Builder::new()
            .name("echo-storage-writer".to_owned())
            .spawn(move || {
                let mut store = match ClipboardStore::open_with_metrics(&data_dir, metrics) {
                    Ok(store) => {
                        let _ = ready_sender.send(Ok(()));
                        store
                    }
                    Err(error) => {
                        let _ = ready_sender.send(Err(error.to_string()));
                        return;
                    }
                };
                writer_loop(&mut store, receiver);
            })
            .map_err(|error| {
                StorageError::Invalid(format!("cannot start storage writer: {error}"))
            })?;
        match ready_receiver.recv() {
            Ok(Ok(())) => Ok(Self { sender }),
            Ok(Err(error)) => Err(StorageError::Invalid(error)),
            Err(error) => Err(StorageError::Invalid(format!(
                "storage writer did not start: {error}"
            ))),
        }
    }

    fn from_store(store: ClipboardStore) -> Self {
        let (sender, receiver) = mpsc::sync_channel(32);
        thread::Builder::new()
            .name("echo-storage-writer".to_owned())
            .spawn(move || {
                let mut store = store;
                writer_loop(&mut store, receiver);
            })
            .expect("Echo storage writer thread");
        Self { sender }
    }

    fn execute<T, F>(&self, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut ClipboardStore) -> Result<T> + Send + 'static,
    {
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        self.sender
            .send(WriteRequest {
                operation: Box::new(move |store| {
                    operation(store).map(|value| Box::new(value) as Box<dyn Any + Send>)
                }),
                response: response_sender,
            })
            .map_err(|_| StorageError::Invalid("storage writer is unavailable".to_owned()))?;
        let result = response_receiver
            .recv()
            .map_err(|_| StorageError::Invalid("storage writer stopped".to_owned()))??;
        result.downcast::<T>().map(|value| *value).map_err(|_| {
            StorageError::Invalid("storage writer returned an invalid result".to_owned())
        })
    }
}

fn writer_loop(store: &mut ClipboardStore, receiver: mpsc::Receiver<WriteRequest>) {
    while let Ok(request) = receiver.recv() {
        let result = (request.operation)(store);
        let _ = request.response.send(result);
    }
}

struct ReaderRuntime {
    store: Mutex<ClipboardStore>,
}

impl ReaderRuntime {
    fn open(data_dir: &Path, metrics: Arc<OperationMetrics>) -> Result<Self> {
        Ok(Self {
            store: Mutex::new(ClipboardStore::open_read_only(data_dir, metrics)?),
        })
    }

    fn empty(data_dir: &Path, metrics: Arc<OperationMetrics>) -> Self {
        Self {
            store: Mutex::new(
                ClipboardStore::open_in_memory_with_metrics(data_dir, metrics)
                    .expect("Echo read runtime database"),
            ),
        }
    }

    fn read<T>(&self, operation: impl FnOnce(&ClipboardStore) -> Result<T>) -> Result<T> {
        let store = self
            .store
            .lock()
            .map_err(|_| StorageError::Invalid("storage reader is unavailable".to_owned()))?;
        operation(&store)
    }
}

struct MaintenanceRuntime {
    sender: SyncSender<Duration>,
}

impl MaintenanceRuntime {
    fn new(writer: Arc<WriterRuntime>, metrics: Arc<OperationMetrics>) -> Self {
        let (sender, receiver) = mpsc::sync_channel(32);
        thread::Builder::new()
            .name("echo-storage-maintenance".to_owned())
            .spawn(move || {
                while let Ok(delay) = receiver.recv() {
                    let mut deadline = Instant::now() + delay;
                    loop {
                        let remaining = deadline.saturating_duration_since(Instant::now());
                        if remaining.is_zero() {
                            break;
                        }
                        match receiver.recv_timeout(remaining) {
                            Ok(next_delay) => {
                                deadline = deadline.max(Instant::now() + next_delay);
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => break,
                            Err(mpsc::RecvTimeoutError::Disconnected) => return,
                        }
                    }
                    let started = Instant::now();
                    let result = writer.execute(|store| store.reconcile_blob_store());
                    metrics.record(
                        "maintenance_reconcile",
                        started.elapsed(),
                        u64::from(result.is_ok()),
                    );
                }
            })
            .expect("Echo storage maintenance thread");
        Self { sender }
    }

    fn request(&self, delay: Duration) {
        let _ = self.sender.try_send(delay);
    }
}

#[derive(Clone)]
pub struct SharedClipboardStore {
    writer: Arc<WriterRuntime>,
    reader: Arc<ReaderRuntime>,
    maintenance: Arc<MaintenanceRuntime>,
    metrics: Arc<OperationMetrics>,
}

impl SharedClipboardStore {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self> {
        let data_dir = data_dir.as_ref().to_path_buf();
        let metrics = Arc::new(OperationMetrics::default());
        let writer = Arc::new(WriterRuntime::open(&data_dir, Arc::clone(&metrics))?);
        let reader = Arc::new(ReaderRuntime::open(&data_dir, Arc::clone(&metrics))?);
        let maintenance = Arc::new(MaintenanceRuntime::new(
            Arc::clone(&writer),
            Arc::clone(&metrics),
        ));
        let shared = Self {
            writer,
            reader,
            maintenance,
            metrics,
        };
        shared.request_maintenance_now();
        Ok(shared)
    }

    pub fn from_store(store: ClipboardStore) -> Self {
        let data_dir = store.data_dir().to_path_buf();
        let metrics = Arc::new(OperationMetrics::default());
        let writer = Arc::new(WriterRuntime::from_store(store));
        let reader = Arc::new(
            ReaderRuntime::open(&data_dir, Arc::clone(&metrics))
                .unwrap_or_else(|_| ReaderRuntime::empty(&data_dir, Arc::clone(&metrics))),
        );
        let maintenance = Arc::new(MaintenanceRuntime::new(
            Arc::clone(&writer),
            Arc::clone(&metrics),
        ));
        let shared = Self {
            writer,
            reader,
            maintenance,
            metrics,
        };
        shared.request_maintenance_now();
        shared
    }

    pub fn with_store<T, F>(&self, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut ClipboardStore) -> Result<T> + Send + 'static,
    {
        self.writer.execute(operation)
    }

    pub fn metrics_snapshot(&self) -> Vec<OperationMetric> {
        self.metrics.snapshot()
    }

    pub fn request_maintenance(&self) {
        self.maintenance.request(Duration::from_millis(100));
    }

    fn request_maintenance_now(&self) {
        self.maintenance.request(Duration::ZERO);
    }

    fn request_after_pending_write(&self, pending: bool) {
        if pending {
            self.request_maintenance();
        }
    }

    pub fn settings(&self) -> Result<ClipboardSettings> {
        self.reader.read(|store| store.settings())
    }

    pub fn update_settings(&self, settings: &ClipboardSettings) -> Result<()> {
        let settings = settings.clone();
        self.with_store(move |store| store.update_settings(&settings))
    }

    pub fn list_entries(&self, query: &str, limit: u32) -> Result<Vec<ClipboardEntry>> {
        let query = query.to_owned();
        self.reader.read(|store| store.list_entries(&query, limit))
    }

    pub fn list_entries_page(
        &self,
        query: &str,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> Result<LibraryPage<ClipboardEntry>> {
        let query = query.to_owned();
        self.reader
            .read(|store| store.list_entries_page(&query, limit, cursor))
    }

    pub fn entry(&self, id: i64) -> Result<Option<StoredClipboardEntry>> {
        self.reader.read(|store| store.entry(id))
    }

    pub fn history_entry(&self, id: i64) -> Result<Option<ClipboardEntry>> {
        self.entry(id).map(|entry| entry.map(|stored| stored.entry))
    }

    pub fn entry_payload(&self, id: i64) -> Result<Vec<ClipboardRepresentation>> {
        self.reader.read(|store| store.entry_payload(id))
    }

    pub fn read_thumbnail(&self, content_hash: &str) -> Result<Option<StoredThumbnail>> {
        let content_hash = content_hash.to_owned();
        self.reader
            .read(|store| store.read_thumbnail(&content_hash))
    }

    pub fn list_saved_items(&self, query: &str, limit: u32) -> Result<Vec<SavedItem>> {
        let query = query.to_owned();
        self.reader
            .read(|store| store.list_saved_items(&query, limit))
    }

    pub fn list_saved_items_page(
        &self,
        query: &str,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> Result<LibraryPage<SavedItem>> {
        let query = query.to_owned();
        self.reader
            .read(|store| store.list_saved_items_page(&query, limit, cursor))
    }

    pub fn saved_item(&self, id: i64) -> Result<Option<StoredSavedItem>> {
        self.reader.read(|store| store.saved_item(id))
    }

    pub fn saved_item_payload(&self, id: i64) -> Result<Vec<ClipboardRepresentation>> {
        self.reader.read(|store| store.saved_item_payload(id))
    }

    pub fn save_history_item(
        &self,
        draft: SavedItemDraft,
        payload: Vec<ClipboardRepresentation>,
    ) -> Result<SavedItem> {
        let saved = self.with_store(move |store| store.save_history_item(draft, payload))?;
        self.request_maintenance();
        Ok(saved)
    }

    pub fn unsave_history_item(&self, id: i64) -> Result<bool> {
        let deleted = self.with_store(move |store| store.unsave_history_item(id))?;
        if deleted {
            self.request_maintenance();
        }
        Ok(deleted)
    }

    pub fn update_saved_item(&self, id: i64, update: SavedItemUpdate) -> Result<SavedItem> {
        let saved = self.with_store(move |store| store.update_saved_item(id, update))?;
        self.request_maintenance();
        Ok(saved)
    }

    pub fn delete_entry(&self, id: i64) -> Result<bool> {
        let deleted = self.with_store(move |store| store.delete_entry(id))?;
        if deleted {
            self.request_maintenance();
        }
        Ok(deleted)
    }

    pub fn delete_saved_items(&self, ids: &[i64]) -> Result<usize> {
        let ids = ids.to_vec();
        let deleted = self.with_store(move |store| store.delete_saved_items(&ids))?;
        if deleted != 0 {
            self.request_maintenance();
        }
        Ok(deleted)
    }

    pub fn clear_history(&self) -> Result<()> {
        self.with_store(|store| store.clear_history())?;
        self.request_maintenance();
        Ok(())
    }

    pub fn reconcile_blob_store(&self) -> Result<()> {
        self.with_store(|store| store.reconcile_blob_store())
    }

    pub fn migrate_legacy(&self, legacy_dir: impl AsRef<Path>) -> Result<MigrationReport> {
        let legacy_dir = legacy_dir.as_ref().to_path_buf();
        let report = self.with_store(move |store| store.migrate_legacy(&legacy_dir))?;
        if !report.already_migrated {
            self.request_maintenance_now();
        }
        Ok(report)
    }
}

impl ClipboardSink for SharedClipboardStore {
    fn settings(&self) -> std::result::Result<CaptureSettings, String> {
        self.settings()
            .map(|settings| CaptureSettings::from(&settings))
            .map_err(|error| error.to_string())
    }

    fn record(&self, capture: NormalizedCapture) -> std::result::Result<RecordResult, String> {
        let (result, pending) = self
            .with_store(move |store| {
                let result = store.record_capture(capture)?;
                let pending = store.maintenance_pending()?;
                Ok((result, pending))
            })
            .map_err(|error| error.to_string())?;
        self.request_after_pending_write(pending);
        Ok(result)
    }

    fn capture_policy(&self) -> std::result::Result<CapturePolicy, String> {
        self.settings()
            .map(|settings| CapturePolicy::from_max_item_bytes(settings.max_item_bytes))
            .map_err(|error| error.to_string())
    }

    fn record_captured(
        &self,
        captured: CapturedCapture,
    ) -> std::result::Result<RecordResult, String> {
        let (result, pending) = self
            .with_store(move |store| {
                let result = store.record_captured(captured)?;
                let pending = store.maintenance_pending()?;
                Ok((result, pending))
            })
            .map_err(|error| error.to_string())?;
        self.request_after_pending_write(pending);
        Ok(result)
    }

    fn request_maintenance(&self) {
        SharedClipboardStore::request_maintenance(self);
    }

    fn reconcile(&self) -> std::result::Result<(), String> {
        self.reconcile_blob_store()
            .map_err(|error| error.to_string())
    }
}

impl LibraryStore for SharedClipboardStore {
    type Error = StorageError;

    fn list_entries(
        &self,
        query: &str,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> std::result::Result<LibraryPage<ClipboardEntry>, Self::Error> {
        SharedClipboardStore::list_entries_page(self, query, limit, cursor)
    }

    fn entry(&self, id: i64) -> std::result::Result<Option<ClipboardEntry>, Self::Error> {
        SharedClipboardStore::history_entry(self, id)
    }

    fn entry_payload(
        &self,
        id: i64,
    ) -> std::result::Result<Vec<ClipboardRepresentation>, Self::Error> {
        SharedClipboardStore::entry_payload(self, id)
    }

    fn save_history_item(
        &self,
        draft: SavedItemDraft,
        payload: Vec<ClipboardRepresentation>,
    ) -> std::result::Result<SavedItem, Self::Error> {
        SharedClipboardStore::save_history_item(self, draft, payload)
    }

    fn unsave_history_item(&self, history_id: i64) -> std::result::Result<bool, Self::Error> {
        SharedClipboardStore::unsave_history_item(self, history_id)
    }

    fn update_saved_item(
        &self,
        id: i64,
        update: SavedItemUpdate,
    ) -> std::result::Result<SavedItem, Self::Error> {
        SharedClipboardStore::update_saved_item(self, id, update)
    }

    fn delete_entry(&self, id: i64) -> std::result::Result<bool, Self::Error> {
        SharedClipboardStore::delete_entry(self, id)
    }

    fn clear_history(&self) -> std::result::Result<(), Self::Error> {
        SharedClipboardStore::clear_history(self)
    }

    fn settings(&self) -> std::result::Result<ClipboardSettings, Self::Error> {
        SharedClipboardStore::settings(self)
    }

    fn update_settings(
        &self,
        settings: &ClipboardSettings,
    ) -> std::result::Result<(), Self::Error> {
        SharedClipboardStore::update_settings(self, settings)
    }

    fn list_saved_items(
        &self,
        query: &str,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> std::result::Result<LibraryPage<SavedItem>, Self::Error> {
        SharedClipboardStore::list_saved_items_page(self, query, limit, cursor)
    }

    fn saved_item_payload(
        &self,
        id: i64,
    ) -> std::result::Result<Vec<ClipboardRepresentation>, Self::Error> {
        SharedClipboardStore::saved_item_payload(self, id)
    }

    fn delete_saved_items(&self, ids: &[i64]) -> std::result::Result<usize, Self::Error> {
        SharedClipboardStore::delete_saved_items(self, ids)
    }
}

fn cursor_filter(alias: &str, cursor: Option<PageCursor>) -> (String, Vec<Value>) {
    match cursor {
        Some(cursor) => (
            format!("AND ({alias}.updated_at < ? OR ({alias}.updated_at = ? AND {alias}.id < ?))"),
            vec![
                Value::Integer(cursor.updated_at),
                Value::Integer(cursor.updated_at),
                Value::Integer(cursor.id),
            ],
        ),
        None => (String::new(), Vec::new()),
    }
}

fn fts_match_query(query: &str) -> String {
    format!("\"{}\"", query.replace('"', "\"\""))
}

fn escape_like_pattern(value: &str) -> String {
    value
        .chars()
        .flat_map(|character| match character {
            '\\' => ['\\', '\\'].into_iter().collect::<Vec<_>>(),
            '%' => ['\\', '%'].into_iter().collect::<Vec<_>>(),
            '_' => ['\\', '_'].into_iter().collect::<Vec<_>>(),
            character => [character].into_iter().collect::<Vec<_>>(),
        })
        .collect()
}

fn is_fts5_table(connection: &Connection, name: &str) -> Result<bool> {
    let sql: Option<String> = connection
        .query_row(
            "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?",
            [name],
            |row| row.get(0),
        )
        .optional()?;
    Ok(sql
        .map(|value| value.to_ascii_lowercase().contains("using fts5"))
        .unwrap_or(false))
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
        saved_item_id: row.get(11)?,
        byte_size: row.get::<_, i64>(12)?.try_into().unwrap_or(0),
        thumbnail: None,
    })
}

fn map_saved_item(row: &Row<'_>) -> rusqlite::Result<SavedItem> {
    Ok(SavedItem {
        id: row.get(0)?,
        source_history_id: row.get(1)?,
        created_at: row.get(2)?,
        updated_at: row.get(3)?,
        name: row.get(4)?,
        content_type: row.get(5)?,
        editable_text: row.get(6)?,
        source_app: row.get(7)?,
        source_executable: row.get(8)?,
        source_window_title: row.get(9)?,
        preview_text: row.get(10)?,
        byte_size: row.get::<_, i64>(11)?.try_into().unwrap_or(0),
        tags: Vec::new(),
        is_independent: row.get::<_, i64>(12)? != 0,
        thumbnail: None,
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
        content_hash: row.get(6)?,
        byte_size: row.get::<_, i64>(7)?.try_into().unwrap_or(0),
    })
}

fn map_thumbnail(row: &Row<'_>) -> rusqlite::Result<Thumbnail> {
    Ok(Thumbnail {
        source_hash: row.get(0)?,
        content_hash: row.get(1)?,
        mime_type: row.get(2)?,
        width: row.get::<_, i64>(3)?.try_into().unwrap_or(0),
        height: row.get::<_, i64>(4)?.try_into().unwrap_or(0),
        byte_size: row.get::<_, i64>(5)?.try_into().unwrap_or(0),
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
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

fn file_fingerprint(path: &Path) -> Result<(u64, u128)> {
    let metadata = fs::metadata(path)?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
        .map(|value| value.as_nanos())
        .unwrap_or(0);
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
    if !table_exists_connection(connection, table)? {
        return Ok(0);
    }
    Ok(
        connection.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get::<_, i64>(0)
        })? as usize,
    )
}

fn count_table_tx(tx: &Transaction<'_>, table: &str) -> Result<usize> {
    Ok(
        tx.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get::<_, i64>(0)
        })? as usize,
    )
}

fn has_column(connection: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    Ok(rows
        .collect::<std::result::Result<Vec<_>, _>>()?
        .iter()
        .any(|name| name == column))
}

fn copy_settings(source: &Connection, tx: &Transaction<'_>) -> Result<()> {
    if !table_exists_connection(source, "clipboard_settings")? {
        return Ok(());
    }
    let record_sensitive = if has_column(source, "clipboard_settings", "record_sensitive")? {
        "record_sensitive"
    } else {
        "0"
    };
    let sql = format!(
        "SELECT history_enabled, {record_sensitive}, store_window_titles, max_entries, max_total_bytes, max_item_bytes FROM clipboard_settings WHERE id = 1"
    );
    let row: Option<(i64, i64, i64, i64, i64, i64)> = source
        .query_row(&sql, [], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
            ))
        })
        .optional()?;
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
    if !table_exists_connection(source, "clipboard_entries")? {
        return Ok(());
    }
    let mut statement = source.prepare(
        "SELECT id, created_at, updated_at, source_app, source_executable, source_window_title,
                content_type, preview_text, searchable_text, sanitized_html, fingerprint, pinned, byte_size
         FROM clipboard_entries ORDER BY id",
    )?;
    let entries = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, Option<String>>(7)?,
            row.get::<_, Option<String>>(8)?,
            row.get::<_, Option<String>>(9)?,
            row.get::<_, String>(10)?,
            row.get::<_, i64>(11)?,
            row.get::<_, i64>(12)?,
        ))
    })?;
    let entries = entries.collect::<std::result::Result<Vec<_>, _>>()?;
    for (
        id,
        created,
        updated,
        app,
        executable,
        title,
        content_type,
        preview,
        searchable,
        html,
        fingerprint,
        _pinned,
        byte_size,
    ) in entries
    {
        tx.execute(
            "INSERT INTO clipboard_entries (id, created_at, updated_at, source_app, source_executable, source_window_title, content_type, preview_text, searchable_text, sanitized_html, fingerprint, byte_size) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            params![id, created, updated, app, executable, title, content_type, preview, searchable, html, fingerprint, byte_size],
        )?;
    }
    if !table_exists_connection(source, "clipboard_representations")? {
        return Ok(());
    }
    let mut statement = source.prepare(
        "SELECT id, entry_id, format, mime_type, inline_data, blob_hash, byte_size FROM clipboard_representations ORDER BY id",
    )?;
    let representations = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<Vec<u8>>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, i64>(6)?,
        ))
    })?;
    for (id, entry_id, format, mime, inline_data, blob_hash, byte_size) in
        representations.collect::<std::result::Result<Vec<_>, _>>()?
    {
        let data = if let Some(bytes) = inline_data {
            (Some(bytes), None)
        } else if let Some(hash) = blob_hash {
            validate_hash(&hash)?;
            let bytes = fs::read(source_blobs.join(&hash))
                .map_err(|_| StorageError::MissingBlob(hash.clone()))?;
            if hash_bytes(&bytes) != hash {
                return Err(StorageError::MissingBlob(hash));
            }
            let destination = destination_blobs.join(&hash);
            if !destination.exists() {
                fs::copy(source_blobs.join(&hash), destination)?;
            }
            (None, Some(hash))
        } else {
            return Err(StorageError::Migration(format!(
                "representation {id} has no inline or blob data"
            )));
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
            row.get::<_, i64>(0)?,
            row.get::<_, Option<i64>>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, Option<String>>(8)?,
            row.get::<_, Option<String>>(9)?,
            row.get::<_, Option<String>>(10)?,
            row.get::<_, i64>(11)?,
        ))
    })?;
    let items = items.collect::<std::result::Result<Vec<_>, _>>()?;
    for (
        id,
        source_id,
        created,
        updated,
        app,
        executable,
        title,
        content_type,
        preview,
        searchable,
        _html,
        byte_size,
    ) in &items
    {
        tx.execute(
            "INSERT INTO saved_items
             (id, source_history_id, created_at, updated_at, name, content_type, editable_text,
              source_app, source_executable, source_window_title, preview_text, byte_size, is_independent)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0)",
            params![
                id,
                source_id,
                created,
                updated,
                legacy_saved_name(content_type, preview.as_deref()),
                content_type,
                is_text_like(content_type).then(|| searchable.clone().or_else(|| preview.clone()).unwrap_or_default()),
                app,
                executable,
                title,
                preview,
                byte_size,
            ],
        )?;
        replace_tags_tx(&tx, *id, &[])?;
    }
    let mut statement = source.prepare(
        "SELECT id, saved_item_id, format, mime_type, inline_data, blob_hash, byte_size FROM saved_insert_representations ORDER BY id",
    )?;
    let representations = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<Vec<u8>>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, i64>(6)?,
        ))
    })?;
    for (id, saved_id, format, mime, inline_data, blob_hash, byte_size) in
        representations.collect::<std::result::Result<Vec<_>, _>>()?
    {
        let (inline, blob) = if let Some(bytes) = inline_data {
            (Some(bytes), None)
        } else if let Some(hash) = blob_hash {
            validate_hash(&hash)?;
            let bytes = fs::read(source_blobs.join(&hash))
                .map_err(|_| StorageError::MissingBlob(hash.clone()))?;
            if hash_bytes(&bytes) != hash {
                return Err(StorageError::MissingBlob(hash));
            }
            let destination = destination_blobs.join(&hash);
            if !destination.exists() {
                fs::copy(source_blobs.join(&hash), destination)?;
            }
            (None, Some(hash))
        } else {
            return Err(StorageError::Migration(format!(
                "saved representation {id} has no data"
            )));
        };
        tx.execute(
            "INSERT INTO saved_item_representations (id, saved_item_id, format, mime_type, inline_data, blob_hash, byte_size) VALUES (?, ?, ?, ?, ?, ?, ?)",
            params![id, saved_id, format, mime, inline, blob, byte_size],
        )?;
        if let Some(hash) = blob {
            tx.execute("INSERT OR IGNORE INTO clipboard_blobs (hash, mime_type, byte_size) VALUES (?, ?, ?)", params![hash, mime, byte_size])?;
        }
        refresh_saved_search_tx(&tx, saved_id)?;
    }
    Ok(items.len())
}

fn synthesize_pinned_items(source: &Connection, tx: &Transaction<'_>) -> Result<usize> {
    let mut statement = source.prepare("SELECT id, created_at, updated_at, source_app, source_executable, source_window_title, content_type, preview_text, searchable_text, sanitized_html, byte_size FROM clipboard_entries WHERE pinned = 1 ORDER BY id")?;
    let entries = statement.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, Option<String>>(7)?,
            row.get::<_, Option<String>>(8)?,
            row.get::<_, Option<String>>(9)?,
            row.get::<_, i64>(10)?,
        ))
    })?;
    let entries = entries.collect::<std::result::Result<Vec<_>, _>>()?;
    for (
        entry_id,
        created,
        updated,
        app,
        executable,
        title,
        content_type,
        preview,
        searchable,
        _html,
        byte_size,
    ) in &entries
    {
        tx.execute("INSERT INTO saved_items (source_history_id, created_at, updated_at, name, content_type, editable_text, source_app, source_executable, source_window_title, preview_text, byte_size, is_independent) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0)", params![entry_id, created, updated, legacy_saved_name(content_type, preview.as_deref()), content_type, is_text_like(content_type).then(|| searchable.clone().or_else(|| preview.clone()).unwrap_or_default()), app, executable, title, preview, byte_size])?;
        let saved_id = tx.last_insert_rowid();
        replace_tags_tx(tx, saved_id, &[])?;
        let mut reps = tx.prepare("SELECT format, mime_type, inline_data, blob_hash, byte_size FROM clipboard_representations WHERE entry_id = ? ORDER BY id")?;
        let rows = reps.query_map([entry_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<Vec<u8>>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, i64>(4)?,
            ))
        })?;
        for (format, mime, inline, blob, size) in
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        {
            tx.execute("INSERT INTO saved_item_representations (saved_item_id, format, mime_type, inline_data, blob_hash, byte_size) VALUES (?, ?, ?, ?, ?, ?)", params![saved_id, format, mime, inline, blob, size])?;
        }
        refresh_saved_search_tx(tx, saved_id)?;
    }
    Ok(entries.len())
}

fn legacy_saved_name(content_type: &str, preview: Option<&str>) -> String {
    preview
        .and_then(|value| value.lines().map(str::trim).find(|line| !line.is_empty()))
        .map(|value| value.chars().take(120).collect())
        .unwrap_or_else(|| match content_type {
            "image" => "Image".to_owned(),
            "files" => "Files".to_owned(),
            _ => "Saved item".to_owned(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use echo_engine::{fingerprint, ClipboardRepresentation, ContentType, SourceContext};
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
    fn duplicate_large_capture_skips_blob_preparation_and_reconcile_is_not_hot_path() {
        let root = TempDir::new().unwrap();
        let mut store = ClipboardStore::open(root.path()).unwrap();
        let mut capture = text_capture("large duplicate", 1);
        capture.representations[0].bytes = vec![9; INLINE_LIMIT + 7];
        capture.fingerprint = fingerprint(&capture.representations);
        let first = store.record_capture(capture.clone()).unwrap();
        let blob_hash = store
            .entry(first.id)
            .unwrap()
            .unwrap()
            .representations
            .into_iter()
            .next()
            .unwrap()
            .blob_hash
            .unwrap();
        fs::write(
            root.path().join("blobs").join(&blob_hash),
            b"not the original",
        )
        .unwrap();

        capture.sequence = 2;
        let duplicate = store.record_capture(capture).unwrap();
        assert!(duplicate.duplicate);
        assert_eq!(duplicate.id, first.id);
        let pending: Option<String> = store
            .connection
            .query_row(
                "SELECT value FROM migration_state WHERE key = 'blob_gc_pending'",
                [],
                |row| row.get(0),
            )
            .optional()
            .unwrap();
        assert!(pending.is_none());
    }

    #[test]
    fn favorites_survive_history_clear_and_source_delete() {
        let root = TempDir::new().unwrap();
        let mut store = ClipboardStore::open(root.path()).unwrap();
        let id = store
            .record_capture(text_capture("favorite", 1))
            .unwrap()
            .id;
        let entry = store.list_entries("", 20).unwrap().remove(0);
        let payload = store.entry_payload(id).unwrap();
        store
            .save_history_item(SavedItemDraft::from_history(&entry), payload)
            .unwrap();
        store.clear_history().unwrap();
        assert!(store.entry(id).unwrap().is_none());
        assert_eq!(store.list_saved_items("", 20).unwrap().len(), 1);
        store.delete_entry(id).unwrap();
        let saved = store.list_saved_items("", 20).unwrap();
        assert_eq!(saved.len(), 1);
        assert!(saved[0].source_history_id.is_none());
    }

    #[test]
    fn saved_text_edits_replace_the_canonical_payload_and_normalize_tags() {
        let root = TempDir::new().unwrap();
        let mut store = ClipboardStore::open(root.path()).unwrap();
        let history_id = store
            .record_capture(text_capture("original", 1))
            .unwrap()
            .id;
        let entry = store.list_entries("", 20).unwrap().remove(0);
        let saved = store
            .save_history_item(
                SavedItemDraft::from_history(&entry),
                store.entry_payload(history_id).unwrap(),
            )
            .unwrap();
        assert_eq!(saved.name, "original");
        assert_eq!(
            store.saved_item_payload(saved.id).unwrap()[0].bytes,
            b"original"
        );

        let updated = store
            .update_saved_item(
                saved.id,
                SavedItemUpdate {
                    name: "  Canonical name  ".to_owned(),
                    tags: vec![" Work ".to_owned(), "work".to_owned(), "工作".to_owned()],
                    editable_text: Some("edited body".to_owned()),
                },
            )
            .unwrap();
        assert_eq!(updated.name, "Canonical name");
        assert_eq!(updated.tags, vec!["Work", "工作"]);
        assert_eq!(
            store.saved_item_payload(saved.id).unwrap()[0].bytes,
            b"edited body"
        );
        assert_eq!(store.list_saved_items("工作", 20).unwrap().len(), 1);
        assert_eq!(store.list_saved_items("CANONICAL", 20).unwrap().len(), 1);
        assert_eq!(store.list_saved_items("edited", 20).unwrap().len(), 1);
    }

    #[test]
    fn image_metadata_edits_preserve_binary_payload_and_bulk_delete_is_atomic() {
        let root = TempDir::new().unwrap();
        let mut store = ClipboardStore::open(root.path()).unwrap();
        let mut capture = text_capture("image", 1);
        capture.content_type = ContentType::Image;
        capture.preview_text = Some("picture.png".to_owned());
        capture.searchable_text = Some("picture.png".to_owned());
        capture.representations[0] = ClipboardRepresentation {
            format: "image".to_owned(),
            mime_type: "image/png".to_owned(),
            bytes: vec![1, 2, 3, 4],
        };
        capture.fingerprint = fingerprint(&capture.representations);
        let first_history = store.record_capture(capture.clone()).unwrap().id;
        capture.sequence = 2;
        capture.representations[0].bytes = vec![5, 6, 7, 8];
        capture.fingerprint = fingerprint(&capture.representations);
        let second_history = store.record_capture(capture).unwrap().id;
        let entries = store.list_entries("", 20).unwrap();
        let first = store
            .save_history_item(
                SavedItemDraft::from_history(
                    &entries
                        .iter()
                        .find(|entry| entry.id == first_history)
                        .unwrap()
                        .clone(),
                ),
                store.entry_payload(first_history).unwrap(),
            )
            .unwrap();
        let second = store
            .save_history_item(
                SavedItemDraft::from_history(
                    &entries
                        .iter()
                        .find(|entry| entry.id == second_history)
                        .unwrap()
                        .clone(),
                ),
                store.entry_payload(second_history).unwrap(),
            )
            .unwrap();
        let bytes = store.saved_item_payload(first.id).unwrap()[0].bytes.clone();
        store
            .update_saved_item(
                first.id,
                SavedItemUpdate {
                    name: "renamed image".to_owned(),
                    tags: vec!["assets".to_owned()],
                    editable_text: None,
                },
            )
            .unwrap();
        assert_eq!(store.saved_item_payload(first.id).unwrap()[0].bytes, bytes);
        assert_eq!(store.delete_saved_items(&[first.id, second.id]).unwrap(), 2);
        assert!(store.list_saved_items("", 20).unwrap().is_empty());
    }

    #[test]
    fn image_capture_persists_a_content_addressed_thumbnail_without_changing_original_bytes() {
        let root = TempDir::new().unwrap();
        let mut store = ClipboardStore::open(root.path()).unwrap();
        let original = one_pixel_png();
        let mut capture = text_capture("image", 1);
        capture.content_type = ContentType::Image;
        capture.preview_text = Some("picture".to_owned());
        capture.searchable_text = Some("picture".to_owned());
        capture.representations = vec![ClipboardRepresentation {
            format: "image".to_owned(),
            mime_type: "image/png".to_owned(),
            bytes: original.clone(),
        }];
        capture.fingerprint = fingerprint(&capture.representations);

        let id = store.record_capture(capture).unwrap().id;
        let entry = store.entry(id).unwrap().unwrap();
        let thumbnail = entry.entry.thumbnail.clone().unwrap();
        assert_eq!(thumbnail.mime_type, THUMBNAIL_MIME_TYPE);
        assert!(thumbnail.width <= 256);
        assert!(thumbnail.height <= 256);
        assert!(root
            .path()
            .join("thumbnails")
            .join(&thumbnail.content_hash)
            .is_file());
        let stored = store
            .read_thumbnail(&thumbnail.content_hash)
            .unwrap()
            .unwrap();
        assert_eq!(stored.metadata, thumbnail);
        assert_eq!(store.entry_payload(id).unwrap()[0].bytes, original);
    }

    #[test]
    fn fts_match_and_cursor_pages_are_bounded_and_stable() {
        let root = TempDir::new().unwrap();
        let mut store = ClipboardStore::open(root.path()).unwrap();
        for sequence in 1..=3 {
            store
                .record_capture(text_capture(&format!("searchable-{sequence}"), sequence))
                .unwrap();
        }

        let first = store.list_entries_page("", 1, None).unwrap();
        assert_eq!(first.items.len(), 1);
        let cursor = first.next_cursor;
        let second = store.list_entries_page("", 1, cursor).unwrap();
        assert_eq!(second.items.len(), 1);
        assert_ne!(first.items[0].id, second.items[0].id);
        assert_eq!(store.list_entries("searchable-2", 20).unwrap().len(), 1);
        assert_eq!(store.list_entries("sea", 20).unwrap().len(), 3);
        assert_eq!(store.list_entries("se", 20).unwrap().len(), 3);

        let marker: String = store
            .connection
            .query_row(
                "SELECT value FROM migration_state WHERE key = ?",
                [SEARCH_FTS_SCHEMA_KEY],
                |row| row.get(0),
            )
            .unwrap();
        assert!(marker.starts_with("2:"));
    }

    #[test]
    fn versioned_fixture_upgrades_preserve_records_and_are_idempotent() {
        for (fixture, entries, favorites) in [
            (include_str!("../fixtures/migrations/pre-r0.sql"), 1, 0),
            (include_str!("../fixtures/migrations/pre-r1.sql"), 1, 1),
            (include_str!("../fixtures/migrations/pre-r2.sql"), 1, 1),
        ] {
            let root = TempDir::new().unwrap();
            let database = root.path().join("echo.sqlite3");
            let connection = Connection::open(&database).unwrap();
            connection.execute_batch(fixture).unwrap();
            drop(connection);

            let store = ClipboardStore::open(root.path()).unwrap();
            let version: i32 = store
                .connection
                .query_row("PRAGMA user_version", [], |row| row.get(0))
                .unwrap();
            assert_eq!(version, CURRENT_SCHEMA_VERSION);
            assert_eq!(store.list_entries("", 20).unwrap().len(), entries);
            assert_eq!(store.list_saved_items("", 20).unwrap().len(), favorites);
            drop(store);

            let reopened = ClipboardStore::open(root.path()).unwrap();
            let reopened_version: i32 = reopened
                .connection
                .query_row("PRAGMA user_version", [], |row| row.get(0))
                .unwrap();
            assert_eq!(reopened_version, CURRENT_SCHEMA_VERSION);
            assert_eq!(reopened.list_entries("", 20).unwrap().len(), entries);
            assert_eq!(reopened.list_saved_items("", 20).unwrap().len(), favorites);
        }
    }

    #[test]
    fn pre_r0_saved_item_fixture_migrates_payload_and_removes_legacy_tables() {
        let root = TempDir::new().unwrap();
        let database = root.path().join("echo.sqlite3");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(include_str!(
                "../fixtures/migrations/pre-r0-saved-items.sql"
            ))
            .unwrap();
        drop(connection);

        let store = ClipboardStore::open(root.path()).unwrap();
        let items = store.list_saved_items("saved text", 20).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].source_history_id, Some(41));
        assert_eq!(
            store.saved_item_payload(items[0].id).unwrap()[0].bytes,
            b"pre-r0 saved text"
        );
        assert!(!table_exists_connection(&store.connection, "saved_insert_items").unwrap());
        assert!(
            !table_exists_connection(&store.connection, "saved_insert_representations").unwrap()
        );

        drop(store);
        let reopened = ClipboardStore::open(root.path()).unwrap();
        assert_eq!(
            reopened.list_saved_items("saved text", 20).unwrap().len(),
            1
        );
        assert_eq!(
            reopened.saved_item_payload(61).unwrap()[0].bytes,
            b"pre-r0 saved text"
        );
    }

    #[test]
    fn storage_reader_progresses_while_writer_is_blocked() {
        let root = TempDir::new().unwrap();
        let store = SharedClipboardStore::open(root.path()).unwrap();
        let writer = store.clone();
        let (entered, entered_receiver) = std::sync::mpsc::channel();
        let (release, release_receiver) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            writer
                .with_store(move |_| {
                    entered.send(()).unwrap();
                    release_receiver.recv().unwrap();
                    Ok::<_, StorageError>(())
                })
                .unwrap();
        });
        entered_receiver
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        let started = Instant::now();
        assert!(store.list_entries("", 20).unwrap().is_empty());
        assert!(started.elapsed() < std::time::Duration::from_millis(500));
        release.send(()).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn preview_file_reads_do_not_wait_for_the_writer_runtime() {
        let root = TempDir::new().unwrap();
        let store = SharedClipboardStore::open(root.path()).unwrap();
        let mut capture = text_capture("preview", 1);
        let original = one_pixel_png();
        capture.content_type = ContentType::Image;
        capture.representations = vec![ClipboardRepresentation {
            format: "image".to_owned(),
            mime_type: "image/png".to_owned(),
            bytes: original,
        }];
        capture.fingerprint = fingerprint(&capture.representations);
        let id = store
            .with_store(move |store| Ok::<_, StorageError>(store.record_capture(capture)?.id))
            .unwrap();
        let hash = store
            .entry(id)
            .unwrap()
            .unwrap()
            .entry
            .thumbnail
            .unwrap()
            .content_hash;

        let writer = store.clone();
        let (entered, entered_receiver) = std::sync::mpsc::channel();
        let (release, release_receiver) = std::sync::mpsc::channel();
        let handle = std::thread::spawn(move || {
            writer
                .with_store(move |_| {
                    entered.send(()).unwrap();
                    release_receiver.recv().unwrap();
                    Ok::<_, StorageError>(())
                })
                .unwrap();
        });
        entered_receiver
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();
        assert!(store.read_thumbnail(&hash).unwrap().is_some());
        release.send(()).unwrap();
        handle.join().unwrap();
    }

    #[test]
    fn maintenance_runs_on_startup_and_delete_not_on_normal_insert() {
        let root = TempDir::new().unwrap();
        let store = SharedClipboardStore::open(root.path()).unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(1);
        while Instant::now() < deadline
            && !store
                .metrics_snapshot()
                .iter()
                .any(|metric| metric.operation == "maintenance_reconcile")
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let startup_runs = store
            .metrics_snapshot()
            .into_iter()
            .find(|metric| metric.operation == "maintenance_reconcile")
            .map(|metric| metric.samples)
            .unwrap_or(0);
        assert_eq!(startup_runs, 1);

        let id = store
            .with_store(|store| {
                Ok::<_, StorageError>(store.record_capture(text_capture("maintenance", 1))?.id)
            })
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(150));
        let after_insert = store
            .metrics_snapshot()
            .into_iter()
            .find(|metric| metric.operation == "maintenance_reconcile")
            .map(|metric| metric.samples)
            .unwrap_or(0);
        assert_eq!(after_insert, startup_runs);

        store.delete_entry(id).unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(1);
        while Instant::now() < deadline
            && store
                .metrics_snapshot()
                .into_iter()
                .find(|metric| metric.operation == "maintenance_reconcile")
                .map(|metric| metric.samples)
                .unwrap_or(0)
                < startup_runs + 1
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let delete_runs = store
            .metrics_snapshot()
            .into_iter()
            .find(|metric| metric.operation == "maintenance_reconcile")
            .map(|metric| metric.samples)
            .unwrap_or(0);
        assert_eq!(delete_runs, startup_runs + 1);
    }

    #[test]
    fn instrumentation_is_structured_and_redacts_capture_values() {
        let root = TempDir::new().unwrap();
        let mut store = ClipboardStore::open(root.path()).unwrap();
        store
            .record_capture(text_capture("never-log-this-payload", 1))
            .unwrap();
        let encoded = serde_json::to_string(&store.metrics_snapshot()).unwrap();
        assert!(encoded.contains("dedupe"));
        assert!(encoded.contains("db_commit"));
        assert!(!encoded.contains("never-log-this-payload"));
        assert!(!encoded.contains(root.path().to_string_lossy().as_ref()));
    }

    fn one_pixel_png() -> Vec<u8> {
        vec![
            0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x04, 0x00, 0x00,
            0x00, 0xb5, 0x1c, 0x0c, 0x02, 0x00, 0x00, 0x00, 0x0b, 0x49, 0x44, 0x41, 0x54, 0x78,
            0xda, 0x63, 0x64, 0xf8, 0x0f, 0x00, 0x01, 0x05, 0x01, 0x01, 0x27, 0x18, 0xe3, 0x66,
            0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
        ]
    }

    #[test]
    fn unsaving_an_edited_item_unlinks_without_destroying_user_content() {
        let root = TempDir::new().unwrap();
        let mut store = ClipboardStore::open(root.path()).unwrap();
        let history_id = store.record_capture(text_capture("source", 1)).unwrap().id;
        let entry = store.list_entries("", 20).unwrap().remove(0);
        let saved = store
            .save_history_item(
                SavedItemDraft::from_history(&entry),
                store.entry_payload(history_id).unwrap(),
            )
            .unwrap();
        store
            .update_saved_item(
                saved.id,
                SavedItemUpdate {
                    name: "authored".to_owned(),
                    tags: vec!["kept".to_owned()],
                    editable_text: Some("edited".to_owned()),
                },
            )
            .unwrap();
        assert!(store.unsave_history_item(history_id).unwrap());
        let remaining = store.list_saved_items("", 20).unwrap();
        assert_eq!(remaining.len(), 1);
        assert!(remaining[0].source_history_id.is_none());
        assert_eq!(
            store.saved_item_payload(saved.id).unwrap()[0].bytes,
            b"edited"
        );
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
             ",
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
