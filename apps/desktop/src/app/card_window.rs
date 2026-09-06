//! The native hit-test shape follows cards, not an invisible oversized rectangle.
use super::*;
use echo_presentation::echo_tokens as t;
impl App {
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
        let full = history && self.deck.phase == Phase::Animating && self.window.get_flow_enabled();
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
        }
        let shapes = if full {
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
            let margin = if self.graphics.perspective {
                t::FLOW_SHADOW_MARGIN
            } else {
                0.0
            };
            let mut shapes = vec![shell::CardShape::Rounded {
                bounds: [
                    ((l - margin) * dpi).floor() as i32,
                    ((top - margin) * dpi).floor() as i32,
                    ((l + w + margin) * dpi).ceil() as i32,
                    ((top + h + margin) * dpi).ceil() as i32,
                ],
                radius: ((t::PANEL_RADIUS + margin) * dpi).round() as i32,
            }];
            if history && self.flow_allowed() && self.window.get_flow_enabled() {
                let padding =
                    margin.max(h * t::FLOW_REFLECTION_HEIGHT_RATIO + t::FLOW_REFLECTION_GAP);
                for pose in self
                    .deck
                    .poses(w)
                    .into_iter()
                    .filter(|p| p.space != self.deck.requested)
                {
                    if let Some(vertices) =
                        echo_presentation::deck::project_panel(pose, w, h, padding)
                    {
                        shapes.push(shell::CardShape::Polygon(
                            vertices
                                .into_iter()
                                .map(|p| {
                                    [
                                        ((p[0] + sw / 2.0) * dpi).round() as i32,
                                        ((p[1] + sh / 2.0) * dpi).round() as i32,
                                    ]
                                })
                                .collect(),
                        ));
                    }
                }
            }
            Some(shapes)
        };
        if self.window_shapes.as_ref() == Some(&shapes) {
            return;
        }
        // Cache before entering Win32: SetWindowRgn posts geometry notifications.
        self.window_shapes = Some(shapes.clone());
        if let Err(error) = shell::set_card_region(hwnd, shapes.as_deref()) {
            self.report(
                format!("Could not update the card window shape: {error}"),
                true,
            );
        }
    }
}
