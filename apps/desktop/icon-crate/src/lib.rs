//! Immutable icon metadata and SVG bytes, independent of Slint and desktop code.
struct Entry {
    key: &'static str,
    label: &'static str,
    terms: &'static [&'static str],
    start: usize,
    end: usize,
}

include!(concat!(env!("OUT_DIR"), "/catalog.rs"));
static SVG_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/icons.bin"));

struct SystemEntry {
    key: &'static str,
    start: usize,
    end: usize,
}

include!(concat!(env!("OUT_DIR"), "/system_catalog.rs"));
static SYSTEM_SVG_BYTES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/system_icons.bin"));

/// Original SVG for a persisted system/legacy key; sentinel and unknown keys are empty.
#[inline(never)]
pub fn system_svg(key: &str) -> Option<&'static [u8]> {
    let index = SYSTEM_CATALOG
        .binary_search_by_key(&key, |entry| entry.key)
        .ok()?;
    let entry = &SYSTEM_CATALOG[index];
    Some(&SYSTEM_SVG_BYTES[entry.start..entry.end])
}

#[derive(Clone, Copy)]
pub struct IconChoice {
    pub key: &'static str,
    pub label: &'static str,
}

// Keep the resource/search implementation out of host monomorphization.
#[inline(never)]
pub fn choices_matching(query: &str) -> Vec<IconChoice> {
    let needle = query.trim().to_lowercase();
    CATALOG
        .iter()
        .filter(|entry| needle.is_empty() || entry.terms.iter().any(|term| term.contains(&needle)))
        .map(|entry| IconChoice {
            key: entry.key,
            label: entry.label,
        })
        .collect()
}

#[inline(never)]
pub fn svg(key: &str) -> Option<&'static [u8]> {
    let index = CATALOG.binary_search_by_key(&key, |entry| entry.key).ok()?;
    let entry = &CATALOG[index];
    Some(&SVG_BYTES[entry.start..entry.end])
}

pub fn contains(key: &str) -> bool {
    svg(key).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_and_packed_svg_ranges_cover_every_vendored_icon() {
        assert_eq!(CATALOG.len(), 2112);
        assert!(CATALOG
            .windows(2)
            .all(|p| p[0].key < p[1].key && p[0].end == p[1].start));
        assert_eq!(CATALOG[0].start, 0);
        assert_eq!(CATALOG.last().unwrap().end, SVG_BYTES.len());
        for entry in CATALOG {
            let name = entry.key.strip_prefix("lucide-").unwrap();
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(format!("../ui/lucide-icons/{name}.svg"));
            assert_eq!(
                svg(entry.key).unwrap(),
                std::fs::read(path).unwrap(),
                "{}",
                entry.key
            );
        }
    }

    #[test]
    fn legacy_catalog_preserves_curated_order_translations_and_original_svg_bytes() {
        let catalog: serde_json::Value =
            serde_json::from_str(include_str!("../system-catalog.json")).unwrap();
        let choices = catalog.as_array().unwrap();
        let keys = choices
            .iter()
            .map(|e| e["key"].as_str().unwrap())
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(choices.len(), 746);
        assert_eq!(keys.len(), choices.len());
        assert_eq!(choices[0]["key"], "none");
        assert_eq!(choices[1]["key"], "Mail");
        let translations: serde_json::Value =
            serde_json::from_str(include_str!("../../i18n/zh-CN.json")).unwrap();
        for choice in choices {
            let key = choice["key"].as_str().unwrap();
            let label = choice["label"].as_str().unwrap();
            assert!(
                translations.get(label).is_some(),
                "missing icon translation: {label}"
            );
            if key != "none" {
                let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("../ui/favorite-icons/{key}.svg"));
                assert_eq!(
                    system_svg(key).unwrap(),
                    std::fs::read(path).unwrap(),
                    "{key}"
                );
            }
        }
        assert_eq!(SYSTEM_CATALOG.len(), 745);
        assert_eq!(SYSTEM_CATALOG[0].start, 0);
        assert_eq!(SYSTEM_CATALOG.last().unwrap().end, SYSTEM_SVG_BYTES.len());
        assert!(SYSTEM_CATALOG
            .windows(2)
            .all(|p| p[0].key < p[1].key && p[0].end == p[1].start));
        for key in [
            "",
            "none",
            "mail",
            "../History",
            "lucide-activity",
            "unknown",
        ] {
            assert!(system_svg(key).is_none(), "{key}");
        }
    }

    #[test]
    fn searches_full_catalog_and_preserves_aliases() {
        assert!(choices_matching(" ACTIVITY ")
            .iter()
            .any(|e| e.key == "lucide-activity"));
        assert!(choices_matching("wheelchair")
            .iter()
            .any(|e| e.key == "lucide-accessibility"));
        assert_eq!(choices_matching("").last().unwrap().key, "lucide-zoom-out");
        for key in ["", "none", "Mail", "lucide-does-not-exist"] {
            assert!(svg(key).is_none());
        }
    }
}
