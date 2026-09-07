//! Deterministic positioning of the visible front card, not the transparent stage.
use super::PopupAnchor;
use echo_engine::PhysicalRect;
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PopupPlacement {
    pub window: PhysicalRect,
    pub card: PhysicalRect,
    pub above: bool,
}
/// Layout metrics are supplied by the presentation owner, in logical pixels.
/// The result is physical screen pixels and includes the transparent card offset.
pub fn place_card(
    anchor: PopupAnchor,
    stage_width: f32,
    card_width: f32,
    card_height: f32,
    padding_y: f32,
) -> PopupPlacement {
    let work = anchor.geometry.work_area;
    let target = anchor.geometry.target;
    let scale = anchor.geometry.dpi.clamp(48, 768) as f64 / 96.0;
    let px = |dip: f32| {
        ((if dip.is_finite() { dip.max(0.0) } else { 0.0 }) as f64 * scale).round() as i32
    };
    let margin = px(12.0)
        .min(work.width.max(1) / 8)
        .min(work.height.max(1) / 8);
    let gap = px(8.0).max(1);
    let padding = px(padding_y).min(work.height.max(1) / 8);
    let maximum_width = px(stage_width).clamp(1, (work.width - 2 * margin).max(1));
    let card_w = px(card_width).clamp(1, maximum_width);
    let left = work.x.saturating_add(margin);
    let right = work.x.saturating_add(work.width).saturating_sub(margin);
    let minimum_offset = px(16.0).min((maximum_width - card_w).max(0) / 2);
    // First constrain the visible card. Only then allocate symmetrical side-stage
    // space that actually fits; never push the card away to preserve invisible padding.
    let card_x = target.x.clamp(
        left.saturating_add(minimum_offset),
        right
            .saturating_sub(card_w)
            .saturating_sub(minimum_offset)
            .max(left.saturating_add(minimum_offset)),
    );
    let offset_x = ((maximum_width - card_w) / 2)
        .min(card_x.saturating_sub(left))
        .min(right.saturating_sub(card_x).saturating_sub(card_w))
        .max(0);
    let width = card_w + 2 * offset_x;
    let top = work.y.saturating_add(margin);
    let bottom = work.y.saturating_add(work.height).saturating_sub(margin);
    let below_y = target
        .y
        .saturating_add(target.height.max(1))
        .saturating_add(gap);
    let above_bottom = target.y.saturating_sub(gap);
    let below_space = bottom
        .saturating_sub(padding)
        .saturating_sub(below_y)
        .max(0);
    let above_space = above_bottom
        .saturating_sub(top)
        .saturating_sub(padding)
        .max(0);
    let wanted = px(card_height).max(1);
    let above = below_space < wanted && above_space > below_space;
    let available = if above { above_space } else { below_space };
    let maximum = (work.height - 2 * margin - 2 * padding).max(1);
    // On a crowded display preserve a useful scroll area; clamp only as a last resort.
    let height = wanted.min(available.max(px(252.0)).min(maximum)).max(1);
    let window_h = height + 2 * padding;
    let x = card_x - offset_x;
    let y = (if above {
        above_bottom.saturating_sub(height).saturating_sub(padding)
    } else {
        below_y.saturating_sub(padding)
    })
    .clamp(top, bottom.saturating_sub(window_h).max(top));
    PopupPlacement {
        window: PhysicalRect {
            x,
            y,
            width,
            height: window_h,
        },
        card: PhysicalRect {
            x: x + offset_x,
            y: y + padding,
            width: card_w,
            height,
        },
        above,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::focus::{AnchorSource, PopupAnchor};
    use echo_engine::InputTargetGeometry;
    fn place(dpi: u32, work: PhysicalRect, target: PhysicalRect) -> PopupPlacement {
        place_card(
            PopupAnchor {
                geometry: InputTargetGeometry {
                    target,
                    work_area: work,
                    dpi,
                },
                source: AnchorSource::NativeCaret,
            },
            900.0,
            520.0,
            560.0,
            24.0,
        )
    }
    #[test]
    fn anchors_front_card_not_outer_window() {
        let p = place(
            96,
            PhysicalRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1040,
            },
            PhysicalRect {
                x: 700,
                y: 100,
                width: 2,
                height: 20,
            },
        );
        assert_eq!((p.card.x, p.card.y), (700, 128));
        assert_eq!((p.window.x, p.window.y), (510, 104));
        assert!(!p.above);
    }
    #[test]
    fn flips_up_at_lower_edge() {
        let p = place(
            96,
            PhysicalRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1040,
            },
            PhysicalRect {
                x: 1600,
                y: 980,
                width: 1,
                height: 20,
            },
        );
        assert!(p.above);
        assert_eq!(p.card.y + p.card.height, 972);
        assert!(p.window.x + p.window.width <= 1908);
    }
    #[test]
    fn negative_monitor_and_mixed_dpi_never_double_scale() {
        for dpi in [96, 144, 192] {
            let p = place(
                dpi,
                PhysicalRect {
                    x: -2560,
                    y: -200,
                    width: 2560,
                    height: 1440,
                },
                PhysicalRect {
                    x: -1600,
                    y: 0,
                    width: 2,
                    height: 30,
                },
            );
            assert!(p.window.x >= -2560 && p.window.x + p.window.width <= 0);
            assert!(p.window.y >= -200 && p.window.y + p.window.height <= 1240);
            assert_eq!(p.card.y, 30 + (8 * dpi / 96) as i32);
        }
    }
    #[test]
    fn screen_edges_shrink_transparent_stage_not_visible_card_proximity() {
        for dpi in [96, 120, 144, 192] {
            for left in [-2560, 0] {
                let work = PhysicalRect {
                    x: left,
                    y: -120,
                    width: 2560,
                    height: 1440,
                };
                for x in [left, left + 10, left + 700, left + 2400, left + 2559] {
                    let p = place(
                        dpi,
                        work,
                        PhysicalRect {
                            x,
                            y: 50,
                            width: 2,
                            height: 24,
                        },
                    );
                    let distance = (p.card.x - x).max(x - (p.card.x + p.card.width)).max(0);
                    assert!(distance <= (28 * dpi / 96) as i32 + 2, "{dpi} {x}: {p:?}");
                    assert!(p.window.x >= left && p.window.x + p.window.width <= left + 2560);
                    assert!(
                        p.card.x >= p.window.x
                            && p.card.x + p.card.width <= p.window.x + p.window.width
                    );
                }
            }
        }
    }
    #[test]
    fn tiny_work_area_remains_bounded() {
        let p = place(
            192,
            PhysicalRect {
                x: 0,
                y: 0,
                width: 320,
                height: 240,
            },
            PhysicalRect {
                x: 310,
                y: 210,
                width: 1,
                height: 20,
            },
        );
        assert!(p.window.x >= 0 && p.window.x + p.window.width <= 320);
        assert!(p.window.y >= 0 && p.window.y + p.window.height <= 240);
    }
}

