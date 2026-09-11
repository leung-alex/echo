//! Draft settings, runtime effect policy and redacted diagnostics.
use super::*;
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
        value.global_hotkey_enabled = w.get_global_hotkey_enabled();
        value.global_hotkey = GlobalShortcut::parse(w.get_global_hotkey().as_str())?.canonical();
        value.caret_anchor = w.get_caret_anchor();
        value.inline_completion = w.get_inline_completion();
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
        let _timing = crate::popup_timing::span("settings_model_update");
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
        w.set_global_hotkey_enabled(u.global_hotkey_enabled);
        w.set_global_hotkey(u.global_hotkey.clone().into());
        w.set_caret_anchor(u.caret_anchor);
        w.set_inline_completion(u.inline_completion);
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
        w.set_restart_required(false);
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
            "retry-hotkey" => {
                self.send(Work::RetryHotkey);
            }
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

    pub(super) fn full_motion(&self) -> bool {
        self.ui.view_mode == SpaceViewMode::CoverFlow
            && !self.environment.high_contrast
            && self.ui.motion == Motion::System
            && self.environment.animations
    }

    pub(super) fn apply_theme(&mut self) {
        let _timing = crate::popup_timing::span("theme_update");
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
            if self.native_theme != Some((hwnd, dark)) {
                // DWM attributes survive hiding. Reapplying the same corner/backdrop
                // policy on every activation synchronously invalidates native chrome.
                // Theme notifications clear this key even when dark mode is unchanged.
                shell::apply_theme(hwnd, dark, false);
                if shell::apply_card_chrome(hwnd).is_ok() {
                    self.native_theme = Some((hwnd, dark));
                }
            }
        }
        self.window.set_native_mica(false);
        if changed && self.deck.phase == Phase::Animating {
            self.finish_motion();
        }
        if changed {
            self.schedule_prewarm();
        }
        self.refresh_diagnostics();
    }
    pub(super) fn refresh_diagnostics(&self) {
        self.window.set_actual_mode(
            if self.ui.view_mode == SpaceViewMode::Flat {
                "Actual: software rendering · static card"
            } else if self.full_motion() {
                "Actual: software rendering · card carousel"
            } else {
                "Actual: software rendering · instant card switching"
            }
            .into(),
        );
        self.window.set_diagnostics(
            format!(
                "Renderer: {}\nAdapter: {}\nBackend: {}\nSystem animation: {}",
                self.graphics.renderer,
                self.graphics.adapter,
                self.graphics.backend,
                self.environment.animations
            )
            .into(),
        );
        self.window.set_cache_status("Display cache budget: 12 MiB\nSearch 4 MiB · Thumbnails 4 MiB · Pages and messages 4 MiB".into());
    }
    pub(super) fn play_preview(&mut self) {
        self.preview_timer.stop();
        self.preview_started = None;
        {
            let draft = self.read_ui_draft().unwrap_or_else(|_| self.ui.clone());
            self.software.clock = (draft.view_mode == SpaceViewMode::CoverFlow
                && draft.motion == Motion::System
                && self.environment.animations
                && !self.environment.high_contrast)
                .then(shell::AnimationClock::start)
                .flatten();
            self.preview_started = Some(Instant::now());
            self.window.set_preview_progress(0.0);
            self.preview_tick();
            self.window.window().request_redraw();
            return;
        }
    }
    pub(super) fn preview_tick(&mut self) {
        let Some(start) = self.preview_started else {
            return;
        };
        if self.window.get_route().as_str() != "settings" {
            self.preview_timer.stop();
            self.preview_started = None;
            self.software.clock = None;
            return;
        }
        {
            let draft = self.read_ui_draft().unwrap_or_else(|_| self.ui.clone());
            let duration = if draft.view_mode == SpaceViewMode::CoverFlow
                && draft.motion == Motion::System
                && self.environment.animations
                && !self.environment.high_contrast
            {
                echo_presentation::slide::duration_ms(draft.motion_speed)
            } else {
                0
            };
            let p =
                echo_presentation::slide::progress(start.elapsed().as_millis() as u64, duration);
            self.window.set_preview_progress(p);
            if p >= 1.0 {
                self.preview_timer.stop();
                self.preview_started = None;
                self.software.clock = None;
            }
            return;
        }
    }

    fn export_diagnostics(&mut self) {
        let report = crate::events::DiagnosticReport {
            schema: "echo.software.diagnostics.v1",
            version: env!("CARGO_PKG_VERSION"),
            renderer: self.graphics.renderer.clone(),
            backend: self.graphics.backend.clone(),
            adapter: self.graphics.adapter.clone(),
            actual_mode: self.window.get_actual_mode().to_string(),
            model_bytes: self.model_bytes,
            outgoing_bytes: self.software.outgoing_bytes,
            side_bytes: self.software_side_bytes(),
            cached_thumbnail_bytes: self.images.bytes,
            queued_bytes: self.hub.data_bytes(),
            software_frame_bytes: crate::graphics::software_frame_bytes(),
            scale_factor: self.window.window().scale_factor(),
            high_contrast: self.environment.high_contrast,
            system_animations: self.environment.animations,
            on_battery: self.environment.on_battery,
        };
        self.report("Exporting redacted diagnostics…", false);
        self.send(Work::Diagnostics(report));
    }
}
