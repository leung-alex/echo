//! Framework-independent interaction state for Echo's native surfaces.
//! IDs remain i64 in Rust and cross the UI boundary only as opaque strings.
use echo_engine::{
    PageCursor, QuickInsertItem, QuickInsertPage, QuickInsertSource, QuickInsertView,
};
use std::{collections::BTreeSet, fmt, str::FromStr};
pub mod deck;
pub mod echo_tokens;
pub mod interaction;
pub mod session;
pub mod space_state;

pub const PAGE_SIZE: u32 = 50;
pub const MAX_RESIDENT_ROWS: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowKey {
    pub source: QuickInsertSource,
    pub id: i64,
}
impl RowKey {
    pub fn of(item: &QuickInsertItem) -> Self {
        Self {
            source: item.source,
            id: item.id,
        }
    }
}
impl fmt::Display for RowKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}:{}",
            if self.source == QuickInsertSource::History {
                "h"
            } else {
                "f"
            },
            self.id
        )
    }
}
impl FromStr for RowKey {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (kind, id) = s.split_once(':').ok_or("invalid item key")?;
        let source = match kind {
            "h" => QuickInsertSource::History,
            "f" => QuickInsertSource::Favorite,
            _ => return Err("invalid item source"),
        };
        let id = id.parse::<i64>().map_err(|_| "invalid item identity")?;
        if id <= 0 {
            return Err("invalid item identity");
        }
        Ok(Self { source, id })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoadTicket {
    pub epoch: u64,
    pub serial: u64,
    pub cursor: Option<PageCursor>,
    pub append: bool,
}

pub struct Surface {
    pub space: echo_engine::SpaceId,
    pub revision: i64,
    pub total: u64,
    pub ready: bool,
    pub row_limit: usize,
    pub view: QuickInsertView,
    pub query: String,
    /// Query represented by the complete displayed snapshot, including zero rows.
    pub presented_query: Option<String>,
    pub items: Vec<QuickInsertItem>,
    pub selection: Option<RowKey>,
    pub selected_ids: BTreeSet<i64>,
    pub batch: bool,
    pub visible: bool,
    pub dirty: bool,
    pub loading: bool,
    pub status: String,
    pub error: bool,
    pub next_cursor: Option<PageCursor>,
    pub window_start: Option<PageCursor>,
    previous_windows: Vec<Option<PageCursor>>,
    epoch: u64,
    serial: u64,
    operation_notice: bool,
}
impl Surface {
    pub fn new(view: QuickInsertView) -> Self {
        Self {
            space: if view == QuickInsertView::History {
                echo_engine::SpaceId::HISTORY
            } else {
                echo_engine::SpaceId::FAVORITES
            },
            revision: 0,
            total: 0,
            ready: false,
            row_limit: MAX_RESIDENT_ROWS,
            view,
            query: String::new(),
            presented_query: None,
            items: Vec::new(),
            selection: None,
            selected_ids: BTreeSet::new(),
            batch: false,
            visible: false,
            dirty: true,
            loading: false,
            status: String::new(),
            error: false,
            next_cursor: None,
            window_start: None,
            previous_windows: Vec::new(),
            epoch: 0,
            serial: 0,
            operation_notice: false,
        }
    }
    pub fn set_space(&mut self, id: echo_engine::SpaceId) {
        if self.space == id {
            return;
        }
        self.space = id;
        self.view = if id == echo_engine::SpaceId::HISTORY {
            QuickInsertView::History
        } else {
            QuickInsertView::Favorites
        };
        self.items.clear();
        self.presented_query = None;
        self.revision = 0;
        self.total = 0;
        self.reset_page();
        self.set_batch(false);
    }
    pub fn query_epoch(&self) -> u64 {
        self.epoch
    }
    pub fn refresh_top(&mut self) {
        self.reset_page();
    }
    pub fn set_query(&mut self, query: String) {
        if self.query == query {
            return;
        }
        self.query = query;
        let selection = self.selection;
        self.reset_page();
        // Retain the displayed identity while execution remains gated by ready.
        // finish_load validates that the key still belongs to the new result.
        self.selection = selection;
    }
    pub fn set_view(&mut self, view: QuickInsertView) {
        if self.view == view {
            return;
        }
        self.view = view;
        self.query.clear();
        self.reset_page();
        self.set_batch(false);
    }
    fn reset_page(&mut self) {
        self.ready = false;
        self.clear_notice();
        self.epoch = self.epoch.wrapping_add(1);
        self.selection = None;
        self.next_cursor = None;
        self.window_start = None;
        self.previous_windows.clear();
        self.selected_ids.clear();
        self.dirty = true;
        self.loading = false;
    }
    pub fn invalidate(&mut self) {
        self.dirty = true;
    }
    pub fn hide(&mut self) {
        self.visible = false;
        self.ready = false;
        self.epoch = self.epoch.wrapping_add(1);
        self.loading = false;
        self.dirty = true;
    }
    pub fn begin_load(&mut self, more: bool) -> Option<LoadTicket> {
        if !self.visible || (more && (self.loading || self.next_cursor.is_none())) {
            return None;
        }
        self.serial = self.serial.wrapping_add(1);
        let mut append = more;
        let cursor = if more {
            self.next_cursor
        } else {
            self.window_start
        };
        if more && self.items.len() >= self.row_limit {
            self.previous_windows.push(self.window_start);
            self.window_start = cursor;
            append = false;
        }
        self.loading = true;
        self.ready = false;
        self.dirty = false;
        Some(LoadTicket {
            epoch: self.epoch,
            serial: self.serial,
            cursor,
            append,
        })
    }
    pub fn previous_window(&mut self) -> bool {
        if let Some(cursor) = self.previous_windows.pop() {
            self.window_start = cursor;
            self.epoch = self.epoch.wrapping_add(1);
            self.loading = false;
            self.dirty = true;
            true
        } else {
            false
        }
    }
    pub fn has_previous(&self) -> bool {
        !self.previous_windows.is_empty()
    }
    /// A bounded worker rejected admission, so no completion will arrive.
    /// Undo only the paging reservation and retain the last presented content.
    pub fn retry_unqueued_load(&mut self, ticket: LoadTicket, more: bool) -> bool {
        if ticket.epoch != self.epoch || ticket.serial != self.serial || !self.visible {
            return false;
        }
        if more && !ticket.append {
            self.window_start = self.previous_windows.pop().unwrap_or(None);
        }
        self.loading = false;
        self.ready = false;
        self.dirty = true;
        true
    }
    pub fn finish_load(
        &mut self,
        ticket: LoadTicket,
        result: Result<QuickInsertPage, String>,
    ) -> bool {
        if ticket.epoch != self.epoch || ticket.serial != self.serial || !self.visible {
            return false;
        }
        self.loading = false;
        match result {
            Ok(page) => {
                self.ready = true;
                self.presented_query = Some(self.query.clone());
                if ticket.append {
                    for item in page.items {
                        if !self
                            .items
                            .iter()
                            .any(|old| RowKey::of(old) == RowKey::of(&item))
                        {
                            self.items.push(item);
                        }
                    }
                } else {
                    self.items = page.items;
                }
                self.next_cursor = page.next_cursor;
                if !self
                    .items
                    .iter()
                    .any(|x| Some(RowKey::of(x)) == self.selection)
                {
                    self.selection = self.items.first().map(RowKey::of);
                }
                self.selected_ids.retain(|id| {
                    self.items
                        .iter()
                        .any(|x| x.source == QuickInsertSource::History && x.id == *id)
                });
                if !self.operation_notice {
                    self.error = false;
                    self.status = format!("{} items", self.items.len());
                }
            }
            Err(error) => {
                self.ready = false;
                self.operation_notice = false;
                self.error = true;
                self.status = error;
            }
        }
        true
    }
    pub fn select(&mut self, key: RowKey) -> bool {
        if !self.items.iter().any(|item| RowKey::of(item) == key) {
            return false;
        }
        self.selection = Some(key);
        true
    }
    pub fn selected(&self) -> Option<&QuickInsertItem> {
        self.items
            .iter()
            .find(|x| Some(RowKey::of(x)) == self.selection)
    }
    pub fn move_selection(&mut self, delta: i32) {
        if self.items.is_empty() {
            self.selection = None;
            return;
        }
        let index = self
            .items
            .iter()
            .position(|x| Some(RowKey::of(x)) == self.selection)
            .unwrap_or(0);
        let next = (index as i64 + i64::from(delta)).rem_euclid(self.items.len() as i64) as usize;
        self.selection = Some(RowKey::of(&self.items[next]));
    }
    pub fn select_index(&mut self, index: usize) {
        if let Some(item) = self.items.get(index) {
            self.selection = Some(RowKey::of(item));
        }
    }
    pub fn set_batch(&mut self, value: bool) {
        self.batch = value && self.view == QuickInsertView::History;
        self.selected_ids.clear();
    }
    pub fn toggle_selected(&mut self, id: i64) {
        if !self.batch
            || !self
                .items
                .iter()
                .any(|x| x.id == id && x.source == QuickInsertSource::History)
        {
            return;
        }
        if !self.selected_ids.remove(&id) {
            self.selected_ids.insert(id);
        }
    }
    pub fn select_all(&mut self) {
        if self.batch {
            self.selected_ids = self
                .items
                .iter()
                .filter(|x| x.source == QuickInsertSource::History)
                .map(|x| x.id)
                .collect();
        }
    }
    pub fn report(&mut self, message: impl Into<String>, error: bool) {
        self.status = message.into();
        self.operation_notice = !self.status.is_empty();
        self.error = error;
    }

