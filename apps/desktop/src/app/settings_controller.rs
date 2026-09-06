//! Draft settings, runtime effect policy and redacted diagnostics.
use super::*;
use echo_presentation::echo_tokens as t;
impl App {
    pub(super) fn read_ui_draft(&self) -> Result<UiSettings, String> {
        let w = &self.window;
        let mut value = self.ui.clone();
        macro_rules! enum_field {
            ($field:ident,$getter:ident,$type:ident) => {
                value.$field = $type::parse(w.$getter().as_str())
                    .ok_or_else(|| format!("Invalid {} setting", stringify!($field)))?;
            };
        }
        enum_field!(view_mode, get_view_mode, SpaceViewMode);
        enum_field!(motion, get_motion, Motion);
        enum_field!(motion_speed, get_motion_speed, MotionSpeed);
        enum_field!(switch_shortcut, get_switch_shortcut, SwitchShortcut);
        enum_field!(startup_space, get_startup_space, StartupSpace);
        enum_field!(query_on_switch, get_query_on_switch, QueryOnSwitch);
        enum_field!(density, get_density, Density);
        enum_field!(side_content, get_side_content, SideContent);
        enum_field!(graphics, get_graphics_mode, GraphicsMode);
        enum_field!(frame_rate, get_frame_rate, FrameRate);
        value.loop_spaces = w.get_loop_spaces();
        value.remember_position = w.get_remember_position();
        value.reflections = w.get_reflections();
        value.trim_when_hidden = w.get_trim_when_hidden();
        value.reduce_on_battery = w.get_reduce_on_battery();
        value.validate()?;
        Ok(value)
    }
    fn read_settings_patch(&self) -> Result<SettingsPatch, String> {
        let w = &self.window;
        let clipboard = formatting::settings(
            w.get_max_entries_text().as_str(),
            w.get_max_total_mib_text().as_str(),
            w.get_max_item_mib_text().as_str(),
            w.get_theme_mode().as_str(),
            w.get_history_enabled(),
            w.get_record_sensitive(),
            w.get_store_window_titles(),
        )?;
        Ok(SettingsPatch {
            expected_revision: self.settings_revision,
            clipboard,
            ui: self.read_ui_draft()?,
        })
    }
    pub(super) fn render_settings(&self) {
        let w = &self.window;
        let c = &self.settings;
        let u = &self.ui;
        w.set_history_enabled(c.history_enabled);
        w.set_record_sensitive(c.record_sensitive);
        w.set_store_window_titles(c.store_window_titles);
        w.set_max_entries_text(c.max_entries.to_string().into());
        w.set_max_total_mib_text((c.max_total_bytes / (1024 * 1024)).to_string().into());
        w.set_max_item_mib_text((c.max_item_bytes / (1024 * 1024)).to_string().into());
        w.set_theme_mode(c.theme.as_str().into());
        w.set_view_mode(u.view_mode.as_str().into());
        w.set_motion(u.motion.as_str().into());
        w.set_motion_speed(u.motion_speed.as_str().into());
        w.set_switch_shortcut(u.switch_shortcut.as_str().into());
        w.set_startup_space(u.startup_space.as_str().into());
        w.set_query_on_switch(u.query_on_switch.as_str().into());
        w.set_density(u.density.as_str().into());
        w.set_side_content(u.side_content.as_str().into());
        w.set_graphics_mode(u.graphics.as_str().into());
        w.set_frame_rate(u.frame_rate.as_str().into());
        w.set_loop_spaces(u.loop_spaces);
        w.set_remember_position(u.remember_position);
        w.set_reflections(u.reflections);
        w.set_trim_when_hidden(u.trim_when_hidden);
        w.set_reduce_on_battery(u.reduce_on_battery);
        w.set_settings_dirty(false);
        w.set_settings_valid(true);
        w.set_settings_error("".into());
        w.set_settings_notice("".into());
        w.set_restart_required(self.worker.bootstrap.ui.graphics != u.graphics);
    }
    pub(super) fn settings_edited(&mut self) {
        if self.window.get_route().as_str() != "settings" {
            return;
        }
        match self.read_settings_patch() {
            Ok(patch) => {
                self.window
                    .set_settings_dirty(patch.clipboard != self.settings || patch.ui != self.ui);
                self.window.set_settings_valid(true);
                self.window.set_settings_error("".into());
            }
            Err(error) => {
                self.window.set_settings_dirty(true);
                self.window.set_settings_valid(false);
                self.window.set_settings_error(error.into());
            }
        }
        self.apply_theme();
        self.refresh_diagnostics();
    }
    pub(super) fn save_settings(&mut self) {
        match self.read_settings_patch() {
            Ok(patch) => self.mutate(Mutation::SettingsPatch(patch)),
            Err(e) => {
                self.window.set_settings_error(e.clone().into());
                self.report(e, true);
            }
        }
    }
    pub(super) fn settings_action(&mut self, action: &str) {
        match action {
            "cancel" => self.request_route("history"),
            "about" => self.request_route("about"),
            "quit" => self.request_quit(),
            "defaults" => {
                let d = UiSettings::default();
                let w = &self.window;
                w.set_view_mode(d.view_mode.as_str().into());
                w.set_motion(d.motion.as_str().into());
                w.set_motion_speed(d.motion_speed.as_str().into());
                w.set_density(d.density.as_str().into());
                w.set_reflections(d.reflections);
                w.set_side_content(d.side_content.as_str().into());
                w.set_theme_mode("system".into());
                self.settings_edited();
            }
            "preview" => self.play_preview(),
            "clear-cache" => {
                self.clear_flow_cache();
                self.previews.clear();
                self.refresh_diagnostics();
                self.report("Motion cache cleared; saved content was not changed", false);
            }
            "diagnostics" => self.export_diagnostics(),
            "restart" => {
                if !self.window.get_settings_dirty() {
                    self.ask_confirmation("Restart Echo?","Echo will reopen with saved settings. Pending insertion is cancelled, never replayed.","Restart Echo",false,Confirmation::Restart);
                }
            }
            _ => {}
        }
    }
    pub(super) fn flow_allowed(&self) -> bool {
        self.graphics.perspective
            && self.graphics_error.is_none()
            && !self.environment.high_contrast
            && self.ui.view_mode == SpaceViewMode::CoverFlow
            && self.window.get_stage_width() >= 520.0
    }
    pub(super) fn full_motion(&self) -> bool {
        self.flow_allowed()
            && self.ui.motion == Motion::System
            && self.environment.animations
            && self.ui.side_content != SideContent::TitlesOnly
    }
    pub(super) fn frame_interval(&self) -> Duration {
        let hz = if self.ui.frame_rate == FrameRate::Fps60
            || self.ui.reduce_on_battery && self.environment.on_battery
        {
            60
        } else {
            self.environment.refresh_hz.clamp(40, 240)
        };
        #[cfg(feature = "cover-flow")]
        let hz = hz.min(
            self.flow
                .as_ref()
                .map_or(60, |flow| flow.policy().frame_cap()),
        );
        Duration::from_micros(1_000_000 / u64::from(hz))
    }
    pub(super) fn apply_theme(&mut self) {
        let theme = if self.window.get_route().as_str() == "settings" {
            ThemeMode::parse(self.window.get_theme_mode().as_str()).unwrap_or(self.settings.theme)
        } else {
            self.settings.theme
        };
        let dark = match theme {
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
            ThemeMode::System => shell::system_dark(),
        };
        let changed = self.window.get_dark() != dark;
        self.window.set_dark(dark);
        let t = self.window.global::<crate::EchoTheme>();
        let color = |c: u32| {
            slint::Color::from_rgb_u8(
                (c & 255) as u8,
                ((c >> 8) & 255) as u8,
                ((c >> 16) & 255) as u8,
            )
        };
        t.set_high_contrast(self.environment.high_contrast);
        t.set_hc_background(color(self.environment.background));
        t.set_hc_text(color(self.environment.text));
        t.set_hc_highlight(color(self.environment.highlight));
        t.set_hc_highlight_text(color(self.environment.highlight_text));
        t.set_hc_muted(color(self.environment.muted));
        let forced = std::env::var("ECHO_WINDOWS_ACCEPTANCE").as_deref() == Ok("1")
            && std::env::var("ECHO_ACCEPTANCE_FORCE_MICA_FALLBACK").as_deref() == Ok("1");
        let _ = forced;
        if let Some(hwnd) = self.hwnd {
            shell::apply_theme(hwnd, dark, false);
        }
        self.window.set_native_mica(false);
        #[cfg(feature = "cover-flow")]
        if self.flow.as_ref().is_some_and(|flow| {
            flow.set_economical(
                self.graphics.integrated
                    || self.ui.reduce_on_battery && self.environment.on_battery,
            )
        }) {
            self.clear_flow_cache();
            self.schedule_prewarm();
        }
        if let Some(hwnd) = self.hwnd {
            let _ = shell::apply_card_chrome(hwnd);
        }
        if changed && self.deck.phase == Phase::Animating {
            self.finish_motion();
        }
        if changed {
            self.clear_flow_cache();
            self.dirty_snapshots
                .extend(self.deck.order().iter().copied());
            self.schedule_prewarm();
        }
        self.refresh_diagnostics();
    }
    pub(super) fn refresh_diagnostics(&self) {
        let reason = if self.environment.high_contrast {
            Some("Windows High Contrast")
        } else if self.window.get_stage_width() < 520.0 {
            Some("Small display work area")
        } else if !self.graphics.perspective {
            self.graphics
                .fallback
                .as_deref()
                .or(Some("Software renderer"))
        } else {
            self.graphics_error.as_deref()
        };
        let actual = if let Some(reason) = reason {
            format!(
                "Preferred: {} · Actual: flat compatibility ({reason})",
                self.ui.view_mode.as_str()
            )
        } else if self.ui.view_mode == SpaceViewMode::Flat {
            "Actual: flat native panels".into()
        } else if self.full_motion() {
            "Actual: Cover Flow · perspective-correct GPU panels".into()
        } else {
            "Actual: Cover Flow · static side panels, instant switching".into()
        };
        self.window.set_actual_mode(actual.into());
        self.window.set_diagnostics(format!("Renderer: {}\nAdapter: {}\nBackend: {}\nSystem animation: {} · Battery: {}\nGraphics preference changes require restart.",
            self.graphics.renderer,self.graphics.adapter,self.graphics.backend,self.environment.animations,self.environment.on_battery).into());
        let (bytes, panels, frames, uploads) = self.flow_stats();
        let (policy, limit, scale, cap) = self.motion_policy_info();
        self.window.set_cache_status(format!("Motion policy: {policy} · {cap} Hz ceiling · raster scale ≤ {scale}\nPanel textures + output: {:.2} MiB / {:.0} MiB target (64 MiB hard limit)\nResident panels: {panels} · Draws: {frames} · Pixel uploads: {uploads}\nSettled text uses native DPI. These counters exclude total process/driver memory.",bytes as f64/1048576.0,limit as f64/1048576.0).into());
    }
    pub(super) fn play_preview(&mut self) {
        self.preview_timer.stop();
        self.preview_started = None;
        #[cfg(feature = "cover-flow")]
        {
            if !self.flow.as_ref().is_some_and(|f| f.ready()) {
                self.report("Flat compatibility: the preview is static", false);
                return;
            }
            self.clear_flow_cache();
            for (id, title) in [(-1, "History"), (-2, "Favorites")] {
                let rows = [
                    ("Email", "hello@example.test"),
                    ("Phone", "+1 202 555 0147"),
                    ("Greeting", "Thanks — I will get back to you."),
                    ("Office", "Synthetic content, stored nowhere"),
                ]
                .into_iter()
                .enumerate()
                .map(|(i, (label, body))| {
                    let mut row = EntryRow::default();
                    row.key = format!("demo-{i}").into();
                    row.title = label.into();
                    row.body = body.into();
                    row.kind = "text".into();
                    row.source_label = "Synthetic preview".into();
                    row.time_label = "10:24".into();
                    row.selected = i == 0;
                    row
                })
                .collect::<Vec<_>>();
                self.window.set_capture_title(title.into());
                self.window.set_capture_query("".into());
                self.window
                    .set_capture_subtitle("Synthetic example content".into());
                self.window.set_capture_icon("".into());
                self.window.set_capture_favorites(id == -2);
                self.window.set_capture_accent(accent_preview());
                self.window
                    .set_capture_rows(ModelRc::new(VecModel::from(rows)));
                self.window.set_capture_scroll(0.0);
                self.window.set_capture_loading(false);
                self.window.set_capture_titles_only(false);
                self.window.set_capture_navigation_label(title.into());
                self.window
                    .set_capture_navigation_hint("Tab / Shift+Tab".into());
                self.window.set_capture_search_focused(false);
                self.window.set_capture_has_more(false);
                self.window.set_capture_has_previous(false);
                self.window.set_capture_previous_enabled(true);
                self.window.set_capture_next_enabled(true);
                self.window.set_capture_batch(false);
                self.window.set_capture_quick_insert(false);
                if let Err(error) =
                    self.flow
                        .as_ref()
                        .unwrap()
                        .capture_panel(&self.window, id, true)
                {
                    self.report(error, true);
                    return;
                }
            }
            self.preview_started = Some(Instant::now());
            self.preview_tick();
            let u = self.read_ui_draft().unwrap_or_else(|_| self.ui.clone());
            if u.motion == Motion::System
                && self.environment.animations
                && !self.environment.high_contrast
            {
                let hub = self.hub.clone();
                self.preview_timer
                    .start(TimerMode::Repeated, self.frame_interval(), move || {
                        hub.post(Event::Command(Command::FlowTick))
                    });
            }
        }
        #[cfg(not(feature = "cover-flow"))]
        self.report("Flat compatibility: the preview is static", false);
    }
    pub(super) fn preview_tick(&mut self) {
        let Some(start) = self.preview_started else {
            return;
        };
        if self.window.get_route().as_str() != "settings" {
            self.preview_timer.stop();
            self.preview_started = None;
            return;
        }
        #[cfg(feature = "cover-flow")]
        {
            let u = self.read_ui_draft().unwrap_or_else(|_| self.ui.clone());
            let elapsed = start.elapsed().as_secs_f32();
            let moving = u.view_mode == SpaceViewMode::CoverFlow
                && u.motion == Motion::System
                && self.environment.animations
                && !self.environment.high_contrast;
            let finished =
                !moving || start.elapsed().as_millis() >= u128::from(u.motion_speed.deadline_ms());
            let omega = u.motion_speed.omega() as f32;
            let progress = if finished {
                1.0
            } else {
                1.0 - (1.0 + omega * elapsed) * (-omega * elapsed).exp()
            };
            let scale = 0.22;
            let w = self.window.get_panel_width() * scale;
            let h = self.window.get_panel_height() * scale;
            let poses = (0..2)
                .map(|i| {
                    let d = i as f32 - progress;
                    let amount = d.abs().min(1.0);
                    crate::cover_flow::compositor::PanelDraw {
                        id: -1 - i,
                        width: w,
                        height: h,
                        x: d * w * t::FLOW_SIDE_X_RATIO,
                        y: t::FLOW_SIDE_Y * scale * amount,
                        z: w * t::FLOW_SIDE_Z_RATIO * amount,
                        yaw: if u.view_mode == SpaceViewMode::CoverFlow {
                            -d.signum() * t::FLOW_SIDE_ANGLE.to_radians() * amount
                        } else {
                            0.0
                        },
                        scale: 1.0 - (1.0 - t::FLOW_SIDE_SCALE) * amount,
                        opacity: 1.0 - (1.0 - t::FLOW_SIDE_OPACITY) * amount,
                        shade: t::FLOW_SIDE_SHADE * amount,
                    }
                })
                .collect();
            if let Some(flow) = &self.flow {
                flow.effects(u.reflections && moving);
                match flow.present(320.0, 166.0, self.window.window().scale_factor(), poses) {
                    Ok((image, changed)) => {
                        self.window.set_preview_image(image);
                        if changed {
                            self.window.window().request_redraw();
                        }
                    }
                    Err(error) => {
                        self.preview_timer.stop();
                        self.preview_started = None;
                        self.report(error, true);
                        return;
                    }
                }
            }
            if finished {
                self.preview_timer.stop();
                self.preview_started = None;
            }
        }
        self.refresh_diagnostics();
    }
    fn motion_policy_info(&self) -> (&'static str, u64, f32, u32) {
        #[cfg(feature = "cover-flow")]
        if let Some(flow) = &self.flow {
            let p = flow.policy();
            return (
                p.label(),
                p.texture_limit(),
                p.maximum_scale(),
                p.frame_cap(),
            );
        }
        ("software", 0, 1.0, 0)
    }
    fn export_diagnostics(&mut self) {
        let (bytes, panels, frames, uploads) = self.flow_stats();
        let (policy, texture_limit, scale_cap, frame_cap) = self.motion_policy_info();
        let report = crate::events::DiagnosticReport {
            schema: "echo.cover-flow.diagnostics.v1",
            version: env!("CARGO_PKG_VERSION"),
            renderer: self.graphics.renderer.clone(),
            backend: self.graphics.backend.clone(),
            adapter: self.graphics.adapter.clone(),
            raster_policy: policy,
            panel_texture_limit: texture_limit,
            motion_scale_cap: scale_cap,
            motion_frame_cap: frame_cap,
            actual_mode: self.window.get_actual_mode().to_string(),
            panel_texture_bytes: bytes,
            resident_panels: panels,
            draw_count: frames,
            upload_count: uploads,
            scale_factor: self.window.window().scale_factor(),
            high_contrast: self.environment.high_contrast,
            system_animations: self.environment.animations,
            on_battery: self.environment.on_battery,
        };
        self.report("Exporting redacted diagnostics…", false);
        self.send(Work::Diagnostics(report));
    }
}
#[cfg(feature = "cover-flow")]
fn accent_preview() -> slint::Color {
    slint::Color::from_rgb_u8(255, 196, 0)
}
