//! Space navigation and event-driven software card readiness.
use super::*;
fn space_icon(space: &echo_engine::Space) -> &str {
    match space.id {
        SpaceId::HISTORY => "History",
        SpaceId::FAVORITES => "Star",
        _ => space.icon_key.as_deref().unwrap_or_default(),
    }
}
pub(super) fn accent(key: &str) -> slint::Color {
    let (r, g, b) = match key {
        "blue" => (87, 146, 230),
        "green" => (72, 166, 118),
        "violet" => (160, 127, 220),
        "rose" => (205, 115, 150),
        "slate" => (138, 153, 166),
        "default" => (54, 120, 155),
        _ => (255, 196, 0),
    };
    slint::Color::from_rgb_u8(r, g, b)
}
impl App {
    pub(super) fn spaces_loaded(&mut self, result: Result<Vec<Space>, String>) {
        let spaces = match result {
            Ok(v) => v,
            Err(e) => {
                self.report(e, true);
                return;
            }
        };
        for space in &spaces {
            if self
                .spaces
                .iter()
                .find(|s| s.id == space.id)
                .is_none_or(|s| {
                    s.revision != space.revision
                        || s.title != space.title
                        || s.icon_key != space.icon_key
                        || s.accent_key != space.accent_key
                })
            {
                self.previews.remove(&space.id);
            }
        }
        self.spaces = spaces;
        self.deck.set_order(self.spaces.iter().map(|s| s.id));
        self.positions.retain(self.deck.order());
        self.render_navigation();
        if let Some(args) = self.pending_args.take() {
            self.activate_args(args);
            return;
        }
        if !self.spaces.iter().any(|s| s.id == self.surface.space) {
            self.navigate_to(SpaceId::FAVORITES);
        }
        if let Some(id) = self.navigate_after_refresh.take() {
            if self.window.get_route().as_str() != "settings" || !self.window.get_settings_dirty() {
                self.set_route("history");
                self.navigate_to(id);
            } else {
                self.report(
                    "Space created. Save or cancel settings to switch to it.",
                    false,
                );
            }
        }
        self.content_ready();
        self.schedule_prewarm();
    }
    pub(super) fn render_navigation(&self) {
        let _timing = crate::popup_timing::span("navigation_model_update");
        // Keep the live native tree frozen while only GPU textures are moving.
        if self.deck.phase == Phase::Animating {
            return;
        }
        let selected = self.deck.requested;
        let model = self
            .spaces
            .iter()
            .map(|s| crate::SpaceVm {
                key: s.id.to_string().into(),
                title: s.title.clone().into(),
                icon_key: space_icon(s).into(),
                accent: accent(if s.id.is_system() {
                    "default"
                } else {
                    &s.accent_key
                }),
                count: s.item_count.to_string().into(),
                system: s.id.is_system(),
                selected: s.id == selected,
            })
            .collect::<Vec<_>>();
        let side = |offset: i64| {
            let count = model.len() as i64;
            let index = self.deck.index(selected).unwrap_or(0) as i64 + offset;
            if count < 2 || (!self.ui.loop_spaces && (index < 0 || index >= count)) {
                return crate::SpaceVm::default();
            }
            model[index.rem_euclid(count) as usize].clone()
        };
        let left = side(-1);
        let right = side(1);
        let index = self.deck.index(selected).unwrap_or(0);
        let duplicate = left.key == right.key;
        let left = if duplicate && index == 0 {
            Default::default()
        } else {
            left.clone()
        };
        let right = if duplicate && index != 0 {
            Default::default()
        } else {
            right
        };
        let (left, right) = {
            let (left, right) = echo_presentation::slide::popup_neighbors(
                (!left.key.is_empty()).then_some(left),
                (!right.key.is_empty()).then_some(right),
                self.popup_side_right.filter(|_| self.quick_geometry_active),
            );
            (left.unwrap_or_default(), right.unwrap_or_default())
        };
        self.window.set_requested_left_space(left.clone());
        self.window.set_requested_right_space(right.clone());
        if !self.software.slide.loading() {
            self.window.set_left_space(left);
            self.window.set_right_space(right);
        }
        self.window.set_spaces(ModelRc::new(VecModel::from(model)));
        self.render_startup_choices();
        let index = self.deck.index(selected).unwrap_or(0);
        let count = self.deck.order().len();
        self.window
            .set_previous_enabled(count > 1 && (self.ui.loop_spaces || index > 0));
        self.window
            .set_next_enabled(count > 1 && (self.ui.loop_spaces || index + 1 < count));
        if let Some(space) = self.spaces.iter().find(|s| s.id == selected) {
            self.window.set_space_title(space.title.clone().into());
            self.window.set_space_icon(space_icon(space).into());
            self.window
                .set_space_accent(accent(if space.id.is_system() {
                    "default"
                } else {
                    &space.accent_key
                }));
            self.window.set_navigation_label(
                format!("{}  ·  {} / {}", space.title, index + 1, count).into(),
            );
            let subtitle = if self.surface.loading {
                "Loading this space…".into()
            } else if self.surface.query.is_empty() {
                format!(
                    "{} items{}",
                    space.item_count,
                    if space.id == SpaceId::HISTORY {
                        " · Clipboard timeline"
                    } else {
                        " · Manual order"
                    }
                )
            } else {
                format!(
                    "{} {}",
                    self.surface.total,
                    if self.surface.total == 1 {
                        "match"
                    } else {
                        "matches"
                    }
                )
            };
            if (self.surface.ready && !self.surface.loading) || self.model.row_count() == 0 {
                self.window.set_space_subtitle(subtitle.into());
            }
        }
        self.window.set_space_clearable(selected.is_system());
        self.window.set_active_view(
            if selected == SpaceId::HISTORY {
                "history"
            } else {
                "favorites"
            }
            .into(),
        );
    }
    pub(super) fn remember_position(&mut self) {
        if self.ui.remember_position && self.surface.ready && self.deck.phase == Phase::Idle {
            self.positions
                .remember(&self.surface, self.window.get_scroll_y());
        }
    }
    pub(super) fn navigate(&mut self, delta: i32) {
        let n = self.deck.order().len();
        if n < 2 || delta == 0 {
            return;
        }
        let intended = self.software.slide.intent().unwrap_or(self.deck.requested);
        let i = self.deck.index(intended).unwrap_or(0) as i64;
        let next = if self.ui.loop_spaces {
            (i + i64::from(delta)).rem_euclid(n as i64)
        } else {
            (i + i64::from(delta)).clamp(0, n as i64 - 1)
        };
        self.navigate_to(self.deck.order()[next as usize]);
    }
    pub(super) fn navigate_to(&mut self, id: SpaceId) {
        if self.inline_active() {
            self.worker.inline.invalidate_results();
        }
        if self.mutation.is_some()
            || self.session.busy()
            || self.window.get_modal()
            || self.window.get_route().as_str() != "history"
            || self.hook.as_ref().is_some_and(WindowHook::is_composing)
            || !self.surface.visible
            || self.deck.index(id).is_none()
        {
            return;
        }
        self.navigate_software_to(id);
    }
    pub(super) fn content_ready(&mut self) {
        self.software_content_ready();
    }
    pub(super) fn finish_motion(&mut self) {
        self.cancel_software_slide();

        if self.deck.phase == Phase::Animating {
            self.deck.snap();
            self.render();
        }
        self.content_ready();
        self.window
            .set_navigation_busy(self.software_navigation_busy());

        self.schedule_prewarm();
    }

