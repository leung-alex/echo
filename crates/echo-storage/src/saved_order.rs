//! Allocate a front key without rewriting every retained Saved Item.
use crate::{Result, StorageError};
use rusqlite::Transaction;

pub(super) fn prepend_key_tx(tx: &Transaction<'_>) -> Result<i64> {
    let first: Option<i64> =
        tx.query_row("SELECT MIN(favorite_order) FROM saved_items", [], |row| {
            row.get(0)
        })?;
    match first {
        None => Ok(0),
        Some(first) => first.checked_sub(1).ok_or_else(|| {
            StorageError::Invalid("Favorite order exhausted; no content was changed".into())
        }),
    }
}

#[cfg(test)]
mod tests;
