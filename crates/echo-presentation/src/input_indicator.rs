//! Placement and validity policy for a passive input-mode badge.
use echo_engine::{InputAnchor, InputMode, InputStatus, PhysicalRect};
use std::time::{Duration, Instant};

pub fn visible(sample: &InputStatus, generation: u64, suppressed: bool, now: Instant) -> bool {
    !suppressed
        && sample.generation == generation
        && now.saturating_duration_since(sample.sampled_at) <= Duration::from_millis(250)
        && sample.mode != InputMode::Unknown
}

pub fn place(sample: &InputStatus) -> Option<PhysicalRect> {
    let g = sample.geometry;
    let scale = |dip: i32| ((f64::from(dip) * f64::from(g.dpi) / 96.).round() as i32).max(1);
    let (width, height, gap) = (scale(48), scale(36), scale(8));
    let work = g.work_area;
    if work.width < width || work.height < height || g.target.height <= 0 {
        return None;
    }
    let right = work.x.saturating_add(work.width);
    let bottom = work.y.saturating_add(work.height);
    if g.target.x >= right
        || g.target.y >= bottom
        || g.target.x.saturating_add(g.target.width) < work.x
        || g.target.y.saturating_add(g.target.height) <= work.y
    {
        return None;
    }
    let x = match sample.anchor {
        InputAnchor::Caret | InputAnchor::Pointer => {
            let x = g
                .target
                .x
                .saturating_add(g.target.width)
                .saturating_add(gap);
            if x.saturating_add(width) > right {
                g.target.x - gap - width
            } else {
                x
            }
        }
        InputAnchor::Control => g.target.x.saturating_add(g.target.width) - width,
    };
    let above = g.target.y - gap - height;
    let y = if above >= work.y {
        above
    } else {
        g.target
            .y
            .saturating_add(g.target.height)
            .saturating_add(gap)
    };
    Some(PhysicalRect {
        x: x.clamp(work.x, right - width),
        y: y.clamp(work.y, bottom - height),
        width,
        height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use echo_engine::CompositionState;
    fn sample() -> InputStatus {
        InputStatus {
            generation: 2,
            window: 1,
            focused_window: 2,
            process: 1,
            process_started: 1,
            mode: InputMode::Chinese,
            composition: CompositionState::Idle,
            anchor: InputAnchor::Caret,
            geometry: echo_engine::InputTargetGeometry {
                target: PhysicalRect {
                    x: 100,
                    y: 100,
                    width: 1,
                    height: 20,
                },
                work_area: PhysicalRect {
                    x: 0,
                    y: 0,
                    width: 800,
                    height: 600,
                },
                dpi: 96,
            },
            sampled_at: Instant::now(),
        }
    }
    #[test]
    fn rejects_stale_unknown_mode_and_suppressed_samples() {
        let mut s = sample();
        assert!(visible(&s, 2, false, s.sampled_at));
        assert!(!visible(&s, 3, false, s.sampled_at));
        assert!(!visible(&s, 2, true, s.sampled_at));
        assert!(!visible(
            &s,
            2,
            false,
            s.sampled_at + Duration::from_millis(251)
        ));
        s.mode = InputMode::Unknown;
        assert!(!visible(&s, 2, false, s.sampled_at));
    }
    #[test]
    fn known_mode_remains_visible_through_composition_and_candidate_selection() {
        let mut s = sample();
        for mode in [InputMode::Chinese, InputMode::English] {
            s.mode = mode;
            for composition in [
                CompositionState::Idle,
                CompositionState::Composing,
                CompositionState::Unknown,
            ] {
                s.composition = composition;
                assert!(visible(&s, 2, false, s.sampled_at));
                assert!(!visible(&s, 3, false, s.sampled_at));
                assert!(!visible(&s, 2, true, s.sampled_at));
            }
        }
    }
    #[test]
    fn pointer_status_follows_pointer_and_flips_at_screen_edges() {
        let mut s = sample();
        s.anchor = InputAnchor::Pointer;
        s.geometry.target.width = 1;
        s.geometry.target.height = 1;
        assert_eq!(
            place(&s).unwrap(),
            PhysicalRect {
                x: 109,
                y: 56,
                width: 48,
                height: 36
            }
        );
        s.geometry.target.x = 200;
        assert_eq!(place(&s).unwrap().x, 209);
        s.geometry.target.x = 790;
        s.geometry.target.y = 0;
        assert_eq!(
            place(&s).unwrap(),
            PhysicalRect {
                x: 734,
                y: 9,
                width: 48,
                height: 36
            }
        );
    }
    #[test]
    fn placement_flips_clamps_and_scales_on_negative_monitors() {
        let mut s = sample();
        assert_eq!(
            place(&s),
            Some(PhysicalRect {
                x: 109,
                y: 56,
                width: 48,
                height: 36
            })
        );
        s.geometry.target.x = 790;
        s.geometry.target.y = 0;
        assert_eq!(place(&s).unwrap().x, 734);
        assert_eq!(place(&s).unwrap().y, 28);
        s.geometry.work_area.x = -800;
        s.geometry.target.x = -790;
        s.geometry.dpi = 144;
        let p = place(&s).unwrap();
        assert_eq!((p.width, p.height), (72, 54));
        assert!(p.x >= -800 && p.x + p.width <= 0);
        s.anchor = InputAnchor::Control;
        s.geometry.target.width = 200;
        assert_eq!(place(&s).unwrap().x, -662);
        s.geometry.target.y = 700;
        assert!(place(&s).is_none());
    }
}
