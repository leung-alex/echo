//! Software cards move existing Slint components. Only the departing row model
//! survives a navigation load, and it is released before another load is admitted.
use super::*;
use echo_presentation::slide::{ContentFrame, Request, Slide};

#[derive(Default)]
pub(super) struct SoftwareDeck {
    pub slide: Slide,
    preparing: Option<ContentFrame>,
    preparation: u64,
    pub clock: Option<shell::AnimationClock>,
    pub outgoing_bytes: usize,
    pub outgoing_image_bytes: usize,
    pub side_errors: HashMap<SpaceId, super::side_previews::PreviewError>,
    pub side_signatures: [Option<u64>; 2],
    pub side_model_bytes: [usize; 2],
    pub image_version: u64,
}

impl App {
    pub(super) fn navigate_software_to(&mut self, id: SpaceId) {
        let from = self.software.slide.source().unwrap_or(self.surface.space);
        let a = self.deck.index(from).unwrap_or(0) as i64;
        let b = self.deck.index(id).unwrap_or(0) as i64;
        let n = self.deck.order().len() as i64;
        let mut delta = b - a;
        if self.ui.loop_spaces && n > 2 && delta.abs() > n / 2 {
            delta -= delta.signum() * n;
        }
        let request = self.software.slide.request(from, id, delta as f32);
        let Request::Load { retain_outgoing } = request else {
            return;
        };
        self.remember_position();
        self.search_timer.stop();
        self.cancel_prewarm();
        self.previews.clear();
        self.inspect_intent = None;
        self.software.preparing = None;
        crate::graphics::cancel_software_frame();
        self.window.set_departing_side(if delta < 0 {
            self.window.get_right_space()
        } else {
            self.window.get_left_space()
        });
        self.window.set_departing_preview(if delta < 0 {
            self.window.get_right_preview()
        } else {
            self.window.get_left_preview()
        });
        if !retain_outgoing {
            let w = &self.window;
            if self.surface.ready && !self.surface.dirty {
                w.set_outgoing_space(
                    w.get_spaces()
                        .iter()
                        .find(|s| s.key.as_str() == from.to_string())
                        .unwrap_or_default(),
                );
                w.set_outgoing(crate::OutgoingPanel {
                    rows: ModelRc::from(self.model.clone()),
                    title: w.get_space_title(),
                    subtitle: w.get_space_subtitle(),
                    icon_key: w.get_space_icon(),
                    accent: w.get_space_accent(),
                    favorites: self.surface.space != SpaceId::HISTORY,
                    clearable: w.get_space_clearable(),
                    scroll_y: w.get_scroll_y(),
                    query: w.get_query(),
                    navigation_label: w.get_navigation_label(),
                    navigation_hint: if self.ui.switch_shortcut == SwitchShortcut::CtrlTab {
                        "Ctrl+Tab / Ctrl+Shift+Tab".into()
                    } else {
                        "Tab / Shift+Tab".into()
                    },
                    previous_enabled: w.get_previous_enabled(),
                    next_enabled: w.get_next_enabled(),
                    has_more: w.get_has_more(),
                    has_previous: w.get_has_previous(),
                    batch: w.get_batch_mode(),
                    selected_count: w.get_selected_count(),
                    selection_index: w.get_selection_index(),
                    status: w.get_status(),
                    status_error: w.get_status_error(),
                    empty_state_text: w.get_empty_state_text(),
                });
                w.set_outgoing_present(true);
                let (preview, bytes) = self.make_side_preview(&self.surface.items, false, "");
                w.set_outgoing_preview(preview);
                self.software.outgoing_bytes =
                    self.model_bytes + bytes + self.software.side_model_bytes.iter().sum::<usize>();
                let mut images: Vec<slint::Image> = Vec::new();
                self.software.outgoing_image_bytes = 0;
                for row in self.model.iter() {
                    if row.thumbnail.size().width > 0 && !images.contains(&row.thumbnail) {
                        self.software.outgoing_image_bytes += row.thumbnail.size().width as usize
                            * row.thumbnail.size().height as usize
                            * 4;
                        images.push(row.thumbnail);
                    }
                }
                for preview in [
                    w.get_left_preview(),
                    w.get_right_preview(),
                    w.get_outgoing_preview(),
                    w.get_departing_preview(),
                ] {
                    for row in preview.rows.iter() {
                        if row.image.size().width > 0 && !images.contains(&row.image) {
                            self.software.outgoing_image_bytes += row.image.size().width as usize
                                * row.image.size().height as usize
                                * 4;
                            images.push(row.image);
                        }
                    }
                }
            }
        }
        // Move ownership instead of mutating the frozen departing VecModel.
        self.model = Rc::new(crate::native_model::EntryModel::default());
        self.model_bytes = 0;
        self.window.set_rows(ModelRc::from(self.model.clone()));
        self.window.set_incoming_x(0.0);
        self.window.set_outgoing_x(0.0);
        self.window.set_slide_moving(false);
        self.window.set_carousel_progress(1.0);
        self.images.epoch = self.images.epoch.wrapping_add(1);
        self.images.pending.clear();
        // The outgoing model keeps its visible pixels; incoming cache gets a
        // separate 2 MiB share of the 4 MiB total, with no adjacent preloading.
        self.images.cache.clear();
        self.images.order.clear();
        self.images.bytes = 0;
        self.deck.request(id, self.now(), false);
        self.surface.hide();
        self.surface.set_space(id);
        self.surface.visible = true;
        let query = if self.inline_active() {
            self.window.get_query().to_string()
        } else {
            String::new()
        };
        self.surface.set_query(query.clone());
        self.window.set_query(query.into());
        self.pending_scroll = Some(if self.ui.remember_position {
            self.positions.restore(&mut self.surface)
        } else {
            0.0
        });
        self.window.set_scroll_y(0.0);
        self.window.set_stale_rows(true);
        self.window.set_navigation_busy(true);
        crate::memory_trace::record(
            "slide_loading",
            serde_json::json!({"space":id.0,"query_epoch":self.surface.query_epoch()}),
        );
        self.load(false);
        self.render_navigation();
        self.update_card_region();
        if !self.inline_active() {
            self.window.invoke_focus_content();
        }
    }

