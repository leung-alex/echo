//! Reconcile bounded native rows without resetting their accessible identities.
use slint::{Model, VecModel};
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
pub fn reconcile_keyed<T: Clone + PartialEq + 'static, K: PartialEq>(
    model: &VecModel<T>,
    rows: Vec<T>,
    key: impl Fn(&T) -> K,
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
            if model.row_data(index).as_ref() != Some(&row) {
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
