//! Space navigation, bounded panel snapshots, and event-driven motion.
use super::*;
use echo_presentation::echo_tokens as t;
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
    pub(super) fn flow_poses(&self) -> Vec<echo_presentation::deck::Pose> {
        let width = self.window.get_panel_width();
        if let Some(right) = self.popup_side_right.filter(|_| self.quick_geometry_active) {
            self.deck.popup_poses(width, right)
        } else {
            self.deck.poses(width)
        }
    }
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
                self.dirty_snapshots.insert(space.id);
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
                icon_key: s.icon_key.clone().unwrap_or_default().into(),
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
        self.window.set_spaces(ModelRc::new(VecModel::from(model)));
        let index = self.deck.index(selected).unwrap_or(0);
        let count = self.deck.order().len();
        self.window
            .set_previous_enabled(count > 1 && (self.ui.loop_spaces || index > 0));
        self.window
            .set_next_enabled(count > 1 && (self.ui.loop_spaces || index + 1 < count));
        if let Some(space) = self.spaces.iter().find(|s| s.id == selected) {
            self.window.set_space_title(space.title.clone().into());
            self.window
                .set_space_icon(space.icon_key.clone().unwrap_or_default().into());
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
        let i = self.deck.index(self.deck.requested).unwrap_or(0) as i64;
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
        if id == self.deck.requested && self.surface.space == id {
            return;
        }
        let navigation_started = Instant::now();
        self.window.set_front_shadow_opacity(1.0);
        #[cfg(feature = "cover-flow")]
        if let Some(flow) = &self.flow {
            flow.begin_transition();
        }
        self.remember_position();
        self.search_timer.stop();
        self.inspect_intent = None;
        if self.full_motion()
            && self.surface.ready
            && self.deck.phase != Phase::Animating
            && self.dirty_snapshots.contains(&self.surface.space)
        {
            let _ = self.capture_space(self.surface.space, false);
        }
        let motion = self.full_motion();
        if !self.deck.request(id, self.now(), motion) {
            self.deck.show(id, self.now());
        }
        self.cancel_prewarm();
        self.surface.hide();
        self.surface.set_space(id);
        self.surface.visible = true;
        let query = if !self.inline_active() {
            String::new()
        } else {
            self.window.get_query().to_string()
        };
        if query != self.surface.query {
            self.previews.clear();
        }
        self.surface.set_query(query.clone());
        self.window.set_query(query.into());
        self.pending_scroll = if self.ui.remember_position {
            Some(self.positions.restore(&mut self.surface))
        } else {
            Some(0.0)
        };
        if self.deck.phase != Phase::Animating {
            self.window.set_scroll_y(0.0);
            self.window.set_stale_rows(true);
        }
        self.window
            .set_navigation_busy(self.deck.phase == Phase::Animating);
        self.load(false);
        self.render_navigation();
        self.prepare_scene();
        if self.deck.phase == Phase::Animating {
            let hub = self.hub.clone();
            self.flow_timer
                .start(TimerMode::Repeated, self.frame_interval(), move || {
                    hub.post(Event::Command(Command::FlowTick))
                });
        } else {
            self.content_ready();
            self.schedule_prewarm();
        }
        if !self.inline_active() {
            self.window.invoke_focus_content();
        }
        self.update_card_region();
        if self.navigation_us.len() < 256 {
            self.navigation_us
                .push(navigation_started.elapsed().as_micros() as u64);
        }
    }
    pub(super) fn content_ready(&mut self) {
        if self.surface.ready && !self.surface.loading && self.surface.space == self.deck.requested
        {
            crate::popup_timing::mark("content_ready");
            self.deck.ready(self.surface.space);
        }
        self.window
            .set_navigation_busy(self.deck.phase == Phase::Animating);
    }
    pub(super) fn finish_motion(&mut self) {
        self.window.set_front_shadow_opacity(1.0);
        self.flow_timer.stop();
        if self.deck.phase == Phase::Animating {
            self.deck.snap();
            self.render();
        }
        self.content_ready();
        self.window.set_navigation_busy(false);
        self.prepare_scene();
        #[cfg(feature = "cover-flow")]
        if let Some(flow) = &self.flow {
            flow.end_transition();
        }
        self.schedule_prewarm();
    }
    pub(super) fn flow_tick(&mut self) {
        if self.window.get_route().as_str() == "settings" {
            self.preview_tick();
            return;
        }
        if !self.surface.visible || self.window.get_route().as_str() != "history" {
            self.window.set_front_shadow_opacity(1.0);
            self.flow_timer.stop();
            return;
        }
        let was_animated = self.deck.phase == Phase::Animating;
        self.deck.tick(
            self.now(),
            self.ui.motion_speed,
            self.window.get_panel_width(),
        );
        if was_animated && self.deck.phase != Phase::Animating {
            self.render();
        }
        self.content_ready();
        self.prepare_scene();
        if self.deck.phase != Phase::Animating {
            #[cfg(feature = "cover-flow")]
            if let Some(flow) = &self.flow {
                flow.end_transition();
            }
            self.flow_timer.stop();
            self.schedule_prewarm();
        }
    }
    pub(super) fn cancel_prewarm(&mut self) {
        self.prewarm_timer.stop();
        self.preview_epoch = self.preview_epoch.wrapping_add(1);
        self.pending_previews.clear();
    }
    pub(super) fn schedule_prewarm(&mut self) {
        self.prepare_visible_neighbors();
        if self.inline_active()
            && (!self.surface.ready || self.surface.loading || self.surface.dirty)
        {
            return;
        }
        if !self.surface.visible
            || !self.flow_allowed()
            || self.window.get_route().as_str() != "history"
            || self.window.get_modal()
            || self.deck.phase == Phase::Animating
        {
            return;
        }
        let hub = self.hub.clone();
        self.prewarm_timer.start(
            TimerMode::SingleShot,
            Duration::from_millis(t::MOTION_SNAPSHOT_IDLE as u64),
            move || hub.post(Event::Command(Command::Prewarm)),
        );
    }
    /// Current neighbors are first-frame work, not speculative navigation prewarm.
    fn prepare_visible_neighbors(&mut self) {
        if !self.inline_active()
            || !self.surface.visible
            || !self.flow_allowed()
            || self.window.get_route().as_str() != "history"
            || self.window.get_modal()
            || self.deck.phase == Phase::Animating
        {
            return;
        }
        let front = self.surface.space;
        for pose in self.flow_poses().into_iter().filter(|p| p.space != front) {
            if self.ui.side_content == SideContent::Visible
                && !self.has_current_preview(pose.space)
                && self.pending_previews.insert(pose.space)
            {
                crate::popup_timing::mark("visible_preview_requested");
                if !self.send(Work::Preview(
                    pose.space,
                    self.preview_epoch,
                    self.surface.query.clone(),
                )) {
                    self.pending_previews.remove(&pose.space);
                }
            }
        }
        self.prepare_scene();
    }
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
            self.geometry = geometry;
            // Validate raster inputs, but keep targets for projection-only changes.
            self.dirty_snapshots
                .extend(self.deck.order().iter().copied());
            self.deck.snap();
            self.render();
            self.content_ready();
            self.prepare_scene();
            self.window.set_navigation_busy(false);
            self.flow_timer.stop();
        } else if self.window.get_scroll_y().to_bits() != self.last_scroll_bits
            && self.deck.phase != Phase::Animating
        {
            self.dirty_snapshots.insert(self.surface.space);
        }
        self.last_scroll_bits = self.window.get_scroll_y().to_bits();
        self.schedule_prewarm();
    }
    pub(super) fn invalidate_flow_scene(&mut self) {
        self.window.set_stage_image(Default::default());
        self.window.set_flow_enabled(false);
        #[cfg(feature = "cover-flow")]
        if let Some(flow) = &self.flow {
            flow.invalidate_scene();
        }
        self.dirty_snapshots
            .extend(self.deck.order().iter().copied());
    }
    fn has_current_preview(&self, id: SpaceId) -> bool {
        self.previews.get(&id).is_some_and(|preview| {
            preview.query == self.surface.query
                && self
                    .spaces
                    .iter()
                    .any(|s| s.id == id && s.revision == preview.revision)
        })
    }
    pub(super) fn prepare_pending_inline_neighbors(&mut self, geometry: InputTargetGeometry) {
        if !self.inline_ui.pending
            || self.surface.visible
            || !self.flow_allowed()
            || self.surface.presented_query.as_deref() != Some(self.surface.query.as_str())
            || (self.window.window().scale_factor() - geometry.dpi as f32 / 96.0).abs() > 0.01
        {
            return;
        }
        let _timing = crate::popup_timing::span("pending_side_preparation");
        // The HWND stays hidden and unmoved while the independent input worker
        // verifies the target. This only prepares already available card pixels;
        // capture_panel validates the actual query, dimensions, DPI and revision
        // again before the eventual scene may use them.
        let saved = (
            self.window.get_popup_card_width(),
            self.window.get_popup_card_height(),
            self.window.get_inline_mode(),
            self.window.get_quick_insert(),
        );
        let scale = geometry.dpi.clamp(48, 768) as f32 / 96.0;
        let width = echo_presentation::echo_tokens::PANEL_MIN_WIDTH
            .min((geometry.work_area.width as f32 / scale - 56.0).max(120.0));
        self.window.set_popup_card_width(width);
        self.window.set_inline_mode(true);
        self.window.set_quick_insert(true);
        slint::private_unstable_api::re_exports::WindowInner::from_pub(self.window.window())
            .ensure_tree_instantiated();
        self.window
            .set_popup_card_height(self.inline_ui.base_height.unwrap_or(300.0));
        let ids: Vec<_> = self
            .flow_poses()
            .into_iter()
            .filter(|p| p.space != self.deck.requested)
            .filter(|p| {
                self.ui.side_content == SideContent::TitlesOnly || self.has_current_preview(p.space)
            })
            .map(|p| p.space)
            .collect();
        for id in ids {
            // A speculative failure is handled by normal scene preparation with
            // final geometry; it must not switch renderers while input is pending.
            let _ = self.capture_space(id, false);
        }
        self.window.set_popup_card_width(saved.0);
        self.window.set_popup_card_height(saved.1);
        self.window.set_inline_mode(saved.2);
        self.window.set_quick_insert(saved.3);
    }
    pub(super) fn clear_flow_cache(&mut self) {
        self.window.set_stage_image(Default::default());
        self.window.set_preview_image(Default::default());
        self.window.set_capture_rows(ModelRc::default());
        self.window.set_flow_enabled(false);
        #[cfg(feature = "cover-flow")]
        if let Some(flow) = &self.flow {
            flow.clear();
        }
        self.dirty_snapshots
            .extend(self.deck.order().iter().copied());
    }
    pub(super) fn flow_stats(&self) -> (u64, usize, u64, u64) {
        #[cfg(feature = "cover-flow")]
        if let Some(flow) = &self.flow {
            return flow.stats();
        }
        (0, 0, 0, 0)
    }
    pub(super) fn prewarm(&mut self) {
        if self.hook.as_ref().is_some_and(WindowHook::is_composing) {
            return;
        }
        if !self.surface.visible
            || !self.flow_allowed()
            || self.window.get_modal()
            || self.window.get_route().as_str() != "history"
            || self.deck.phase == Phase::Animating
        {
            return;
        }
        #[cfg(feature = "cover-flow")]
        {
            if !self.flow.as_ref().is_some_and(|f| f.ready()) {
                return;
            }
            let poses = self.flow_poses();
            let ids = poses.iter().map(|p| p.space.0).collect::<Vec<_>>();
            self.flow.as_ref().unwrap().retain(&ids);
            self.previews.retain(|id, _| ids.contains(&id.0));
            for pose in &poses {
                if pose.space != self.surface.space
                    && !self
                        .previews
                        .get(&pose.space)
                        .is_some_and(|p| p.query == self.surface.query)
                    && (self.dirty_snapshots.contains(&pose.space)
                        || !self.flow.as_ref().unwrap().contains(pose.space.0))
                    && self.ui.side_content == SideContent::Visible
                    && self.pending_previews.insert(pose.space)
                {
                    if !self.send(Work::Preview(
                        pose.space,
                        self.preview_epoch,
                        self.surface.query.clone(),
                    )) {
                        self.pending_previews.remove(&pose.space);
                    }
                }
            }
            let candidate = poses
                .iter()
                .filter(|p| {
                    !self.inline_active()
                        || p.space == self.surface.space
                        || self.ui.side_content == SideContent::TitlesOnly
                        || self.previews.contains_key(&p.space)
                })
                .filter(|p| {
                    p.space != self.surface.space || self.full_motion() && self.surface.ready
                })
                .find(|p| {
                    self.dirty_snapshots.contains(&p.space)
                        || !self.flow.as_ref().unwrap().contains(p.space.0)
                })
                .map(|p| p.space);
            if let Some(id) = candidate {
                if let Err(error) = self.capture_space(id, false) {
                    if id == self.surface.space || self.capture_space(id, true).is_err() {
                        self.motion_fallback(error);
                        return;
                    }
                }
                self.prepare_scene();
                self.schedule_prewarm();
            } else {
                self.prepare_scene();
            }
        }
        self.refresh_diagnostics();
    }
    pub(super) fn preview_loaded(
        &mut self,
        id: SpaceId,
        epoch: u64,
        result: Result<crate::events::LoadedPage, String>,
    ) {
        if epoch != self.preview_epoch
            || !self.surface.visible
            || self.ui.side_content == SideContent::TitlesOnly
        {
            return;
        }
        self.pending_previews.remove(&id);
        crate::popup_timing::mark("preview_received");
        if !self.flow_poses().iter().any(|p| p.space == id) || id == self.surface.space {
            return;
        }
        if let Ok(mut data) = result {
            if self
                .spaces
                .iter()
                .find(|s| s.id == id)
                .is_some_and(|s| s.revision != data.revision)
            {
                self.send(Work::Spaces);
                return;
            }
            data.page.items.truncate(12);
            let hashes = data
                .page
                .items
                .iter()
                .filter_map(|i| i.thumbnail.as_ref().map(|t| t.content_hash.clone()))
                .take(3)
                .collect::<Vec<_>>();
            self.previews.insert(
                id,
                Preview {
                    query: self.surface.query.clone(),
                    items: data.page.items,
                    revision: data.revision,
                    total: data.total,
                },
            );
            self.dirty_snapshots.insert(id);
            for hash in hashes {
                if !self.images.cache.contains_key(&hash)
                    && self.images.pending.insert(hash.clone())
                {
                    if !self.send(Work::Thumbnail(self.images.epoch, hash.clone())) {
                        self.images.pending.remove(&hash);
                    }
                }
            }
        }
        self.schedule_prewarm();
    }
    fn motion_fallback(&mut self, error: String) {
        self.graphics_error = Some(error.clone());
        self.flow_timer.stop();
        self.deck.snap();
        self.content_ready();
        self.window.set_navigation_busy(false);
        self.clear_flow_cache();
        self.report(format!("Flat compatibility: {error}"), false);
        self.refresh_diagnostics();
    }
    pub(super) fn capture_space(&mut self, id: SpaceId, downsample: bool) -> Result<(), String> {
        #[cfg(feature = "cover-flow")]
        {
            let flow = self.flow.as_ref().ok_or("GPU compositor is unavailable")?;
            if !flow.ready() {
                return Err("GPU compositor is not ready".into());
            }
            let space = self
                .spaces
                .iter()
                .find(|s| s.id == id)
                .ok_or("Space no longer exists")?;
            let private = self.ui.side_content == SideContent::TitlesOnly;
            let rows: ModelRc<EntryRow> = if private {
                ModelRc::default()
            } else if id == self.surface.space
                && self.surface.ready
                && self.deck.phase != Phase::Animating
            {
                self.window.get_rows()
            } else {
                let items = if id == self.surface.space && self.surface.ready {
                    self.surface.items.as_slice()
                } else {
                    self.previews
                        .get(&id)
                        .filter(|p| p.revision == space.revision && p.query == self.surface.query)
                        .map_or(&[][..], |p| p.items.as_slice())
                };
                let mut section = String::new();
                let mut matcher = FuzzyMatcher::new(&self.surface.query);
                let rows = items
                    .iter()
                    .map(|item| {
                        let mut row = formatting::row(item, &mut section);
                        if !self.surface.query.trim().is_empty() {
                            crate::match_highlight::apply(
                                &mut row,
                                &mut matcher,
                                self.window.get_dark(),
                            );
                        }
                        if id == self.surface.space {
                            row.selected = self.surface.selection == Some(RowKey::of(item));
                        }
                        if let Some((image, _)) = item
                            .thumbnail
                            .as_ref()
                            .and_then(|t| self.images.cache.get(&t.content_hash))
                        {
                            row.thumbnail = image.clone();
                        }
                        row
                    })
                    .collect::<Vec<_>>();
                ModelRc::new(VecModel::from(rows))
            };
            let loading = rows.row_count() == 0
                && !private
                && if id == self.surface.space {
                    !self.surface.ready
                } else {
                    !self.previews.contains_key(&id)
                };
            self.window.set_capture_rows(rows);
            self.window.set_capture_title(space.title.clone().into());
            self.window.set_capture_query(if private {
                "".into()
            } else {
                self.surface.query.clone().into()
            });
            self.window
                .set_capture_icon(space.icon_key.clone().unwrap_or_default().into());
            self.window
                .set_capture_accent(accent(if space.id.is_system() {
                    "default"
                } else {
                    &space.accent_key
                }));
            self.window.set_capture_subtitle(
                format!(
                    "{} items{}",
                    self.previews.get(&id).map_or(space.item_count, |p| p.total),
                    if id == SpaceId::HISTORY {
                        " · Clipboard timeline"
                    } else {
                        " · Manual order"
                    }
                )
                .into(),
            );
            let index = self.deck.index(id).unwrap_or(0);
            let count = self.deck.order().len();
            self.window.set_capture_navigation_label(
                format!("{}  ·  {} / {}", space.title, index + 1, count).into(),
            );
            self.window.set_capture_navigation_hint(
                if self.ui.switch_shortcut == SwitchShortcut::CtrlTab {
                    "Ctrl+Tab / Ctrl+Shift+Tab"
                } else {
                    "Tab / Shift+Tab"
                }
                .into(),
            );
            self.window
                .set_capture_previous_enabled(count > 1 && (self.ui.loop_spaces || index > 0));
            self.window
                .set_capture_next_enabled(count > 1 && (self.ui.loop_spaces || index + 1 < count));
            self.window
                .set_capture_has_more(if id == self.surface.space {
                    self.surface.next_cursor.is_some()
                } else {
                    space.item_count > self.window.get_capture_rows().row_count() as u64
                });
            self.window
                .set_capture_has_previous(id == self.surface.space && self.surface.has_previous());
            self.window
                .set_capture_batch(id == self.surface.space && self.surface.batch);
            self.window
                .set_capture_selected_count(if id == self.surface.space {
                    self.surface.selected_ids.len() as i32
                } else {
                    0
                });
            self.window
                .set_capture_quick_insert(self.window.get_quick_insert());
            self.window.set_capture_favorites(id != SpaceId::HISTORY);
            self.window.set_capture_loading(loading);
            self.window.set_capture_titles_only(private);
            self.window.set_capture_scroll(if id == self.surface.space {
                if self.deck.phase == Phase::Animating {
                    self.pending_scroll.unwrap_or(0.0)
                } else {
                    self.window.get_scroll_y()
                }
            } else {
                0.0
            });
            match flow.capture_panel(&self.window, id.0, downsample, space.revision) {
                Ok(()) => {
                    self.dirty_snapshots.remove(&id);
                    Ok(())
                }
                Err(error)
                    if !downsample && id != self.surface.space && error.contains("budget") =>
                {
                    // Rebuild a smaller neighbor once; the live center never loses text resolution.
                    self.dirty_snapshots.insert(id);
                    Err(error)
                }
                Err(error) => Err(error),
            }
        }
        #[cfg(not(feature = "cover-flow"))]
        {
            let _ = (id, downsample);
            Err("GPU compositor is not included in this build".into())
        }
    }
    pub(super) fn prepare_scene(&mut self) {
        if !self.flow_allowed()
            || !self.surface.visible
            || self.window.get_route().as_str() != "history"
        {
            self.window.set_flow_enabled(false);
            return;
        }
        #[cfg(feature = "cover-flow")]
        {
            if !self.flow.as_ref().is_some_and(|f| f.ready()) {
                return;
            }
            let animated = self.deck.phase == Phase::Animating;
            let poses = self.flow_poses();
            // Drive the handoff from the incoming card's position, not a timer
            // started after arrival. Finish before the spring's final snap.
            let distance = (self.deck.spring.position - self.deck.spring.target).abs() as f32;
            let progress = if animated {
                ((0.65 - distance) / 0.55).clamp(0.0, 1.0)
            } else {
                1.0
            };
            self.window
                .set_front_shadow_opacity(progress * progress * (3.0 - 2.0 * progress));
            if let Some(front) = poses.iter().find(|p| p.space == self.deck.requested) {
                let width = self.window.get_panel_width();
                let perspective = (1.0
                    - front.z / (width * echo_presentation::echo_tokens::FLOW_PERSPECTIVE_RATIO))
                    .max(0.01);
                let shadow_width = width * front.scale * front.yaw.cos() / perspective;
                let shadow_height = self.window.get_panel_height() * front.scale / perspective;
                self.window.set_moving_shadow_width(shadow_width);
                self.window.set_moving_shadow_height(shadow_height);
                self.window.set_moving_shadow_x(
                    self.window.get_panel_left() + width / 2.0 + front.x / perspective
                        - shadow_width / 2.0,
                );
                self.window.set_moving_shadow_y(
                    self.window.get_panel_top()
                        + self.window.get_panel_height() / 2.0
                        + front.y / perspective
                        - shadow_height / 2.0,
                );
            }
            let ids = poses.iter().map(|p| p.space.0).collect::<Vec<_>>();
            self.flow.as_ref().unwrap().retain(&ids);
            if !animated {
                let front = self.deck.requested;
                for pose in poses.iter().filter(|p| p.space != front) {
                    if self.has_current_preview(pose.space)
                        || self.ui.side_content == SideContent::TitlesOnly
                    {
                        if let Err(error) = self.capture_space(pose.space, false) {
                            self.motion_fallback(error);
                            return;
                        }
                    }
                }
            }
            if animated {
                for pose in &poses {
                    if !self.flow.as_ref().unwrap().contains(pose.space.0) {
                        if let Err(error) = self.capture_space(pose.space, false) {
                            self.motion_fallback(error);
                            return;
                        }
                    }
                }
            }
            let panels = poses
                .iter()
                .filter(|p| animated || p.space != self.deck.requested)
                .filter(|p| {
                    animated
                        || !self.inline_active()
                        || self.has_current_preview(p.space)
                        || self.ui.side_content == SideContent::TitlesOnly
                })
                .map(|p| crate::cover_flow::compositor::PanelDraw {
                    id: p.space.0,
                    shadow_opacity: if animated && p.space == self.deck.requested {
                        1.0 - self.window.get_front_shadow_opacity()
                    } else {
                        1.0
                    },
                    origin_x: self.window.get_panel_left() + self.window.get_panel_width() / 2.0
                        - self.window.get_stage_width() / 2.0,
                    width: self.window.get_panel_width(),
                    height: self.window.get_panel_height(),
                    x: p.x,
                    y: p.y,
                    z: p.z,
                    yaw: p.yaw,
                    scale: p.scale,
                    opacity: p.opacity,
                    shade: p.shade,
                })
                .collect();
            self.flow.as_ref().unwrap().effects(
                self.ui.reflections
                    && self.full_motion()
                    && !(self.ui.reduce_on_battery && self.environment.on_battery),
            );
            let result = self.flow.as_ref().unwrap().present(
                self.window.get_stage_width(),
                self.window.get_stage_height(),
                self.window.window().scale_factor(),
                panels,
            );
            match result {
                Ok((image, changed)) => {
                    // WGPU images also lack value equality in the pinned runtime.
                    // Avoid invalidating Slint's image item for an unchanged scene.
                    if changed || !self.window.get_flow_enabled() {
                        self.window.set_stage_image(image);
                    }
                    self.window.set_flow_enabled(true);
                    if changed {
                        self.window.window().request_redraw();
                    }
                }
                Err(error) => self.motion_fallback(error),
            }
        }
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
    pub(super) fn stage_click(&mut self, x: f32, y: f32) {
        if !self.flow_allowed()
            || !x.is_finite()
            || !y.is_finite()
            || self.deck.phase == Phase::Animating
        {
            return;
        }
        let width = self.window.get_panel_width();
        let height = self.window.get_panel_height();
        let point = [
            x - self.window.get_panel_left() - width / 2.0,
            y - self.window.get_panel_top() - height / 2.0,
        ];
        if point[0].abs() <= width / 2.0 && point[1].abs() <= height / 2.0 {
            return;
        }
        let mut poses = self.flow_poses();
        poses.sort_by(|a, b| b.z.total_cmp(&a.z));
        for pose in poses {
            if pose.space != self.deck.requested
                && echo_presentation::deck::hit_panel(pose, point, width, height)
            {
                self.navigate_to(pose.space);
                return;
            }
        }
    }
}