    pub(super) fn software_content_ready(&mut self) {
        if !self.surface.visible {
            return;
        }
        if self.surface.error && !self.surface.loading {
            self.cancel_software_slide();
            self.deck.block_content();
            return;
        }
        if !self.surface.ready
            || self.surface.loading
            || self.surface.dirty
            || self.surface.space != self.deck.requested
            || self.surface.presented_query.as_deref() != Some(self.surface.query.as_str())
        {
            self.window.set_navigation_busy(true);
            return;
        }
        self.prepare_software_neighbors();
        if !self.software_neighbors_ready() {
            self.window.set_navigation_busy(true);
            return;
        }
        if self.software.slide.loading() {
            if self
                .spaces
                .iter()
                .any(|s| s.id == self.surface.space && s.revision > self.surface.revision)
            {
                self.surface.invalidate();
                self.load(false);
                return;
            }
            if self
                .spaces
                .iter()
                .find(|s| s.id == self.surface.space)
                .is_none_or(|s| s.revision != self.surface.revision)
            {
                self.send(Work::Spaces);
                return;
            }
            // The incoming live tree sits behind the outgoing card while loading.
            // Instantiate it at full size so viewport thumbnail requests are real,
            // then allow their typed events to finish before moving either card.
            if self.software.preparing.is_none() && self.images.pending.is_empty() {
                slint::private_unstable_api::re_exports::WindowInner::from_pub(
                    self.window.window(),
                )
                .ensure_tree_instantiated();
                self.software.preparation = self.software.preparation.wrapping_add(1);
                let stamp = self.software_frame_stamp();
                self.software.preparing = Some(stamp);
                crate::graphics::expect_software_frame(stamp, self.hub.clone());
                self.window.window().request_redraw();
            }
            return;
        }
        if !self.software.slide.moving() {
            let was_idle = self.deck.phase == Phase::Idle;
            self.deck.ready(self.surface.space);
            if !was_idle && self.deck.can_insert(self.surface.space) {
                crate::memory_trace::record(
                    "view_ready",
                    serde_json::json!({"space":self.surface.space.0,"revision":self.surface.revision,"query_epoch":self.surface.query_epoch(),"rows":self.model.row_count(),"outgoing":false,"dpi_scale":self.window.window().scale_factor(),"window":[self.window.get_stage_width(),self.window.get_stage_height()]}),
                );
            }
        }
        self.window
            .set_navigation_busy(!self.deck.can_insert(self.surface.space));
    }

