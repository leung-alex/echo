// Generated from design/tokens/echo.tokens.json.
use super::{StyleValue, StyleSnapshot};
#[cfg(any(debug_assertions, test))]
pub(super) const SCHEMA: &[(&str, &str, bool)] = &[
    ("budget.live-model-lru", "integer", false),
    ("budget.page-size", "integer", false),
    ("budget.texture-hard-mib", "integer", false),
    ("budget.texture-mib", "integer", false),
    ("budget.total-resident-rows", "integer", false),
    ("budget.transient-cpu-mib", "integer", false),
    ("budget.transition-panels", "integer", false),
    ("budget.visible-panels", "integer", false),
    ("color.accent", "color", true),
    ("color.accent-text", "color", true),
    ("color.canvas", "color", true),
    ("color.danger", "color", true),
    ("color.faint", "color", true),
    ("color.focus", "color", true),
    ("color.line", "color", true),
    ("color.muted", "color", true),
    ("color.on-accent", "color", true),
    ("color.panel", "color", true),
    ("color.raised", "color", true),
    ("color.row-text", "color", true),
    ("color.search-background", "color", true),
    ("color.search-text", "color", true),
    ("color.selection", "color", true),
    ("color.shadow", "color", true),
    ("color.stage-edge", "color", true),
    ("color.success", "color", true),
    ("color.text", "color", true),
    ("control.height", "length", true),
    ("control.hit-size", "length", true),
    ("control.icon-size", "length", true),
    ("control.radius", "length", true),
    ("dialog.radius", "length", true),
    ("dialog.width", "length", true),
    ("flow.shadow-offset-y", "length", false),
    ("flow.shadow-opacity", "number", false),
    ("flow.shadow-softness", "length", false),
    ("focus.offset", "length", true),
    ("focus.width", "length", true),
    ("font.body", "length", true),
    ("font.caption", "length", true),
    ("font.code", "length", true),
    ("font.family", "string", false),
    ("font.line-height", "length", true),
    ("font.mono", "string", false),
    ("font.panel-title", "length", true),
    ("font.secondary", "length", true),
    ("font.setting-title", "length", true),
    ("font.weight-medium", "integer", true),
    ("font.weight-normal", "integer", true),
    ("font.weight-semibold", "integer", true),
    ("line.width", "length", true),
    ("panel.footer-height", "length", false),
    ("panel.header-height", "length", false),
    ("panel.max-width", "length", false),
    ("panel.min-width", "length", false),
    ("panel.padding", "length", false),
    ("panel.radius", "length", false),
    ("panel.width-ratio", "number", false),
    ("quality.card-raster-economical", "number", false),
    ("quality.card-raster-standard", "number", false),
    ("row.action-color", "color", true),
    ("row.action-gap", "length", true),
    ("row.action-hover", "color", true),
    ("row.action-icon-size", "length", true),
    ("row.action-radius", "length", true),
    ("row.action-size", "length", true),
    ("row.compact-height", "length", true),
    ("row.history-min-height", "length", true),
    ("row.icon-size", "length", true),
    ("row.image-height", "length", true),
    ("row.max-text-height", "length", true),
    ("row.radius", "length", true),
    ("row.saved-min-height", "length", true),
    ("row.selected-shadow-color", "color", true),
    ("row.selected-shadow-offset-y", "length", true),
    ("row.selected-shadow-softness", "length", true),
    ("row.shadow-color", "color", true),
    ("row.shadow-offset-y", "length", true),
    ("row.shadow-softness", "length", true),
    ("row.timeline-width", "length", true),
    ("search.height", "length", true),
    ("search.radius", "length", true),
    ("settings.card-max-width", "length", false),
    ("settings.max-form-width", "length", false),
    ("settings.nav-width", "length", false),
    ("settings.row-height", "length", false),
    ("shell.footer-height", "length", false),
    ("shell.header-height", "length", false),
    ("space.1", "length", true),
    ("space.10", "length", true),
    ("space.2", "length", true),
    ("space.3", "length", true),
    ("space.4", "length", true),
    ("space.5", "length", true),
    ("space.6", "length", true),
    ("space.8", "length", true),
    ("stage.padding-x", "length", false),
    ("stage.padding-y", "length", false),
    ("window.height", "length", false),
    ("window.min-height", "length", false),
    ("window.min-width", "length", false),
    ("window.width", "length", false),
    ("window.workarea-margin", "length", false),
];
pub(super) fn defaults() -> StyleSnapshot {
    StyleSnapshot(std::collections::BTreeMap::from([
        ("color.accent".into(), StyleValue::Color(0x202020ff, 0xeeeeeeff)),
        ("color.accent-text".into(), StyleValue::Color(0x202020ff, 0xeeeeeeff)),
        ("color.canvas".into(), StyleValue::Color(0xe9edebff, 0x0b1014ff)),
        ("color.danger".into(), StyleValue::Color(0xa62727ff, 0xff9898ff)),
        ("color.faint".into(), StyleValue::Color(0x707981ff, 0x929da6ff)),
        ("color.focus".into(), StyleValue::Color(0x202020ff, 0xeeeeeeff)),
        ("color.line".into(), StyleValue::Color(0xe4e7e9ff, 0x35434eff)),
        ("color.muted".into(), StyleValue::Color(0x707070ff, 0xbcbcbcff)),
        ("color.on-accent".into(), StyleValue::Color(0xffffffff, 0x111111ff)),
        ("color.panel".into(), StyleValue::Color(0xf7f7f6ff, 0x161d23ff)),
        ("color.raised".into(), StyleValue::Color(0xffffffff, 0x1d252cff)),
        ("color.row-text".into(), StyleValue::Color(0x626a70ff, 0xf4f6f7ff)),
        ("color.search-background".into(), StyleValue::Color(0xffe59aff, 0x69501cff)),
        ("color.search-text".into(), StyleValue::Color(0x493600ff, 0xfff0bdff)),
        ("color.selection".into(), StyleValue::Color(0xeeeeeeff, 0x383838ff)),
        ("color.shadow".into(), StyleValue::Color(0x00000020, 0x00000066)),
        ("color.stage-edge".into(), StyleValue::Color(0xdfe5e2ff, 0x080d10ff)),
        ("color.success".into(), StyleValue::Color(0x246442ff, 0x83cea4ff)),
        ("color.text".into(), StyleValue::Color(0x171b1eff, 0xf4f6f7ff)),
        ("control.height".into(), StyleValue::Length(36.0)),
        ("control.hit-size".into(), StyleValue::Length(32.0)),
        ("control.icon-size".into(), StyleValue::Length(18.0)),
        ("control.radius".into(), StyleValue::Length(9.0)),
        ("dialog.radius".into(), StyleValue::Length(18.0)),
        ("dialog.width".into(), StyleValue::Length(456.0)),
        ("focus.offset".into(), StyleValue::Length(2.0)),
        ("focus.width".into(), StyleValue::Length(2.0)),
        ("font.body".into(), StyleValue::Length(15.0)),
        ("font.caption".into(), StyleValue::Length(11.0)),
        ("font.code".into(), StyleValue::Length(13.0)),
        ("font.line-height".into(), StyleValue::Length(22.0)),
        ("font.panel-title".into(), StyleValue::Length(26.0)),
        ("font.secondary".into(), StyleValue::Length(12.0)),
        ("font.setting-title".into(), StyleValue::Length(26.0)),
        ("font.weight-medium".into(), StyleValue::Integer(500)),
        ("font.weight-normal".into(), StyleValue::Integer(400)),
        ("font.weight-semibold".into(), StyleValue::Integer(600)),
        ("line.width".into(), StyleValue::Length(1.0)),
        ("row.action-color".into(), StyleValue::Color(0x707070ff, 0xbcbcbcff)),
        ("row.action-gap".into(), StyleValue::Length(2.0)),
        ("row.action-hover".into(), StyleValue::Color(0x0000000a, 0xffffff14)),
        ("row.action-icon-size".into(), StyleValue::Length(20.0)),
        ("row.action-radius".into(), StyleValue::Length(6.0)),
        ("row.action-size".into(), StyleValue::Length(28.0)),
        ("row.compact-height".into(), StyleValue::Length(56.0)),
        ("row.history-min-height".into(), StyleValue::Length(68.0)),
        ("row.icon-size".into(), StyleValue::Length(28.0)),
        ("row.image-height".into(), StyleValue::Length(116.0)),
        ("row.max-text-height".into(), StyleValue::Length(168.0)),
        ("row.radius".into(), StyleValue::Length(12.0)),
        ("row.saved-min-height".into(), StyleValue::Length(76.0)),
        ("row.selected-shadow-color".into(), StyleValue::Color(0x152b4033, 0x00000088)),
        ("row.selected-shadow-offset-y".into(), StyleValue::Length(4.0)),
        ("row.selected-shadow-softness".into(), StyleValue::Length(14.0)),
        ("row.shadow-color".into(), StyleValue::Color(0x152b4018, 0x00000055)),
        ("row.shadow-offset-y".into(), StyleValue::Length(2.0)),
        ("row.shadow-softness".into(), StyleValue::Length(8.0)),
        ("row.timeline-width".into(), StyleValue::Length(76.0)),
        ("search.height".into(), StyleValue::Length(44.0)),
        ("search.radius".into(), StyleValue::Length(12.0)),
        ("space.1".into(), StyleValue::Length(4.0)),
        ("space.10".into(), StyleValue::Length(40.0)),
        ("space.2".into(), StyleValue::Length(8.0)),
        ("space.3".into(), StyleValue::Length(12.0)),
        ("space.4".into(), StyleValue::Length(16.0)),
        ("space.5".into(), StyleValue::Length(20.0)),
        ("space.6".into(), StyleValue::Length(24.0)),
        ("space.8".into(), StyleValue::Length(32.0)),
    ]))
}
macro_rules! apply_style {
    ($global:expr, $style:expr, $dark:expr) => {{
        let g = $global;
        let s = $style;
        let dark = $dark;
        g.set_color_accent(s.color("color.accent", dark));
        g.set_color_accent_text(s.color("color.accent-text", dark));
        g.set_color_canvas(s.color("color.canvas", dark));
        g.set_color_danger(s.color("color.danger", dark));
        g.set_color_faint(s.color("color.faint", dark));
        g.set_color_focus(s.color("color.focus", dark));
        g.set_color_line(s.color("color.line", dark));
        g.set_color_muted(s.color("color.muted", dark));
        g.set_color_on_accent(s.color("color.on-accent", dark));
        g.set_color_panel(s.color("color.panel", dark));
        g.set_color_raised(s.color("color.raised", dark));
        g.set_color_row_text(s.color("color.row-text", dark));
        g.set_color_search_background(s.color("color.search-background", dark));
        g.set_color_search_text(s.color("color.search-text", dark));
        g.set_color_selection(s.color("color.selection", dark));
        g.set_color_shadow(s.color("color.shadow", dark));
        g.set_color_stage_edge(s.color("color.stage-edge", dark));
        g.set_color_success(s.color("color.success", dark));
        g.set_color_text(s.color("color.text", dark));
        g.set_control_height(s.length("control.height"));
        g.set_control_hit_size(s.length("control.hit-size"));
        g.set_control_icon_size(s.length("control.icon-size"));
        g.set_control_radius(s.length("control.radius"));
        g.set_dialog_radius(s.length("dialog.radius"));
        g.set_dialog_width(s.length("dialog.width"));
        g.set_focus_offset(s.length("focus.offset"));
        g.set_focus_width(s.length("focus.width"));
        g.set_font_body(s.length("font.body"));
        g.set_font_caption(s.length("font.caption"));
        g.set_font_code(s.length("font.code"));
        g.set_font_line_height(s.length("font.line-height"));
        g.set_font_panel_title(s.length("font.panel-title"));
        g.set_font_secondary(s.length("font.secondary"));
        g.set_font_setting_title(s.length("font.setting-title"));
        g.set_font_weight_medium(s.integer("font.weight-medium"));
        g.set_font_weight_normal(s.integer("font.weight-normal"));
        g.set_font_weight_semibold(s.integer("font.weight-semibold"));
        g.set_line_width(s.length("line.width"));
        g.set_row_action_color(s.color("row.action-color", dark));
        g.set_row_action_gap(s.length("row.action-gap"));
        g.set_row_action_hover(s.color("row.action-hover", dark));
        g.set_row_action_icon_size(s.length("row.action-icon-size"));
        g.set_row_action_radius(s.length("row.action-radius"));
        g.set_row_action_size(s.length("row.action-size"));
        g.set_row_compact_height(s.length("row.compact-height"));
        g.set_row_history_min_height(s.length("row.history-min-height"));
        g.set_row_icon_size(s.length("row.icon-size"));
        g.set_row_image_height(s.length("row.image-height"));
        g.set_row_max_text_height(s.length("row.max-text-height"));
        g.set_row_radius(s.length("row.radius"));
        g.set_row_saved_min_height(s.length("row.saved-min-height"));
        g.set_row_selected_shadow_color(s.color("row.selected-shadow-color", dark));
        g.set_row_selected_shadow_offset_y(s.length("row.selected-shadow-offset-y"));
        g.set_row_selected_shadow_softness(s.length("row.selected-shadow-softness"));
        g.set_row_shadow_color(s.color("row.shadow-color", dark));
        g.set_row_shadow_offset_y(s.length("row.shadow-offset-y"));
        g.set_row_shadow_softness(s.length("row.shadow-softness"));
        g.set_row_timeline_width(s.length("row.timeline-width"));
        g.set_search_height(s.length("search.height"));
        g.set_search_radius(s.length("search.radius"));
        g.set_space_1(s.length("space.1"));
        g.set_space_10(s.length("space.10"));
        g.set_space_2(s.length("space.2"));
        g.set_space_3(s.length("space.3"));
        g.set_space_4(s.length("space.4"));
        g.set_space_5(s.length("space.5"));
        g.set_space_6(s.length("space.6"));
        g.set_space_8(s.length("space.8"));
    }};
}
pub(crate) use apply_style;
