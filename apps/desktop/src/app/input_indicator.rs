use super::*;
use echo_windows::input_indicator::{Monitor, Update};

pub(super) struct Indicator {
    monitor: Option<Monitor>,
    window: crate::InputIndicatorWindow,
    sample: Option<InputStatus>,
    visible: bool,
    hwnd: Option<isize>,
    position: Option<PhysicalRect>,
    timer: Timer,
    hub: Arc<Hub>,
}
impl Indicator {
    pub fn new(enabled: bool, hub: Arc<Hub>) -> Result<Self, String> {
        let events = hub.clone();
        let last_observation = std::sync::Mutex::new(None);
        let monitor = Monitor::start(
            enabled,
            Arc::new(move |update| {
                let key = (
                    update.generation,
                    update
                        .sample
                        .map(|s| (s.mode, s.window, s.focused_window, s.process)),
                );
                let mut last = last_observation.lock().unwrap_or_else(|e| e.into_inner());
                if last.as_ref() != Some(&key) {
                    crate::indicator_trace::record(
                        "observation-changed",
                        serde_json::json!({
                        "generation":update.generation,"sample":update.sample.map(|s| serde_json::json!({
                            "mode":format!("{:?}",s.mode),"window":s.window,"focused_window":s.focused_window,"process":s.process,
                            "age_ms":s.sampled_at.elapsed().as_millis()}))}),
                    );
                    *last = Some(key);
                }
                events.post(Event::InputIndicator(update));
            }),
        )?;
        let window = crate::InputIndicatorWindow::new().map_err(|e| e.to_string())?;
        let weak = window.as_weak();
        window.on_trace(move |event, target, displayed, phase| {
            if let Some(window) = weak.upgrade() {
                crate::indicator_trace::record(event.as_str(), serde_json::json!({
                    "target":target.as_str(),"displayed":displayed.as_str(),"phase":format!("{phase:?}"),
                    "reveal":window.get_reveal(),"animations":window.get_animations()}));
            }
        });
        Ok(Self {
            monitor: Some(monitor),
            window,
            sample: None,
            visible: false,
            hwnd: None,
            position: None,
            timer: Timer::default(),
            hub,
        })
    }
    pub fn update(&mut self, update: Update) {
        if self
            .monitor
            .as_ref()
            .is_none_or(|m| m.generation() != update.generation)
        {
            crate::indicator_trace::record(
                "discard-stale-update",
                serde_json::json!({"generation":update.generation}),
            );
            return;
        }
        self.sample = update.sample;
        self.timer.stop();
        if self.sample.is_some() {
            let hub = self.hub.clone();
            self.timer.start(
                TimerMode::SingleShot,
                Duration::from_millis(251),
                move || hub.post(Event::InputIndicatorExpired),
            );
        }
    }
    pub fn hide(&mut self) {
        if self.visible {
            crate::indicator_trace::record("window-hide", serde_json::Value::Null);
            self.window.set_reveal(false);
            let _ = self.window.hide();
            self.visible = false;
        }
    }
    pub fn sync(&mut self, enabled: bool, suppressed: bool, environment: shell::UiEnvironment) {
        let Some(monitor) = self.monitor.as_ref() else {
            return;
        };
        monitor.set_enabled(enabled && !suppressed);
        let sample = self.sample.filter(|s| {
            enabled
                && echo_presentation::input_indicator::visible(
                    s,
                    monitor.generation(),
                    suppressed,
                    Instant::now(),
                )
                && echo_windows::input_indicator::foreground_matches(s)
        });
        let Some((sample, rect)) =
            sample.and_then(|s| echo_presentation::input_indicator::place(&s).map(|p| (s, p)))
        else {
            if self.visible {
                crate::indicator_trace::record(
                    "hide-reason",
                    serde_json::json!({
                    "enabled":enabled,"suppressed":suppressed,"generation":monitor.generation(),
                    "sample":self.sample.map(|s| serde_json::json!({"generation":s.generation,
                        "mode":format!("{:?}",s.mode),"age_ms":s.sampled_at.elapsed().as_millis(),
                        "foreground_matches":echo_windows::input_indicator::foreground_matches(&s),
                        "placement_valid":echo_presentation::input_indicator::place(&s).is_some()}))}),
                );
            }
            self.hide();
            return;
        };
        let mode_changed = self.window.get_mode().as_str()
            != match sample.mode {
                InputMode::Chinese => "中",
                InputMode::English => "EN",
                InputMode::Unknown => "",
            };
        if mode_changed {
            crate::indicator_trace::record(
                "mode-delivered",
                serde_json::json!({
                "generation":sample.generation,"old":self.window.get_mode().as_str(),
                "new":format!("{:?}",sample.mode),"visible":self.visible,"age_ms":sample.sampled_at.elapsed().as_millis()}),
            );
        }
        self.window.set_animations(environment.animations);
        self.window.set_mode(
            match sample.mode {
                InputMode::Chinese => "中",
                InputMode::English => "EN",
                InputMode::Unknown => "",
            }
            .into(),
        );
        if mode_changed {
            // Mode change handlers must not wait for the next observation tick
            // to wake this passive window after a hide/show cycle.
            self.window.window().request_redraw();
        }
        if self.position != Some(rect) {
            self.window
                .window()
                .set_position(slint::PhysicalPosition::new(rect.x, rect.y));
            self.window.window().set_size(slint::PhysicalSize::new(
                rect.width as u32,
                rect.height as u32,
            ));
            self.position = Some(rect);
        }
        if !self.visible {
            if let Err(e) = self.show() {
                eprintln!("Input indicator unavailable: {e}");
                let _ = self.window.hide();
                self.visible = false;
            }
        }
    }
    fn show(&mut self) -> Result<(), String> {
        self.window.set_reveal(false);
        self.window.show().map_err(|e| e.to_string())?;
        if self.hwnd.is_none() {
            let handle = self.window.window().window_handle();
            let RawWindowHandle::Win32(handle) =
                handle.window_handle().map_err(|e| e.to_string())?.as_raw()
            else {
                return Err("Windows badge required".into());
            };
            let hwnd = handle.hwnd.get();
            let hub = self.hub.clone();
            shell::configure_input_badge(
                hwnd,
                Arc::new(move |event| hub.post(Event::Shell(event))),
            )?;
            crate::graphics::register_input_badge(hwnd);
            self.hwnd = Some(hwnd);
        }
        crate::indicator_trace::record("window-show", serde_json::Value::Null);
        self.visible = true;
        self.window.set_reveal(true);
        Ok(())
    }
    pub fn stop(&mut self) {
        self.timer.stop();
        self.hide();
        self.monitor.take();
        crate::graphics::release_input_badge();
    }
    #[cfg(feature = "native-test")]
    pub fn snapshot(&self) -> Result<slint::SharedPixelBuffer<slint::Rgba8Pixel>, String> {
        self.window
            .window()
            .take_snapshot()
            .map_err(|e| e.to_string())
    }
    #[cfg(feature = "native-test")]
    pub fn diagnostics(&self) -> serde_json::Value {
        serde_json::json!({"visible":self.visible,"hwnd":self.hwnd,"label":self.window.get_mode().to_string(),
            "generation":self.monitor.as_ref().map(Monitor::generation),
            "counts":self.monitor.as_ref().map(Monitor::observation_counts),
            "sample":self.sample.map(|s| serde_json::json!({"mode":format!("{:?}",s.mode),"composition":format!("{:?}",s.composition),
                "pid":s.process,"window":s.window,"age_ms":s.sampled_at.elapsed().as_millis(),"generation":s.generation})),
            "rect":self.position.map(|p|[p.x,p.y,p.width,p.height])})
    }
}
