//! Reconcile bounded native rows without resetting their accessible identities.
use slint::{Model, VecModel};
#[derive(Debug, PartialEq, Eq)]
pub struct Changes {
    pub retained: usize,
    pub inserted: usize,
    pub removed: usize,
}
pub fn reconcile<T: Clone + 'static>(model: &VecModel<T>, rows: Vec<T>) -> Changes {
    let old = model.row_count();
    let new = rows.len();
    while model.row_count() > new {
        model.remove(model.row_count() - 1);
    }
    for (index, row) in rows.into_iter().enumerate() {
        if index < model.row_count() {
            model.set_row_data(index, row);
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
