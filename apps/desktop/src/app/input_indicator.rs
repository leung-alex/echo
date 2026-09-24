use super::*;
use echo_windows::input_indicator::{Monitor, Observation, Update};

const RETENTION_MS: u64 = 250;

#[derive(Clone, Copy)]
struct Retention {
    started_at: Instant,
}

fn retention_active(retention: Option<Retention>, now: Instant, target_matches: bool) -> bool {
    target_matches
        && retention.is_some_and(|retention| {
            now.saturating_duration_since(retention.started_at)
                <= Duration::from_millis(RETENTION_MS)
        })
}

pub(super) struct Indicator {
    monitor: Option<Monitor>,
    window: crate::InputIndicatorWindow,
    sample: Option<InputStatus>,
    process_name: Option<String>,
    retention: Option<Retention>,
    pending_hide_reason: Option<&'static str>,
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
                let (key, details) = match &update.observation {
                    Observation::Revalidating { trigger } => (
                        format!("{}:revalidating:{}", update.generation, trigger.as_str()),
                        serde_json::json!({
                            "generation": update.generation,
                            "state": "revalidating",
                            "trigger": trigger.as_str()
                        }),
                    ),
                    Observation::Observed {
                        sample,
                        process_name,
                        trigger,
                        elapsed_ms,
                    } => (
                        format!(
                            "{}:observed:{:?}:{}:{}:{}:{}:{:?}:{}:{}:{}:{}:{}",
                            update.generation,
                            sample.mode,
                            sample.window,
                            sample.focused_window,
                            sample.process,
                            trigger.as_str(),
                            sample.geometry_stamp.source,
                            sample.geometry_stamp.context_epoch,
                            sample.geometry.target.x,
                            sample.geometry.target.y,
                            sample.geometry.target.width,
                            sample.geometry.target.height,
                        ),
                        serde_json::json!({
                            "generation": update.generation,
                            "state": "observed",
                            "process_name": process_name,
                            "trigger": trigger.as_str(),
                            "probe_stage": "input-state",
                            "elapsed_ms": elapsed_ms,
                            "sample": {
                                "mode": format!("{:?}", sample.mode),
                                "window": sample.window,
                                "focused_window": sample.focused_window,
                                "process": sample.process,
                                "age_ms": sample.sampled_at.elapsed().as_millis()
                                ,"geometry": {
                                    "source": format!("{:?}", sample.geometry_stamp.source),
                                    "confidence": format!("{:?}", sample.geometry_stamp.confidence),
                                    "sequence": sample.geometry_stamp.sequence,
                                    "context_epoch": sample.geometry_stamp.context_epoch,
                                    "observed_age_ms": sample.geometry_stamp.observed_at.elapsed().as_millis(),
                                    "rect": [sample.geometry.target.x, sample.geometry.target.y, sample.geometry.target.width, sample.geometry.target.height]
                                }
                            }
                        }),
                    ),
                    Observation::Unavailable {
                        reason,
                        process_name,
                        trigger,
                        elapsed_ms,
                    } => (
                        format!(
                            "{}:unavailable:{}:{}",
                            update.generation,
                            reason.as_str(),
                            trigger.as_str()
                        ),
                        serde_json::json!({
                            "generation": update.generation,
                            "state": "unavailable",
                            "reason": reason.as_str(),
                            "process_name": process_name,
                            "trigger": trigger.as_str(),
                            "probe_stage": "input-state",
                            "elapsed_ms": elapsed_ms
                        }),
                    ),
                };
                let mut last = last_observation.lock().unwrap_or_else(|e| e.into_inner());
                if last.as_ref() != Some(&key) {
                    crate::indicator_trace::record("observation-changed", details);
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
            process_name: None,
            retention: None,
            pending_hide_reason: None,
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
        self.timer.stop();
        match update.observation {
            Observation::Revalidating { trigger } => {
                let target_matches = self.sample.is_some_and(|sample| {
                    echo_windows::input_indicator::foreground_matches(&sample)
                });
                if self.sample.is_some() && target_matches {
                    if self.retention.is_none() {
                        self.retention = Some(Retention {
                            started_at: Instant::now(),
                        });
                        crate::indicator_trace::record(
                            "retention-start",
                            serde_json::json!({
                                "generation": update.generation,
                                "trigger": trigger.as_str(),
                                "process_name": self.process_name
                            }),
                        );
                    }
                    let elapsed = self
                        .retention
                        .map_or(Duration::ZERO, |retention| retention.started_at.elapsed());
                    let remaining = Duration::from_millis(RETENTION_MS + 1).saturating_sub(elapsed);
                    self.start_expiry_timer(remaining);
                } else {
                    if self.retention.take().is_some() || self.sample.is_some() {
                        crate::indicator_trace::record(
                            "retention-cancelled-target-changed",
                            serde_json::json!({
                                "generation": update.generation,
                                "trigger": trigger.as_str(),
                                "process_name": self.process_name
                            }),
                        );
                    }
                    self.sample = None;
                    self.process_name = None;
                    self.pending_hide_reason = Some("target-changed");
                }
            }
            Observation::Observed {
                sample,
                process_name,
                trigger: _,
                elapsed_ms,
            } => {
                if self.retention.take().is_some() {
                    crate::indicator_trace::record(
                        "retention-replaced",
                        serde_json::json!({
                            "generation": update.generation,
                            "process_name": process_name,
                            "elapsed_ms": elapsed_ms
                        }),
                    );
                }
                self.sample = Some(sample);
                self.process_name = process_name;
                self.pending_hide_reason = None;
                self.start_expiry_timer(Duration::from_millis(251));
            }
            Observation::Unavailable {
                reason,
                process_name,
                trigger: _,
                elapsed_ms: _,
            } => {
                self.retention = None;
                self.sample = None;
                self.process_name = process_name;
                self.pending_hide_reason = Some(reason.as_str());
            }
        }
    }
    fn start_expiry_timer(&mut self, duration: Duration) {
        let hub = self.hub.clone();
        self.timer.start(TimerMode::SingleShot, duration, move || {
            hub.post(Event::InputIndicatorExpired)
        });
    }
    fn hide(&mut self, reason: &str) {
        if self.visible {
            crate::indicator_trace::record(
                "window-hide",
                serde_json::json!({"hide_reason":reason,"process_name":self.process_name}),
            );
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
        if !enabled || suppressed {
            self.retention = None;
            self.sample = None;
            self.process_name = None;
            self.pending_hide_reason = None;
            self.hide(if suppressed { "suppressed" } else { "disabled" });
            return;
        }

        let now = Instant::now();
        let target_matches = self
            .sample
            .is_some_and(|sample| echo_windows::input_indicator::foreground_matches(&sample));
        let retaining = retention_active(self.retention, now, target_matches);
        if self.retention.is_some() && !retaining {
            let reason = if target_matches {
                "retention-expired"
            } else {
                "retention-cancelled-target-changed"
            };
            crate::indicator_trace::record(
                reason,
                serde_json::json!({
                    "generation": monitor.generation(),
                    "process_name": self.process_name
                }),
            );
            self.retention = None;
            self.sample = None;
            self.pending_hide_reason = Some(reason);
        }

        let sample = self.sample.filter(|sample| {
            target_matches
                && if retaining {
                    sample.mode != InputMode::Unknown
                } else {
                    echo_presentation::input_indicator::visible(
                        sample,
                        monitor.generation(),
                        false,
                        now,
                    )
                }
        });
        let Some((sample, rect)) = sample.and_then(|sample| {
            echo_presentation::input_indicator::place(&sample).map(|p| (sample, p))
        }) else {
            let hide_reason = self.pending_hide_reason.take().unwrap_or_else(|| {
                if !target_matches && self.sample.is_some() {
                    "target-changed"
                } else if self.sample.is_some_and(|sample| {
                    echo_presentation::input_indicator::place(&sample).is_none()
                }) {
                    "placement-invalid"
                } else if self
                    .sample
                    .is_some_and(|sample| sample.mode == InputMode::Unknown)
                {
                    "mode-unavailable"
                } else if self.retention.is_none() && self.sample.is_some() {
                    "sample-stale"
                } else {
                    "observation-unavailable"
                }
            });
            self.hide(hide_reason);
            return;
        };
        let mode_changed = self.window.get_mode().as_str()
            != match sample.mode {
                InputMode::Chinese => "\u{4e2d}",
                InputMode::English => "EN",
                InputMode::Unknown => "",
            };
        if mode_changed {
            crate::indicator_trace::record(
                "mode-delivered",
                serde_json::json!({
                "generation":sample.generation,"old":self.window.get_mode().as_str(),
                "new":format!("{:?}",sample.mode),"visible":self.visible,"age_ms":sample.sampled_at.elapsed().as_millis(),
                "retained":retaining,"process_name":self.process_name}),
            );
        }
        self.window.set_animations(environment.animations);
        self.window.set_mode(
            match sample.mode {
                InputMode::Chinese => "\u{4e2d}",
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
            crate::indicator_trace::record(
                "geometry-applied",
                serde_json::json!({
                    "generation": sample.generation,
                    "source": format!("{:?}", sample.geometry_stamp.source),
                    "sequence": sample.geometry_stamp.sequence,
                    "context_epoch": sample.geometry_stamp.context_epoch,
                    "badge_rect": [rect.x, rect.y, rect.width, rect.height]
                }),
            );
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
        crate::indicator_trace::record(
            "window-show",
            serde_json::json!({"process_name":self.process_name}),
        );
        self.visible = true;
        self.window.set_reveal(true);
        Ok(())
    }
    pub fn stop(&mut self) {
        self.timer.stop();
        self.retention = None;
        self.sample = None;
        self.pending_hide_reason = None;
        self.hide("stopped");
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
            "process_name":self.process_name,
            "retaining":self.retention.is_some(),
            "sample":self.sample.map(|s| serde_json::json!({"mode":format!("{:?}",s.mode),"composition":format!("{:?}",s.composition),
                "pid":s.process,"window":s.window,"age_ms":s.sampled_at.elapsed().as_millis(),"generation":s.generation})),
            "rect":self.position.map(|p|[p.x,p.y,p.width,p.height])})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retention_is_bounded_and_requires_the_same_target() {
        let start = Instant::now();
        let retention = Some(Retention { started_at: start });
        assert!(retention_active(
            retention,
            start + Duration::from_millis(250),
            true
        ));
        assert!(!retention_active(
            retention,
            start + Duration::from_millis(251),
            true
        ));
        assert!(!retention_active(retention, start, false));
    }

    #[test]
    fn repeated_revalidation_keeps_the_original_deadline() {
        let start = Instant::now();
        let mut retention = Some(Retention { started_at: start });
        retention.get_or_insert(Retention {
            started_at: start + Duration::from_millis(200),
        });
        assert_eq!(retention.unwrap().started_at, start);
        assert!(!retention_active(
            retention,
            start + Duration::from_millis(251),
            true
        ));
    }
}
