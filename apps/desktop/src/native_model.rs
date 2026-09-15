//! Reconcile bounded native rows without resetting their accessible identities.
use slint::Model;
#[cfg(test)]
use slint::VecModel;
use std::cell::RefCell;

pub(crate) struct OwnedRow {
    pub row: crate::EntryRow,
    pub bytes: usize,
}
/// Charges follow the retained row, including when a new equal row is discarded.
/// The backing capacity is known here instead of being hidden inside VecModel.
#[derive(Default)]
pub(crate) struct EntryModel {
    rows: RefCell<Vec<OwnedRow>>,
    notify: slint::ModelNotify,
}
impl EntryModel {
    pub fn held_bytes(&self) -> usize {
        let rows = self.rows.borrow();
        std::mem::size_of::<Self>()
            + rows.capacity() * std::mem::size_of::<OwnedRow>()
            + rows.iter().map(|r| r.bytes).sum::<usize>()
    }
    pub fn clear(&self) {
        *self.rows.borrow_mut() = Vec::new();
        self.notify.reset();
    }
    // Existing callers only change selection flags or a cache-owned thumbnail.
    pub fn update_visual(&self, index: usize, change: impl FnOnce(&mut crate::EntryRow)) {
        let mut rows = self.rows.borrow_mut();
        let Some(row) = rows.get_mut(index) else {
            return;
        };
        change(&mut row.row);
        drop(rows);
        self.notify.row_changed(index);
    }
    pub fn reconcile(&self, incoming: Vec<OwnedRow>) {
        let mut i = 0;
        while i < self.row_count() {
            let keep = incoming
                .iter()
                .any(|r| r.row.key == self.rows.borrow()[i].row.key);
            if keep {
                i += 1;
            } else {
                self.rows.borrow_mut().remove(i);
                self.notify.row_removed(i, 1);
            }
        }
        for (index, row) in incoming.into_iter().enumerate() {
            let same_key = self
                .rows
                .borrow()
                .get(index)
                .is_some_and(|r| r.row.key == row.row.key);
            if same_key {
                if !entry_row_equal(&self.rows.borrow()[index].row, &row.row) {
                    self.rows.borrow_mut()[index] = row;
                    self.notify.row_changed(index);
                }
            } else {
                let found = self
                    .rows
                    .borrow()
                    .iter()
                    .position(|r| r.row.key == row.row.key);
                if let Some(found) = found {
                    self.rows.borrow_mut().remove(found);
                    self.notify.row_removed(found, 1);
                }
                self.rows.borrow_mut().insert(index, row);
                self.notify.row_added(index, 1);
            }
        }
    }
}
impl Model for EntryModel {
    type Data = crate::EntryRow;
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn row_count(&self) -> usize {
        self.rows.borrow().len()
    }
    fn row_data(&self, index: usize) -> Option<Self::Data> {
        self.rows.borrow().get(index).map(|r| r.row.clone())
    }
    fn model_tracker(&self) -> &dyn slint::ModelTracker {
        &self.notify
    }
}