    pub(super) fn cancel_prewarm(&mut self) {
        self.preview_epoch = self.preview_epoch.wrapping_add(1);
        self.pending_previews.clear();
        self.software.side_errors.clear();
    }
    pub(super) fn schedule_prewarm(&mut self) {
        self.prepare_software_neighbors();
    }
    /// Current neighbors are first-frame work, not speculative navigation prewarm.

    pub(super) fn viewport_changed(&mut self) {
        let dpi = self.window.window().scale_factor();
        let geometry = (
            self.window.get_stage_width().round() as u32,
            self.window.get_stage_height().round() as u32,
            dpi.to_bits(),
            self.window.get_panel_height().to_bits(),
            self.window.get_panel_top().to_bits(),
            self.window.get_panel_left().to_bits(),
        );
        if geometry != self.geometry {
            {
                self.cancel_software_slide();
            }
            self.geometry = geometry;
            // Layout changes invalidate the software transition geometry.

            self.deck.snap();
            self.render();
            self.content_ready();

            self.window.set_navigation_busy(false);
        }
        self.schedule_prewarm();
    }

    pub(super) fn has_current_preview(&self, id: SpaceId) -> bool {
        self.previews.get(&id).is_some_and(|preview| {
            preview.query == self.surface.query
                && self
                    .spaces
                    .iter()
                    .any(|s| s.id == id && s.revision == preview.revision)
        })
    }

    pub(super) fn preview_loaded(
        &mut self,
        id: SpaceId,
        epoch: u64,
        result: Result<crate::events::LoadedPage, String>,
    ) {
        self.software_preview_loaded(id, epoch, result);
    }

    pub(super) fn stage_scroll(&mut self, delta: f32) {
        if !delta.is_finite()
            || self.window.get_modal()
            || self.window.get_route().as_str() != "history"
        {
            return;
        }
        self.wheel_delta += delta;
        if self.wheel_delta.abs() >= 40.0 {
            let direction = if self.wheel_delta < 0.0 { 1 } else { -1 };
            self.wheel_delta = 0.0;
            self.navigate(direction);
        }
    }
}
