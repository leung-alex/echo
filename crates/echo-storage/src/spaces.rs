//! Schema-v6 space queries and transactional collection mutations.
use super::*;
use echo_engine::DeleteSpaceContents;
use echo_engine::{
    SettingsPatch, SettingsSnapshot, Space, SpaceAction, SpaceCommand, SpaceDraft, SpaceError,
    SpaceId, SpaceKind, SpaceMutationResult, SpacePage, SpaceStore,
};

const SPACE_SELECT: &str = "SELECT s.id,s.kind,s.title,s.icon_key,s.description,
    s.order_key,s.revision,s.created_at,s.updated_at,
    CASE WHEN s.id=1 THEN (SELECT COUNT(*) FROM clipboard_entries)
    ELSE (SELECT COUNT(*) FROM space_memberships m WHERE m.space_id=s.id) END
    FROM spaces s";
fn map_space(row: &Row<'_>) -> rusqlite::Result<Space> {
    let kind: String = row.get(1)?;
    let kind = match kind.as_str() {
        "history" => SpaceKind::History,
        "favorites" => SpaceKind::Favorites,
        "collection" => SpaceKind::Collection,
        _ => return Err(rusqlite::Error::InvalidQuery),
    };
    Ok(Space {
        id: SpaceId(row.get(0)?),
        kind,
        title: row.get(2)?,
        icon_key: row.get(3)?,
        description: row.get(4)?,
        order_key: row.get(5)?,
        revision: row.get(6)?,
        created_at: row.get(7)?,
        updated_at: row.get(8)?,
        item_count: row.get::<_, i64>(9)?.max(0) as u64,
    })
}
// Copy the immutable original representations and metadata, never the preview text.
fn copy_item_tx(tx: &Transaction<'_>, item: i64) -> Result<i64> {
    let order = super::saved_order::prepend_key_tx(tx)?;
    let copied = tx.execute("INSERT INTO saved_items(source_history_id,created_at,updated_at,name,content_type,editable_text,
        source_app,source_executable,preview_text,byte_size,icon_key,favorite_order,is_independent)
        SELECT NULL,created_at,updated_at,name,content_type,editable_text,source_app,source_executable,
        preview_text,byte_size,icon_key,?,1 FROM saved_items WHERE id=?", params![order,item])?;
    if copied == 0 {
        return Err(SpaceError::NotFound.into());
    }
    let copy = tx.last_insert_rowid();
    tx.execute("INSERT INTO saved_item_representations(saved_item_id,format,mime_type,inline_data,blob_hash,content_hash,byte_size)
        SELECT ?,format,mime_type,inline_data,blob_hash,content_hash,byte_size FROM saved_item_representations WHERE saved_item_id=?", params![copy,item])?;
    tx.execute("INSERT INTO saved_item_tags(saved_item_id,tag_id) SELECT ?,tag_id FROM saved_item_tags WHERE saved_item_id=?", params![copy,item])?;
    refresh_saved_search_tx(tx, copy)?;
    Ok(copy)
}

impl ClipboardStore {
    pub(super) fn migrate_schema_v9(&mut self) -> Result<()> {
        let tx = self.connection.transaction()?;
        let links = {
            let mut statement = tx.prepare("SELECT space_id,saved_item_id FROM space_memberships
                WHERE (space_id,saved_item_id) NOT IN (SELECT MIN(space_id),saved_item_id FROM space_memberships GROUP BY saved_item_id)
                ORDER BY space_id,sort_key,saved_item_id")?;
            let rows =
                statement.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        for (space, item) in links {
            Self::verify_saved_representations_tx(&tx, &self.blobs_dir, item)?;
            let copy = copy_item_tx(&tx, item)?;
            tx.execute(
                "UPDATE space_memberships SET saved_item_id=? WHERE space_id=? AND saved_item_id=?",
                params![copy, space, item],
            )?;
        }
        tx.execute_batch(include_str!("../migrations/v9.sql"))?;
        tx.commit()?;
        Ok(())
    }

    pub(super) fn migrate_schema_v6(&mut self) -> Result<()> {
        let tx = self
            .connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        tx.execute_batch(include_str!("../migrations/v6.sql"))?;
        tx.commit()?;
        Ok(())
    }
    pub fn list_spaces(&self) -> Result<Vec<Space>> {
        let mut statement = self
            .connection
            .prepare(&format!("{SPACE_SELECT} ORDER BY s.order_key,s.id"))?;
        let result = statement
            .query_map([], map_space)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(result)
    }
    fn space_record(&self, id: SpaceId) -> Result<Space> {
        self.connection
            .query_row(&format!("{SPACE_SELECT} WHERE s.id=?"), [id.0], map_space)
            .optional()?
            .ok_or_else(|| SpaceError::NotFound.into())
    }
    pub fn spaces_for_item(&self, item: i64) -> Result<Vec<SpaceId>> {
        let mut statement = self.connection.prepare(
            "SELECT space_id FROM space_memberships WHERE saved_item_id=? ORDER BY space_id",
        )?;
        let rows = statement
            .query_map([item], |r| Ok(SpaceId(r.get(0)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
    pub fn list_space_items(
        &self,
        id: SpaceId,
        query: &str,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> Result<SpacePage> {
        if id.0 < 2 {
            return Err(
                SpaceError::Invalid("History uses its existing capture provider".into()).into(),
            );
        }
        if query.len() > 16 * 1024 {
            return Err(SpaceError::Invalid("Search query is too long".into()).into());
        }
        let started = Instant::now();
        // All SELECTs observe one WAL snapshot, including membership revision and tags.
        let tx = self.connection.unchecked_transaction()?;
        let space = self.space_record(id)?;
        let size = if limit == 0 {
            DEFAULT_PAGE_SIZE
        } else {
            limit.clamp(1, MAX_PAGE_SIZE)
        };
        let mut join =
            String::from("FROM space_memberships m JOIN saved_items s ON s.id=m.saved_item_id");
        let mut filter = String::from("m.space_id=?");
        let mut values = vec![Value::Integer(id.0)];
        let query = query.trim();
        if !query.is_empty() {
            join.push_str(" JOIN saved_items_fts f ON CAST(f.saved_item_id AS INTEGER)=s.id");
            if query.chars().count() < 3 {
                filter.push_str(" AND lower(f.document) LIKE ? ESCAPE '\\'");
                values.push(Value::Text(format!(
                    "%{}%",
                    escape_like_pattern(&query.to_lowercase())
                )));
            } else {
                filter.push_str(" AND saved_items_fts MATCH ?");
                values.push(Value::Text(fts_match_query(query)));
            }
        }
        // space_record already counted this membership set in the same WAL
        // snapshot. Do not fetch every Saved Item again on every scan page.
        let total: i64 = if query.is_empty() {
            i64::try_from(space.item_count).unwrap_or(i64::MAX)
        } else {
            self.connection.query_row(
                &format!("SELECT COUNT(*) {join} WHERE {filter}"),
                params_from_iter(values.clone()),
                |row| row.get(0),
            )?
        };
        if let Some(cursor) = cursor {
            let PageCursor::Space {
                space_id,
                revision,
                sort_key,
                id: row_id,
            } = cursor
            else {
                return Err(SpaceError::StaleCursor.into());
            };
            if space_id != id.0 || revision != space.revision {
                return Err(SpaceError::StaleCursor.into());
            }
            filter.push_str(" AND (m.sort_key,m.saved_item_id) > (?,?)");
            values.extend([Value::Integer(sort_key), Value::Integer(row_id)]);
        }
        values.push(Value::Integer(i64::from(size) + 1));
        let sql = format!(
            "SELECT s.id,s.source_history_id,s.created_at,s.updated_at,s.name,
            s.content_type,s.editable_text,s.source_app,s.source_executable,s.preview_text,s.byte_size,s.icon_key,m.sort_key {join} WHERE {filter}
            ORDER BY m.sort_key,m.saved_item_id LIMIT ?"
        );
        let mut items = {
            let mut statement = self.connection.prepare_cached(&sql)?;
            let rows = statement.query_map(params_from_iter(values), map_saved_item)?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        let more = items.len() > size as usize;
        items.truncate(size as usize);
        for item in &mut items {
            item.tags = self.tags_for_item(item.id)?;
            item.thumbnail = self.thumbnail_for_saved_item(item.id)?;
        }
        let next_cursor = if more {
            items.last().map(|item| PageCursor::Space {
                space_id: id.0,
                revision: space.revision,
                sort_key: item.favorite_order,
                id: item.id,
            })
        } else {
            None
        };
        tx.commit()?;
        self.metrics
            .record("space_query", started.elapsed(), items.len() as u64);
        Ok(SpacePage {
            space_id: id,
            revision: space.revision,
            total: total.max(0) as u64,
            page: LibraryPage { items, next_cursor },
        })
    }
}

pub(super) fn validate_space_tx(
    tx: &Transaction<'_>,
    id: SpaceId,
    expected: Option<i64>,
) -> Result<()> {
    if id.0 < 2 {
        return Err(SpaceError::SystemSpace.into());
    }
    let revision: Option<i64> = tx
        .query_row("SELECT revision FROM spaces WHERE id=?", [id.0], |r| {
            r.get(0)
        })
        .optional()?;
    let revision = revision.ok_or(SpaceError::NotFound)?;
    if expected.is_some_and(|expected| expected != revision) {
        return Err(SpaceError::Conflict.into());
    }
    Ok(())
}
pub(super) fn bump_space_tx(tx: &Transaction<'_>, id: SpaceId) -> Result<()> {
    if tx.execute("UPDATE spaces SET revision=revision+1,updated_at=? WHERE id=? AND revision<9223372036854775807",
        params![now_millis(),id.0])?!=1 { return Err(SpaceError::Conflict.into()); }
    Ok(())
}
pub(super) fn bump_saved_spaces_tx(tx: &Transaction<'_>, item: i64) -> Result<()> {
    let ids = {
        let mut s = tx.prepare("SELECT space_id FROM space_memberships WHERE saved_item_id=?")?;
        let rows = s.query_map([item], |r| Ok(SpaceId(r.get(0)?)))?;
        rows.collect::<std::result::Result<Vec<_>, _>>()?
    };
    for id in ids {
        bump_space_tx(tx, id)?;
    }
    Ok(())
}
fn compact_order_tx(tx: &Transaction<'_>, space: SpaceId) -> Result<()> {
    tx.execute("WITH ordered AS MATERIALIZED (
        SELECT saved_item_id,(ROW_NUMBER() OVER(ORDER BY sort_key,saved_item_id)-1)*1024 AS pos
        FROM space_memberships WHERE space_id=?1)
        UPDATE space_memberships SET sort_key=(SELECT pos FROM ordered WHERE ordered.saved_item_id=space_memberships.saved_item_id)
        WHERE space_id=?1",[space.0])?;
    Ok(())
}
pub(super) fn add_membership_tx(
    tx: &Transaction<'_>,
    space: SpaceId,
    item: i64,
    front: bool,
) -> Result<bool> {
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM space_memberships WHERE space_id=? AND saved_item_id=?)",
        params![space.0, item],
        |r| r.get::<_, bool>(0),
    )? {
        return Ok(false);
    }
    if !tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM saved_items WHERE id=?)",
        [item],
        |r| r.get::<_, bool>(0),
    )? {
        return Err(SpaceError::NotFound.into());
    }
    let sql = if front {
        "SELECT MIN(sort_key) FROM space_memberships WHERE space_id=?"
    } else {
        "SELECT MAX(sort_key) FROM space_memberships WHERE space_id=?"
    };
    let end: Option<i64> = tx.query_row(sql, [space.0], |r| r.get(0))?;
    let delta = if front { -1024 } else { 1024 };
    let order = if let Some(end) = end {
        if let Some(order) = end.checked_add(delta) {
            order
        } else {
            compact_order_tx(tx, space)?;
            let end: i64 = tx.query_row(sql, [space.0], |r| r.get(0))?;
            end.checked_add(delta)
                .ok_or_else(|| SpaceError::Invalid("Space ordering overflow".into()))?
        }
    } else {
        0
    };
    tx.execute("INSERT INTO space_memberships(space_id,saved_item_id,sort_key,created_at) VALUES (?,?,?,?)",
        params![space.0,item,order,now_millis()])?;
    Ok(true)
}
fn unique_title_tx(tx: &Transaction<'_>, draft: &SpaceDraft, except: i64) -> Result<()> {
    if tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM spaces WHERE normalized_title=? AND id<>?)",
        params![draft.normalized_title(), except],
        |r| r.get::<_, bool>(0),
    )? {
        return Err(SpaceError::DuplicateName.into());
    }
    Ok(())
}
impl ClipboardStore {
    fn create_space(&mut self, draft: SpaceDraft) -> Result<SpaceMutationResult> {
        let draft = draft.normalize()?;
        let now = now_millis();
        let tx = self.connection.transaction()?;
        unique_title_tx(&tx, &draft, 0)?;
        let last: i64 = tx.query_row("SELECT MAX(order_key) FROM spaces", [], |r| r.get(0))?;
        let next = last
            .checked_add(1)
            .ok_or_else(|| SpaceError::Invalid("Space ordering overflow".into()))?;
        tx.execute("INSERT INTO spaces(kind,title,normalized_title,icon_key,description,order_key,revision,created_at,updated_at)
            VALUES ('collection',?,?,?,?,?,1,?,?)",
            params![draft.title,draft.normalized_title(),draft.icon_key,draft.description,next,now,now])?;
        let id = SpaceId(tx.last_insert_rowid());
        tx.commit()?;
        Ok(SpaceMutationResult {
            affected_spaces: vec![id],
            created_space: Some(id),
            ..Default::default()
        })
    }
    fn update_space(
        &mut self,
        id: SpaceId,
        revision: i64,
        draft: SpaceDraft,
    ) -> Result<SpaceMutationResult> {
        if id.is_system() {
            return Err(SpaceError::SystemSpace.into());
        }
        let draft = draft.normalize()?;
        let tx = self.connection.transaction()?;
        validate_space_tx(&tx, id, Some(revision))?;
        unique_title_tx(&tx, &draft, id.0)?;
        tx.execute(
            "UPDATE spaces SET title=?,normalized_title=?,icon_key=?,description=? WHERE id=?",
            params![
                draft.title,
                draft.normalized_title(),
                draft.icon_key,
                draft.description,
                id.0
            ],
        )?;
        bump_space_tx(&tx, id)?;
        tx.commit()?;
        Ok(SpaceMutationResult {
            affected_spaces: vec![id],
            ..Default::default()
        })
    }
    fn delete_space(
        &mut self,
        id: SpaceId,
        revision: i64,
        contents: DeleteSpaceContents,
    ) -> Result<SpaceMutationResult> {
        if id.is_system() {
            return Err(SpaceError::SystemSpace.into());
        }
        let tx = self.connection.transaction()?;
        validate_space_tx(&tx, id, Some(revision))?;
        let items = {
            let mut statement = tx.prepare("SELECT saved_item_id FROM space_memberships WHERE space_id=? ORDER BY sort_key,saved_item_id")?;
            let rows = statement.query_map([id.0], |r| r.get::<_, i64>(0))?;
            rows.collect::<std::result::Result<Vec<_>, _>>()?
        };
        let mut affected = vec![id];
        match contents {
            DeleteSpaceContents::MoveToFavorites => {
                // Detach first: each saved item has exactly one owning space.
                tx.execute("DELETE FROM space_memberships WHERE space_id=?", [id.0])?;
                for item in &items {
                    add_membership_tx(&tx, SpaceId::FAVORITES, *item, false)?;
                }
                if !items.is_empty() {
                    bump_space_tx(&tx, SpaceId::FAVORITES)?;
                    affected.push(SpaceId::FAVORITES);
                }
            }
            DeleteSpaceContents::Delete => {
                tx.execute("DELETE FROM saved_items_fts WHERE saved_item_id IN (SELECT saved_item_id FROM space_memberships WHERE space_id=?)", [id.0])?;
                tx.execute("DELETE FROM saved_items WHERE id IN (SELECT saved_item_id FROM space_memberships WHERE space_id=?)", [id.0])?;
                if !items.is_empty() {
                    tx.execute("INSERT INTO migration_state(key,value,completed_at) VALUES ('blob_gc_pending','1',?)
                        ON CONFLICT(key) DO UPDATE SET value=excluded.value,completed_at=excluded.completed_at", [now_millis()])?;
                }
            }
        }
        tx.execute("DELETE FROM spaces WHERE id=?", [id.0])?;
        tx.commit()?;
        Ok(SpaceMutationResult {
            affected_spaces: affected,
            migrated_count: if contents == DeleteSpaceContents::MoveToFavorites {
                items.len()
            } else {
                0
            },
            ..Default::default()
        })
    }
    fn move_space(
        &mut self,
        id: SpaceId,
        revision: i64,
        delta: i32,
    ) -> Result<SpaceMutationResult> {
        if id.is_system() {
            return Err(SpaceError::SystemSpace.into());
        }
        if delta != -1 && delta != 1 {
            return Err(SpaceError::InvalidRequest.into());
        }
        let tx = self.connection.transaction()?;
        validate_space_tx(&tx, id, Some(revision))?;
        let key: i64 = tx.query_row("SELECT order_key FROM spaces WHERE id=?", [id.0], |r| {
            r.get(0)
        })?;
        let (op, order) = if delta < 0 {
            ("<", "DESC")
        } else {
            (">", "ASC")
        };
        let sql=format!("SELECT id,order_key FROM spaces WHERE kind='collection' AND (order_key {op} ? OR (order_key=? AND id {op} ?))
            ORDER BY order_key {order},id {order} LIMIT 1");
        let neighbor: Option<(i64, i64)> = tx
            .query_row(&sql, params![key, key, id.0], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        let mut affected = Vec::new();
        if let Some((other, other_key)) = neighbor {
            tx.execute(
                "UPDATE spaces SET order_key=CASE id WHEN ? THEN ? ELSE ? END WHERE id IN (?,?)",
                params![id.0, other_key, key, id.0, other],
            )?;
            bump_space_tx(&tx, id)?;
            bump_space_tx(&tx, SpaceId(other))?;
            affected = vec![id, SpaceId(other)];
        }
        tx.commit()?;
        Ok(SpaceMutationResult {
            affected_spaces: affected,
            ..Default::default()
        })
    }
}

impl ClipboardStore {
    fn add_space_items(
        &mut self,
        id: SpaceId,
        revision: i64,
        items: &[i64],
    ) -> Result<SpaceMutationResult> {
        if items.len() > 1000 || items.iter().any(|id| *id <= 0) {
            return Err(SpaceError::InvalidRequest.into());
        }
        let tx = self.connection.transaction()?;
        validate_space_tx(&tx, id, Some(revision))?;
        let mut changed = false;
        let mut seen = std::collections::HashSet::new();
        for item in items {
            if !seen.insert(*item) {
                continue;
            }
            let already_here: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM space_memberships WHERE space_id=? AND saved_item_id=?)",params![id.0,item],|r|r.get(0))?;
            if already_here {
                continue;
            }
            Self::verify_saved_representations_tx(&tx, &self.blobs_dir, *item)?;
            let copy = copy_item_tx(&tx, *item)?;
            changed |= add_membership_tx(&tx, id, copy, false)?;
        }
        if changed {
            bump_space_tx(&tx, id)?;
        }
        tx.commit()?;
        Ok(SpaceMutationResult {
            affected_spaces: if changed { vec![id] } else { vec![] },
            ..Default::default()
        })
    }
    fn remove_space_item(
        &mut self,
        id: SpaceId,
        revision: i64,
        item: i64,
    ) -> Result<SpaceMutationResult> {
        let tx = self.connection.transaction()?;
        validate_space_tx(&tx, id, Some(revision))?;
        if !tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM space_memberships WHERE space_id=? AND saved_item_id=?)",
            params![id.0, item],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(SpaceError::NotFound.into());
        }
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM space_memberships WHERE saved_item_id=?",
            [item],
            |r| r.get(0),
        )?;
        let mut affected = vec![id];
        if count == 1 {
            if id == SpaceId::FAVORITES {
                return Err(SpaceError::Invalid(
                    "Use Delete everywhere to remove the last saved copy".into(),
                )
                .into());
            }
            tx.execute(
                "DELETE FROM space_memberships WHERE space_id=? AND saved_item_id=?",
                params![id.0, item],
            )?;
            add_membership_tx(&tx, SpaceId::FAVORITES, item, false)?;
            bump_space_tx(&tx, SpaceId::FAVORITES)?;
            affected.push(SpaceId::FAVORITES);
        }
        tx.execute(
            "DELETE FROM space_memberships WHERE space_id=? AND saved_item_id=?",
            params![id.0, item],
        )?;
        bump_space_tx(&tx, id)?;
        tx.commit()?;
        Ok(SpaceMutationResult {
            affected_spaces: affected,
            migrated_count: usize::from(count == 1),
            ..Default::default()
        })
    }
    fn clear_favorites(&mut self, id: SpaceId, revision: i64) -> Result<SpaceMutationResult> {
        if id != SpaceId::FAVORITES {
            return Err(
                SpaceError::Invalid("Only Favorites supports this clear action".into()).into(),
            );
        }
        let tx = self.connection.transaction()?;
        validate_space_tx(&tx, id, Some(revision))?;
        let count: i64 = tx.query_row(
            "SELECT COUNT(*) FROM space_memberships WHERE space_id=?",
            [id.0],
            |row| row.get(0),
        )?;
        // Select the entire space in one transaction, independent of UI search
        // and pagination. Shared payloads, memberships and ordering survive.
        let exclusive = "SELECT m.saved_item_id FROM space_memberships m
            WHERE m.space_id=? AND NOT EXISTS (
                SELECT 1 FROM space_memberships other
                WHERE other.saved_item_id=m.saved_item_id AND other.space_id<>m.space_id)";
        tx.execute(
            &format!("DELETE FROM saved_items_fts WHERE saved_item_id IN ({exclusive})"),
            [id.0],
        )?;
        let deleted = tx.execute(
            &format!("DELETE FROM saved_items WHERE id IN ({exclusive})"),
            [id.0],
        )?;
        tx.execute("DELETE FROM space_memberships WHERE space_id=?", [id.0])?;
        if count > 0 {
            bump_space_tx(&tx, id)?;
        }
        if deleted > 0 {
            tx.execute(
                "INSERT INTO migration_state (key,value,completed_at) VALUES ('blob_gc_pending','1',?)
                 ON CONFLICT(key) DO UPDATE SET value=excluded.value,completed_at=excluded.completed_at",
                [now_millis()],
            )?;
        }
        tx.commit()?;
        Ok(SpaceMutationResult {
            affected_spaces: if count > 0 { vec![id] } else { vec![] },
            ..Default::default()
        })
    }
    fn reorder_space_item(
        &mut self,
        space: SpaceId,
        revision: i64,
        item: i64,
        before: Option<i64>,
        delta: i32,
    ) -> Result<SpaceMutationResult> {
        let tx = self.connection.transaction()?;
        validate_space_tx(&tx, space, Some(revision))?;
        let key: i64 = tx
            .query_row(
                "SELECT sort_key FROM space_memberships WHERE space_id=? AND saved_item_id=?",
                params![space.0, item],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(SpaceError::NotFound)?;
        if before == Some(item) {
            return Ok(SpaceMutationResult::default());
        }
        let mut changed = false;
        if let Some(target) = before {
            let mut target_key: i64 = tx
                .query_row(
                    "SELECT sort_key FROM space_memberships WHERE space_id=? AND saved_item_id=?",
                    params![space.0, target],
                    |r| r.get(0),
                )
                .optional()?
                .ok_or(SpaceError::NotFound)?;
            let previous = |tx: &Transaction<'_>, target_key: i64| -> Result<Option<i64>> {
                Ok(tx.query_row("SELECT sort_key FROM space_memberships WHERE space_id=? AND saved_item_id<>?
                    AND (sort_key<? OR (sort_key=? AND saved_item_id<?)) ORDER BY sort_key DESC,saved_item_id DESC LIMIT 1",
                    params![space.0,item,target_key,target_key,target],|r|r.get(0)).optional()?)
            };
            let mut previous_key = previous(&tx, target_key)?;
            if previous_key.is_some_and(|prev| i128::from(target_key) - i128::from(prev) <= 1)
                || target_key < i64::MIN + 1024
            {
                compact_order_tx(&tx, space)?;
                target_key = tx.query_row(
                    "SELECT sort_key FROM space_memberships WHERE space_id=? AND saved_item_id=?",
                    params![space.0, target],
                    |r| r.get(0),
                )?;
                previous_key = previous(&tx, target_key)?;
            }
            let new_key = previous_key.map_or(target_key - 1024, |prev| {
                ((i128::from(prev) + i128::from(target_key)) / 2) as i64
            });
            tx.execute(
                "UPDATE space_memberships SET sort_key=? WHERE space_id=? AND saved_item_id=?",
                params![new_key, space.0, item],
            )?;
            changed = true;
        } else if delta == -1 || delta == 1 {
            let (op, order) = if delta < 0 {
                ("<", "DESC")
            } else {
                (">", "ASC")
            };
            let sql=format!("SELECT saved_item_id,sort_key FROM space_memberships WHERE space_id=?
                AND (sort_key {op} ? OR (sort_key=? AND saved_item_id {op} ?)) ORDER BY sort_key {order},saved_item_id {order} LIMIT 1");
            let other: Option<(i64, i64)> = tx
                .query_row(&sql, params![space.0, key, key, item], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .optional()?;
            if let Some((other, other_key)) = other {
                tx.execute("UPDATE space_memberships SET sort_key=CASE saved_item_id WHEN ? THEN ? ELSE ? END
                    WHERE space_id=? AND saved_item_id IN (?,?)",params![item,other_key,key,space.0,item,other])?;
                changed = true;
            }
        } else if delta != 0 {
            return Err(SpaceError::InvalidRequest.into());
        }
        if changed {
            bump_space_tx(&tx, space)?;
        }
        tx.commit()?;
        Ok(SpaceMutationResult {
            affected_spaces: if changed { vec![space] } else { vec![] },
            ..Default::default()
        })
    }
    fn duplicate_into_space(
        &mut self,
        space: SpaceId,
        revision: i64,
        item: i64,
    ) -> Result<SpaceMutationResult> {
        let original = self.saved_item(item)?.ok_or(SpaceError::NotFound)?;
        let tx = self.connection.transaction()?;
        validate_space_tx(&tx, space, Some(revision))?;
        Self::verify_saved_representations_tx(&tx, &self.blobs_dir, item)?;
        let now = now_millis();
        let order = super::saved_order::prepend_key_tx(&tx)?;
        tx.execute("INSERT INTO saved_items(source_history_id,created_at,updated_at,name,content_type,editable_text,
            source_app,source_executable,preview_text,byte_size,icon_key,favorite_order,is_independent)
            SELECT NULL,?,?,name || ' copy',content_type,editable_text,source_app,source_executable,
            preview_text,byte_size,icon_key,?,1 FROM saved_items WHERE id=?",params![now,now,order,item])?;
        let copy = tx.last_insert_rowid();
        tx.execute("INSERT INTO saved_item_representations(saved_item_id,format,mime_type,inline_data,blob_hash,content_hash,byte_size)
            SELECT ?,format,mime_type,inline_data,blob_hash,content_hash,byte_size FROM saved_item_representations WHERE saved_item_id=?",
            params![copy,item])?;
        replace_tags_tx(&tx, copy, &original.item.tags)?;
        refresh_saved_search_tx(&tx, copy)?;
        add_membership_tx(&tx, space, copy, true)?;
        bump_space_tx(&tx, space)?;
        tx.commit()?;
        Ok(SpaceMutationResult {
            affected_spaces: vec![space],
            created_item: Some(copy),
            ..Default::default()
        })
    }
    pub fn apply_space_command(&mut self, command: SpaceCommand) -> Result<SpaceMutationResult> {
        command.validate()?;
        // Only a digest, request id and small result are cached; never retain content drafts.
        let digest: [u8; 32] = Sha256::digest(format!("{command:?}").as_bytes()).into();
        if let Some((_, old_digest, result)) = self
            .space_replays
            .iter()
            .find(|(id, _, _)| id == &command.request_id)
        {
            if old_digest != &digest {
                return Err(SpaceError::InvalidRequest.into());
            }
            return Ok(result.clone());
        }
        let id = command.space_id.unwrap_or(SpaceId::FAVORITES);
        let revision = command.expected_revision.unwrap_or(0);
        let result = match command.action {
            SpaceAction::Create(draft) => self.create_space(draft)?,
            SpaceAction::Update(draft) => self.update_space(id, revision, draft)?,
            SpaceAction::Delete(contents) => self.delete_space(id, revision, contents)?,
            SpaceAction::MoveSpace(delta) => self.move_space(id, revision, delta)?,
            SpaceAction::AddItems(items) => self.add_space_items(id, revision, &items)?,
            SpaceAction::RemoveItem(item) => self.remove_space_item(id, revision, item)?,
            SpaceAction::ClearFavorites => self.clear_favorites(id, revision)?,
            SpaceAction::ReorderItem {
                id: item,
                before,
                delta,
            } => self.reorder_space_item(id, revision, item, before, delta)?,
            SpaceAction::DuplicateItem(item) => self.duplicate_into_space(id, revision, item)?,
            SpaceAction::CreateItem(draft) => {
                let item = self.create_favorite_in_space(draft, id, Some(revision))?;
                SpaceMutationResult {
                    affected_spaces: vec![id],
                    created_item: Some(item.id),
                    ..Default::default()
                }
            }
            SpaceAction::MoveHistory(items) => {
                let items = self.move_history_many_to_space(&items, id, Some(revision))?;
                SpaceMutationResult {
                    affected_spaces: vec![id, SpaceId::HISTORY],
                    created_item: items.first().map(|item| item.id),
                    ..Default::default()
                }
            }
        };
        self.space_replays
            .push_back((command.request_id, digest, result.clone()));
        while self.space_replays.len() > 256 {
            self.space_replays.pop_front();
        }
        Ok(result)
    }
}

impl SharedClipboardStore {
    pub fn list_spaces(&self) -> Result<Vec<Space>> {
        let _admission = self.runtime.enter()?;
        self.runtime.reader.read(|store| store.list_spaces())
    }
    pub fn list_space_items(
        &self,
        space: SpaceId,
        query: &str,
        limit: u32,
        cursor: Option<PageCursor>,
    ) -> Result<SpacePage> {
        let _admission = self.runtime.enter()?;
        self.runtime
            .reader
            .read(|store| store.list_space_items(space, query, limit, cursor))
    }
    pub fn spaces_for_item(&self, item: i64) -> Result<Vec<SpaceId>> {
        let _admission = self.runtime.enter()?;
        self.runtime
            .reader
            .read(|store| store.spaces_for_item(item))
    }
    pub fn apply_space_command(&self, command: SpaceCommand) -> Result<SpaceMutationResult> {
        let result = self.with_store(move |store| store.apply_space_command(command))?;
        self.request_maintenance();
        Ok(result)
    }
}
impl SpaceStore for SharedClipboardStore {
    fn list_spaces(&self) -> Result<Vec<Space>> {
        Self::list_spaces(self)
    }
    fn list_space_items(
        &self,
        id: SpaceId,
        q: &str,
        n: u32,
        c: Option<PageCursor>,
    ) -> Result<SpacePage> {
        Self::list_space_items(self, id, q, n, c)
    }
    fn spaces_for_item(&self, id: i64) -> Result<Vec<SpaceId>> {
        Self::spaces_for_item(self, id)
    }
    fn apply_space_command(&self, c: SpaceCommand) -> Result<SpaceMutationResult> {
        Self::apply_space_command(self, c)
    }
    fn settings_snapshot(&self) -> Result<SettingsSnapshot> {
        Self::settings_snapshot(self)
    }
    fn save_settings_patch(&self, p: SettingsPatch) -> Result<SettingsSnapshot> {
        Self::save_settings_patch(self, p)
    }
    fn save_resume_space(&self, id: SpaceId) -> Result<()> {
        Self::save_resume_space(self, id)
    }
}
