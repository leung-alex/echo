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
                let height = if let Some(height) = self.inline_ui.base_height {
                    height
                } else if self.surface.ready
                    && !self.surface.loading
                    && self.surface.presented_query.as_deref() == Some(self.surface.query.as_str())
                    && self.deck.phase != Phase::Animating
                {
                    // Slint 1.17.1 materializes repeaters before its normal draw.
                    // Geometry is needed before that draw, so run the same bounded
                    // UI-thread pass here. No rendering or GPU readback is involved.
                    // Keep this pinned-runtime seam local to native composition.
                    let _timing = crate::popup_timing::span("instantiate_main_tree");
                    slint::private_unstable_api::re_exports::WindowInner::from_pub(
                        self.window.window(),
                    )
                    .ensure_tree_instantiated();
                    let height = self.window.get_inline_content_height().clamp(180.0, 520.0);
                    self.inline_ui.base_height = Some(height);
                    height
                } else if let Some(previous) = self.popup_placement {
                    (previous.card.height as f32 / scale).clamp(180.0, 520.0)
                } else {
                    300.0
                };
                let placement = echo_windows::focus::place_inline_stage(
                    anchor,
                    width,
                    card,
                    height,
                    t::STAGE_PADDING_Y,
                    self.inline_ui.above,
                );
                self.inline_ui.above = Some(placement.above);
                placement
            } else {
                place_card(anchor, width, card, 560.0, t::STAGE_PADDING_Y)
            };
            let extents = if self.graphics.perspective {
                echo_presentation::deck::popup_horizontal_extents(card, 560.0)
            } else if self.ui.view_mode == SpaceViewMode::CoverFlow {
                [
                    card / 2.0 + 16.0,
                    card / 2.0 + echo_presentation::slide::side_width(card) + 28.0,
                ]
            } else {
                [card / 2.0 + 16.0; 2]
            };
            let preferred_right = self.popup_side_right.unwrap_or_else(|| {
                if !self.graphics.perspective {
                    true
                } else {
                    self.deck
                        .poses(card)
                        .iter()
                        .find(|p| p.offset != 0.0)
                        .is_none_or(|p| p.offset > 0.0)
                }
            });
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
            if side_changed && !self.graphics.perspective {
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
                    let center = [
                        placement.card.x as f32 + placement.card.width as f32 / 2.0,
                        placement.card.y as f32 + placement.card.height as f32 / 2.0,
                    ];
                    let side_quad = if !self.graphics.perspective {
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
                        (self.ui.view_mode == SpaceViewMode::CoverFlow).then_some([
                            [l, top],
                            [l + side * scale, top],
                            [l + side * scale, bottom],
                            [l, bottom],
                        ])
                    } else {
                        self.flow_poses()
                            .into_iter()
                            .find(|p| p.offset != 0.0)
                            .and_then(|pose| {
                                echo_presentation::deck::project_panel(
                                    pose,
                                    card,
                                    placement.card.height as f32 / scale,
                                    0.0,
                                )
                            })
                            .map(|q| {
                                q.map(|p| [center[0] + p[0] * scale, center[1] + p[1] * scale])
                            })
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
    use echo_engine::{InputTargetGeometry, PhysicalRect, SpaceId};
    use echo_presentation::deck::{popup_horizontal_extents, project_panel, Deck};
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
                            popup_horizontal_extents(520.0, 560.0),
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
                        let mut deck = Deck::default();
                        deck.show(SpaceId::HISTORY, 0);
                        for pose in deck.popup_poses(520.0, right) {
                            for [x, _] in project_panel(pose, 520.0, height, 0.0).unwrap() {
                                let screen =
                                    p.card.x as f32 + p.card.width as f32 / 2.0 + x * scale;
                                assert!(
                                    screen >= p.window.x as f32
                                        && screen <= (p.window.x + p.window.width) as f32
                                );
                            }
                        }
                    }
                }
            }
        }
    }
}
