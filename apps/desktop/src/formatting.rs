//! Pure validation and domain-to-view formatting.
use crate::EntryRow;
use chrono::{Local, TimeZone};
use echo_engine::{ClipboardSettings, QuickInsertItem, ThemeMode};
use echo_presentation::RowKey;
pub fn hotkey_status_is_informational(status: &str) -> bool {
    status.is_empty()
        || status.starts_with("Active globally: ")
        || status == "Global shortcut is off"
        || status
            == "Global shortcut disabled for this isolated instance (ECHO_DISABLE_GLOBAL_HOTKEY)"
}

#[test]
fn hotkey_errors_remain_visible_even_when_the_previous_binding_is_active() {
    assert!(hotkey_status_is_informational("Active globally: Alt+V"));
    assert!(hotkey_status_is_informational("Global shortcut is off"));
    assert!(!hotkey_status_is_informational(
        "Cannot register Ctrl+Alt+J. Active globally: Alt+V. Your previous binding is unchanged."
    ));
}
pub fn row_section(item: &QuickInsertItem, previous_section: &mut String) -> String {
    let time = Local.timestamp_millis_opt(item.updated_at).single();
    let date = time.map(|t| t.date_naive());
    let today = Local::now().date_naive();
    let section = if item.pinned_at.is_some() {
        "Pinned".to_owned()
    } else if date == Some(today) {
        "Today".to_owned()
    } else if date == today.pred_opt() {
        "Yesterday".to_owned()
    } else {
        time.map(|t| t.format("%b %d, %Y").to_string())
            .unwrap_or_default()
    };
    let section_label = if *previous_section == section {
        String::new()
    } else {
        section.clone()
    };
    *previous_section = section;
    section_label
}
/// Build a fresh row using borrowed inputs, so its owned allocations can be charged.
pub fn row_content(item: &QuickInsertItem, section_label: &str) -> EntryRow {
    let time = Local.timestamp_millis_opt(item.updated_at).single();
    EntryRow {
        key: RowKey::of(item).to_string().into(),
        title: item
            .name
            .clone()
            .unwrap_or_else(|| {
                if item.source == echo_engine::QuickInsertSource::Favorite {
                    "Untitled favorite".into()
                } else {
                    String::new()
                }
            })
            .into(),
        body: item.preview_text.clone().unwrap_or_default().into(),
        kind: item.content_type.clone().into(),
        time_label: time
            .map(|t| t.format("%H:%M").to_string())
            .unwrap_or_default()
            .into(),
        section_label: section_label.into(),
        pinned: item.pinned_at.is_some(),
        has_thumbnail: item.thumbnail.is_some(),
        thumbnail: Default::default(),
        selected: false,
        batch_selected: false,
        icon_key: item.icon_key.clone().unwrap_or_default().into(),
        tags: item.tags.join(" · ").into(),
        body_rich: slint::StyledText::from_plain_text(item.preview_text.as_deref().unwrap_or("")),
        title_rich: slint::StyledText::from_plain_text(item.name.as_deref().unwrap_or("")),
        tags_rich: slint::StyledText::from_plain_text(&item.tags.join(" · ")),
        match_count: 0,
        title_matches: Default::default(),
        body_matches: Default::default(),
        tags_matches: Default::default(),
    }
}
pub fn optional(value: &str) -> Option<String> {
    let v = value.trim();
    (!v.is_empty()).then(|| v.to_owned())
}
pub fn settings(theme: &str) -> Result<ClipboardSettings, String> {
    let theme = ThemeMode::parse(theme).ok_or("Unknown theme")?;
    Ok(ClipboardSettings {
        theme,
        ..ClipboardSettings::default()
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_identity_is_not_projected_or_counted_as_a_visible_match() {
        let item: QuickInsertItem = serde_json::from_value(serde_json::json!({
            "id": 1, "source": "history", "content_type": "text",
            "preview_text": "visible payload", "tags": [], "updated_at": 0,
            "source_app": "PrivateSource"
        }))
        .unwrap();
        let mut row = row_content(&item, "Today");
        crate::match_highlight::apply(
            &mut row,
            &mut echo_engine::FuzzyMatcher::new("PrivateSource"),
            "#285f80",
        );
        assert_eq!(row.match_count, 0);
        assert_eq!(row.body.as_str(), "visible payload");
        assert!(row.title.is_empty());
        assert_eq!(item.source_app.as_deref(), Some("PrivateSource"));
        crate::match_highlight::apply(
            &mut row,
            &mut echo_engine::FuzzyMatcher::new("visible"),
            "#285f80",
        );
        assert!(row.match_count > 0);
    }
    #[test]
    fn settings_preserve_all_fields() {
        let s = settings("dark").unwrap();
        assert_eq!(s.max_total_bytes, 512 * 1024 * 1024);
        assert_eq!(s.theme, ThemeMode::Dark);
        assert!(s.history_enabled);
        assert!(s.record_sensitive);
    }
}
