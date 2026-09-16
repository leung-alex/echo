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
        let monitor = Monitor::start(
            enabled,
            Arc::new(move |update| events.post(Event::InputIndicator(update))),
        )?;
        Ok(Self {
            monitor: Some(monitor),
            window: crate::InputIndicatorWindow::new().map_err(|e| e.to_string())?,
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
            self.window.set_reveal(false);
            let _ = self.window.hide();
            self.visible = false;
        }
    }
    pub fn sync(
        &mut self,
        enabled: bool,
        suppressed: bool,
        theme: crate::EchoTheme<'_>,
        environment: shell::UiEnvironment,
    ) {
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
            self.hide();
            return;
        };
        self.window.set_fill(if environment.high_contrast {
            theme.get_surface()
        } else {
            theme.get_selection()
        });
        self.window.set_ink(theme.get_text());
        self.window.set_contrast(environment.high_contrast);
        self.window.set_animations(environment.animations);
        self.window.set_mode(
            match sample.mode {
                InputMode::Chinese => "中",
                InputMode::English => "EN",
                InputMode::Unknown => "",
            }
            .into(),
        );
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
