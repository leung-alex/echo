use std::any::Any;
use std::collections::HashSet;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use echo_engine::{
    is_text_like, CapturePolicy, CaptureSettings, CapturedCapture, ClipboardRepresentation,
    ClipboardSink, ContentIdentity, FavoriteDraft, FavoriteUpdate, LibraryPage, LibraryStore,
    NormalizedCapture, OperationMetric, OperationMetrics, PageCursor, PreviewAsset,
    PreviewDisposition, RecordResult, RepresentationIdentity, SavedItemDraft, ThemeMode, Thumbnail,
    DEFAULT_PAGE_SIZE, MAX_PAGE_SIZE, THUMBNAIL_MIME_TYPE,
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
const CURRENT_SCHEMA_VERSION: i32 = 5;
const THEME_COLUMN_DEFINITION: &str =
    "TEXT NOT NULL DEFAULT 'system' CHECK (theme IN ('system', 'light', 'dark'))";

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
    #[error("schema migration failed: {0}")]
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
    blobs_dir: PathBuf,
    thumbnails_dir: PathBuf,
    metrics: Arc<OperationMetrics>,
}

impl ClipboardStore {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_metrics(data_dir.as_ref(), Arc::new(OperationMetrics::default()))
    }

    #[cfg(any(test, feature = "test-storage"))]
    pub fn open_in_memory_for_tests(data_dir: impl AsRef<Path>) -> Result<Self> {
        let data_dir = data_dir.as_ref().to_path_buf();
        let mut store = Self {
            connection: Connection::open_in_memory()?,
            blobs_dir: data_dir.join("blobs"),
            thumbnails_dir: data_dir.join("thumbnails"),
            metrics: Arc::new(OperationMetrics::default()),
        };
        store.configure()?;
        store.ensure_schema()?;
        Ok(store)
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
            blobs_dir,
            thumbnails_dir,
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
                5 => self.migrate_schema_v5()?,
                _ => unreachable!("schema version is bounded above"),
            }
            self.connection
                .execute_batch(&format!("PRAGMA user_version = {next};"))?;
        }
        // Schema v5 was introduced before the theme field was integrated into
        // the shared settings contract. Keep already-v5 databases on the same
        // settings table instead of creating a second persistence path.
        self.ensure_column("clipboard_settings", "theme", THEME_COLUMN_DEFINITION)?;
        Ok(())
    }

    fn migrate_schema_v1(&mut self) -> Result<()> {
        self.ensure_base_objects()?;
        if table_exists_connection(&self.connection, "saved_insert_items")? {
            self.migrate_pre_r0_saved_tables()?;
        } else if has_column(&self.connection, "clipboard_entries", "pinned")?
            && self.count_table("saved_items")? == 0
        {
            self.migrate_pinned_saved_items()?;
        }
        Ok(())
    }

    fn migrate_pre_r0_saved_tables(&mut self) -> Result<()> {
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
                    default_saved_item_name(content_type, preview.as_deref()),
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
                    default_saved_item_name(&content_type, preview.as_deref()),
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

    fn ensure_column_tx(
        tx: &Transaction<'_>,
        table: &str,
        column: &str,
        definition: &str,
    ) -> Result<()> {
        if !Self::has_column_tx(tx, table, column)? {
            tx.execute_batch(&format!(
                "ALTER TABLE {table} ADD COLUMN {column} {definition}"
            ))?;
        }
        Ok(())
    }

    fn has_column_tx(tx: &Transaction<'_>, table: &str, column: &str) -> Result<bool> {
        let mut statement = tx.prepare(&format!("PRAGMA table_info({table})"))?;
        let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
        Ok(rows
            .collect::<std::result::Result<Vec<_>, _>>()?
            .iter()
            .any(|name| name == column))
    }

    fn verify_saved_representations_tx(
        tx: &Transaction<'_>,
        blobs_dir: &Path,
        saved_item_id: i64,
    ) -> Result<()> {
        let representations = {
            let mut statement = tx.prepare(
                "SELECT id, inline_data, blob_hash, content_hash, byte_size
             FROM saved_item_representations WHERE saved_item_id = ? ORDER BY id",
            )?;
            let rows = statement.query_map([saved_item_id], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<Vec<u8>>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        if representations.is_empty() {
            return Err(StorageError::Migration(format!(
                "saved item {saved_item_id} has no representations"
            )));
        }
        let mut total_size = 0_i64;
        for (id, inline_data, blob_hash, content_hash, byte_size) in representations {
            if byte_size < 0 || inline_data.is_some() == blob_hash.is_some() {
                return Err(StorageError::Migration(format!(
                    "saved item {saved_item_id} representation {id} is incomplete"
                )));
            }
            let bytes = if let Some(bytes) = inline_data {
                bytes
            } else {
                let hash = blob_hash.as_deref().expect("checked above");
                validate_hash(hash).map_err(|error| {
                    StorageError::Migration(format!(
                        "saved item {saved_item_id} representation {id}: {error}"
                    ))
                })?;
                let bytes = fs::read(blobs_dir.join(hash)).map_err(|_| {
                    StorageError::Migration(format!(
                        "saved item {saved_item_id} representation {id} blob is missing"
                    ))
                })?;
                if hash_bytes(&bytes) != hash {
                    return Err(StorageError::Migration(format!(
                        "saved item {saved_item_id} representation {id} blob hash mismatch"
                    )));
                }
                bytes
            };
            let actual_size = i64::try_from(bytes.len()).unwrap_or(i64::MAX);
            total_size = total_size.saturating_add(actual_size);
            if actual_size != byte_size {
                // A few pre-v1 fixtures recorded the preview length instead of
                // the representation length. The bytes are authoritative, so
                // repair only this derived metadata while retaining the payload.
                tx.execute(
                    "UPDATE saved_item_representations SET byte_size = ? WHERE id = ?",
                    params![actual_size, id],
                )?;
            }
            let actual_hash = hash_bytes(&bytes);
            if content_hash.as_deref() != Some(actual_hash.as_str()) {
                // Some pre-r2 fixtures carried a non-content source marker here;
                // bytes are authoritative, so repair this derived column while
                // the same migration transaction is still open.
                tx.execute(
                    "UPDATE saved_item_representations SET content_hash = ? WHERE id = ?",
                    params![actual_hash, id],
                )?;
            }
        }
        tx.execute(
            "UPDATE saved_items SET byte_size = ? WHERE id = ?",
            params![total_size, saved_item_id],
        )?;
        Ok(())
    }

    /// Add the Wave 1 state in one transactional migration. In particular,
    /// linked Saved Items from schema v4 are made authoritative before their
    /// duplicate History rows are removed. Any representation/blob failure
    /// aborts the transaction and leaves both records readable.
    fn migrate_schema_v5(&mut self) -> Result<()> {
        let blobs_dir = self.blobs_dir.clone();
        let tx = self.connection.transaction()?;
        Self::ensure_column_tx(&tx, "clipboard_entries", "history_pinned_at", "INTEGER")?;
        Self::ensure_column_tx(&tx, "clipboard_settings", "theme", THEME_COLUMN_DEFINITION)?;
        Self::ensure_column_tx(&tx, "saved_items", "icon_key", "TEXT")?;
        Self::ensure_column_tx(
            &tx,
            "saved_items",
            "favorite_order",
            "INTEGER NOT NULL DEFAULT 0",
        )?;

        let favorite_ids = {
            let mut statement =
                tx.prepare("SELECT id FROM saved_items ORDER BY updated_at DESC, id DESC")?;
            let rows = statement.query_map([], |row| row.get::<_, i64>(0))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        for (order, id) in favorite_ids.into_iter().enumerate() {
            tx.execute(
                "UPDATE saved_items SET favorite_order = ? WHERE id = ?",
                params![i64::try_from(order).unwrap_or(i64::MAX), id],
            )?;
        }

        let linked = {
            let mut statement = tx.prepare(
                "SELECT id, source_history_id FROM saved_items
                 WHERE source_history_id IS NOT NULL ORDER BY id",
            )?;
            let rows = statement
                .query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        let mut removed_history = false;
        for (saved_id, history_id) in linked {
            Self::verify_saved_representations_tx(&tx, &blobs_dir, saved_id)?;
            let history_exists: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM clipboard_entries WHERE id = ?)",
                [history_id],
                |row| row.get::<_, i64>(0),
            )? != 0;
            if history_exists {
                tx.execute(
                    "DELETE FROM clipboard_fts WHERE entry_id = ?",
                    [history_id.to_string()],
                )?;
                tx.execute("DELETE FROM clipboard_entries WHERE id = ?", [history_id])?;
                removed_history = true;
            } else {
                // A dangling source can occur after an interrupted legacy
                // cleanup. Clearing only the link preserves the Favorite.
            }
            tx.execute(
                "UPDATE saved_items SET source_history_id = NULL WHERE id = ?",
                [saved_id],
            )?;
        }
        tx.commit()?;
        if removed_history {
            self.schedule_blob_gc()?;
        }
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
                max_item_bytes INTEGER NOT NULL DEFAULT 33554432,
                theme TEXT NOT NULL DEFAULT 'system'
                    CHECK (theme IN ('system', 'light', 'dark'))
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

    pub fn metrics_snapshot(&self) -> Vec<OperationMetric> {
        self.metrics.snapshot()
    }

    pub fn settings(&self) -> Result<ClipboardSettings> {
        let (
            history_enabled,
            record_sensitive,
            store_window_titles,
            max_entries,
            max_total_bytes,
            max_item_bytes,
            theme_value,
        ): (bool, bool, bool, i64, i64, i64, String) = self
            .connection
            .query_row(
                "SELECT history_enabled, record_sensitive, store_window_titles,
                        max_entries, max_total_bytes, max_item_bytes, theme
                 FROM clipboard_settings WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)? != 0,
                        row.get::<_, i64>(1)? != 0,
                        row.get::<_, i64>(2)? != 0,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .map_err(StorageError::from)?;
        let theme = ThemeMode::parse(&theme_value).ok_or_else(|| {
            StorageError::Invalid(format!(
                "clipboard settings contain invalid theme mode {theme_value:?}"
            ))
        })?;
        Ok(ClipboardSettings {
            history_enabled,
            record_sensitive,
            store_window_titles,
            max_entries: max_entries.try_into().unwrap_or(DEFAULT_MAX_ENTRIES),
            max_total_bytes: max_total_bytes
                .try_into()
                .unwrap_or(DEFAULT_MAX_TOTAL_BYTES),
            max_item_bytes: max_item_bytes.try_into().unwrap_or(DEFAULT_MAX_ITEM_BYTES),
            theme,
        })
    }

    pub fn update_settings(&mut self, settings: &ClipboardSettings) -> Result<()> {
        self.connection.execute(
            "UPDATE clipboard_settings SET history_enabled = ?, record_sensitive = ?,
             store_window_titles = ?, max_entries = ?, max_total_bytes = ?, max_item_bytes = ?,
             theme = ?
             WHERE id = 1",
            params![
                settings.history_enabled as i64,
                settings.record_sensitive as i64,
                settings.store_window_titles as i64,
                i64::from(settings.max_entries),
                i64::try_from(settings.max_total_bytes).unwrap_or(i64::MAX),
                i64::try_from(settings.max_item_bytes).unwrap_or(i64::MAX),
                settings.theme.as_str(),
            ],
        )?;
        Ok(())
    }

    /// Convenience path for normalized callers. Production ingestion normally
    /// supplies identity and preview work through `record_captured` instead.
    pub fn record_capture(&mut self, capture: NormalizedCapture) -> Result<RecordResult> {
        let captured = CapturedCapture::from_capture(capture);
        let preview_disposition = captured
            .preview_source()
            .map(|source| self.preview_disposition_for_source(&source.hash))
            .transpose()?;
        let captured = generate_preview_if_requested(captured, preview_disposition, &self.metrics);
        self.record_captured(captured)
    }

    pub fn record_captured(&mut self, captured: CapturedCapture) -> Result<RecordResult> {
        self.record_capture_with_identity_and_preview(
            captured.capture,
            captured.identity,
            captured.preview,
        )
    }

    /// Commits a caller-provided identity without generating a preview.
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
        let previous_thumbnail_hash = preview
            .as_ref()
            .map(|asset| {
                self.connection
                    .query_row(
                        "SELECT thumbnail_hash FROM preview_assets WHERE source_hash = ?",
                        [&asset.thumbnail.source_hash],
                        |row| row.get::<_, String>(0),
                    )
                    .optional()
            })
            .transpose()?
            .flatten();
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
        let preview_replaced = preview.as_ref().is_some_and(|thumbnail| {
            previous_thumbnail_hash
                .as_ref()
                .is_some_and(|previous| previous != &thumbnail.content_hash)
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
            Self::insert_thumbnail_tx(&tx, thumbnail)?;
        }
        let (evicted, over_target) = Self::enforce_capacity_tx(&tx, &settings)?;
        let commit_started = Instant::now();
        tx.commit()?;
        self.metrics
            .record("db_commit", commit_started.elapsed(), 1);
        if over_target {
            self.metrics
                .record("capacity_over_target", Duration::ZERO, 1);
        }
        if evicted || preview_replaced {
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
            "INSERT INTO preview_assets
             (source_hash, thumbnail_hash, mime_type, width, height, byte_size)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(source_hash) DO UPDATE SET
               thumbnail_hash = excluded.thumbnail_hash,
               mime_type = excluded.mime_type,
               width = excluded.width,
               height = excluded.height,
               byte_size = excluded.byte_size",
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

    fn enforce_capacity_tx(
        tx: &Transaction<'_>,
        settings: &ClipboardSettings,
    ) -> Result<(bool, bool)> {
        let mut evicted = false;
        let mut over_target = false;
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
                    "SELECT id FROM clipboard_entries
                     WHERE history_pinned_at IS NULL
                     ORDER BY updated_at ASC, id ASC LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(oldest) = oldest else {
                over_target = true;
                break;
            };
            tx.execute(
                "DELETE FROM clipboard_fts WHERE entry_id = ?",
                [oldest.to_string()],
            )?;
            tx.execute("DELETE FROM clipboard_entries WHERE id = ?", [oldest])?;
            evicted = true;
        }
        Ok((evicted, over_target))
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
        let mut relevances = Vec::new();
        let trimmed = query.trim();
        if trimmed.is_empty() {
            let (cursor_clause, cursor_values) = history_cursor_filter(cursor)?;
            let sql = format!(
                "SELECT e.id, e.created_at, e.updated_at, e.source_app, e.source_executable,
                        e.source_window_title, e.content_type, e.preview_text, e.searchable_text,
                        e.sanitized_html, e.fingerprint, e.history_pinned_at,
                        e.byte_size
                 FROM clipboard_entries e
                 WHERE 1 = 1 {cursor_clause}
                 ORDER BY CASE WHEN e.history_pinned_at IS NULL THEN 1 ELSE 0 END ASC,
                          e.history_pinned_at DESC,
                          CASE WHEN e.history_pinned_at IS NULL THEN e.updated_at END DESC,
                          e.id DESC LIMIT ?"
            );
            let mut values = cursor_values;
            values.push(Value::Integer(fetch_limit));
            let mut statement = self.connection.prepare(&sql)?;
            let rows = statement.query_map(params_from_iter(values), map_entry)?;
            for row in rows {
                entries.push(row?);
            }
        } else if trimmed.chars().count() < 3 {
            let query_lower = trimmed.to_lowercase();
            let short_rank = "CASE WHEN lower(COALESCE(e.searchable_text, '')) = ? THEN 0
                                    WHEN lower(COALESCE(e.searchable_text, '')) LIKE ? || '%' THEN 1
                                    ELSE 2 END";
            let (cursor_clause, cursor_values) = history_search_cursor_filter_with_rank(
                cursor,
                short_rank,
                &[
                    Value::Text(query_lower.clone()),
                    Value::Text(query_lower.clone()),
                ],
                RankOrder::Ascending,
            )?;
            let pattern = format!("%{}%", escape_like_pattern(&query_lower));
            let sql = format!(
                "SELECT e.id, e.created_at, e.updated_at, e.source_app, e.source_executable,
                        e.source_window_title, e.content_type, e.preview_text, e.searchable_text,
                        e.sanitized_html, e.fingerprint, e.history_pinned_at, e.byte_size,
                        {short_rank} AS relevance
                 FROM clipboard_entries e
                 WHERE (lower(COALESCE(e.searchable_text, '')) LIKE ? ESCAPE '\\'
                    OR lower(COALESCE(e.source_app, '')) LIKE ? ESCAPE '\\'
                    OR lower(COALESCE(e.preview_text, '')) LIKE ? ESCAPE '\\')
                   {cursor_clause}
                 ORDER BY CASE WHEN e.history_pinned_at IS NULL THEN 1 ELSE 0 END ASC,
                          relevance ASC,
                          CASE WHEN e.history_pinned_at IS NULL THEN e.updated_at END DESC,
                          e.id DESC LIMIT ?"
            );
            let mut values = vec![
                Value::Text(query_lower.clone()),
                Value::Text(query_lower),
                Value::Text(pattern.clone()),
                Value::Text(pattern.clone()),
                Value::Text(pattern),
            ];
            values.extend(cursor_values);
            values.push(Value::Integer(fetch_limit));
            let mut statement = self.connection.prepare(&sql)?;
            let rows = statement.query_map(params_from_iter(values), |row| {
                Ok((map_entry(row)?, row.get::<_, i64>(13)?))
            })?;
            for row in rows {
                let (entry, relevance) = row?;
                entries.push(entry);
                relevances.push(Some(relevance));
            }
        } else {
            let (cursor_clause, cursor_values) = history_search_cursor_filter_with_rank(
                cursor,
                "CAST((-bm25(clipboard_fts)) * 1000000 AS INTEGER)",
                &[],
                RankOrder::Descending,
            )?;
            let sql = format!(
                "SELECT e.id, e.created_at, e.updated_at, e.source_app, e.source_executable,
                        e.source_window_title, e.content_type, e.preview_text, e.searchable_text,
                        e.sanitized_html, e.fingerprint, e.history_pinned_at, e.byte_size,
                        CAST((-bm25(clipboard_fts)) * 1000000 AS INTEGER) AS relevance
                 FROM clipboard_entries e
                 JOIN clipboard_fts f ON CAST(f.entry_id AS INTEGER) = e.id
                 WHERE clipboard_fts MATCH ? {cursor_clause}
                 ORDER BY CASE WHEN e.history_pinned_at IS NULL THEN 1 ELSE 0 END ASC,
                          relevance DESC,
                          CASE WHEN e.history_pinned_at IS NULL THEN e.updated_at END DESC,
                          e.id DESC LIMIT ?"
            );
            let mut values = vec![Value::Text(fts_match_query(trimmed))];
            values.extend(cursor_values);
            values.push(Value::Integer(fetch_limit));
            let mut statement = self.connection.prepare(&sql)?;
            let rows = statement.query_map(params_from_iter(values), |row| {
                Ok((map_entry(row)?, row.get::<_, i64>(13)?))
            })?;
            for row in rows {
                let (entry, relevance) = row?;
                entries.push(entry);
                relevances.push(Some(relevance));
            }
        }
        for entry in &mut entries {
            entry.thumbnail = self.thumbnail_for_entry(entry.id)?;
        }
        let has_more = entries.len() > usize::try_from(page_size).unwrap_or(usize::MAX);
        entries.truncate(usize::try_from(page_size).unwrap_or(usize::MAX));
        let next_cursor = has_more
            .then(|| {
                entries.last().map(|entry| {
                    if trimmed.is_empty() {
                        PageCursor::History {
                            pinned_at: entry.pinned_at,
                            updated_at: entry.updated_at,
                            id: entry.id,
                        }
                    } else {
                        PageCursor::HistorySearch {
                            pinned_at: entry.pinned_at,
                            relevance: relevances
                                .get(usize::try_from(page_size).unwrap_or(usize::MAX) - 1)
                                .and_then(|value| *value)
                                .unwrap_or_default(),
                            updated_at: entry.updated_at,
                            id: entry.id,
                        }
                    }
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
                        sanitized_html, fingerprint, history_pinned_at, byte_size
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
        let Some(bytes) = self.valid_thumbnail_bytes(&metadata) else {
            return Ok(None);
        };
        Ok(Some(StoredThumbnail { metadata, bytes }))
    }

    fn preview_disposition_for_source(&self, source_hash: &str) -> Result<PreviewDisposition> {
        validate_hash(source_hash)?;
        let metadata = self
            .connection
            .query_row(
                "SELECT source_hash, thumbnail_hash, mime_type, width, height, byte_size
                 FROM preview_assets WHERE source_hash = ?",
                [source_hash],
                map_thumbnail,
            )
            .optional()?;
        Ok(match metadata {
            Some(metadata) if self.valid_thumbnail_bytes(&metadata).is_some() => {
                PreviewDisposition::ReuseExisting
            }
            _ => PreviewDisposition::Generate,
        })
    }

    fn valid_thumbnail_bytes(&self, metadata: &Thumbnail) -> Option<Vec<u8>> {
        if validate_hash(&metadata.source_hash).is_err()
            || validate_hash(&metadata.content_hash).is_err()
            || metadata.mime_type != THUMBNAIL_MIME_TYPE
            || metadata.width == 0
            || metadata.height == 0
            || metadata.width > 256
            || metadata.height > 256
        {
            return None;
        }
        let bytes = fs::read(self.thumbnails_dir.join(&metadata.content_hash)).ok()?;
        (metadata.byte_size == bytes.len() as u64 && hash_bytes(&bytes) == metadata.content_hash)
            .then_some(bytes)
    }

    pub fn move_history_to_favorite(&mut self, history_id: i64) -> Result<SavedItem> {
        self.move_history_many_to_favorites(&[history_id])?
            .into_iter()
            .next()
            .ok_or_else(|| {
                StorageError::Invalid(format!("history entry {history_id} does not exist"))
            })
    }

    pub fn move_history_many_to_favorites(
        &mut self,
        history_ids: &[i64],
    ) -> Result<Vec<SavedItem>> {
        let mut ids = Vec::with_capacity(history_ids.len());
        for id in history_ids.iter().copied().filter(|id| *id > 0) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        // Resolve and validate the source before opening the transaction. The
        // representation bytes are never rebuilt from the preview; the SQL
        // move below copies every original representation row verbatim.
        let mut sources = Vec::with_capacity(ids.len());
        for id in &ids {
            let stored = self.entry(*id)?.ok_or_else(|| {
                StorageError::Invalid(format!("history entry {id} does not exist"))
            })?;
            if stored.representations.is_empty() {
                return Err(StorageError::Invalid(format!(
                    "history entry {id} has no representations"
                )));
            }
            let payload = self.entry_payload(*id)?;
            let mut entry = stored.entry;
            entry.byte_size = payload
                .iter()
                .try_fold(0_u64, |total, representation| {
                    total.checked_add(representation.bytes.len() as u64)
                })
                .ok_or_else(|| {
                    StorageError::Invalid("history payload byte size overflow".to_owned())
                })?;
            sources.push(entry);
        }

        let tx = self.connection.transaction()?;
        let mut saved_ids = Vec::with_capacity(sources.len());
        for entry in &sources {
            let existing: Option<i64> = tx
                .query_row(
                    "SELECT id FROM saved_items WHERE source_history_id = ?",
                    [entry.id],
                    |row| row.get(0),
                )
                .optional()?;
            let saved_id = if let Some(saved_id) = existing {
                Self::verify_saved_representations_tx(&tx, &self.blobs_dir, saved_id)?;
                saved_id
            } else {
                let draft = SavedItemDraft::from_history(entry);
                tx.execute(
                    "UPDATE saved_items SET favorite_order = favorite_order + 1",
                    [],
                )?;
                tx.execute(
                    "INSERT INTO saved_items
                     (source_history_id, created_at, updated_at, name, content_type, editable_text,
                      source_app, source_executable, source_window_title, preview_text, byte_size,
                      icon_key, favorite_order, is_independent)
                     VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0, 0)",
                    params![
                        entry.id,
                        draft.created_at,
                        draft.updated_at,
                        draft.name,
                        draft.content_type,
                        draft.editable_text,
                        draft.source_app,
                        draft.source_executable,
                        draft.source_window_title,
                        draft.preview_text,
                        i64::try_from(entry.byte_size).unwrap_or(i64::MAX),
                        draft.icon_key,
                    ],
                )?;
                let saved_id = tx.last_insert_rowid();
                let copied = tx.execute(
                    "INSERT INTO saved_item_representations
                     (saved_item_id, format, mime_type, inline_data, blob_hash, content_hash, byte_size)
                     SELECT ?, format, mime_type, inline_data, blob_hash, content_hash, byte_size
                     FROM clipboard_representations WHERE entry_id = ?",
                    params![saved_id, entry.id],
                )?;
                if copied == 0 {
                    return Err(StorageError::Invalid(format!(
                        "history entry {} representations disappeared",
                        entry.id
                    )));
                }
                replace_tags_tx(&tx, saved_id, &[])?;
                refresh_saved_search_tx(&tx, saved_id)?;
                saved_id
            };
            tx.execute(
                "DELETE FROM clipboard_fts WHERE entry_id = ?",
                [entry.id.to_string()],
            )?;
            tx.execute("DELETE FROM clipboard_entries WHERE id = ?", [entry.id])?;
            tx.execute(
                "UPDATE saved_items SET source_history_id = NULL WHERE id = ?",
                [saved_id],
            )?;
            saved_ids.push(saved_id);
        }
        tx.commit()?;
        self.schedule_blob_gc()?;
        saved_ids
            .into_iter()
            .map(|id| {
                self.saved_item(id)?
                    .map(|stored| stored.item)
                    .ok_or_else(|| StorageError::Invalid(format!("saved item {id} disappeared")))
            })
            .collect()
    }

    pub fn create_favorite(&mut self, draft: FavoriteDraft) -> Result<SavedItem> {
        let draft = draft
            .normalize()
            .map_err(|error| StorageError::Invalid(error.to_string()))?;
        let now = now_millis();
        let name = draft
            .name
            .clone()
            .unwrap_or_else(|| default_favorite_name_from_content(&draft.content));
        let icon_key = draft.icon_key.clone();
        let tags = draft.tags.clone();
        let representation = ClipboardRepresentation {
            format: "text".to_owned(),
            mime_type: "text/plain;charset=utf-8".to_owned(),
            bytes: draft.content.as_bytes().to_vec(),
        };
        let prepared = self.prepare_representation(&representation, None)?;
        let tx = self.connection.transaction()?;
        tx.execute(
            "UPDATE saved_items SET favorite_order = favorite_order + 1",
            [],
        )?;
        tx.execute(
            "INSERT INTO saved_items
             (source_history_id, created_at, updated_at, name, content_type, editable_text,
              source_app, source_executable, source_window_title, preview_text, byte_size,
              icon_key, favorite_order, is_independent)
             VALUES (NULL, ?, ?, ?, 'text', ?, NULL, NULL, NULL, ?, ?, ?, 0, 1)",
            params![
                now,
                now,
                name,
                draft.content,
                preview_text(&draft.content),
                i64::try_from(representation.bytes.len()).unwrap_or(i64::MAX),
                icon_key,
            ],
        )?;
        let id = tx.last_insert_rowid();
        Self::insert_saved_representation_tx(&tx, id, &prepared)?;
        replace_tags_tx(&tx, id, &tags)?;
        refresh_saved_search_tx(&tx, id)?;
        tx.commit()?;
        self.schedule_blob_gc()?;
        self.saved_item(id)?
            .map(|stored| stored.item)
            .ok_or_else(|| StorageError::Invalid(format!("saved item {id} disappeared")))
    }

    pub fn update_favorite(&mut self, id: i64, update: FavoriteUpdate) -> Result<SavedItem> {
        let update = update
            .normalize()
            .map_err(|error| StorageError::Invalid(error.to_string()))?;
        let current = self
            .saved_item(id)?
            .ok_or_else(|| StorageError::Invalid(format!("saved item {id} does not exist")))?;
        let name = update
            .name
            .clone()
            .unwrap_or_else(|| current.item.name.clone());
        let icon_key = update.icon_key.clone();
        let tags = update.tags.clone();
        let editable_text = if is_text_like(&current.item.content_type) {
            update.editable_text.clone()
        } else if update.editable_text.is_some() {
            return Err(StorageError::Invalid(
                "binary favorites do not support content edits".to_owned(),
            ));
        } else {
            None
        };
        let text_payload = editable_text.as_ref().map(|text| ClipboardRepresentation {
            format: "text".to_owned(),
            mime_type: "text/plain;charset=utf-8".to_owned(),
            bytes: text.as_bytes().to_vec(),
        });
        let prepared = text_payload
            .as_ref()
            .map(|representation| self.prepare_representation(representation, None))
            .transpose()?;
        let byte_size = text_payload
            .as_ref()
            .map(|representation| representation.bytes.len() as u64)
            .unwrap_or(current.item.byte_size);
        let preview = editable_text.as_deref().map(preview_text);
        let tx = self.connection.transaction()?;
        let changed = tx.execute(
            "UPDATE saved_items SET updated_at = ?, name = ?, icon_key = ?,
             editable_text = CASE WHEN ? IS NULL THEN editable_text ELSE ? END,
             preview_text = CASE WHEN ? IS NULL THEN preview_text ELSE ? END,
             byte_size = ?, is_independent = 1 WHERE id = ?",
            params![
                now_millis(),
                name,
                icon_key,
                editable_text,
                editable_text,
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
        replace_tags_tx(&tx, id, &tags)?;
        refresh_saved_search_tx(&tx, id)?;
        tx.commit()?;
        self.schedule_blob_gc()?;
        self.saved_item(id)?
            .map(|stored| stored.item)
            .ok_or_else(|| StorageError::Invalid(format!("saved item {id} disappeared")))
    }

    pub fn pin_history(&mut self, id: i64) -> Result<bool> {
        let tx = self.connection.transaction()?;
        let changed = tx.execute(
            "UPDATE clipboard_entries
             SET history_pinned_at = ?
             WHERE id = ? AND history_pinned_at IS NULL",
            params![next_pin_order_tx(&tx)?, id],
        )? > 0;
        tx.commit()?;
        Ok(changed)
    }

    pub fn unpin_history(&mut self, id: i64) -> Result<bool> {
        let tx = self.connection.transaction()?;
        let changed = tx.execute(
            "UPDATE clipboard_entries SET history_pinned_at = NULL
             WHERE id = ? AND history_pinned_at IS NOT NULL",
            [id],
        )? > 0;
        tx.commit()?;
        Ok(changed)
    }

    pub fn pin_history_many(&mut self, ids: &[i64]) -> Result<usize> {
        let mut ids = ids.iter().copied().filter(|id| *id > 0).collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        let tx = self.connection.transaction()?;
        let mut changed = 0;
        for id in ids {
            let pin_order = next_pin_order_tx(&tx)?;
            changed += tx.execute(
                "UPDATE clipboard_entries SET history_pinned_at = ?
                 WHERE id = ? AND history_pinned_at IS NULL",
                params![pin_order, id],
            )?;
        }
        tx.commit()?;
        Ok(changed)
    }

    pub fn delete_history_many(&mut self, ids: &[i64]) -> Result<usize> {
        let ids = ids.iter().copied().filter(|id| *id > 0).collect::<Vec<_>>();
        if ids.is_empty() {
            return Ok(0);
        }
        let placeholders = std::iter::repeat("?")
            .take(ids.len())
            .collect::<Vec<_>>()
            .join(", ");
        let tx = self.connection.transaction()?;
        let fts_sql = format!("DELETE FROM clipboard_fts WHERE entry_id IN ({placeholders})");
        let values = ids.iter().map(|id| Value::Integer(*id)).collect::<Vec<_>>();
        // FTS stores the integer id as text, so bind the string form here.
        let fts_values = ids
            .iter()
            .map(|id| Value::Text(id.to_string()))
            .collect::<Vec<_>>();
        tx.execute(&fts_sql, params_from_iter(fts_values))?;
        let sql = format!("DELETE FROM clipboard_entries WHERE id IN ({placeholders})");
        let deleted = tx.execute(&sql, params_from_iter(values))?;
        tx.commit()?;
        if deleted != 0 {
            self.schedule_blob_gc()?;
        }
        Ok(deleted)
    }

    pub fn clear_unpinned_history(&mut self) -> Result<usize> {
        let tx = self.connection.transaction()?;
        tx.execute(
            "DELETE FROM clipboard_fts WHERE entry_id IN
                    (SELECT id FROM clipboard_entries WHERE history_pinned_at IS NULL)",
            [],
        )?;
        let deleted = tx.execute(
            "DELETE FROM clipboard_entries WHERE history_pinned_at IS NULL",
            [],
        )?;
        tx.commit()?;
        self.schedule_blob_gc()?;
        Ok(deleted)
    }

    pub fn reorder_favorites(&mut self, ordered_ids: &[i64]) -> Result<()> {
        if ordered_ids.iter().any(|id| *id <= 0) {
            return Err(StorageError::Invalid(
                "favorite order contains an invalid id".to_owned(),
            ));
        }
        let mut ids = ordered_ids.to_vec();
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        if unique.len() != ids.len() {
            return Err(StorageError::Invalid(
                "favorite order contains duplicate ids".to_owned(),
            ));
        }
        let tx = self.connection.transaction()?;
        let expected = tx.query_row("SELECT COUNT(*) FROM saved_items", [], |row| {
            row.get::<_, i64>(0)
        })?;
        if expected != ids.len() as i64 {
            return Err(StorageError::Invalid(
                "favorite order must include every favorite exactly once".to_owned(),
            ));
        }
        for id in &ids {
            let exists: bool = tx.query_row(
                "SELECT EXISTS (SELECT 1 FROM saved_items WHERE id = ?)",
                [id],
                |row| row.get::<_, i64>(0),
            )? != 0;
            if !exists {
                return Err(StorageError::Invalid(format!(
                    "saved item {id} does not exist"
                )));
            }
        }
        // Use a temporary offset to avoid unique/order collisions if a future
        // schema adds a uniqueness constraint to favorite_order.
        tx.execute(
            "UPDATE saved_items SET favorite_order = favorite_order + ?",
            [expected + 1],
        )?;
        for (order, id) in ids.drain(..).enumerate() {
            tx.execute(
                "UPDATE saved_items SET favorite_order = ? WHERE id = ?",
                params![i64::try_from(order).unwrap_or(i64::MAX), id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_favorite(&mut self, id: i64) -> Result<bool> {
        Ok(self.delete_saved_items(&[id])? == 1)
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
        let select = "SELECT s.id, s.source_history_id, s.created_at, s.updated_at, s.name,
                    s.content_type, s.editable_text, s.source_app, s.source_executable,
                    s.source_window_title, s.preview_text, s.byte_size, s.icon_key,
                    s.favorite_order
             FROM saved_items s";
        let mut items = Vec::new();
        let mut relevances = Vec::new();
        if trimmed.is_empty() {
            let (cursor_clause, cursor_values) = favorites_cursor_filter(cursor)?;
            let sql = format!(
                "{select}
                 WHERE 1 = 1 {cursor_clause}
                 ORDER BY s.favorite_order ASC, s.id ASC LIMIT ?"
            );
            let mut values = cursor_values;
            values.push(Value::Integer(fetch_limit));
            let mut statement = self.connection.prepare(&sql)?;
            let rows = statement.query_map(params_from_iter(values), map_saved_item)?;
            for row in rows {
                items.push(row?);
            }
        } else if trimmed.chars().count() < 3 {
            let query_lower = trimmed.to_lowercase();
            let short_rank = "CASE WHEN lower(f.document) = ? THEN 0
                                    WHEN lower(f.document) LIKE ? || '%' THEN 1
                                    ELSE 2 END";
            let (cursor_clause, cursor_values) = favorites_search_cursor_filter_with_rank(
                cursor,
                short_rank,
                &[
                    Value::Text(query_lower.clone()),
                    Value::Text(query_lower.clone()),
                ],
                RankOrder::Ascending,
            )?;
            let pattern = format!("%{}%", escape_like_pattern(&query_lower));
            let sql = format!(
                "SELECT s.id, s.source_history_id, s.created_at, s.updated_at, s.name,
                        s.content_type, s.editable_text, s.source_app, s.source_executable,
                        s.source_window_title, s.preview_text, s.byte_size, s.icon_key,
                        s.favorite_order, {short_rank} AS relevance
                 FROM saved_items s
                 JOIN saved_items_fts f ON CAST(f.saved_item_id AS INTEGER) = s.id
                 WHERE lower(f.document) LIKE ? ESCAPE '\\' {cursor_clause}
                 ORDER BY relevance ASC, s.favorite_order ASC, s.id ASC LIMIT ?"
            );
            let mut values = vec![
                Value::Text(query_lower.clone()),
                Value::Text(query_lower),
                Value::Text(pattern),
            ];
            values.extend(cursor_values);
            values.push(Value::Integer(fetch_limit));
            let mut statement = self.connection.prepare(&sql)?;
            let rows = statement.query_map(params_from_iter(values), |row| {
                Ok((map_saved_item(row)?, row.get::<_, i64>(14)?))
            })?;
            for row in rows {
                let (item, relevance) = row?;
                items.push(item);
                relevances.push(relevance);
            }
        } else {
            let (cursor_clause, cursor_values) = favorites_search_cursor_filter_with_rank(
                cursor,
                "CAST((-bm25(saved_items_fts)) * 1000000 AS INTEGER)",
                &[],
                RankOrder::Descending,
            )?;
            let sql = format!(
                "SELECT s.id, s.source_history_id, s.created_at, s.updated_at, s.name,
                        s.content_type, s.editable_text, s.source_app, s.source_executable,
                        s.source_window_title, s.preview_text, s.byte_size, s.icon_key,
                        s.favorite_order,
                        CAST((-bm25(saved_items_fts)) * 1000000 AS INTEGER) AS relevance
                 FROM saved_items s
                 JOIN saved_items_fts f ON CAST(f.saved_item_id AS INTEGER) = s.id
                 WHERE saved_items_fts MATCH ? {cursor_clause}
                 ORDER BY relevance DESC, s.favorite_order ASC, s.id ASC LIMIT ?"
            );
            let mut values = vec![Value::Text(fts_match_query(trimmed))];
            values.extend(cursor_values);
            values.push(Value::Integer(fetch_limit));
            let mut statement = self.connection.prepare(&sql)?;
            let rows = statement.query_map(params_from_iter(values), |row| {
                Ok((map_saved_item(row)?, row.get::<_, i64>(14)?))
            })?;
            for row in rows {
                let (item, relevance) = row?;
                items.push(item);
                relevances.push(relevance);
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
                items.last().map(|item| {
                    if trimmed.is_empty() {
                        PageCursor::Favorites {
                            favorite_order: item.favorite_order,
                            id: item.id,
                        }
                    } else {
                        PageCursor::FavoritesSearch {
                            relevance: relevances
                                .get(usize::try_from(page_size).unwrap_or(usize::MAX) - 1)
                                .copied()
                                .unwrap_or_default(),
                            favorite_order: item.favorite_order,
                            id: item.id,
                        }
                    }
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
                        preview_text, byte_size, icon_key, favorite_order
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
                        icon_key: row.get(12)?,
                        favorite_order: row.get(13)?,
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
    sender: Mutex<Option<SyncSender<WriteRequest>>>,
    handle: Mutex<Option<thread::JoinHandle<()>>>,
}

impl WriterRuntime {
    fn open(data_dir: &Path, metrics: Arc<OperationMetrics>) -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(32);
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let data_dir = data_dir.to_path_buf();
        let handle = thread::Builder::new()
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
            Ok(Ok(())) => Ok(Self {
                sender: Mutex::new(Some(sender)),
                handle: Mutex::new(Some(handle)),
            }),
            Ok(Err(error)) => {
                drop(sender);
                let _ = handle.join();
                Err(StorageError::Invalid(error))
            }
            Err(error) => {
                drop(sender);
                let _ = handle.join();
                Err(StorageError::Invalid(format!(
                    "storage writer did not start: {error}"
                )))
            }
        }
    }

    fn execute<T, F>(&self, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut ClipboardStore) -> Result<T> + Send + 'static,
    {
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .cloned()
            .ok_or_else(storage_runtime_shutdown_error)?;
        sender
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

    fn shutdown(&self) -> Result<()> {
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        drop(sender);

        let handle = self
            .handle
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(handle) = handle {
            handle.join().map_err(|_| {
                StorageError::Invalid("storage writer thread panicked during shutdown".to_owned())
            })?;
        }
        Ok(())
    }
}

fn writer_loop(store: &mut ClipboardStore, receiver: mpsc::Receiver<WriteRequest>) {
    while let Ok(request) = receiver.recv() {
        let result = (request.operation)(store);
        let _ = request.response.send(result);
    }
}

struct ReaderRuntime {
    store: Mutex<Option<ClipboardStore>>,
}

impl ReaderRuntime {
    fn open(data_dir: &Path, metrics: Arc<OperationMetrics>) -> Result<Self> {
        Ok(Self {
            store: Mutex::new(Some(ClipboardStore::open_read_only(data_dir, metrics)?)),
        })
    }

    fn read<T>(&self, operation: impl FnOnce(&ClipboardStore) -> Result<T>) -> Result<T> {
        let store = self
            .store
            .lock()
            .map_err(|_| StorageError::Invalid("storage reader is unavailable".to_owned()))?;
        let store = store.as_ref().ok_or_else(storage_runtime_shutdown_error)?;
        operation(store)
    }

    fn shutdown(&self) {
        let store = self
            .store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        drop(store);
    }
}

struct MaintenanceRuntime {
    sender: Mutex<Option<SyncSender<Duration>>>,
    handle: Mutex<Option<thread::JoinHandle<()>>>,
}

impl MaintenanceRuntime {
    fn new(writer: Arc<WriterRuntime>, metrics: Arc<OperationMetrics>) -> Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(32);
        let handle = thread::Builder::new()
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
            .map_err(|error| {
                StorageError::Invalid(format!("cannot start storage maintenance: {error}"))
            })?;
        Ok(Self {
            sender: Mutex::new(Some(sender)),
            handle: Mutex::new(Some(handle)),
        })
    }

    fn request(&self, delay: Duration) {
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .cloned();
        if let Some(sender) = sender {
            let _ = sender.try_send(delay);
        }
    }

    fn shutdown(&self) -> Result<()> {
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        drop(sender);

        let handle = self
            .handle
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(handle) = handle {
            handle.join().map_err(|_| {
                StorageError::Invalid(
                    "storage maintenance thread panicked during shutdown".to_owned(),
                )
            })?;
        }
        Ok(())
    }
}

const STORAGE_RUNTIME_SHUTDOWN_ERROR: &str = "storage runtime is shut down";

fn storage_runtime_shutdown_error() -> StorageError {
    StorageError::Invalid(STORAGE_RUNTIME_SHUTDOWN_ERROR.to_owned())
}

struct RuntimeState {
    accepting: bool,
    active: usize,
    shutdown_complete: bool,
}

struct SharedRuntime {
    writer: Arc<WriterRuntime>,
    reader: Arc<ReaderRuntime>,
    maintenance: Arc<MaintenanceRuntime>,
    metrics: Arc<OperationMetrics>,
    admission: Mutex<RuntimeState>,
    admission_changed: Condvar,
    shutdown_lock: Mutex<()>,
}

struct AdmissionGuard<'a> {
    runtime: &'a SharedRuntime,
}

impl Drop for AdmissionGuard<'_> {
    fn drop(&mut self) {
        let mut state = self
            .runtime
            .admission
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.active = state.active.saturating_sub(1);
        self.runtime.admission_changed.notify_all();
    }
}

impl SharedRuntime {
    fn enter(&self) -> Result<AdmissionGuard<'_>> {
        let mut state = self
            .admission
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !state.accepting {
            return Err(storage_runtime_shutdown_error());
        }
        state.active += 1;
        drop(state);
        Ok(AdmissionGuard { runtime: self })
    }

    fn shutdown(&self) -> Result<()> {
        let _shutdown_lock = self
            .shutdown_lock
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        {
            let mut state = self
                .admission
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if state.shutdown_complete {
                return Ok(());
            }
            state.accepting = false;
            while state.active != 0 {
                state = self
                    .admission_changed
                    .wait(state)
                    .unwrap_or_else(|error| error.into_inner());
            }
        }

        let mut first_error = None;
        if let Err(error) = self.maintenance.shutdown() {
            first_error = Some(error);
        }

        let final_reconcile = self.writer.execute(|store| {
            if store.maintenance_pending()? {
                store.reconcile_blob_store()?;
            }
            Ok::<_, StorageError>(())
        });
        if let Err(error) = final_reconcile {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }

        if let Err(error) = self.writer.shutdown() {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }
        self.reader.shutdown();

        self.admission
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .shutdown_complete = true;
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

#[derive(Clone)]
pub struct SharedClipboardStore {
    runtime: Arc<SharedRuntime>,
}

impl SharedClipboardStore {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self> {
        let data_dir = data_dir.as_ref().to_path_buf();
        let metrics = Arc::new(OperationMetrics::default());
        let writer = Arc::new(WriterRuntime::open(&data_dir, Arc::clone(&metrics))?);
        let reader = match ReaderRuntime::open(&data_dir, Arc::clone(&metrics)) {
            Ok(reader) => Arc::new(reader),
            Err(error) => {
                let _ = writer.shutdown();
                return Err(error);
            }
        };
        let maintenance = match MaintenanceRuntime::new(Arc::clone(&writer), Arc::clone(&metrics)) {
            Ok(maintenance) => Arc::new(maintenance),
            Err(error) => {
                let _ = writer.shutdown();
                reader.shutdown();
                return Err(error);
            }
        };
        let shared = Self {
            runtime: Arc::new(SharedRuntime {
                writer,
                reader,
                maintenance,
                metrics,
                admission: Mutex::new(RuntimeState {
                    accepting: true,
                    active: 0,
                    shutdown_complete: false,
                }),
                admission_changed: Condvar::new(),
                shutdown_lock: Mutex::new(()),
            }),
        };
        shared.request_maintenance_now();
        Ok(shared)
    }

    pub fn shutdown(&self) -> Result<()> {
        self.runtime.shutdown()
    }

    fn with_store<T, F>(&self, operation: F) -> Result<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut ClipboardStore) -> Result<T> + Send + 'static,
    {
        let _admission = self.runtime.enter()?;
        self.runtime.writer.execute(operation)
    }

    pub fn metrics_snapshot(&self) -> Vec<OperationMetric> {
        self.runtime.metrics.snapshot()
    }

    fn request_maintenance(&self) {
        self.runtime.maintenance.request(Duration::from_millis(100));
    }

    fn request_maintenance_now(&self) {
        self.runtime.maintenance.request(Duration::ZERO);
    }

    fn request_after_pending_write(&self, pending: bool) {
        if pending {
            self.request_maintenance();
        }
    }

    pub fn settings(&self) -> Result<ClipboardSettings> {
        let _admission = self.runtime.enter()?;
        self.runtime.reader.read(|store| store.settings())
    }

    pub fn update_settings(&self, settings: &ClipboardSettings) -> Result<()> {
        let settings = settings.clone();
        self.with_store(move |store| store.update_settings(&settings))
    }

    pub fn list_entries(&self, query: &str, limit: u32) -> Result<Vec<ClipboardEntry>> {
        let query = query.to_owned();
        let _admission = self.runtime.enter()?;
        self.runtime
            .reader
            .read(|store| store.list_entries(&query, limit))
    }

    pub fn list_entries_page(
        &self,
        query: &str,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> Result<LibraryPage<ClipboardEntry>> {
        let query = query.to_owned();
        let _admission = self.runtime.enter()?;
        self.runtime
            .reader
            .read(|store| store.list_entries_page(&query, limit, cursor))
    }

    pub fn entry(&self, id: i64) -> Result<Option<StoredClipboardEntry>> {
        let _admission = self.runtime.enter()?;
        self.runtime.reader.read(|store| store.entry(id))
    }

    pub fn history_entry(&self, id: i64) -> Result<Option<ClipboardEntry>> {
        self.entry(id).map(|entry| entry.map(|stored| stored.entry))
    }

    pub fn entry_payload(&self, id: i64) -> Result<Vec<ClipboardRepresentation>> {
        let _admission = self.runtime.enter()?;
        self.runtime.reader.read(|store| store.entry_payload(id))
    }

    pub fn read_thumbnail(&self, content_hash: &str) -> Result<Option<StoredThumbnail>> {
        let content_hash = content_hash.to_owned();
        let _admission = self.runtime.enter()?;
        self.runtime
            .reader
            .read(|store| store.read_thumbnail(&content_hash))
    }

    pub fn list_saved_items(&self, query: &str, limit: u32) -> Result<Vec<SavedItem>> {
        let query = query.to_owned();
        let _admission = self.runtime.enter()?;
        self.runtime
            .reader
            .read(|store| store.list_saved_items(&query, limit))
    }

    pub fn list_saved_items_page(
        &self,
        query: &str,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> Result<LibraryPage<SavedItem>> {
        let query = query.to_owned();
        let _admission = self.runtime.enter()?;
        self.runtime
            .reader
            .read(|store| store.list_saved_items_page(&query, limit, cursor))
    }

    pub fn saved_item(&self, id: i64) -> Result<Option<StoredSavedItem>> {
        let _admission = self.runtime.enter()?;
        self.runtime.reader.read(|store| store.saved_item(id))
    }

    pub fn saved_item_payload(&self, id: i64) -> Result<Vec<ClipboardRepresentation>> {
        let _admission = self.runtime.enter()?;
        self.runtime
            .reader
            .read(|store| store.saved_item_payload(id))
    }

    pub fn move_history_to_favorite(&self, id: i64) -> Result<SavedItem> {
        let saved = self.with_store(move |store| store.move_history_to_favorite(id))?;
        self.request_maintenance();
        Ok(saved)
    }

    pub fn move_history_many_to_favorites(&self, ids: &[i64]) -> Result<Vec<SavedItem>> {
        let ids = ids.to_vec();
        let saved = self.with_store(move |store| store.move_history_many_to_favorites(&ids))?;
        if !saved.is_empty() {
            self.request_maintenance();
        }
        Ok(saved)
    }

    pub fn create_favorite(&self, draft: FavoriteDraft) -> Result<SavedItem> {
        let saved = self.with_store(move |store| store.create_favorite(draft))?;
        self.request_maintenance();
        Ok(saved)
    }

    pub fn update_favorite(&self, id: i64, update: FavoriteUpdate) -> Result<SavedItem> {
        let saved = self.with_store(move |store| store.update_favorite(id, update))?;
        self.request_maintenance();
        Ok(saved)
    }

    pub fn pin_history(&self, id: i64) -> Result<bool> {
        self.with_store(move |store| store.pin_history(id))
    }

    pub fn unpin_history(&self, id: i64) -> Result<bool> {
        self.with_store(move |store| store.unpin_history(id))
    }

    pub fn pin_history_many(&self, ids: &[i64]) -> Result<usize> {
        let ids = ids.to_vec();
        self.with_store(move |store| store.pin_history_many(&ids))
    }

    pub fn delete_history_many(&self, ids: &[i64]) -> Result<usize> {
        let ids = ids.to_vec();
        let deleted = self.with_store(move |store| store.delete_history_many(&ids))?;
        if deleted != 0 {
            self.request_maintenance();
        }
        Ok(deleted)
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

    pub fn clear_unpinned_history(&self) -> Result<usize> {
        let deleted = self.with_store(|store| store.clear_unpinned_history())?;
        self.request_maintenance();
        Ok(deleted)
    }

    pub fn reorder_favorites(&self, ordered_ids: &[i64]) -> Result<()> {
        let ids = ordered_ids.to_vec();
        self.with_store(move |store| store.reorder_favorites(&ids))
    }

    pub fn delete_favorite(&self, id: i64) -> Result<bool> {
        let deleted = self.with_store(move |store| store.delete_favorite(id))?;
        if deleted {
            self.request_maintenance();
        }
        Ok(deleted)
    }

    pub fn reconcile_blob_store(&self) -> Result<()> {
        self.with_store(|store| store.reconcile_blob_store())
    }
}

impl Drop for SharedClipboardStore {
    fn drop(&mut self) {
        if Arc::strong_count(&self.runtime) == 1 {
            let _ = self.runtime.shutdown();
        }
    }
}

impl ClipboardSink for SharedClipboardStore {
    fn settings(&self) -> std::result::Result<CaptureSettings, String> {
        self.settings()
            .map(|settings| CaptureSettings::from(&settings))
            .map_err(|error| error.to_string())
    }

    fn record(&self, capture: NormalizedCapture) -> std::result::Result<RecordResult, String> {
        let captured = CapturedCapture::from_capture(capture);
        let preview_disposition = captured
            .preview_source()
            .map(|source| self.preview_disposition(source))
            .transpose()?;
        let captured =
            generate_preview_if_requested(captured, preview_disposition, &self.runtime.metrics);
        self.record_captured(captured)
    }

    fn capture_policy(&self) -> std::result::Result<CapturePolicy, String> {
        self.settings()
            .map(|settings| CapturePolicy::from_max_item_bytes(settings.max_item_bytes))
            .map_err(|error| error.to_string())
    }

    fn preview_disposition(
        &self,
        source: &RepresentationIdentity,
    ) -> std::result::Result<PreviewDisposition, String> {
        let _admission = self.runtime.enter().map_err(|error| error.to_string())?;
        self.runtime
            .reader
            .read(|store| store.preview_disposition_for_source(&source.hash))
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

    fn move_history_to_favorite(
        &self,
        history_id: i64,
    ) -> std::result::Result<SavedItem, Self::Error> {
        SharedClipboardStore::move_history_to_favorite(self, history_id)
    }

    fn move_history_many_to_favorites(
        &self,
        history_ids: &[i64],
    ) -> std::result::Result<Vec<SavedItem>, Self::Error> {
        SharedClipboardStore::move_history_many_to_favorites(self, history_ids)
    }

    fn create_favorite(&self, draft: FavoriteDraft) -> std::result::Result<SavedItem, Self::Error> {
        SharedClipboardStore::create_favorite(self, draft)
    }

    fn update_favorite(
        &self,
        id: i64,
        update: FavoriteUpdate,
    ) -> std::result::Result<SavedItem, Self::Error> {
        SharedClipboardStore::update_favorite(self, id, update)
    }

    fn pin_history(&self, history_id: i64) -> std::result::Result<bool, Self::Error> {
        SharedClipboardStore::pin_history(self, history_id)
    }

    fn unpin_history(&self, history_id: i64) -> std::result::Result<bool, Self::Error> {
        SharedClipboardStore::unpin_history(self, history_id)
    }

    fn pin_history_many(&self, history_ids: &[i64]) -> std::result::Result<usize, Self::Error> {
        SharedClipboardStore::pin_history_many(self, history_ids)
    }

    fn delete_history_many(&self, history_ids: &[i64]) -> std::result::Result<usize, Self::Error> {
        SharedClipboardStore::delete_history_many(self, history_ids)
    }

    fn delete_entry(&self, id: i64) -> std::result::Result<bool, Self::Error> {
        SharedClipboardStore::delete_entry(self, id)
    }

    fn clear_unpinned_history(&self) -> std::result::Result<usize, Self::Error> {
        SharedClipboardStore::clear_unpinned_history(self)
    }

    fn reorder_favorites(&self, ordered_ids: &[i64]) -> std::result::Result<(), Self::Error> {
        SharedClipboardStore::reorder_favorites(self, ordered_ids)
    }

    fn delete_favorite(&self, id: i64) -> std::result::Result<bool, Self::Error> {
        SharedClipboardStore::delete_favorite(self, id)
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

fn history_cursor_filter(cursor: Option<PageCursor>) -> Result<(String, Vec<Value>)> {
    match cursor {
        None => Ok((String::new(), Vec::new())),
        Some(PageCursor::History {
            pinned_at: Some(pinned_at),
            id,
            ..
        }) => Ok((
            "AND (e.history_pinned_at IS NULL OR
                  (e.history_pinned_at < ? OR (e.history_pinned_at = ? AND e.id < ?)))"
                .to_owned(),
            vec![
                Value::Integer(pinned_at),
                Value::Integer(pinned_at),
                Value::Integer(id),
            ],
        )),
        Some(PageCursor::History {
            pinned_at: None,
            updated_at,
            id,
        }) => Ok((
            "AND e.history_pinned_at IS NULL
             AND (e.updated_at < ? OR (e.updated_at = ? AND e.id < ?))"
                .to_owned(),
            vec![
                Value::Integer(updated_at),
                Value::Integer(updated_at),
                Value::Integer(id),
            ],
        )),
        Some(_) => Err(StorageError::Invalid(
            "cursor does not belong to the History order".to_owned(),
        )),
    }
}

#[derive(Clone, Copy)]
enum RankOrder {
    Ascending,
    Descending,
}

impl RankOrder {
    fn after_operator(self) -> &'static str {
        match self {
            Self::Ascending => ">",
            Self::Descending => "<",
        }
    }
}

fn history_search_cursor_filter_with_rank(
    cursor: Option<PageCursor>,
    rank_expression: &str,
    rank_values: &[Value],
    rank_order: RankOrder,
) -> Result<(String, Vec<Value>)> {
    let after_operator = rank_order.after_operator();
    match cursor {
        None => Ok((String::new(), Vec::new())),
        Some(PageCursor::HistorySearch {
            pinned_at: Some(_),
            relevance,
            updated_at,
            id,
        }) => Ok((
            format!(
                "AND (e.history_pinned_at IS NULL OR
                      (e.history_pinned_at IS NOT NULL AND
                       ({rank_expression} {after_operator} ? OR
                        ({rank_expression} = ? AND
                         (e.updated_at < ? OR (e.updated_at = ? AND e.id < ?))))))"
            ),
            [
                rank_values.to_vec(),
                vec![Value::Integer(relevance)],
                rank_values.to_vec(),
                vec![
                    Value::Integer(relevance),
                    Value::Integer(updated_at),
                    Value::Integer(updated_at),
                    Value::Integer(id),
                ],
            ]
            .into_iter()
            .flatten()
            .collect(),
        )),
        Some(PageCursor::HistorySearch {
            pinned_at: None,
            relevance,
            updated_at,
            id,
        }) => Ok((
            format!(
                "AND e.history_pinned_at IS NULL AND
                      ({rank_expression} {after_operator} ? OR
                       ({rank_expression} = ? AND
                        (e.updated_at < ? OR (e.updated_at = ? AND e.id < ?))))"
            ),
            [
                rank_values.to_vec(),
                vec![Value::Integer(relevance)],
                rank_values.to_vec(),
                vec![
                    Value::Integer(relevance),
                    Value::Integer(updated_at),
                    Value::Integer(updated_at),
                    Value::Integer(id),
                ],
            ]
            .into_iter()
            .flatten()
            .collect(),
        )),
        Some(_) => Err(StorageError::Invalid(
            "cursor does not belong to the History search order".to_owned(),
        )),
    }
}

fn favorites_cursor_filter(cursor: Option<PageCursor>) -> Result<(String, Vec<Value>)> {
    match cursor {
        None => Ok((String::new(), Vec::new())),
        Some(PageCursor::Favorites { favorite_order, id }) => Ok((
            "AND (s.favorite_order > ? OR (s.favorite_order = ? AND s.id > ?))".to_owned(),
            vec![
                Value::Integer(favorite_order),
                Value::Integer(favorite_order),
                Value::Integer(id),
            ],
        )),
        Some(_) => Err(StorageError::Invalid(
            "cursor does not belong to the Favorites order".to_owned(),
        )),
    }
}

fn favorites_search_cursor_filter_with_rank(
    cursor: Option<PageCursor>,
    rank_expression: &str,
    rank_values: &[Value],
    rank_order: RankOrder,
) -> Result<(String, Vec<Value>)> {
    let after_operator = rank_order.after_operator();
    match cursor {
        None => Ok((String::new(), Vec::new())),
        Some(PageCursor::FavoritesSearch {
            relevance,
            favorite_order,
            id,
        }) => Ok((
            format!(
                "AND ({rank_expression} {after_operator} ? OR
                      ({rank_expression} = ? AND
                       (s.favorite_order > ? OR
                        (s.favorite_order = ? AND s.id > ?))))"
            ),
            [
                rank_values.to_vec(),
                vec![Value::Integer(relevance)],
                rank_values.to_vec(),
                vec![
                    Value::Integer(relevance),
                    Value::Integer(favorite_order),
                    Value::Integer(favorite_order),
                    Value::Integer(id),
                ],
            ]
            .into_iter()
            .flatten()
            .collect(),
        )),
        Some(_) => Err(StorageError::Invalid(
            "cursor does not belong to the Favorites search order".to_owned(),
        )),
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
        pinned_at: row.get(11)?,
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
        icon_key: row.get(12)?,
        favorite_order: row.get(13)?,
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

fn generate_preview_if_requested(
    captured: CapturedCapture,
    disposition: Option<PreviewDisposition>,
    metrics: &OperationMetrics,
) -> CapturedCapture {
    if disposition != Some(PreviewDisposition::Generate) {
        return captured;
    }
    let started = Instant::now();
    let captured = captured.prepare_preview();
    metrics.record(
        "thumbnail_generation",
        started.elapsed(),
        u64::from(captured.preview.is_some()),
    );
    captured
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

fn has_column(connection: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut statement = connection.prepare(&format!("PRAGMA table_info({table})"))?;
    let rows = statement.query_map([], |row| row.get::<_, String>(1))?;
    Ok(rows
        .collect::<std::result::Result<Vec<_>, _>>()?
        .iter()
        .any(|name| name == column))
}

fn default_saved_item_name(content_type: &str, preview: Option<&str>) -> String {
    preview
        .and_then(|value| value.lines().map(str::trim).find(|line| !line.is_empty()))
        .map(|value| value.chars().take(120).collect())
        .unwrap_or_else(|| match content_type {
            "image" => "Image".to_owned(),
            "files" => "Files".to_owned(),
            _ => "Saved item".to_owned(),
        })
}

fn default_favorite_name_from_content(content: &str) -> String {
    content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(120).collect())
        .unwrap_or_else(|| "Favorite".to_owned())
}

fn next_pin_order_tx(tx: &Transaction<'_>) -> Result<i64> {
    let now = now_millis();
    let maximum: i64 = tx.query_row(
        "SELECT COALESCE(MAX(history_pinned_at), 0) FROM clipboard_entries",
        [],
        |row| row.get(0),
    )?;
    Ok(now.max(maximum.saturating_add(1)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use echo_engine::{fingerprint, ClipboardRepresentation, ContentType, SourceContext};
    use tempfile::{tempdir_in, TempDir};

    fn disk_tempdir() -> TempDir {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/test-tmp/echo-storage");
        std::fs::create_dir_all(&root).unwrap();
        tempdir_in(root).unwrap()
    }

    fn memory_store() -> ClipboardStore {
        ClipboardStore::open_in_memory_for_tests("echo-storage-memory-test").unwrap()
    }

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
    fn shared_store_drop_releases_physical_database_handles() {
        let test_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/test-tmp/echo-storage");
        std::fs::create_dir_all(&test_root).unwrap();
        let root = tempfile::tempdir_in(test_root).unwrap();
        let data_dir = root.path().to_path_buf();
        let store = SharedClipboardStore::open(&data_dir).unwrap();
        let recorded =
            ClipboardSink::record(&store, text_capture("shutdown regression", 1)).unwrap();
        assert!(recorded.id > 0);
        drop(store);

        let renamed = data_dir.with_extension("renamed");
        std::fs::rename(&data_dir, &renamed)
            .expect("dropping SharedClipboardStore must release the database handles");
        std::fs::remove_dir_all(renamed).unwrap();
    }

    #[test]
    fn shared_store_shutdown_is_idempotent_and_rejects_new_work() {
        let root = disk_tempdir();
        let data_dir = root.path().to_path_buf();
        let store = SharedClipboardStore::open(&data_dir).unwrap();
        ClipboardSink::record(&store, text_capture("explicit shutdown", 1)).unwrap();
        for name in ["echo.sqlite3", "echo.sqlite3-wal", "echo.sqlite3-shm"] {
            assert!(
                data_dir.join(name).is_file(),
                "representative write did not create {name}"
            );
        }

        store.shutdown().unwrap();
        store.shutdown().unwrap();

        let error = store.list_entries("", 20).unwrap_err();
        assert!(matches!(
            error,
            StorageError::Invalid(message) if message == STORAGE_RUNTIME_SHUTDOWN_ERROR
        ));
        let error = store
            .update_settings(&ClipboardSettings::default())
            .unwrap_err();
        assert!(matches!(
            error,
            StorageError::Invalid(message) if message == STORAGE_RUNTIME_SHUTDOWN_ERROR
        ));
        let error = ClipboardSink::record(&store, text_capture("rejected", 2)).unwrap_err();
        assert!(error.ends_with(STORAGE_RUNTIME_SHUTDOWN_ERROR));

        drop(store);
        let renamed = data_dir.with_extension("shutdown-complete");
        std::fs::rename(&data_dir, &renamed)
            .expect("explicit shutdown must release the database handles");
        std::fs::remove_dir_all(renamed).unwrap();
    }

    #[test]
    fn shutdown_waits_for_an_admitted_write_before_joining_the_writer() {
        let root = disk_tempdir();
        let store = SharedClipboardStore::open(root.path()).unwrap();
        let writer = store.clone();
        let (entered, entered_receiver) = std::sync::mpsc::sync_channel(0);
        let (release, release_receiver) = std::sync::mpsc::sync_channel(0);
        let write_handle = std::thread::spawn(move || {
            writer
                .with_store(move |store| {
                    entered.send(()).unwrap();
                    release_receiver.recv().unwrap();
                    Ok::<_, StorageError>(store.record_capture(text_capture("drained", 1))?.id)
                })
                .unwrap()
        });
        entered_receiver
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap();

        let shutdown_store = store.clone();
        let (shutdown_done, shutdown_done_receiver) = std::sync::mpsc::sync_channel(0);
        let shutdown_handle = std::thread::spawn(move || {
            shutdown_done.send(shutdown_store.shutdown()).unwrap();
        });
        assert!(shutdown_done_receiver
            .recv_timeout(std::time::Duration::from_millis(50))
            .is_err());

        release.send(()).unwrap();
        assert!(shutdown_done_receiver
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap()
            .is_ok());
        assert!(write_handle.join().unwrap() > 0);
        shutdown_handle.join().unwrap();
    }

    #[test]
    fn record_deduplicates_and_refreshes_the_timestamp() {
        let mut store = memory_store();
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
        let root = disk_tempdir();
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
        let root = disk_tempdir();
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
        let mut store = memory_store();
        let id = store
            .record_capture(text_capture("favorite", 1))
            .unwrap()
            .id;
        let saved_item = store.move_history_to_favorite(id).unwrap();
        assert_eq!(store.clear_unpinned_history().unwrap(), 0);
        assert!(store.entry(id).unwrap().is_none());
        assert_eq!(store.list_saved_items("", 20).unwrap().len(), 1);
        let saved = store.list_saved_items("", 20).unwrap();
        assert_eq!(saved.len(), 1);
        assert!(saved[0].source_history_id.is_none());
        assert_eq!(saved[0].id, saved_item.id);
    }

    #[test]
    fn saved_text_edits_replace_the_canonical_payload_and_normalize_tags() {
        let mut store = memory_store();
        let history_id = store
            .record_capture(text_capture("original", 1))
            .unwrap()
            .id;
        let saved = store.move_history_to_favorite(history_id).unwrap();
        assert_eq!(saved.name, "original");
        assert_eq!(
            store.saved_item_payload(saved.id).unwrap()[0].bytes,
            b"original"
        );

        let updated = store
            .update_favorite(
                saved.id,
                FavoriteUpdate {
                    name: Some("  Canonical name  ".to_owned()),
                    icon_key: None,
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
        let root = disk_tempdir();
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
        let first = store.move_history_to_favorite(first_history).unwrap();
        let second = store.move_history_to_favorite(second_history).unwrap();
        let bytes = store.saved_item_payload(first.id).unwrap()[0].bytes.clone();
        store
            .update_favorite(
                first.id,
                FavoriteUpdate {
                    name: Some("renamed image".to_owned()),
                    icon_key: None,
                    tags: vec!["assets".to_owned()],
                    editable_text: None,
                },
            )
            .unwrap();
        assert_eq!(store.saved_item_payload(first.id).unwrap()[0].bytes, bytes);
        assert!(store
            .update_favorite(
                first.id,
                FavoriteUpdate {
                    name: None,
                    icon_key: None,
                    tags: Vec::new(),
                    editable_text: Some("must stay binary".to_owned()),
                },
            )
            .is_err());
        assert_eq!(store.saved_item_payload(first.id).unwrap()[0].bytes, bytes);
        assert_eq!(store.delete_saved_items(&[first.id, second.id]).unwrap(), 2);
        assert!(store.list_saved_items("", 20).unwrap().is_empty());
    }

    #[test]
    fn image_capture_persists_a_content_addressed_thumbnail_without_changing_original_bytes() {
        let root = disk_tempdir();
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
    fn low_level_capture_reuses_valid_preview_and_repairs_corrupt_metadata() {
        fn generation_samples(store: &ClipboardStore) -> u64 {
            store
                .metrics_snapshot()
                .into_iter()
                .find(|metric| metric.operation == "thumbnail_generation")
                .map(|metric| metric.samples)
                .unwrap_or(0)
        }

        let root = disk_tempdir();
        let mut store = ClipboardStore::open(root.path()).unwrap();
        let mut capture = text_capture("picture", 1);
        capture.content_type = ContentType::Image;
        capture.representations = vec![ClipboardRepresentation {
            format: "image".to_owned(),
            mime_type: "image/png".to_owned(),
            bytes: one_pixel_png(),
        }];
        capture.fingerprint = fingerprint(&capture.representations);
        let source_hash = hash_bytes(&capture.representations[0].bytes);

        let first = store.record_capture(capture.clone()).unwrap();
        capture.sequence = 2;
        assert!(store.record_capture(capture.clone()).unwrap().duplicate);
        assert_eq!(generation_samples(&store), 1);

        store
            .connection
            .execute(
                "UPDATE preview_assets SET thumbnail_hash = ? WHERE source_hash = ?",
                params!["0".repeat(64), source_hash],
            )
            .unwrap();
        capture.sequence = 3;
        assert!(store.record_capture(capture).unwrap().duplicate);
        assert_eq!(generation_samples(&store), 2);
        assert!(store.maintenance_pending().unwrap());
        let repaired = store
            .entry(first.id)
            .unwrap()
            .unwrap()
            .entry
            .thumbnail
            .unwrap();
        assert_ne!(repaired.content_hash, "0".repeat(64));
        assert!(store
            .read_thumbnail(&repaired.content_hash)
            .unwrap()
            .is_some());
    }

    #[test]
    fn fts_match_and_cursor_pages_are_bounded_and_stable() {
        let mut store = memory_store();
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
    fn move_history_to_favorite_is_atomic_and_preserves_original_representations() {
        let root = disk_tempdir();
        let mut store = ClipboardStore::open(root.path()).unwrap();
        let mut capture = text_capture("move payload", 1);
        capture.representations.push(ClipboardRepresentation {
            format: "application/octet-stream".to_owned(),
            mime_type: "application/octet-stream".to_owned(),
            bytes: vec![7; INLINE_LIMIT + 11],
        });
        capture.fingerprint = fingerprint(&capture.representations);
        let history_id = store.record_capture(capture).unwrap().id;
        let payload = store.entry_payload(history_id).unwrap();
        let favorite = store.move_history_to_favorite(history_id).unwrap();

        assert!(store.entry(history_id).unwrap().is_none());
        assert!(store.list_entries("", 20).unwrap().is_empty());
        assert_eq!(store.saved_item_payload(favorite.id).unwrap(), payload);
        assert_eq!(
            favorite.byte_size,
            payload
                .iter()
                .map(|item| item.bytes.len() as u64)
                .sum::<u64>()
        );
        assert_eq!(
            store
                .connection
                .query_row(
                    "SELECT COUNT(*) FROM clipboard_fts WHERE entry_id = ?",
                    [history_id.to_string()],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );
        assert_eq!(
            store
                .connection
                .query_row(
                    "SELECT COUNT(*) FROM saved_items_fts WHERE saved_item_id = ?",
                    [favorite.id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            1
        );
        store.reconcile_blob_store().unwrap();
        assert_eq!(store.saved_item_payload(favorite.id).unwrap(), payload);
    }

    #[test]
    fn move_history_to_favorite_rolls_back_when_history_delete_fails() {
        let mut store = memory_store();
        let history_id = store
            .record_capture(text_capture("rollback", 1))
            .unwrap()
            .id;
        store
            .connection
            .execute_batch(
                "CREATE TRIGGER fail_history_delete BEFORE DELETE ON clipboard_entries
                 BEGIN SELECT RAISE(ABORT, 'injected move failure'); END;",
            )
            .unwrap();
        assert!(store.move_history_to_favorite(history_id).is_err());
        store
            .connection
            .execute_batch("DROP TRIGGER fail_history_delete")
            .unwrap();
        assert!(store.entry(history_id).unwrap().is_some());
        assert!(store.list_saved_items("", 20).unwrap().is_empty());
        assert_eq!(store.list_entries("rollback", 20).unwrap().len(), 1);
    }

    #[test]
    fn pinned_history_is_stable_and_protected_from_capacity_and_clear_all() {
        let mut store = memory_store();
        let first = store.record_capture(text_capture("first", 1)).unwrap().id;
        let second = store.record_capture(text_capture("second", 2)).unwrap().id;
        let third = store.record_capture(text_capture("third", 3)).unwrap().id;
        assert_eq!(store.pin_history(first).unwrap(), true);
        assert_eq!(store.pin_history(second).unwrap(), true);
        assert_eq!(store.pin_history(second).unwrap(), false);
        let ordered = store.list_entries("", 20).unwrap();
        assert_eq!(
            ordered.iter().map(|entry| entry.id).collect::<Vec<_>>(),
            vec![second, first, third]
        );
        assert_eq!(store.unpin_history(second).unwrap(), true);
        assert_eq!(store.list_entries("", 20).unwrap()[0].id, first);

        let mut settings = store.settings().unwrap();
        settings.max_entries = 1;
        store.update_settings(&settings).unwrap();
        store.record_capture(text_capture("fourth", 4)).unwrap();
        assert_eq!(store.list_entries("", 20).unwrap().len(), 1);
        assert_eq!(store.list_entries("", 20).unwrap()[0].id, first);
        assert_eq!(store.clear_unpinned_history().unwrap(), 0);
        assert!(store.entry(first).unwrap().is_some());
        settings.max_entries = 0;
        store.update_settings(&settings).unwrap();
        store
            .record_capture(text_capture("over target", 5))
            .unwrap();
        assert!(store
            .metrics_snapshot()
            .iter()
            .any(|metric| metric.operation == "capacity_over_target"));
        assert_eq!(store.delete_entry(first).unwrap(), true);
        assert!(store.list_entries("", 20).unwrap().is_empty());
    }

    #[test]
    fn favorite_create_order_reorder_and_relevance_persist() {
        let root = disk_tempdir();
        let mut store = ClipboardStore::open(root.path()).unwrap();
        let first = store
            .create_favorite(FavoriteDraft {
                content: "alpha reusable command".to_owned(),
                name: None,
                icon_key: Some("Terminal".to_owned()),
                tags: vec!["commands".to_owned()],
            })
            .unwrap();
        let second = store
            .create_favorite(FavoriteDraft {
                content: "zulu reusable command".to_owned(),
                name: Some("Zulu".to_owned()),
                icon_key: None,
                tags: Vec::new(),
            })
            .unwrap();
        assert_eq!(store.list_saved_items("", 20).unwrap()[0].id, second.id);
        store.reorder_favorites(&[first.id, second.id]).unwrap();
        assert_eq!(
            store
                .list_saved_items("", 20)
                .unwrap()
                .iter()
                .map(|item| item.id)
                .collect::<Vec<_>>(),
            vec![first.id, second.id]
        );
        let updated = store
            .update_favorite(
                second.id,
                FavoriteUpdate {
                    name: None,
                    icon_key: Some("Mail".to_owned()),
                    tags: vec!["shortcuts".to_owned()],
                    editable_text: Some("alpha second command".to_owned()),
                },
            )
            .unwrap();
        assert_eq!(updated.icon_key.as_deref(), Some("Mail"));
        assert_eq!(store.list_saved_items("alpha", 20).unwrap().len(), 2);
        drop(store);
        let reopened = ClipboardStore::open(root.path()).unwrap();
        assert_eq!(reopened.list_saved_items("", 20).unwrap()[0].id, first.id);
        assert_eq!(
            reopened
                .saved_item(second.id)
                .unwrap()
                .unwrap()
                .item
                .icon_key
                .as_deref(),
            Some("Mail")
        );
    }

    #[test]
    fn theme_setting_defaults_round_trips_and_reopens_from_clipboard_settings() {
        let root = disk_tempdir();
        let mut store = ClipboardStore::open(root.path()).unwrap();
        assert_eq!(store.settings().unwrap().theme, ThemeMode::System);
        assert_eq!(
            store
                .connection
                .query_row(
                    "SELECT theme FROM clipboard_settings WHERE id = 1",
                    [],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
            "system"
        );

        for mode in [ThemeMode::Light, ThemeMode::Dark, ThemeMode::System] {
            let mut settings = store.settings().unwrap();
            settings.theme = mode;
            store.update_settings(&settings).unwrap();
            assert_eq!(store.settings().unwrap().theme, mode);
        }

        drop(store);
        let reopened = ClipboardStore::open(root.path()).unwrap();
        assert_eq!(reopened.settings().unwrap().theme, ThemeMode::System);
    }

    #[test]
    fn schema_v5_settings_backfill_adds_theme_without_bumping_version() {
        let root = disk_tempdir();
        let database = root.path().join("echo.sqlite3");
        {
            let store = ClipboardStore::open(root.path()).unwrap();
            assert_eq!(
                store
                    .connection
                    .query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))
                    .unwrap(),
                CURRENT_SCHEMA_VERSION
            );
        }

        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(
                "PRAGMA foreign_keys = OFF;
                 ALTER TABLE clipboard_settings RENAME TO clipboard_settings_with_theme;
                 CREATE TABLE clipboard_settings (
                     id INTEGER PRIMARY KEY CHECK (id = 1),
                     history_enabled INTEGER NOT NULL DEFAULT 1,
                     record_sensitive INTEGER NOT NULL DEFAULT 0,
                     store_window_titles INTEGER NOT NULL DEFAULT 0,
                     max_entries INTEGER NOT NULL DEFAULT 5000,
                     max_total_bytes INTEGER NOT NULL DEFAULT 536870912,
                     max_item_bytes INTEGER NOT NULL DEFAULT 33554432
                 );
                 INSERT INTO clipboard_settings
                     (id, history_enabled, record_sensitive, store_window_titles,
                      max_entries, max_total_bytes, max_item_bytes)
                 SELECT id, history_enabled, record_sensitive, store_window_titles,
                        max_entries, max_total_bytes, max_item_bytes
                 FROM clipboard_settings_with_theme;
                 DROP TABLE clipboard_settings_with_theme;
                 PRAGMA user_version = 5;
                 PRAGMA foreign_keys = ON;",
            )
            .unwrap();
        drop(connection);

        let reopened = ClipboardStore::open(root.path()).unwrap();
        assert_eq!(reopened.settings().unwrap().theme, ThemeMode::System);
        assert_eq!(
            reopened
                .connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))
                .unwrap(),
            CURRENT_SCHEMA_VERSION
        );
    }

    #[test]
    fn migration_failure_keeps_linked_history_and_schema_version() {
        let root = disk_tempdir();
        let database = root.path().join("echo.sqlite3");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(include_str!("../fixtures/migrations/pre-r1.sql"))
            .unwrap();
        connection
            .execute(
                "DELETE FROM saved_item_representations WHERE saved_item_id = 31",
                [],
            )
            .unwrap();
        drop(connection);
        assert!(ClipboardStore::open(root.path()).is_err());
        let connection = Connection::open(database).unwrap();
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))
                .unwrap(),
            4
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM clipboard_entries WHERE id = 12",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
    }

    #[test]
    fn migration_failure_rejects_a_corrupt_linked_blob_without_deleting_history() {
        let root = disk_tempdir();
        let database = root.path().join("echo.sqlite3");
        let connection = Connection::open(&database).unwrap();
        connection
            .execute_batch(include_str!("../fixtures/migrations/pre-r1.sql"))
            .unwrap();
        let expected_hash = hash_bytes(b"expected");
        connection
            .execute(
                "UPDATE saved_item_representations
                 SET inline_data = NULL, blob_hash = ?, byte_size = 8
                 WHERE saved_item_id = 31",
                params![expected_hash],
            )
            .unwrap();
        drop(connection);
        fs::create_dir_all(root.path().join("blobs")).unwrap();
        fs::write(root.path().join("blobs").join(&expected_hash), b"corrupt!").unwrap();

        assert!(ClipboardStore::open(root.path()).is_err());
        let connection = Connection::open(database).unwrap();
        assert_eq!(
            connection
                .query_row("PRAGMA user_version", [], |row| row.get::<_, i32>(0))
                .unwrap(),
            4
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM clipboard_entries WHERE id = 12",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM saved_items WHERE id = 31",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
    }

    #[test]
    fn batch_move_and_pin_are_transactional_and_idempotent() {
        let mut store = memory_store();
        let first = store
            .record_capture(text_capture("batch one", 1))
            .unwrap()
            .id;
        let second = store
            .record_capture(text_capture("batch two", 2))
            .unwrap()
            .id;
        let third = store
            .record_capture(text_capture("batch three", 3))
            .unwrap()
            .id;
        assert_eq!(store.pin_history_many(&[first, second, second]).unwrap(), 2);
        assert_eq!(store.pin_history_many(&[first, second]).unwrap(), 0);
        let moved = store
            .move_history_many_to_favorites(&[first, second, third, third])
            .unwrap();
        assert_eq!(moved.len(), 3);
        assert!(store.list_entries("", 20).unwrap().is_empty());
        assert_eq!(store.list_saved_items("", 20).unwrap().len(), 3);
    }

    #[test]
    fn authoritative_search_cursors_cross_pinned_and_relevance_partitions() {
        let mut store = memory_store();
        let mut history_ids = Vec::new();
        for sequence in 1..=4 {
            history_ids.push(
                store
                    .record_capture(text_capture(&format!("cursor search {sequence}"), sequence))
                    .unwrap()
                    .id,
            );
        }
        store.pin_history(history_ids[0]).unwrap();
        let mut cursor = None;
        let mut seen = Vec::new();
        loop {
            let page = store.list_entries_page("search", 1, cursor).unwrap();
            seen.extend(page.items.iter().map(|entry| entry.id));
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(seen.len(), 4);
        seen.sort_unstable();
        assert_eq!(seen, history_ids.iter().copied().collect::<Vec<_>>());

        let mut favorite_ids = Vec::new();
        for value in ["cursor alpha", "cursor beta", "cursor gamma"] {
            favorite_ids.push(
                store
                    .create_favorite(FavoriteDraft {
                        content: value.to_owned(),
                        name: None,
                        icon_key: None,
                        tags: Vec::new(),
                    })
                    .unwrap()
                    .id,
            );
        }
        let mut cursor = None;
        let mut seen_favorites = Vec::new();
        loop {
            let page = store.list_saved_items_page("cursor", 1, cursor).unwrap();
            seen_favorites.extend(page.items.iter().map(|item| item.id));
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(seen_favorites.len(), 3);
        seen_favorites.sort_unstable();
        favorite_ids.sort_unstable();
        assert_eq!(seen_favorites, favorite_ids);
    }

    #[test]
    fn short_history_search_cursor_crosses_relevance_ranks_without_duplicates() {
        let mut store = memory_store();
        let exact = store.record_capture(text_capture("ab", 1)).unwrap().id;
        let prefix_one = store
            .record_capture(text_capture("ab prefix one", 2))
            .unwrap()
            .id;
        let prefix_two = store
            .record_capture(text_capture("ab prefix two", 3))
            .unwrap()
            .id;
        let contains = store
            .record_capture(text_capture("contains ab", 4))
            .unwrap()
            .id;

        let expected = store.list_entries("ab", 20).unwrap();
        assert_eq!(expected.len(), 4);
        assert_eq!(expected[0].id, exact);
        assert!(expected[1..3]
            .iter()
            .all(|entry| [prefix_one, prefix_two].contains(&entry.id)));
        assert_eq!(expected[3].id, contains);
        let expected_ids = expected.iter().map(|entry| entry.id).collect::<Vec<_>>();

        let mut cursor = None;
        let mut seen = Vec::new();
        for _ in 0..8 {
            let page = store.list_entries_page("ab", 1, cursor).unwrap();
            seen.extend(page.items.iter().map(|entry| entry.id));
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }

        assert!(
            cursor.is_none(),
            "short history pagination did not terminate"
        );
        assert_eq!(seen, expected_ids);
        assert_eq!(
            seen.iter()
                .copied()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            seen.len()
        );
    }

    #[test]
    fn short_favorites_search_cursor_crosses_relevance_ranks_without_duplicates() {
        let mut store = memory_store();
        let prefix_one = store
            .create_favorite(FavoriteDraft {
                content: "ab favorite one".to_owned(),
                name: None,
                icon_key: None,
                tags: Vec::new(),
            })
            .unwrap()
            .id;
        let prefix_two = store
            .create_favorite(FavoriteDraft {
                content: "ab favorite two".to_owned(),
                name: None,
                icon_key: None,
                tags: Vec::new(),
            })
            .unwrap()
            .id;
        let contains_one = store
            .create_favorite(FavoriteDraft {
                content: "contains ab favorite one".to_owned(),
                name: Some("Other one".to_owned()),
                icon_key: None,
                tags: Vec::new(),
            })
            .unwrap()
            .id;
        let contains_two = store
            .create_favorite(FavoriteDraft {
                content: "contains ab favorite two".to_owned(),
                name: Some("Other two".to_owned()),
                icon_key: None,
                tags: Vec::new(),
            })
            .unwrap()
            .id;

        let expected = store.list_saved_items("ab", 20).unwrap();
        assert_eq!(expected.len(), 4);
        assert!(expected[..2]
            .iter()
            .all(|item| [prefix_one, prefix_two].contains(&item.id)));
        assert!(expected[2..]
            .iter()
            .all(|item| [contains_one, contains_two].contains(&item.id)));
        let expected_ids = expected.iter().map(|item| item.id).collect::<Vec<_>>();

        let mut cursor = None;
        let mut seen = Vec::new();
        for _ in 0..8 {
            let page = store.list_saved_items_page("ab", 1, cursor).unwrap();
            seen.extend(page.items.iter().map(|item| item.id));
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }

        assert!(
            cursor.is_none(),
            "short favorites pagination did not terminate"
        );
        assert_eq!(seen, expected_ids);
        assert_eq!(
            seen.iter()
                .copied()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            seen.len()
        );
    }

    #[test]
    fn versioned_fixture_upgrades_preserve_records_and_are_idempotent() {
        for (fixture, entries, favorites) in [
            (include_str!("../fixtures/migrations/pre-r0.sql"), 1, 0),
            (include_str!("../fixtures/migrations/pre-r1.sql"), 0, 1),
            (include_str!("../fixtures/migrations/pre-r2.sql"), 0, 1),
        ] {
            let root = disk_tempdir();
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
            assert_eq!(store.settings().unwrap().theme, ThemeMode::System);
            assert_eq!(
                store
                    .connection
                    .query_row(
                        "SELECT theme FROM clipboard_settings WHERE id = 1",
                        [],
                        |row| row.get::<_, String>(0),
                    )
                    .unwrap(),
                "system"
            );
            assert_eq!(store.list_entries("", 20).unwrap().len(), entries);
            let saved = store.list_saved_items("", 20).unwrap();
            assert_eq!(saved.len(), favorites);
            assert!(saved.iter().all(|item| item.source_history_id.is_none()));
            drop(store);

            let reopened = ClipboardStore::open(root.path()).unwrap();
            let reopened_version: i32 = reopened
                .connection
                .query_row("PRAGMA user_version", [], |row| row.get(0))
                .unwrap();
            assert_eq!(reopened_version, CURRENT_SCHEMA_VERSION);
            assert_eq!(reopened.settings().unwrap().theme, ThemeMode::System);
            assert_eq!(reopened.list_entries("", 20).unwrap().len(), entries);
            let reopened_saved = reopened.list_saved_items("", 20).unwrap();
            assert_eq!(reopened_saved.len(), favorites);
            assert!(reopened_saved
                .iter()
                .all(|item| item.source_history_id.is_none()));
        }
    }

    #[test]
    fn pre_r0_saved_item_fixture_migrates_payload_and_removes_legacy_tables() {
        let root = disk_tempdir();
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
        assert_eq!(items[0].source_history_id, None);
        assert!(store.list_entries("", 20).unwrap().is_empty());
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
        let root = disk_tempdir();
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
        store.shutdown().unwrap();
    }

    #[test]
    fn preview_file_reads_do_not_wait_for_the_writer_runtime() {
        let root = disk_tempdir();
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
        store.shutdown().unwrap();
    }

    #[test]
    fn maintenance_runs_on_startup_and_delete_not_on_normal_insert() {
        let root = disk_tempdir();
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
        store.shutdown().unwrap();
    }

    #[test]
    fn instrumentation_is_structured_and_redacts_capture_values() {
        let mut store = memory_store();
        store
            .record_capture(text_capture("never-log-this-payload", 1))
            .unwrap();
        let encoded = serde_json::to_string(&store.metrics_snapshot()).unwrap();
        assert!(encoded.contains("dedupe"));
        assert!(encoded.contains("db_commit"));
        assert!(!encoded.contains("never-log-this-payload"));
        assert!(!encoded.contains("echo-storage-memory-test"));
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
    fn deleting_a_favorite_does_not_resurrect_its_history_source() {
        let mut store = memory_store();
        let history_id = store.record_capture(text_capture("source", 1)).unwrap().id;
        let saved = store.move_history_to_favorite(history_id).unwrap();
        store
            .update_favorite(
                saved.id,
                FavoriteUpdate {
                    name: Some("authored".to_owned()),
                    icon_key: None,
                    tags: vec!["kept".to_owned()],
                    editable_text: Some("edited".to_owned()),
                },
            )
            .unwrap();
        assert_eq!(store.delete_favorite(saved.id).unwrap(), true);
        assert!(store.entry(history_id).unwrap().is_none());
        assert!(store.list_saved_items("", 20).unwrap().is_empty());
    }
}
