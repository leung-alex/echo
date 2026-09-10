//! Pure validation and domain-to-view formatting.
use crate::EntryRow;
use chrono::{Local, TimeZone};
use echo_engine::{ClipboardSettings, QuickInsertItem, ThemeMode};
use echo_presentation::RowKey;
#[cfg(feature = "cover-flow")]
pub fn row(item: &QuickInsertItem, previous_section: &mut String) -> EntryRow {
    let section = row_section(item, previous_section);
    row_content(item, &section)
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
        source_label: item
            .source_app
            .clone()
            .unwrap_or_else(|| "Clipboard".into())
            .into(),
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
        source_rich: slint::StyledText::from_plain_text(
            item.source_app.as_deref().unwrap_or("Clipboard"),
        ),
        match_count: 0,
    }
}
pub fn optional(value: &str) -> Option<String> {
    let v = value.trim();
    (!v.is_empty()).then(|| v.to_owned())
}
pub fn settings(
    entries: &str,
    total_mib: &str,
    item_mib: &str,
    theme: &str,
    enabled: bool,
    sensitive: bool,
    titles: bool,
) -> Result<ClipboardSettings, String> {
    let max_entries = entries
        .trim()
        .parse::<u32>()
        .map_err(|_| "Maximum entries must be a positive integer")?;
    let parse_mib = |value: &str| -> Result<u64, String> {
        let n = value
            .trim()
            .parse::<u64>()
            .map_err(|_| "Storage limits must be positive integer MiB")?;
        n.checked_mul(1024 * 1024)
            .filter(|n| *n > 0)
            .ok_or_else(|| "Storage limit is invalid or too large".into())
    };
    if max_entries == 0 || max_entries > echo_engine::MAX_HISTORY_ENTRIES {
        return Err("Maximum entries must be between 1 and 2000".into());
    }
    let max_total_bytes = parse_mib(total_mib)?;
    let max_item_bytes = parse_mib(item_mib)?;
    if max_item_bytes > max_total_bytes {
        return Err("Maximum item size cannot exceed total storage".into());
    }
    let theme = ThemeMode::parse(theme).ok_or("Unknown theme")?;
    Ok(ClipboardSettings {
        history_enabled: enabled,
        record_sensitive: sensitive,
        store_window_titles: titles,
        max_entries,
        max_total_bytes,
        max_item_bytes,
        theme,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn limits_reject_overflow_and_zero() {
        for (entries, total, item) in [
            ("0", "10", "1"),
            ("2001", "10", "1"),
            ("1", "0", "1"),
            ("1", "1", "2"),
            ("1", "18446744073709551615", "1"),
            ("-1", "10", "1"),
        ] {
            assert!(settings(entries, total, item, "system", true, false, false).is_err());
        }
    }
    #[test]
    fn settings_preserve_all_fields() {
        let s = settings("2000", "512", "32", "dark", false, true, true).unwrap();
        assert_eq!(s.max_total_bytes, 512 * 1024 * 1024);
        assert_eq!(s.theme, ThemeMode::Dark);
        assert!(!s.history_enabled);
        assert!(s.record_sensitive && s.store_window_titles);
    }
}
