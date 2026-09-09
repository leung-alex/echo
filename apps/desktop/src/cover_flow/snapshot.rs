//! Exact, bounded visual input for a cached card. Projection is deliberately separate.
use crate::{AppWindow, EntryRow};
use slint::{Color, Model, SharedString};
fn row_changes(rows: &[EntryRow], old: &[EntryRow]) -> Vec<&'static str> {
    let mut changed = Vec::new();
    if rows.len() != old.len() {
        changed.push("row_count");
    }
    macro_rules! fields {
        ($($name:ident),*) => { $(if rows.iter().zip(old).any(|(a,b)| a.$name != b.$name) { changed.push(stringify!($name)); })* }
    }
    fields!(
        key,
        title,
        body,
        kind,
        title_rich,
        body_rich,
        tags_rich,
        source_rich,
        match_count,
        source_label,
        time_label,
        section_label,
        has_thumbnail,
        pinned,
        selected,
        batch_selected,
        icon_key,
        tags
    );
    if rows
        .iter()
        .zip(old)
        .any(|(a, b)| !crate::native_model::image_equal(&a.thumbnail, &b.thumbnail))
    {
        changed.push("thumbnail");
    }
    changed
}

macro_rules! snapshot {
    ($($field:ident : $ty:ty = $read:ident => $write:ident),* $(,)?) => {
        #[derive(Clone, Default)]
        pub(super) struct PanelSnapshot {
            pub width: f32,
            pub height: f32,
            pub dpi: f32,
            pub compact: bool,
            pub rows: Vec<EntryRow>,
            $(pub $field: $ty,)*
        }
        impl PartialEq for PanelSnapshot {
            fn eq(&self, other: &Self) -> bool {
                self.width == other.width && self.height == other.height && self.dpi == other.dpi
                    && self.compact == other.compact && self.rows.len() == other.rows.len()
                    && self.rows.iter().zip(&other.rows).all(|(a,b)| crate::native_model::entry_row_equal(a,b))
                    $(&& self.$field == other.$field)*
            }
        }
        impl PanelSnapshot {
            pub fn read(source: &AppWindow, dpi: f32) -> Self {
                let mut snapshot = Self {
                    width: source.get_panel_width(), height: source.get_panel_height(), dpi,
                    compact: source.get_density().as_str() == "compact",
                    rows: source.get_capture_rows().iter().collect(),
                    $($field: source.$read(),)*
                };
                if snapshot.titles_only {
                    snapshot.rows.clear();
                    snapshot.query = SharedString::default();
                }
                snapshot
            }
            pub fn apply(&self, target: &crate::CardSnapshot) {
                $(target.$write(self.$field.clone());)*
                target.set_compact(self.compact);
            }
            pub fn changes(&self, other: &Self) -> Vec<&'static str> {
                let mut changed = Vec::new();
                if self.width != other.width { changed.push("width"); }
                if self.height != other.height { changed.push("height"); }
                if self.dpi != other.dpi { changed.push("dpi"); }
                if self.compact != other.compact { changed.push("compact"); }
                changed.extend(row_changes(&self.rows, &other.rows));
                $(if self.$field != other.$field { changed.push(stringify!($field)); })*
                changed
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cached_pixels_change_with_content_geometry_and_presentation() {
        let original = PanelSnapshot {
            width: 520.0,
            height: 300.0,
            dpi: 2.0,
            rows: vec![EntryRow::default()],
            ..Default::default()
        };
        assert!(original == original.clone());
        let changes: [fn(&mut PanelSnapshot); 10] = [
            |s| s.query = "new query".into(),
            |s| s.title = "renamed".into(),
            |s| s.height += 20.0,
            |s| s.dpi = 1.0,
            |s| s.dark = true,
            |s| s.titles_only = true,
            |s| s.scroll = -40.0,
            |s| s.inline_mode = true,
            |s| s.rows[0].selected = true,
            |s| s.rows.clear(),
        ];
        for change in changes {
            let mut next = original.clone();
            change(&mut next);
            assert!(original != next);
        }
    }
}

snapshot! {
    dark: bool = get_dark => set_dark,
    title: SharedString = get_capture_title => set_panel_title,
    subtitle: SharedString = get_capture_subtitle => set_subtitle,
    icon: SharedString = get_capture_icon => set_icon_key,
    accent: Color = get_capture_accent => set_accent,
    favorites: bool = get_capture_favorites => set_favorites,
    loading: bool = get_capture_loading => set_loading,
    titles_only: bool = get_capture_titles_only => set_titles_only,
    scroll: f32 = get_capture_scroll => set_scroll_y,
    query: SharedString = get_capture_query => set_query,
    navigation_label: SharedString = get_capture_navigation_label => set_navigation_label,
    navigation_hint: SharedString = get_capture_navigation_hint => set_navigation_hint,
    previous_enabled: bool = get_capture_previous_enabled => set_previous_enabled,
    next_enabled: bool = get_capture_next_enabled => set_next_enabled,
    has_more: bool = get_capture_has_more => set_has_more,
    has_previous: bool = get_capture_has_previous => set_has_previous,
    batch: bool = get_capture_batch => set_batch,
    selected_count: i32 = get_capture_selected_count => set_selected_count,
    quick_insert: bool = get_capture_quick_insert => set_quick_insert,
    inline_mode: bool = get_inline_mode => set_inline_mode,
}
