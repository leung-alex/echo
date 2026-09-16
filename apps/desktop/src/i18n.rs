//! UI message catalog. Content, commands, diagnostics and stored identities never pass here.
use echo_engine::Language;
use std::{collections::BTreeMap, sync::OnceLock};

fn catalog() -> &'static BTreeMap<String, String> {
    static CATALOG: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        serde_json::from_str(include_str!("../i18n/zh-CN.json"))
            .expect("validated translation catalog")
    })
}

/// A message ID is its English source, as in Slint/gettext. Parameters are values,
/// not translatable strings; in particular names, paths and error details stay intact.
pub fn message(language: Language, id: &str, parameters: &[(&str, &str)]) -> String {
    let mut result = if language == Language::Chinese {
        catalog().get(id).map(String::as_str).unwrap_or(id)
    } else {
        id
    }
    .to_owned();
    // Substitute in one pass, so braces inside a user-supplied value are literal.
    let source = std::mem::take(&mut result);
    let mut rest = source.as_str();
    while let Some(start) = rest.find('{') {
        result.push_str(&rest[..start]);
        let Some(end) = rest[start..].find('}') else {
            result.push_str(&rest[start..]);
            return result;
        };
        let end = start + end;
        let key = &rest[start + 1..end];
        result.push_str(
            parameters
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| *value)
                .unwrap_or(&rest[start..=end]),
        );
        rest = &rest[end + 1..];
    }
    result.push_str(rest);
    result
}

/// Adapter for source messages from existing worker/error contracts. Match only
/// catalogued templates; never substitute or translate within their parameters.
pub fn text(language: Language, source: &str) -> String {
    if language == Language::English || source.is_empty() {
        return source.to_owned();
    }
    if let Some(translated) = catalog().get(source) {
        return translated.clone();
    }
    for id in catalog().keys().filter(|id| id.contains('{')) {
        if let Some(parameters) = parameters(id, source) {
            return message(language, id, &parameters);
        }
    }
    source.to_owned()
}

fn parameters<'a, 'b>(mut id: &'a str, mut source: &'b str) -> Option<Vec<(&'a str, &'b str)>> {
    let mut result = Vec::new();
    while let Some(start) = id.find('{') {
        source = source.strip_prefix(&id[..start])?;
        let end = id.find('}')?;
        let key = &id[start + 1..end];
        id = &id[end + 1..];
        let boundary = id.find('{').unwrap_or(id.len());
        let separator = &id[..boundary];
        let value = if boundary == id.len() {
            source.strip_suffix(separator)?
        } else {
            source.get(..source.find(separator)?)?
        };
        result.push((key, value));
        source = &source[value.len()..];
    }
    (source == id).then_some(result)
}

#[cfg(windows)]
pub fn tray_labels(language: Language) -> echo_windows::shell::TrayLabels {
    echo_windows::shell::TrayLabels {
        tooltip: text(language, "Echo — clipboard history"),
        menu: ["Open Echo", "Favorites", "Settings", "Quit Echo"].map(|s| text(language, s)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn placeholders(s: &str) -> Vec<&str> {
        let mut values = Vec::new();
        let mut rest = s;
        while let Some(start) = rest.find('{') {
            let end = rest[start..].find('}').expect("closed placeholder") + start;
            values.push(&rest[start..=end]);
            rest = &rest[end + 1..];
        }
        values.sort();
        values
    }
    #[test]
    fn catalog_has_translations_and_identical_parameters() {
        for (id, zh) in catalog() {
            assert!(!zh.trim().is_empty(), "{id}");
            assert_eq!(placeholders(id), placeholders(zh), "{id}");
        }
    }
    #[test]
    fn confirmation_source_strings_have_chinese_translations() {
        for source in [include_str!("app.rs"), include_str!("app/dialogs.rs")] {
            for call in source.split("self.ask_confirmation(").skip(1) {
                let mut arguments = call.split("Confirmation::").next().unwrap();
                while let Some(start) = arguments.find('"') {
                    let mut strings = serde_json::Deserializer::from_str(&arguments[start..])
                        .into_iter::<String>();
                    let id = strings.next().unwrap().unwrap();
                    assert!(
                        catalog().contains_key(&id),
                        "missing confirmation translation: {id}"
                    );
                    arguments = &arguments[start + strings.byte_offset()..];
                }
            }
        }
    }
    #[test]
    fn every_slint_translation_is_in_the_catalog() {
        let ui = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
        for entry in std::fs::read_dir(ui).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "slint") {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            for part in source.split("@tr(").skip(1) {
                let mut values =
                    serde_json::Deserializer::from_str(part.trim_start()).into_iter::<String>();
                let id = values
                    .next()
                    .expect("literal translation")
                    .expect("quoted message ID");
                assert!(
                    catalog().contains_key(&id),
                    "{}: missing {id}",
                    path.display()
                );
            }
        }
    }
    #[test]
    fn parameters_and_unknown_external_errors_are_not_translated() {
        assert_eq!(
            text(
                Language::Chinese,
                "Could not focus settings: History {error}"
            ),
            "无法聚焦 settings：History {error}"
        );
        assert_eq!(
            text(Language::Chinese, "Delete ‘History’ space?"),
            "删除“History”空间？"
        );
        assert_eq!(
            message(
                Language::Chinese,
                "Operation failed: {error}",
                &[("error", "History {error}")]
            ),
            "操作失败：History {error}"
        );
        assert_eq!(
            text(Language::Chinese, "HRESULT 0x123: external detail"),
            "HRESULT 0x123: external detail"
        );
        assert_eq!(text(Language::English, "Settings"), "Settings");
    }
}