/// Inline suggestions keep the edge beside the caret fixed as content shrinks.
/// `previous_above` prevents the popup from changing sides with each query.
pub fn place_inline(
    anchor: PopupAnchor,
    stage_width: f32,
    card_width: f32,
    wanted_height: f32,
    padding_y: f32,
    previous_above: Option<bool>,
) -> PopupPlacement {
    let mut p = place_card(anchor, stage_width, card_width, wanted_height, padding_y);
    let scale = anchor.geometry.dpi.clamp(48, 768) as f32 / 96.0;
    let px = |v: f32| (v.max(0.0) * scale).round() as i32;
    let work = anchor.geometry.work_area;
    let caret = anchor.geometry.target;
    let padding = px(padding_y).min(work.height.max(1) / 8);
    let margin = px(12.0).min(work.height.max(1) / 8);
    let gap = px(8.0).max(1);
    let top = work.y.saturating_add(margin);
    let bottom = work.y.saturating_add(work.height).saturating_sub(margin);
    let above_edge = caret.y.saturating_sub(gap);
    let below_edge = caret.y.saturating_add(caret.height).saturating_add(gap);
    let above_room = (above_edge - top - padding).max(0);
    let below_room = (bottom - below_edge - padding).max(0);
    // Prefer space before sizing; a small loading panel must not lock later
    // results to a cramped side. Preserve direction only while content fits or
    // the alternative is not meaningfully larger (avoids edge jitter).
    let wanted = px(wanted_height).max(1);
    let hysteresis = px(32.0);
    let above = match previous_above {
        Some(true) if above_room >= wanted || below_room <= above_room + hysteresis => true,
        Some(false) if below_room >= wanted || above_room <= below_room + hysteresis => false,
        _ => above_room >= below_room,
    };
    let available = if above { above_room } else { below_room };
    let height = px(wanted_height).max(1).min(available.max(1));
    p.above = above;
    p.card.height = height;
    p.window.height = height + 2 * padding;
    p.card.y = if above {
        above_edge - height
    } else {
        below_edge
    };
    p.window.y = (p.card.y - padding).clamp(top, (bottom - p.window.height).max(top));
    p.card.y = p.window.y + padding;
    p
}
#[cfg(test)]
mod inline_tests {
    use super::*;
    use crate::focus::AnchorSource;
    use echo_engine::InputTargetGeometry;
    fn anchor(y: i32) -> PopupAnchor {
        PopupAnchor {
            source: AnchorSource::NativeCaret,
            geometry: InputTargetGeometry {
                dpi: 96,
                target: PhysicalRect {
                    x: 800,
                    y,
                    width: 1,
                    height: 20,
                },
                work_area: PhysicalRect {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1040,
                },
            },
        }
    }
    #[test]
    fn above_popup_shrinks_from_top_not_toward_composer() {
        let a = anchor(750);
        let large = place_inline(a, 900.0, 520.0, 520.0, 24.0, None);
        let small = place_inline(a, 900.0, 520.0, 190.0, 24.0, Some(large.above));
        assert!(large.above && small.above);
        assert_eq!(
            large.card.y + large.card.height,
            small.card.y + small.card.height
        );
        assert!(small.window.height < large.window.height);
        assert_eq!(small.card.x, large.card.x);
    }
    #[test]
    fn top_edge_opens_below_and_preserves_nearest_edge() {
        let a = anchor(24);
        let large = place_inline(a, 900.0, 520.0, 520.0, 24.0, None);
        let small = place_inline(a, 900.0, 520.0, 180.0, 24.0, Some(large.above));
        assert!(!large.above && !small.above);
        assert_eq!(large.card.y, small.card.y);
        assert!(small.card.y >= 44);
        assert!(small.window.height < large.window.height);
    }
}

