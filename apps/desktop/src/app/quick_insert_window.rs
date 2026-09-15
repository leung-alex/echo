//! Quick Insert activation and owned-window placement. Win32 stays in echo-windows.
use super::*;
use echo_presentation::echo_tokens as t;
use echo_windows::focus::{place_card, FocusSnapshot};
impl App {
    pub(super) fn external_focus_lost(&mut self) {
        if self.inline_active() || self.inline_ui.pending {
            return;
        }
        if self.session.context != Context::QuickInsert
            || !self.surface.visible
            || self.capture_pending
            || self.session.busy()
            || !self
                .hook
                .as_ref()
                .is_some_and(WindowHook::foreground_is_external)
        {
            return;
        }
        // A dirty editor stays visible, but its old paste session is invalidated.
        if self.window.get_modal()
            || self.unsaved()
            || self.mutation.is_some()
            || self.window.get_route().as_str() != "history"
        {
            self.activation_focus = None;
            self.session.dismiss();
            self.worker
                .epoch
                .store(self.session.epoch, Ordering::Release);
            self.window.set_paste_target_available(false);
            self.window.set_quick_insert(false);
            self.send(Work::Cancel);
        } else {
            // The user's new external focus must never be stolen back.
            self.activation_focus = None;
            self.dismiss();
        }
    }
    pub(super) fn hotkey_activate(&mut self, snapshot: FocusSnapshot) {
        crate::popup_timing::begin();
        if self.inline_active()
            && self.activation_focus.as_ref().is_some_and(|old| {
                old.window_id == snapshot.window_id && old.process_id == snapshot.process_id
            })
        {
            self.dismiss();
            return;
        }
        if snapshot.is_echo() {
            if self.surface.visible {
                self.request_hide();
            }
            return;
        }
        if self.capture_pending
            && self
                .activation_focus
                .as_ref()
                .is_some_and(|old| old.window_id == snapshot.window_id)
        {
            self.dismiss();
            return;
        }
        if self.session.busy() || self.mutation.is_some() {
            return;
        }
        self.activate_from(vec!["--quick-insert".into()], snapshot);
    }
    /// Returns whether the manager needs its initial centering. For a popup, position
    /// and size are both assigned before show(), including the visible card offset.
    pub(super) fn prepare_window_geometry(&mut self) -> bool {
        let settings = matches!(self.window.get_route().as_str(), "settings" | "about");
        if settings {
            if self.settings_geometry.active() {
                return false;
            }
            let monitor_position = self.window.window().position();
            // Retire the caret-sized host before saving the ordinary History geometry.
            self.prepare_content_geometry();
            let current = self.hwnd.map(|_| self.current_geometry());
            if let Some(geometry) = self.settings_geometry.enter(current) {
                self.apply_geometry(geometry);
                return false;
            }
            // Center on the monitor we entered from, even when leaving a caret popup.
            self.window.window().set_position(monitor_position);
            return true;
        }
        let mut center = false;
        if self.settings_geometry.active() {
            let current = self.current_geometry();
            if let Some(geometry) = self.settings_geometry.leave(current) {
                self.apply_geometry(geometry);
            } else {
                self.window
                    .window()
                    .set_size(slint::LogicalSize::new(t::WINDOW_WIDTH, t::WINDOW_HEIGHT));
                center = true;
            }
        }
        let content_center = self.prepare_content_geometry();
        !self.quick_geometry_active && (center || content_center)
    }
    fn prepare_content_geometry(&mut self) -> bool {
        let _timing = crate::popup_timing::span("geometry_update");
        let anchor = self.popup_anchor.filter(|_| {
            self.session.context == Context::QuickInsert
                && (self.ui.caret_anchor || self.inline_active())
                && self.window.get_route().as_str() == "history"
        });
        if let Some(anchor) = anchor {
            if !self.quick_geometry_active {
                self.manager_geometry = self
                    .hwnd
                    .map(|_| (self.window.window().position(), self.window.window().size()));
                self.quick_geometry_active = true;
            }
            let scale = anchor.geometry.dpi.clamp(48, 768) as f32 / 96.0;
            // Size the readable front card independently from its transparent stage.
            let card = t::PANEL_MIN_WIDTH
                .min((anchor.geometry.work_area.width as f32 / scale - 56.0).max(120.0));
            let width = card + 32.0;
            if self.popup_placement.is_none() {
                self.popup_side_right = None;
            }
            self.window.set_popup_card_width(card);
            let placement = if self.inline_active() {
                // Candidate count must not change the panel size, including a fresh empty session.
                let placement = echo_windows::focus::place_inline_stage(
                    anchor,
                    width,
                    card,
                    520.0,
                    t::STAGE_PADDING_Y,
                    self.inline_ui.above,
                );
                self.inline_ui.above = Some(placement.above);
                placement
            } else {
                place_card(anchor, width, card, 560.0, t::STAGE_PADDING_Y)
            };
            let extents = {
                [
                    card / 2.0 + 16.0,
                    card / 2.0 + echo_presentation::slide::side_width(card) + 28.0,
                ]
            };
            let preferred_right = self.popup_side_right.unwrap_or(true);
            let (mut placement, right) = echo_windows::focus::expand_popup_stage(
                anchor,
                placement,
                extents,
                preferred_right,
            );
            // The editor needs a work-area-sized host, independently of the
            // session's anchored cards. Closing it recomputes the original host.
            if self.window.get_editor_open() {
                let work = anchor.geometry.work_area;
                let margin = (16.0 * scale).round() as i32;
                let available = (work.height - 2 * margin).max(1);
                let height = placement
                    .window
                    .height
                    .max((660.0 * scale).round() as i32)
                    .min(available);
                placement.window.y = (placement.window.y + (placement.window.height - height) / 2)
                    .clamp(work.y + margin, work.y + work.height - margin - height);
                placement.window.height = height;
            }
            let side_changed = self.popup_side_right != Some(right);
            self.popup_side_right = Some(right);
            if side_changed {
                self.render_navigation();
            }
            self.window
                .set_popup_card_left((placement.card.x - placement.window.x) as f32 / scale);
            let card_changed = self.popup_placement.map(|p| p.card) != Some(placement.card);
            if self.inline_active() {
                self.window
                    .set_popup_card_height(placement.card.height as f32 / scale);
                self.window
                    .set_popup_card_top((placement.card.y - placement.window.y) as f32 / scale);
            }
            if self.popup_placement != Some(placement) {
                let bounds = placement.window;
                if self.popup_placement.map(|p| p.window) != Some(bounds) {
                    self.window
                        .window()
                        .set_position(slint::PhysicalPosition::new(bounds.x, bounds.y));
                    self.window.window().set_size(slint::PhysicalSize::new(
                        bounds.width as u32,
                        bounds.height as u32,
                    ));
                }
                self.popup_placement = Some(placement);
                if crate::popup_timing::enabled() {
                    let side_quad = {
                        let side = echo_presentation::slide::side_width(card);
                        let l = placement.card.x as f32
                            + if right {
                                placement.card.width as f32 + 12.0 * scale
                            } else {
                                -(side + 12.0) * scale
                            };
                        let top = placement.card.y as f32 + 16.0 * scale;
                        let bottom =
                            (placement.card.y + placement.card.height) as f32 - 16.0 * scale;
                        Some([
                            [l, top],
                            [l + side * scale, top],
                            [l + side * scale, bottom],
                            [l, bottom],
                        ])
                    };
                    crate::popup_timing::event(
                        "layout_ready",
                        serde_json::json!({
                            "card":[placement.card.x,placement.card.y,placement.card.width,placement.card.height],
                            "side_quad":side_quad,"right":right,"dpi":anchor.geometry.dpi
                        }),
                    );
                }
            }
            if (card_changed || side_changed) && self.surface.visible {
                // Reflow side textures before returning to Slint's next paint.
                self.viewport_changed();
            }
            false
        } else if self.quick_geometry_active {
            self.window.set_popup_card_width(0.0);
            self.window.set_popup_card_height(0.0);
            self.window.set_popup_card_top(0.0);
            self.window.set_popup_card_left(0.0);
            self.popup_side_right = None;
            self.quick_geometry_active = false;
            self.popup_placement = None;
            if let Some((position, size)) = self.manager_geometry.take() {
                self.window.window().set_position(position);
                self.window.window().set_size(size);
                false
            } else {
                self.window
                    .window()
                    .set_size(slint::LogicalSize::new(t::WINDOW_WIDTH, t::WINDOW_HEIGHT));
                true
            }
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use echo_engine::{InputTargetGeometry, PhysicalRect};
    use echo_windows::focus::{expand_popup_stage, place_inline_stage, AnchorSource, PopupAnchor};

    #[test]
    fn popup_anchors_front_and_contains_the_whole_neighbor_at_screen_edges() {
        for dpi in [96, 120, 144, 192] {
            let scale = dpi as f32 / 96.0;
            let px = |v: f32| (v * scale).round() as i32;
            for left in [-3840, 0] {
                for x in [20.0, 700.0, 1880.0] {
                    let anchor = PopupAnchor {
                        source: AnchorSource::NativeCaret,
                        geometry: InputTargetGeometry {
                            dpi,
                            work_area: PhysicalRect {
                                x: left,
                                y: 0,
                                width: px(1920.0),
                                height: px(1080.0),
                            },
                            target: PhysicalRect {
                                x: left + px(x),
                                y: px(900.0),
                                width: 2,
                                height: px(24.0),
                            },
                        },
                    };
                    let mut previous = None;
                    let mut stage = None;
                    for height in [520.0, 180.0, 300.0, 520.0] {
                        let anchored =
                            place_inline_stage(anchor, 552.0, 520.0, height, 24.0, Some(true));
                        let (p, right) = expand_popup_stage(
                            anchor,
                            anchored,
                            [
                                520.0 / 2.0 + 16.0,
                                520.0 / 2.0 + echo_presentation::slide::side_width(520.0) + 28.0,
                            ],
                            previous.unwrap_or(true),
                        );
                        assert_eq!(
                            p.card, anchored.card,
                            "the canvas must not displace the front card"
                        );
                        if let Some(previous) = previous {
                            assert_eq!(right, previous);
                        }
                        if let Some(stage) = stage {
                            assert_eq!(p.window, stage);
                        }
                        previous = Some(right);
                        stage = Some(p.window);
                        assert!(
                            p.window.x >= left && p.window.x + p.window.width <= left + px(1920.0)
                        );
                        assert_eq!(right, x < 1880.0);
                        let side = echo_presentation::slide::side_width(520.0);
                        let side_left = p.card.x as f32
                            + if right {
                                p.card.width as f32 + 12.0 * scale
                            } else {
                                -(side + 12.0) * scale
                            };
                        assert!(side_left >= p.window.x as f32);
                        assert!(side_left + side * scale <= (p.window.x + p.window.width) as f32);
                    }
                }
            }
        }
    }
}
