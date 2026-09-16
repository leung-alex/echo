//! Search returns original indices so filtering never changes selection identity.
use slint::{Model, ModelRc, SharedString, VecModel};

pub(crate) fn filter(model: ModelRc<SharedString>, query: SharedString) -> ModelRc<i32> {
    let needle = query.trim().to_lowercase();
    let indices: Vec<i32> = model
        .iter()
        .enumerate()
        .filter(|(_, label)| needle.is_empty() || label.to_lowercase().contains(&needle))
        .map(|(index, _)| index as i32)
        .collect();
    ModelRc::new(VecModel::from(indices))
}

#[test]
fn filtering_preserves_duplicate_labels_and_original_indices() {
    let model = ModelRc::new(VecModel::from(vec![
        "History".into(),
        "工作".into(),
        "WORK".into(),
        "工作".into(),
    ]));
    assert_eq!(
        filter(model.clone(), " 工作 ".into())
            .iter()
            .collect::<Vec<_>>(),
        [1, 3]
    );
    assert_eq!(
        filter(model.clone(), "work".into())
            .iter()
            .collect::<Vec<_>>(),
        [2]
    );
    assert_eq!(filter(model.clone(), "".into()).row_count(), 4);
    assert_eq!(filter(model, "missing".into()).row_count(), 0);
}