#[cfg(test)]
mod placement_regressions {
    use super::*;
    use crate::focus::AnchorSource;
    use echo_engine::InputTargetGeometry;
    fn a(y: i32, dpi: u32) -> PopupAnchor {
        PopupAnchor {
            source: AnchorSource::NativeCaret,
            geometry: InputTargetGeometry {
                target: PhysicalRect {
                    x: 320,
                    y,
                    width: 2,
                    height: 24,
                },
                dpi,
                work_area: PhysicalRect {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080,
                },
            },
        }
    }
    #[test]
    fn top_composer_uses_larger_side_not_any_side_with_160_pixels() {
        let p = place_inline(a(280, 96), 900., 520., 520., 24., None);
        assert!(!p.above);
        assert_eq!(p.card.height, 520);
        assert_eq!(p.card.y, 312);
    }
    #[test]
    fn loading_side_does_not_lock_results_into_a_sliver() {
        let p = place_inline(a(280, 96), 900., 520., 520., 24., Some(true));
        assert!(!p.above);
        assert_eq!(p.card.height, 520);
    }
    #[test]
    fn lower_composer_stays_adjacent_at_multiple_dpi() {
        for dpi in [96, 120, 144, 192] {
            let p = place_inline(a(990, dpi), 900., 520., 520., 24., None);
            assert!(p.above);
            assert_eq!(p.card.y + p.card.height, 990 - (8 * dpi / 96) as i32);
            assert!(p.card.height >= (400 * dpi / 96) as i32);
        }
    }
}
