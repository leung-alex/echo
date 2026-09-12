//! The native hit-test shape follows cards, not an invisible oversized rectangle.
use super::*;
use echo_presentation::echo_tokens as t;
#[derive(Clone, Copy, PartialEq, Eq)]
struct FrameStamp {
    session: u64,
    query: u64,
    content: i64,
    ready: bool,
    modal: bool,
    scene: u64,
    layout: [u32; 7],
    sides: [bool; 3],
}
pub(super) struct PendingCardFrame {
    generation: u64,
    stamp: FrameStamp,
    shapes: Option<Vec<shell::CardShape>>,
}
impl PendingCardFrame {
    fn matches(&self, generation: u64, stamp: FrameStamp) -> bool {
        self.generation == generation && self.stamp == stamp
    }
}
impl App {
    fn card_frame_stamp(&self) -> FrameStamp {
        let scene = 0;
        FrameStamp {
            session: self.session.epoch,
            query: self.surface.query_epoch(),
            content: self.surface.revision,
            ready: self.surface.ready && !self.surface.loading,
            modal: self.window.get_modal(),
            scene,
            layout: [
                self.window.get_panel_left().to_bits(),
                self.window.get_panel_top().to_bits(),
                self.window.get_panel_width().to_bits(),
                self.window.get_panel_height().to_bits(),
                self.window.get_stage_width().to_bits(),
                self.window.get_stage_height().to_bits(),
                self.window.window().scale_factor().to_bits(),
            ],
            sides: [
                self.window.get_left_side_visible(),
                self.window.get_right_side_visible(),
                self.window.get_slide_moving(),
            ],
        }
    }
    pub(super) fn update_card_region(&mut self) {
        let Some(hwnd) = self.hwnd else {
            return;
        };
        if !self.surface.visible || self.quitting {
            return;
        }
        let dpi = self.window.window().scale_factor();
        let [sw, sh] = [
            self.window.get_stage_width(),
            self.window.get_stage_height(),
        ];
        if !dpi.is_finite() || dpi <= 0.0 || sw <= 0.0 || sh <= 0.0 {
            return;
        }
        let history = self.window.get_route().as_str() == "history";
        let full =
            history && self.deck.phase == Phase::Animating && (self.window.get_slide_moving());
        if let Some(hook) = &self.hook {
            let bounds = if full {
                Some([0, 0, 0, 0])
            } else if history {
                Some([
                    (self.window.get_panel_left() * dpi).round() as i32,
                    (self.window.get_panel_top() * dpi).round() as i32,
                    ((self.window.get_panel_left() + self.window.get_panel_width()) * dpi).round()
                        as i32,
                    ((self.window.get_panel_top() + self.window.get_panel_height()) * dpi).round()
                        as i32,
                ])
            } else {
                Some([
                    (self.window.get_settings_card_left() * dpi).round() as i32,
                    (24.0 * dpi).round() as i32,
                    ((self.window.get_settings_card_left() + self.window.get_settings_card_width())
                        * dpi)
                        .round() as i32,
                    ((sh - 24.0) * dpi).round() as i32,
                ])
            };
            hook.set_resize_bounds(bounds);
            hook.set_caption_bounds(
                (self.window.get_route().as_str() == "settings" && !self.window.get_modal()).then(
                    || {
                        [
                            (self.window.get_settings_card_left() * dpi).round() as i32,
                            (24.0 * dpi).round() as i32,
                            ((self.window.get_settings_card_left()
                                + self.window.get_settings_card_width())
                                * dpi)
                                .round() as i32,
                            ((24.0 + 58.0) * dpi).round() as i32,
                        ]
                    },
                ),
            );
        }
        let shapes = if self.window.get_modal() || (full) {
            None
        } else {
            let [l, top, w, h] = if history {
                [
                    self.window.get_panel_left(),
                    self.window.get_panel_top(),
                    self.window.get_panel_width(),
                    self.window.get_panel_height(),
                ]
            } else {
                [
                    self.window.get_settings_card_left(),
                    24.0,
                    self.window.get_settings_card_width(),
                    sh - 48.0,
                ]
            };
            // Alpha is composited by DX12. Software needs an exact native binary card clip.
            let margin = { 16.0 };
            let mut shapes = if full {
                vec![]
            } else {
                vec![shell::CardShape::Rounded {
                    bounds: [
                        ((l - margin) * dpi).floor() as i32,
                        ((top - margin) * dpi).floor() as i32,
                        ((l + w + margin) * dpi).ceil() as i32,
                        ((top + h + margin) * dpi).ceil() as i32,
                    ],
                    radius: ((t::PANEL_RADIUS + margin) * dpi).round() as i32,
                }]
            };

            if history {
                let side = self.window.get_side_width();
                for (visible, left) in [
                    (self.window.get_left_side_visible(), l - side - 12.0),
                    (self.window.get_right_side_visible(), l + w + 12.0),
                ] {
                    if visible {
                        shapes.push(shell::CardShape::Rounded {
                            bounds: [
                                ((left - 16.0) * dpi).round() as i32,
                                (top * dpi).round() as i32,
                                ((left + side + 16.0) * dpi).round() as i32,
                                ((top + h + 4.0) * dpi).round() as i32,
                            ],
                            radius: ((t::PANEL_RADIUS + 16.0) * dpi).round() as i32,
                        });
                    }
                }
            }
            Some(shapes)
        };
        let stamp = self.card_frame_stamp();
        let same_frame = self
            .pending_card_region
            .as_ref()
            .is_some_and(|p| p.stamp == stamp);
        if self.window_shapes.as_ref() == Some(&shapes)
            && (!(self.popup_first_frame_pending || self.pending_card_region.is_some())
                || same_frame)
        {
            return;
        }
        // Cache before entering Win32: SetWindowRgn posts geometry notifications.
        let defer = self.popup_first_frame_pending
            || (self.inline_active() && self.window_shapes.is_some());
        self.window_shapes = Some(shapes.clone());
        let result = if defer {
            self.card_region_serial = self.card_region_serial.wrapping_add(1).max(1);
            let generation = self.card_region_serial;
            self.card_region_generation.set(generation);
            self.pending_card_region = Some(PendingCardFrame {
                generation,
                stamp,
                shapes: shapes.clone(),
            });
            // Expand before painting so neither complete frame can be clipped.
            // Shrink only after the matching presentation reaches DWM.
            let result = shell::expand_card_region(hwnd, shapes.as_deref());
            {
                crate::graphics::invalidate_software_frame();
            }
            self.window.window().request_redraw();
            result
        } else {
            self.pending_card_region = None;
            self.card_region_generation.set(0);
            shell::set_card_region(hwnd, shapes.as_deref())
        };
        if let Err(error) = result {
            self.report(
                format!("Could not update the card window shape: {error}"),
                true,
            );
        }
    }
    pub(super) fn commit_card_region(&mut self, generation: u64) {
        let Some(pending) = &self.pending_card_region else {
            return;
        };
        if generation != pending.generation || !self.surface.visible {
            return;
        }
        if self.popup_first_frame_pending
            && self.window.get_route().as_str() == "history"
            && (!self.surface.ready
                || self.surface.loading
                || self.surface.dirty
                || !self.images.pending.is_empty()
                || !self.software_neighbors_ready()
                || !self.deck.can_insert(self.surface.space))
        {
            return;
        }
        if !pending.matches(generation, self.card_frame_stamp()) {
            self.update_card_region();
            return;
        }
        let shapes = &pending.shapes;
        let Some(hwnd) = self.hwnd else {
            return;
        };
        // A later result may already have requested a different region. The
        // generation comparison above prevents an older frame shrinking it.
        crate::popup_timing::mark("frame_commit_received");
        let result = shell::finish_card_frame(hwnd)
            .and_then(|()| shell::set_card_region(hwnd, shapes.as_deref()))
            .and_then(|()| {
                if self.popup_first_frame_pending {
                    crate::popup_timing::mark("dwm_frame_finished");
                    let result = shell::cloak_card_frame(hwnd, false);
                    crate::popup_timing::mark("uncloaked");
                    result
                } else {
                    Ok(())
                }
            });
        match result {
            Ok(()) => {
                self.pending_card_region = None;
                self.card_region_generation.set(0);
                self.popup_first_frame_pending = false;
            }
            Err(error) => self.report(format!("Could not finish the card frame: {error}"), true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_the_current_session_content_and_scene_can_uncloak() {
        let stamp = FrameStamp {
            session: 3,
            query: 5,
            content: 7,
            ready: true,
            modal: false,
            scene: 11,
            layout: [0; 7],
            sides: [false; 3],
        };
        let pending = PendingCardFrame {
            generation: 13,
            stamp,
            shapes: None,
        };
        assert!(pending.matches(13, stamp));
        assert!(!pending.matches(12, stamp));
        for next in [
            FrameStamp {
                session: 4,
                ..stamp
            },
            FrameStamp { query: 6, ..stamp },
            FrameStamp {
                content: 8,
                ..stamp
            },
            FrameStamp {
                ready: false,
                ..stamp
            },
            FrameStamp { scene: 12, ..stamp },
            FrameStamp {
                layout: [1; 7],
                ..stamp
            },
            FrameStamp {
                sides: [true; 3],
                ..stamp
            },
            FrameStamp {
                modal: true,
                ..stamp
            },
        ] {
            assert!(!pending.matches(13, next));
        }
    }
}
