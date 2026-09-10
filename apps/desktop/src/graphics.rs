//! Renderer selection happens once, after single-instance admission and settings bootstrap.
#[cfg(feature = "cover-flow")]
use crate::events::Event;
use crate::events::Hub;
use echo_engine::GraphicsMode;
use std::sync::Arc;
thread_local! { static SOFTWARE_FRAME: std::cell::RefCell<echo_windows::shell::SoftwareFrame> = Default::default(); }
thread_local! { static EXPECTED_FRAME: std::cell::RefCell<Option<(echo_presentation::slide::ContentFrame, Arc<Hub>)>> = const { std::cell::RefCell::new(None) }; }
thread_local! { static CARD_COMMITS: std::cell::RefCell<Option<(std::rc::Rc<std::cell::Cell<u64>>, Arc<Hub>)>> = const { std::cell::RefCell::new(None) }; }
pub fn software_card_commits(generation: std::rc::Rc<std::cell::Cell<u64>>, hub: Arc<Hub>) {
    CARD_COMMITS.with(|commits| *commits.borrow_mut() = Some((generation, hub)));
}
pub fn expect_software_frame(stamp: echo_presentation::slide::ContentFrame, hub: Arc<Hub>) {
    EXPECTED_FRAME.with(|expected| *expected.borrow_mut() = Some((stamp, hub)));
    SOFTWARE_FRAME.with(|frame| frame.borrow_mut().invalidate());
}
pub fn cancel_software_frame() {
    EXPECTED_FRAME.with(|expected| *expected.borrow_mut() = None);
}
pub fn invalidate_software_frame() {
    SOFTWARE_FRAME.with(|frame| frame.borrow_mut().invalidate());
}
pub fn release_software_frame() {
    SOFTWARE_FRAME.with(|frame| frame.borrow_mut().release());
}
pub fn software_frame_bytes() -> usize {
    SOFTWARE_FRAME.with(|frame| frame.borrow().bytes())
}
#[derive(Clone, Debug)]
pub struct GraphicsInfo {
    pub renderer: String,
    pub adapter: String,
    pub backend: String,
    pub fallback: Option<String>,
    pub perspective: bool,
    #[cfg_attr(not(feature = "cover-flow"), allow(dead_code))]
    pub integrated: bool,
}
fn software(reason: Option<String>) -> Result<GraphicsInfo, String> {
    let renderer = if cfg!(feature = "memory-diagnostics-skia")
        && std::env::var("ECHO_RENDERER").as_deref() == Ok("skia-software")
    {
        "skia-software"
    } else {
        "software"
    };
    if renderer == "software" {
        i_slint_backend_winit::echo_software::install_before_frame(std::rc::Rc::new(
            crate::app::before_software_frame,
        ));
        i_slint_backend_winit::echo_software::install(std::rc::Rc::new(
            |hwnd, width, height, draw| {
                let started = std::time::Instant::now();
                let commit = CARD_COMMITS.with(|commits| {
                    commits
                        .borrow()
                        .as_ref()
                        .map(|(generation, hub)| (generation.get(), hub.clone()))
                });
                let outcome = match SOFTWARE_FRAME
                    .with(|frame| frame.borrow_mut().render(hwnd, width, height, draw))
                {
                    Ok(outcome) => outcome,
                    Err(error) => {
                        cancel_software_frame();
                        return Err(error);
                    }
                };
                use echo_windows::shell::FrameOutcome;
                if outcome == FrameOutcome::Hidden {
                    return Ok(false);
                }
                if outcome == FrameOutcome::Unchanged {
                    return Ok(true);
                }
                {
                    crate::memory_trace::record(
                        "frame_presented",
                        serde_json::json!({"render_present_us":started.elapsed().as_micros(),"width":width,"height":height}),
                    );
                }
                if let Some((stamp, hub)) =
                    EXPECTED_FRAME.with(|expected| expected.borrow_mut().take())
                {
                    hub.post(crate::events::Event::Command(
                        crate::events::Command::SoftwareFrameReady(stamp),
                    ));
                }
                if let Some((generation, hub)) = commit {
                    if generation != 0 {
                        hub.post(crate::events::Event::Command(
                            crate::events::Command::CommitCardRegion(generation),
                        ));
                    }
                }
                Ok(true)
            },
        ));
    }
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name(renderer.into())
        .select()
        .map_err(|e| e.to_string())?;
    Ok(GraphicsInfo {
        renderer: renderer.into(),
        adapter: "CPU rasterizer".into(),
        backend: "Winit".into(),
        fallback: reason,
        perspective: false,
        integrated: false,
    })
}
pub fn select(mode: GraphicsMode, hub: Arc<Hub>) -> Result<GraphicsInfo, String> {
    let forced = std::env::var("ECHO_RENDERER").ok();
    #[cfg(feature = "memory-diagnostics-skia")]
    if forced.as_deref() == Some("skia-software") {
        return Err("The preserved Skia comparison requires the pre-software-deck baseline; this build supports the Slint software presenter".into());
    }
    if forced.as_deref() == Some("software") {
        return software(Some("Software renderer selected by ECHO_RENDERER".into()));
    }
    if forced.is_none() && mode == GraphicsMode::Software {
        return software(None);
    }
    if std::env::var("ECHO_WINDOWS_ACCEPTANCE").as_deref() == Ok("1")
        && std::env::var("ECHO_ACCEPTANCE_FORCE_GRAPHICS_FALLBACK").as_deref() == Ok("1")
    {
        return software(Some("Acceptance test: hardware adapter unavailable".into()));
    }
    if let Some(value) = forced.as_deref() {
        if !matches!(value, "femtovg-wgpu" | "software")
            && !(cfg!(feature = "memory-diagnostics-skia") && value == "skia-wgpu")
        {
            return Err(format!(
                "Unsupported ECHO_RENDERER '{value}'; use software or femtovg-wgpu"
            ));
        }
    }
    #[cfg(feature = "cover-flow")]
    {
        match initialize(hub) {
            Ok((configuration, mut info)) => {
                if cfg!(feature = "memory-diagnostics-skia")
                    && forced.as_deref() == Some("skia-wgpu")
                {
                    info.renderer = "skia-wgpu".into();
                }
                slint::BackendSelector::new()
                    .backend_name("winit".into())
                    .renderer_name(info.renderer.clone())
                    .require_wgpu_29(configuration)
                    .with_winit_window_attributes_hook(|attributes| {
                        use slint::winit_030::winit::platform::windows::WindowAttributesExtWindows;
                        attributes.with_no_redirection_bitmap(true)
                    })
                    .select()
                    .map_err(|e| e.to_string())?;
                Ok(info)
            }
            Err(reason) => software(Some(format!("GPU initialization failed: {reason}"))),
        }
    }
    #[cfg(not(feature = "cover-flow"))]
    {
        let _ = hub;
        software(None)
    }
}
/// Diagnostic until native restoration and memory results justify promotion.
pub fn destroy_graphics_on_reclaim() -> bool {
    cfg!(feature = "memory-diagnostics")
        && std::env::var("ECHO_MEMORY_DESTROY_GRAPHICS").as_deref() == Ok("1")
}
#[cfg(feature = "cover-flow")]
fn ready<F: std::future::Future>(future: F) -> Result<F::Output, String> {
    use std::task::{Context, Poll, Waker};
    let mut future = std::pin::pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => Ok(value),
        Poll::Pending => {
            Err("Native graphics initialization did not complete synchronously".into())
        }
    }
}
#[cfg(feature = "cover-flow")]
fn initialize(hub: Arc<Hub>) -> Result<(slint::wgpu_29::WGPUConfiguration, GraphicsInfo), String> {
    use slint::wgpu_29::{wgpu, WGPUConfiguration};
    let mut backend_options = wgpu::BackendOptions::default();
    backend_options.dx12.presentation_system = wgpu::Dx12SwapchainKind::DxgiFromVisual;
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::DX12,
        flags: wgpu::InstanceFlags::from_build_config(),
        backend_options,
        memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
        display: None,
    });
    let adapter = ready(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::LowPower,
        force_fallback_adapter: false,
        compatible_surface: None,
    }))?
    .map_err(|e| e.to_string())?;
    let details = adapter.get_info();
    if details.device_type == wgpu::DeviceType::Cpu {
        return Err("Only a CPU-backed WGPU adapter is available".into());
    }
    let info = GraphicsInfo {
        renderer: "femtovg-wgpu".into(),
        adapter: details.name,
        backend: format!("{:?}", details.backend),
        fallback: None,
        perspective: true,
        integrated: details.device_type == wgpu::DeviceType::IntegratedGpu,
    };
    if destroy_graphics_on_reclaim() {
        // Probe adapter availability without creating a disposable extra device.
        // The persistent backend keeps settings; its renderer owns the live device.
        let mut settings = slint::wgpu_29::WGPUSettings::default();
        settings.backends = wgpu::Backends::DX12;
        settings.backend_options.dx12.presentation_system = wgpu::Dx12SwapchainKind::DxgiFromVisual;
        settings.power_preference = wgpu::PowerPreference::LowPower;
        settings.device_memory_hints = wgpu::MemoryHints::MemoryUsage;
        return Ok((WGPUConfiguration::Automatic(settings), info));
    }
    let (device, queue) = ready(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("Echo shared UI and Cover Flow device"),
        required_features: wgpu::Features::empty(),
        required_limits:
            wgpu::Limits::downlevel_webgl2_defaults().using_resolution(adapter.limits()),
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        ..Default::default()
    }))?
    .map_err(|e| e.to_string())?;
    // Slint retains the manual queue in its thread-local backend context. WGPU 29's
    // debug snatch-lock trace must be initialized first so it is destroyed last.
    // This startup-only, nonblocking poll prevents Queue::drop accessing destroyed TLS.
    #[cfg(debug_assertions)]
    device
        .poll(wgpu::PollType::Poll)
        .map_err(|e| e.to_string())?;
    let error_hub = hub.clone();
    device.on_uncaptured_error(Arc::new(move |error: wgpu::Error| {
        error_hub.post(Event::GraphicsError(format!(
            "Graphics resource failure: {error}"
        )));
    }));
    device.set_device_lost_callback(move |reason, message| {
        if !matches!(reason, wgpu::DeviceLostReason::Destroyed) {
            hub.post(Event::GraphicsError(format!(
                "Graphics device lost ({reason:?}): {message}"
            )));
        }
    });
    Ok((
        WGPUConfiguration::Manual {
            instance,
            adapter,
            device,
            queue,
        },
        info,
    ))
}