    pub(super) fn software_frame_stamp(&self) -> ContentFrame {
        ContentFrame {
            preparation: self.software.preparation,
            session: self.session.epoch,
            query: self.surface.query_epoch(),
            revision: self.surface.revision,
            space: self.surface.space,
        }
    }
    pub(super) fn software_frame_ready(&mut self, stamp: ContentFrame) {
        if self.software.preparing != Some(stamp) {
            return;
        }
        self.software.preparing = None;
        if stamp != self.software_frame_stamp()
            || !self.surface.visible
            || !self.surface.ready
            || self.surface.loading
            || self.surface.dirty
            || !self.images.pending.is_empty()
            || !self.software_neighbors_ready()
            || !self.software.slide.loading()
        {
            return;
        }
        let motion = self.full_motion() && self.window.get_outgoing_present();
        self.software
            .slide
            .ready(self.surface.space, self.now(), self.ui.motion_speed, motion);
        self.deck.phase = Phase::Animating;
        self.deck.interaction = None;
        self.window.set_slide_moving(motion);
        self.flow_timer.stop();
        self.window
            .set_left_space(self.window.get_requested_left_space());
        self.window
            .set_right_space(self.window.get_requested_right_space());
        self.render_software_side_previews();
        let (preview, bytes) = self.make_side_preview(&self.surface.items, false, "");
        self.window.set_incoming_preview(preview);
        self.software.outgoing_bytes += bytes;
        self.window.set_incoming_space(
            self.window
                .get_spaces()
                .iter()
                .find(|s| s.key.as_str() == self.surface.space.to_string())
                .unwrap_or_default(),
        );
        self.update_card_region();
        if motion {
            self.software.clock = shell::AnimationClock::start();
        }
        crate::memory_trace::record(
            "slide_started",
            serde_json::json!({"space":self.surface.space.0,"revision":self.surface.revision,"query_epoch":stamp.query,"prepared_frame":true,"duration_ms":if motion {echo_presentation::slide::duration_ms(self.ui.motion_speed)} else {0}}),
        );
        self.software_tick();
    }

    pub(super) fn software_tick(&mut self) {
        if !self.surface.visible || self.window.get_route().as_str() != "history" {
            self.cancel_software_slide();
            return;
        }
        if self.software.slide.loading() {
            self.software_content_ready();
            return;
        }
        let now = self.now();
        if let Some((p, direction)) = self.software.slide.motion(now) {
            let width = self.window.get_panel_width();
            let side = self.window.get_side_width();
            let outgoing = -direction
                * if direction > 0.0 {
                    side + 12.0
                } else {
                    width + 12.0
                }
                * p;
            let incoming = direction
                * if direction > 0.0 {
                    width + 12.0
                } else {
                    side + 12.0
                }
                * (1.0 - p);
            self.window.set_carousel_progress(p);
            self.window.set_carousel_direction(direction);
            self.window.set_outgoing_x(outgoing);
            self.window.set_incoming_x(incoming);
            if !self.software.slide.finished(now) {
                return;
            }
            let pending = self.software.slide.finish();
            self.release_outgoing_panel();
            self.flow_timer.stop();
            self.software.clock = None;
            self.deck.snap();
            self.render();
            crate::memory_trace::record(
                "slide_finished",
                serde_json::json!({"space":self.surface.space.0,"revision":self.surface.revision,"outgoing_released":true}),
            );
            if let Some(id) = pending.filter(|id| *id != self.surface.space) {
                self.navigate_to(id);
            } else {
                self.software_content_ready();
                self.inline_results_ready();
            }
            self.update_card_region();
        } else {
            self.flow_timer.stop();
            self.software.clock = None;
        }
    }

    fn release_outgoing_panel(&mut self) {
        self.window.set_outgoing_present(false);
        self.window.set_outgoing(Default::default());
        self.software.outgoing_bytes = 0;
        self.software.outgoing_image_bytes = 0;
        self.window.set_slide_moving(false);
        self.window.set_incoming_x(0.0);
        self.window.set_outgoing_x(0.0);
        self.window.set_carousel_progress(1.0);
        self.window.set_outgoing_space(Default::default());
        self.window.set_departing_side(Default::default());
        self.window.set_incoming_preview(Default::default());
        self.window.set_outgoing_preview(Default::default());
        self.window.set_departing_preview(Default::default());
        self.software.preparing = None;
        crate::graphics::cancel_software_frame();
    }

    pub(super) fn cancel_software_slide(&mut self) {
        self.software.slide.cancel();
        self.flow_timer.stop();
        self.software.clock = None;
        self.release_outgoing_panel();
        self.clear_software_side_models();
    }
}
