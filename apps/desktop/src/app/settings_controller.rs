//! Draft settings, runtime effect policy and redacted diagnostics.
use super::*;
impl App {
    pub(super) fn render_startup_choices(&self) {
        let current = self.window.get_startup_space();
        let selected = StartupSpace::parse(current.as_str()).unwrap_or_default();
        let index = match selected {
            StartupSpace::Last => self.spaces.len(),
            _ => {
                let id = selected.resolve(None, self.spaces.iter().map(|space| space.id));
                self.spaces
                    .iter()
                    .position(|space| space.id == id)
                    .unwrap_or(0)
            }
        };
        let mut names: Vec<slint::SharedString> = self
            .spaces
            .iter()
            .map(|space| {
                if space.id.is_system() {
                    crate::i18n::text(self.active_language.unwrap_or_default(), &space.title).into()
                } else {
                    space.title.clone().into()
                }
            })
            .collect();
        names.push(
            crate::i18n::text(self.active_language.unwrap_or_default(), "Last used space").into(),
        );
        self.window
            .set_startup_space_names(ModelRc::new(VecModel::from(names)));
        self.window.set_startup_space_index(index as i32);
    }
    pub(super) fn read_ui_draft(&self) -> Result<UiSettings, String> {
        let w = &self.window;
        let mut value = self.ui.clone();
        macro_rules! enum_field {
            ($field:ident,$getter:ident,$type:ident) => {
                value.$field = $type::parse(w.$getter().as_str())
                    .ok_or_else(|| format!("Invalid {} setting", stringify!($field)))?;
            };
        }
        enum_field!(language, get_language, Language);
        enum_field!(startup_space, get_startup_space, StartupSpace);
        enum_field!(query_on_switch, get_query_on_switch, QueryOnSwitch);
        enum_field!(side_content, get_side_content, SideContent);
        enum_field!(graphics, get_graphics_mode, GraphicsMode);
        enum_field!(frame_rate, get_frame_rate, FrameRate);
        value.global_hotkey_enabled = w.get_global_hotkey_enabled();
        value.global_hotkey = GlobalShortcut::parse(w.get_global_hotkey().as_str())?.canonical();
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
        let clipboard = formatting::settings(w.get_theme_mode().as_str())?;
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
        w.set_theme_mode(c.theme.as_str().into());
        w.set_language(u.language.as_str().into());
        w.set_global_hotkey_enabled(u.global_hotkey_enabled);
        w.set_global_hotkey(u.global_hotkey.clone().into());
        w.set_startup_space(u.startup_space.key().into());
        self.render_startup_choices();
        w.set_query_on_switch(u.query_on_switch.as_str().into());
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
        let hotkey_status = w.get_hotkey_status();
        w.set_settings_error(
            if formatting::hotkey_status_is_informational(&hotkey_status) {
                "".into()
            } else {
                hotkey_status
            },
        );
        w.set_settings_notice("".into());
        w.set_restart_required(false);
    }
    pub(super) fn settings_edited(&mut self) {
        if self.window.get_route().as_str() != "settings" {
            return;
        }
        self.render_startup_choices();
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

    pub(super) fn apply_theme(&mut self) {
        let _timing = crate::popup_timing::span("theme_update");
        let language = if self.window.get_route().as_str() == "settings" {
            Language::parse(&self.window.get_language()).unwrap_or(self.ui.language)
        } else {
            self.ui.language
        };
        if self.active_language != Some(language) {
            if let Err(error) = slint::select_bundled_translation(language.as_str()) {
                self.report(format!("Operation failed: {error}"), true);
                return;
            }
            self.window
                .global::<crate::I18n>()
                .set_language(language.as_str().into());
            self.tray.set_labels(crate::i18n::tray_labels(language));
            self.active_language = Some(language);
            self.render_navigation();
        }
        let theme = if self.window.get_route().as_str() == "settings" {
            ThemeMode::parse(self.window.get_theme_mode().as_str()).unwrap_or(self.settings.theme)
        } else {
            self.settings.theme
        };
        let dark = match theme {
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
        };
        let changed = self.window.get_dark() != dark;
        self.window.set_dark(dark);
        self.apply_styles();
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
        self.window
            .set_actual_mode("Actual: software rendering · card carousel".into());
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
