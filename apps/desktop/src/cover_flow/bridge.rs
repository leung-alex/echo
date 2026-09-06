//! UI-thread bridge. The rendering notifier only submits prebuilt GPU scenes.
use super::budget::RasterPolicy;
use super::compositor::{Compositor, PanelDraw};
use crate::{
    events::{Command, Event, Hub},
    AppWindow,
};
use slint::{ComponentHandle, GraphicsAPI, RenderingState};
use std::{cell::RefCell, rc::Rc, sync::Arc, time::Instant};
#[derive(Clone, Default, PartialEq)]
struct Scene {
    width: f32,
    height: f32,
    dpi: f32,
    panels: Vec<PanelDraw>,
}
#[derive(Default)]
struct State {
    compositor: Option<Compositor>,
    panel_renderer: Option<super::offscreen::PanelRenderer>,
    scene: Scene,
    dirty: bool,
    captures: u64,
    capture_us: u64,
    perf: FrameMetrics,
    policy: RasterPolicy,
    content_revision: u64,
    present_revision: u64,
}
pub struct FlowBridge {
    state: Rc<RefCell<State>>,
}
impl FlowBridge {
    pub fn install(window: &AppWindow, hub: Arc<Hub>, economical: bool) -> Result<Self, String> {
        let state = Rc::new(RefCell::new(State {
            policy: RasterPolicy::for_device(economical),
            ..Default::default()
        }));
        let render = state.clone();
        window
            .window()
            .set_rendering_notifier(move |phase, api| {
                let mut state = render.borrow_mut();
                state.perf.phase(&phase);
                match phase {
                    RenderingState::RenderingSetup => {
                        if let GraphicsAPI::WGPU29 {
                            instance,
                            device,
                            queue,
                            ..
                        } = api
                        {
                            state.compositor = Some(Compositor::new(device.clone(), queue.clone()));
                            match super::offscreen::PanelRenderer::new(
                                instance.clone(),
                                device.clone(),
                                queue.clone(),
                            ) {
                                Ok(renderer) => state.panel_renderer = Some(renderer),
                                Err(error) => hub.post(Event::GraphicsError(error)),
                            }
                            hub.post(Event::Command(Command::Prewarm));
                        }
                    }
                    RenderingState::BeforeRendering if state.dirty => {
                        let scene = state.scene.clone();
                        state.dirty = false;
                        if let Some(compositor) = state.compositor.as_mut() {
                            if let Err(error) =
                                compositor.draw(scene.width, scene.height, scene.dpi, &scene.panels)
                            {
                                hub.post(Event::GraphicsError(error));
                            }
                        }
                    }
                    RenderingState::RenderingTeardown => {
                        state.panel_renderer = None;
                        state.compositor = None;
                        state.dirty = false;
                    }
                    _ => {}
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { state })
    }
    pub fn effects(&self, reflections: bool) {
        let mut s = self.state.borrow_mut();
        let enabled = reflections && s.policy == RasterPolicy::Standard;
        if let Some(c) = s.compositor.as_mut() {
            if c.reflections != enabled {
                c.reflections = enabled;
                s.content_revision = s.content_revision.wrapping_add(1);
            }
        }
    }
    pub fn policy(&self) -> RasterPolicy {
        self.state.borrow().policy
    }
    pub fn set_economical(&self, economical: bool) -> bool {
        let policy = RasterPolicy::for_device(economical);
        if self.state.borrow().policy == policy {
            return false;
        }
        self.clear();
        self.state.borrow_mut().policy = policy;
        true
    }
    pub fn ready(&self) -> bool {
        let s = self.state.borrow();
        s.compositor.is_some() && s.panel_renderer.is_some()
    }
    pub fn contains(&self, id: i64) -> bool {
        self.state
            .borrow()
            .compositor
            .as_ref()
            .is_some_and(|c| c.contains(id))
    }
    pub fn retain(&self, ids: &[i64]) {
        if let Some(c) = self.state.borrow_mut().compositor.as_mut() {
            c.retain(ids);
        }
    }
    pub fn clear(&self) {
        let mut s = self.state.borrow_mut();
        if let Some(c) = s.compositor.as_mut() {
            c.clear();
        }
        if let Some(r) = s.panel_renderer.as_ref() {
            r.clear();
        }
        s.dirty = false;
        s.scene = Scene::default();
        s.content_revision = s.content_revision.wrapping_add(1);
    }
    pub fn stats(&self) -> (u64, usize, u64, u64) {
        self.state
            .borrow()
            .compositor
            .as_ref()
            .map(|c| (c.bytes(), c.panel_count(), c.frames, c.uploads))
            .unwrap_or_default()
    }
    pub fn present(
        &self,
        width: f32,
        height: f32,
        dpi: f32,
        panels: Vec<PanelDraw>,
    ) -> Result<(slint::Image, bool), String> {
        let mut state = self.state.borrow_mut();
        let dimensions = panels
            .first()
            .map(|p| [p.width, p.height])
            .unwrap_or([1.0, 1.0]);
        let maximum = state
            .compositor
            .as_ref()
            .ok_or("GPU is not ready")?
            .max_dimension();
        let raster = state
            .policy
            .scale([width, height], dimensions, dpi, maximum)?;
        let image = state.compositor.as_mut().unwrap().resize(
            (width * raster).round() as u32,
            (height * raster).round() as u32,
        )?;
        let scene = Scene {
            width,
            height,
            dpi: raster,
            panels,
        };
        let changed = scene != state.scene || state.present_revision != state.content_revision;
        state.scene = scene;
        state.dirty |= changed;
        state.present_revision = state.content_revision;
        Ok((image, changed))
    }
    /// GPU-only panel rendering. This function never calls take_snapshot/map/poll.
    pub fn capture_panel(
        &self,
        window: &AppWindow,
        id: i64,
        downsample: bool,
    ) -> Result<(), String> {
        let start = Instant::now();
        let mut state = self.state.borrow_mut();
        let maximum = state
            .compositor
            .as_ref()
            .ok_or("GPU is not ready")?
            .max_dimension();
        let mut dpi = state.policy.panel_scale(
            [window.get_stage_width(), window.get_stage_height()],
            [window.get_panel_width(), window.get_panel_height()],
            window.window().scale_factor(),
            maximum,
        )?;
        if downsample {
            dpi = dpi.min(1.0);
        }
        let (width, height) = (
            (window.get_panel_width() * dpi).round() as u32,
            (window.get_panel_height() * dpi).round() as u32,
        );
        let texture = state
            .compositor
            .as_mut()
            .ok_or("GPU is not ready")?
            .panel_target(id, width, height)?;
        state
            .panel_renderer
            .as_ref()
            .ok_or("Panel renderer is not ready")?
            .render(window, &texture, dpi)?;
        state.content_revision = state.content_revision.wrapping_add(1);
        state.captures += 1;
        let us = start.elapsed().as_micros() as u64;
        state.capture_us = state.capture_us.saturating_add(us);
        if state.perf.capture_us.len() < 512 {
            state.perf.capture_us.push(us);
        }
        Ok(())
    }
}
#[derive(Default)]
struct FrameMetrics {
    active: bool,
    ending: bool,
    input: Option<Instant>,
    frame: Option<Instant>,
    last: Option<Instant>,
    cpu_us: Vec<u64>,
    gaps_us: Vec<u64>,
    first_us: Vec<u64>,
    capture_us: Vec<u64>,
}
impl FrameMetrics {
    fn phase(&mut self, phase: &RenderingState) {
        if !self.active {
            return;
        }
        match phase {
            RenderingState::BeforeRendering => {
                let now = Instant::now();
                if let Some(start) = self.input.take() {
                    if self.first_us.len() < 256 {
                        self.first_us
                            .push(now.duration_since(start).as_micros() as u64);
                    }
                }
                if let Some(last) = self.last {
                    if self.gaps_us.len() < 4096 {
                        self.gaps_us
                            .push(now.duration_since(last).as_micros() as u64);
                    }
                }
                self.last = Some(now);
                self.frame = Some(now);
            }
            RenderingState::AfterRendering => {
                if let Some(start) = self.frame.take() {
                    if self.cpu_us.len() < 4096 {
                        self.cpu_us.push(start.elapsed().as_micros() as u64);
                    }
                }
                if self.ending {
                    self.active = false;
                    self.ending = false;
                }
            }
            _ => {}
        }
    }
}
impl FlowBridge {
    pub fn begin_transition(&self) {
        let mut s = self.state.borrow_mut();
        s.perf.active = true;
        s.perf.ending = false;
        s.perf.input = Some(Instant::now());
        s.perf.last = None;
    }
    pub fn end_transition(&self) {
        self.state.borrow_mut().perf.ending = true;
    }
    pub fn shutdown(&self) {
        // Destroy the offscreen Slint tree while thread-local font/backend state is alive.
        let resources = {
            let mut state = self.state.borrow_mut();
            state.dirty = false;
            state.scene = Scene::default();
            (state.panel_renderer.take(), state.compositor.take())
        };
        drop(resources);
    }
    #[cfg(feature = "native-test")]
    pub fn reset_metrics(&self) {
        self.state.borrow_mut().perf = FrameMetrics::default();
    }
    #[cfg(feature = "native-test")]
    pub fn metrics(&self) -> serde_json::Value {
        let s = self.state.borrow();
        serde_json::json!({
        "cpu_frame_us":s.perf.cpu_us,"frame_interval_us":s.perf.gaps_us,"input_to_first_render_us":s.perf.first_us,
        "capture_us":s.perf.capture_us,"readbacks":0,"capture_total_us":s.capture_us,
        "raster_policy":s.policy.label(),"motion_texture_limit":s.policy.texture_limit(),"motion_scale_cap":s.policy.maximum_scale(),
        "output_raster_scale":s.scene.dpi,"panel_textures":s.compositor.as_ref().map(|c|c.panel_dimensions()),
        "lifetime_captures":s.captures,"stats":s.compositor.as_ref().map(|c|(c.bytes(),c.panel_count(),c.frames,c.uploads))})
    }
}
