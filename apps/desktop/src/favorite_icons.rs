//! UI-thread decoding for the separately compiled system/legacy SVG catalog.
pub fn image(key: &str) -> slint::Image {
    echo_icon_assets::system_svg(key)
        .and_then(|data| slint::Image::load_from_svg_data(data).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_system_svg_decodes_and_unknown_keys_stay_empty() {
        let catalog: serde_json::Value =
            serde_json::from_str(include_str!("../icon-crate/system-catalog.json")).unwrap();
        for entry in catalog.as_array().unwrap() {
            let key = entry["key"].as_str().unwrap();
            if key == "none" {
                continue;
            }
            let decoded = image(key);
            assert!(
                decoded.size().width > 0 && decoded.size().height > 0,
                "{key}"
            );
        }
        for key in ["", "none", "unknown", "lucide-activity"] {
            let size = image(key).size();
            assert_eq!((size.width, size.height), (0, 0), "{key}");
        }
    }
}