    fn clear_notice(&mut self) {
        self.operation_notice = false;
        self.status.clear();
        self.error = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn item(id: i64) -> QuickInsertItem {
        QuickInsertItem {
            id,
            source: QuickInsertSource::History,
            name: None,
            preview_text: Some(format!("{id}")),
            content_type: "text".into(),
            editable_text: None,
            tags: vec![],
            source_app: None,
            updated_at: 0,
            pinned_at: None,
            icon_key: None,
            favorite_order: None,
            thumbnail: None,
        }
    }
    fn shown() -> Surface {
        let mut s = Surface::new(QuickInsertView::History);
        s.visible = true;
        s
    }
    fn page(ids: &[i64]) -> QuickInsertPage {
        QuickInsertPage {
            items: ids.iter().copied().map(item).collect(),
            next_cursor: None,
        }
    }
    #[test]
    fn identity_never_narrows() {
        let key = RowKey {
            source: QuickInsertSource::Favorite,
            id: i64::MAX,
        };
        assert_eq!(key.to_string().parse::<RowKey>(), Ok(key));
        for bad in ["", "0", "h:-1", "f:0", "x:1", "h:9223372036854775808"] {
            assert!(bad.parse::<RowKey>().is_err());
        }
    }
    #[test]
    fn stale_query_cannot_replace_current_rows() {
        let mut s = shown();
        let old = s.begin_load(false).unwrap();
        s.set_query("new".into());
        let now = s.begin_load(false).unwrap();
        assert!(!s.finish_load(old, Ok(page(&[1]))));
        assert!(s.finish_load(now, Ok(page(&[2]))));
        assert_eq!(s.selected().unwrap().id, 2);
    }
    #[test]
    fn newer_refresh_wins() {
        let mut s = shown();
        let a = s.begin_load(false).unwrap();
        let b = s.begin_load(false).unwrap();
        assert!(s.finish_load(b, Ok(page(&[2]))));
        assert!(!s.finish_load(a, Ok(page(&[1]))));
    }
    #[test]
    fn complete_empty_snapshot_survives_pending_failed_and_stale_queries() {
        let mut s = shown();
        s.set_query("missing".into());
        let empty = s.begin_load(false).unwrap();
        assert!(s.finish_load(empty, Ok(page(&[]))));
        assert_eq!(s.presented_query.as_deref(), Some("missing"));
        s.set_query("next".into());
        let failed = s.begin_load(false).unwrap();
        assert!(!s.ready);
        assert_eq!(s.presented_query.as_deref(), Some("missing"));
        assert!(s.finish_load(failed, Err("unavailable".into())));
        assert_eq!(s.presented_query.as_deref(), Some("missing"));
        s.set_query("latest".into());
        let latest = s.begin_load(false).unwrap();
        assert!(!s.finish_load(empty, Ok(page(&[7]))));
        assert_eq!(s.presented_query.as_deref(), Some("missing"));
        assert!(s.finish_load(latest, Ok(page(&[9]))));
        assert_eq!(s.presented_query.as_deref(), Some("latest"));
        assert_eq!(s.selected().unwrap().id, 9);
        s.set_space(echo_engine::SpaceId::FAVORITES);
        assert_eq!(s.presented_query, None);
    }
    #[test]
    fn selection_survives_reorder() {
        let mut s = shown();
        let a = s.begin_load(false).unwrap();
        s.finish_load(a, Ok(page(&[1, 2])));
        s.select_index(1);
        let b = s.begin_load(false).unwrap();
        s.finish_load(b, Ok(page(&[2, 1])));
        assert_eq!(s.selected().unwrap().id, 2);
    }
    #[test]
    fn narrowing_query_keeps_surviving_selection_but_disables_execution() {
        let mut s = shown();
        let first = s.begin_load(false).unwrap();
        s.finish_load(first, Ok(page(&[1, 2, 3])));
        s.select_index(1);
        s.set_query("narrow".into());
        assert!(!s.ready);
        let next = s.begin_load(false).unwrap();
        s.finish_load(next, Ok(page(&[3, 2])));
        assert_eq!(s.selected().unwrap().id, 2);
        s.set_query("different".into());
        let next = s.begin_load(false).unwrap();
        s.finish_load(next, Ok(page(&[3])));
        assert_eq!(s.selected().unwrap().id, 3);
    }
    #[test]
    fn hidden_surface_never_queries_or_accepts_late_results() {
        let mut s = shown();
        let a = s.begin_load(false).unwrap();
        s.hide();
        assert!(s.begin_load(false).is_none());
        assert!(!s.finish_load(a, Ok(page(&[1]))));
        assert!(s.dirty);
    }
    #[test]
    fn panel_switch_cancels_batch_and_pending_load() {
        let mut s = shown();
        let a = s.begin_load(false).unwrap();
        s.set_batch(true);
        s.set_view(QuickInsertView::Favorites);
        assert!(!s.batch);
        assert!(!s.finish_load(a, Ok(page(&[1]))));
    }
    #[test]
    fn navigation_wraps_and_empty_is_safe() {
        let mut s = shown();
        s.move_selection(-1);
        assert!(s.selection.is_none());
        let a = s.begin_load(false).unwrap();
        s.finish_load(a, Ok(page(&[1, 2])));
        s.move_selection(-1);
        assert_eq!(s.selected().unwrap().id, 2);
        s.move_selection(1);
        assert_eq!(s.selected().unwrap().id, 1);
    }
    #[test]
    fn batch_does_not_invent_ids() {
        let mut s = shown();
        let a = s.begin_load(false).unwrap();
        s.finish_load(a, Ok(page(&[1, 2])));
        s.set_batch(true);
        s.toggle_selected(9);
        assert!(s.selected_ids.is_empty());
        s.select_all();
        assert_eq!(s.selected_ids.len(), 2);
        s.toggle_selected(1);
        assert_eq!(s.selected_ids.len(), 1);
    }
    #[test]
    fn failure_is_visible_and_preserves_rows() {
        let mut s = shown();
        let a = s.begin_load(false).unwrap();
        s.finish_load(a, Ok(page(&[1])));
        let b = s.begin_load(false).unwrap();
        s.finish_load(b, Err("busy".into()));
        assert_eq!(s.items.len(), 1);
        assert!(s.error);
        assert_eq!(s.status, "busy");
    }
    #[test]
    fn large_history_has_bounded_windows_and_a_way_back() {
        let mut s = shown();
        s.items = (1..=500).map(item).collect();
        s.next_cursor = Some(PageCursor::History {
            pinned_at: None,
            updated_at: 0,
            id: 500,
        });
        let a = s.begin_load(true).unwrap();
        assert!(!a.append);
        s.finish_load(a, Ok(page(&[501])));
        assert_eq!(s.items.len(), 1);
        assert!(s.has_previous());
        assert!(s.previous_window());
        assert!(s.window_start.is_none());
    }

    #[test]
    fn rejected_queue_admission_retries_without_losing_rows_or_paging_position() {
        let mut s = shown();
        s.items = (1..=500).map(item).collect();
        s.next_cursor = Some(PageCursor::History {
            pinned_at: None,
            updated_at: 0,
            id: 500,
        });
        let expected = s.next_cursor;
        for _ in 0..3 {
            let rejected = s.begin_load(true).unwrap();
            assert_eq!(rejected.cursor, expected);
            assert!(s.retry_unqueued_load(rejected, true));
            assert!(!s.loading && s.dirty && !s.ready);
            assert_eq!(s.items.len(), 500);
            assert!(!s.has_previous());
        }
        let accepted = s.begin_load(true).unwrap();
        assert!(s.finish_load(accepted, Ok(page(&[501]))));
        assert!(s.has_previous());
        s.set_query("new".into());
        let rejected = s.begin_load(false).unwrap();
        assert!(s.retry_unqueued_load(rejected, false));
        let accepted = s.begin_load(false).unwrap();
        assert!(!s.finish_load(rejected, Ok(page(&[999]))));
        assert!(s.finish_load(accepted, Ok(page(&[502]))));
        assert_eq!(s.items[0].id, 502);
    }

    #[test]
    fn mutation_success_survives_the_result_refresh() {
        let mut surface = shown();
        surface.report("Favorite created", false);
        let ticket = surface.begin_load(false).unwrap();
        assert!(surface.finish_load(ticket, Ok(page(&[]))));
        assert_eq!(surface.status, "Favorite created");
        assert!(!surface.error);
    }

    #[test]
    fn successful_retry_clears_a_previous_query_error() {
        let mut surface = shown();
        let first = surface.begin_load(false).unwrap();
        assert!(surface.finish_load(first, Err("Temporary read failure".into())));
        let retry = surface.begin_load(false).unwrap();
        assert!(surface.finish_load(retry, Ok(page(&[]))));
        assert!(!surface.error);
        assert_eq!(surface.status, "0 items");
    }

    #[test]
    fn a_new_query_starts_with_a_clean_notice() {
        let mut surface = Surface::new(QuickInsertView::History);
        surface.report("Previous operation failed", true);
        surface.set_query("new query".into());
        assert!(!surface.error);
        assert!(surface.status.is_empty());
    }

    #[test]
    fn empty_report_allows_the_result_count_to_return() {
        let mut surface = shown();
        surface.report("", false);
        let ticket = surface.begin_load(false).unwrap();
        assert!(surface.finish_load(ticket, Ok(page(&[1]))));
        assert!(!surface.error);
        assert_eq!(surface.status, "1 items");
    }
}