#[cfg(all(test, windows))]
#[test]
fn allocation_accounting_measures_capacity_and_releases_temporary_buffers() {
    let (data, bytes) = echo_windows::allocation::measure_owned(|| {
        let temporary = vec![0u8; 20000];
        std::hint::black_box(&temporary);
        let mut value = Vec::with_capacity(4096);
        value.push(7u8);
        value
    });
    assert_eq!(bytes, data.capacity());
    let (_, clean) = echo_windows::allocation::measure_owned(|| ());
    assert_eq!(clean, 0);
}
/// Slint 1.17's default Image is non-reflexive. Two absent thumbnails still
/// represent identical pixels and must not invalidate every ordinary text row.
pub(crate) fn image_equal(a: &slint::Image, b: &slint::Image) -> bool {
    if a == b {
        return true;
    }
    let (a, b) = (a.size(), b.size());
    a.width == 0 && a.height == 0 && b.width == 0 && b.height == 0
}
pub(crate) fn entry_row_equal(a: &crate::EntryRow, b: &crate::EntryRow) -> bool {
    macro_rules! fields {
        ($($field:ident),* $(,)?) => {{
            // Exhaustive: adding a Slint row field requires updating this comparison.
            let crate::EntryRow { thumbnail: _, title_matches: _, body_matches: _, tags_matches: _, $($field: _,)* } = a;
            image_equal(&a.thumbnail, &b.thumbnail) $(&& a.$field == b.$field)*
        }}
    }
    let same_ranges = |a: &slint::ModelRc<crate::MatchRange>,
                       b: &slint::ModelRc<crate::MatchRange>| {
        a.row_count() == b.row_count() && (0..a.row_count()).all(|i| a.row_data(i) == b.row_data(i))
    };
    same_ranges(&a.title_matches, &b.title_matches)
        && same_ranges(&a.body_matches, &b.body_matches)
        && same_ranges(&a.tags_matches, &b.tags_matches)
        && fields!(
            key,
            title,
            body,
            kind,
            title_rich,
            body_rich,
            tags_rich,
            match_count,
            time_label,
            section_label,
            has_thumbnail,
            pinned,
            selected,
            batch_selected,
            icon_key,
            tags
        )
}
#[cfg(test)]
#[derive(Debug, PartialEq, Eq)]
pub struct Changes {
    pub retained: usize,
    pub inserted: usize,
    pub removed: usize,
}
#[cfg(test)]
pub fn reconcile<T: Clone + PartialEq + 'static>(model: &VecModel<T>, rows: Vec<T>) -> Changes {
    let old = model.row_count();
    let new = rows.len();
    while model.row_count() > new {
        model.remove(model.row_count() - 1);
    }
    for (index, row) in rows.into_iter().enumerate() {
        if index < model.row_count() {
            if model.row_data(index).as_ref() != Some(&row) {
                model.set_row_data(index, row);
            }
        } else {
            model.push(row);
        }
    }
    Changes {
        retained: old.min(new),
        inserted: new.saturating_sub(old),
        removed: old.saturating_sub(new),
    }
}
/// Retain rows by content identity, and don't invalidate unchanged row properties.
/// All notifications are local additions/removals/changes, never a model reset.
#[cfg(test)]
pub fn reconcile_keyed<T: Clone + PartialEq + 'static, K: PartialEq>(
    model: &VecModel<T>,
    rows: Vec<T>,
    key: impl Fn(&T) -> K,
) {
    reconcile_keyed_by(model, rows, key, |a, b| a == b);
}
#[cfg(test)]
pub fn reconcile_keyed_by<T: Clone + 'static, K: PartialEq>(
    model: &VecModel<T>,
    rows: Vec<T>,
    key: impl Fn(&T) -> K,
    equal: impl Fn(&T, &T) -> bool,
) {
    let mut i = 0;
    while i < model.row_count() {
        let current = model.row_data(i).unwrap();
        if rows.iter().any(|r| key(r) == key(&current)) {
            i += 1;
        } else {
            model.remove(i);
        }
    }
    for (index, row) in rows.into_iter().enumerate() {
        if model
            .row_data(index)
            .is_some_and(|old| key(&old) == key(&row))
        {
            if !model.row_data(index).is_some_and(|old| equal(&old, &row)) {
                model.set_row_data(index, row);
            }
        } else {
            if let Some(found) = (index..model.row_count())
                .find(|&j| model.row_data(j).is_some_and(|old| key(&old) == key(&row)))
            {
                model.remove(found);
            }
            model.insert(index, row);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn retained_row_keeps_its_charge_and_backing_capacity_is_counted() {
        let model = EntryModel::default();
        let row = |key: &str, body: &str, bytes| OwnedRow {
            row: crate::EntryRow {
                key: key.into(),
                body: body.into(),
                ..Default::default()
            },
            bytes,
        };
        model.reconcile(vec![row("a", "same", 600), row("b", "old", 200)]);
        let container = model.held_bytes() - 800;
        model.reconcile(vec![row("a", "same", 10), row("b", "new", 300)]);
        assert_eq!(model.held_bytes(), container + 900);
        model.reconcile(vec![row("b", "new", 10)]);
        assert_eq!(model.held_bytes(), container + 300);
        model.clear();
        assert_eq!(model.held_bytes(), std::mem::size_of::<EntryModel>());
    }
    #[test]
    fn visual_row_equality_handles_absent_and_changed_thumbnails() {
        let row = crate::EntryRow {
            key: "opaque-key".into(),
            body: "unchanged text".into(),
            ..Default::default()
        };
        assert!(entry_row_equal(&row, &row.clone()));
        let mut next = row.clone();
        next.body_rich = slint::StyledText::from_markdown("**highlight**").unwrap();
        assert!(!entry_row_equal(&row, &next));
        next = row.clone();
        next.thumbnail = slint::Image::from_rgba8(slint::SharedPixelBuffer::new(2, 2));
        assert!(!entry_row_equal(&row, &next));
        assert!(entry_row_equal(&next, &next.clone()));
        let mut pixels = slint::SharedPixelBuffer::new(2, 2);
        pixels.make_mut_bytes().fill(255);
        let mut changed = next.clone();
        changed.thumbnail = slint::Image::from_rgba8(pixels);
        assert!(!entry_row_equal(&next, &changed));
    }
    #[test]
    fn repeated_refresh_reuses_all_existing_row_slots() {
        let model = VecModel::from((0..50).collect::<Vec<i32>>());
        for round in 0..550 {
            let change = reconcile(&model, (0..50).map(|i| i + round).collect());
            assert_eq!(
                change,
                Changes {
                    retained: 50,
                    inserted: 0,
                    removed: 0
                }
            );
            assert_eq!(model.row_data(49), Some(49 + round));
        }
    }
    #[test]
    fn shrink_and_growth_preserve_the_common_prefix_and_order() {
        let model = VecModel::from(vec!["a", "b", "c"]);
        assert_eq!(
            reconcile(&model, vec!["x", "y"]),
            Changes {
                retained: 2,
                inserted: 0,
                removed: 1
            }
        );
        assert_eq!(
            reconcile(&model, vec!["p", "q", "r", "s"]),
            Changes {
                retained: 2,
                inserted: 2,
                removed: 0
            }
        );
        assert_eq!(model.iter().collect::<Vec<_>>(), vec!["p", "q", "r", "s"]);
        assert_eq!(
            reconcile(&model, vec![]),
            Changes {
                retained: 0,
                inserted: 0,
                removed: 4
            }
        );
    }
}

#[cfg(test)]
mod keyed_tests {
    use super::*;
    #[test]
    fn narrowing_and_widening_preserve_order_and_current_content() {
        let model = VecModel::from(vec![(1, "a"), (2, "b"), (3, "c"), (4, "d")]);
        reconcile_keyed(&model, vec![(2, "b"), (4, "d")], |r| r.0);
        assert_eq!(model.iter().collect::<Vec<_>>(), vec![(2, "b"), (4, "d")]);
        reconcile_keyed(&model, vec![(4, "new"), (1, "a"), (2, "b")], |r| r.0);
        assert_eq!(
            model.iter().collect::<Vec<_>>(),
            vec![(4, "new"), (1, "a"), (2, "b")]
        );
        reconcile_keyed(&model, vec![], |r| r.0);
        assert_eq!(model.row_count(), 0);
    }
}
