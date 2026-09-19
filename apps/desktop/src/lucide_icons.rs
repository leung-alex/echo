//! UI-thread adapters for the independently compiled Lucide resource catalog.
use crate::FavoriteIconChoice;
use slint::{ModelRc, VecModel};
use std::rc::Rc;

pub fn choices() -> ModelRc<FavoriteIconChoice> {
    choices_matching("")
}

pub fn choices_matching(query: &str) -> ModelRc<FavoriteIconChoice> {
    let rows = echo_icon_assets::choices_matching(query)
        .into_iter()
        .map(|entry| FavoriteIconChoice {
            key: entry.key.into(),
            label: entry.label.into(),
        })
        .collect::<Vec<_>>();
    Rc::new(VecModel::from(rows)).into()
}

pub fn normalize_user_key(key: Option<&str>) -> Option<String> {
    key.filter(|key| echo_icon_assets::contains(key))
        .map(str::to_owned)
}

// Slint retains the result in the requesting image property's binding. Only
// instantiated rows/surfaces load SVGs; there is no process-wide gallery cache
// keeping decoded icons alive after those components are released.
pub fn image(key: &str) -> slint::Image {
    echo_icon_assets::svg(key)
        .and_then(|data| slint::Image::load_from_svg_data(data).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::Model;

    #[test]
    fn searches_and_validates_persisted_user_keys() {
        assert!(choices_matching("wheelchair")
            .iter()
            .any(|e| e.key == "lucide-accessibility"));
        assert_eq!(normalize_user_key(Some("Mail")), None);
        assert_eq!(
            normalize_user_key(Some("lucide-activity")).as_deref(),
            Some("lucide-activity")
        );
    }

    #[test]
    fn every_original_svg_decodes_and_unknown_keys_stay_empty() {
        for entry in echo_icon_assets::choices_matching("") {
            let decoded =
                slint::Image::load_from_svg_data(echo_icon_assets::svg(entry.key).unwrap())
                    .unwrap_or_else(|_| panic!("invalid SVG: {}", entry.key));
            assert_eq!(
                (decoded.size().width, decoded.size().height),
                (24, 24),
                "{}",
                entry.key
            );
        }
        assert_eq!(
            (
                image("lucide-unknown").size().width,
                image("lucide-unknown").size().height
            ),
            (0, 0)
        );
    }
}
