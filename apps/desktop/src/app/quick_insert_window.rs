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
            let scale = anchor.geometry.dpi as f32 / 96.0;
            // A compact stage retains the same front card, native text and side-card
            // renderer, while avoiding the manager's 1600-dip transparent canvas.
            let width =
                900.0_f32.min((anchor.geometry.work_area.width as f32 / scale - 24.0).max(120.0));
            let card = (width - 32.0)
                .min(t::PANEL_MAX_WIDTH.min(t::PANEL_MIN_WIDTH.max(width * t::PANEL_WIDTH_RATIO)))
                .max(120.0);
            self.window.set_popup_card_width(card);
            let placement = if self.inline_active() {
                let height = if self.surface.ready && !self.surface.loading {
                    self.window.get_inline_content_height().clamp(180.0, 520.0)
                } else if let Some(previous) = self.popup_placement {
                    (previous.card.height as f32 / scale).clamp(180.0, 520.0)
                } else {
                    300.0
                };
                let placement = echo_windows::focus::place_inline(
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
            if self.popup_placement != Some(placement) {
                let bounds = placement.window;
                self.window
                    .window()
                    .set_position(slint::PhysicalPosition::new(bounds.x, bounds.y));
                self.window.window().set_size(slint::PhysicalSize::new(
                    bounds.width as u32,
                    bounds.height as u32,
                ));
                self.popup_placement = Some(placement);
            }
            false
        } else if self.quick_geometry_active {
            self.window.set_popup_card_width(0.0);
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
