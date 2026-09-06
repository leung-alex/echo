//! Renderer selection happens once, after single-instance admission and settings bootstrap.
use crate::events::{Event, Hub};
use echo_engine::GraphicsMode;
use std::sync::Arc;
#[derive(Clone, Debug)]
pub struct GraphicsInfo {
    pub renderer: String,
    pub adapter: String,
    pub backend: String,
    pub fallback: Option<String>,
    pub perspective: bool,
    pub integrated: bool,
}
fn software(reason: Option<String>) -> Result<GraphicsInfo, String> {
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .select()
        .map_err(|e| e.to_string())?;
    Ok(GraphicsInfo {
        renderer: "software".into(),
        adapter: "CPU rasterizer".into(),
        backend: "Winit".into(),
        fallback: reason,
        perspective: false,
        integrated: false,
    })
}
pub fn select(mode: GraphicsMode, hub: Arc<Hub>) -> Result<GraphicsInfo, String> {
    let forced = std::env::var("ECHO_RENDERER").ok();
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
        if !matches!(value, "femtovg-wgpu" | "software") {
            return Err(format!(
                "Unsupported ECHO_RENDERER '{value}'; use software or femtovg-wgpu"
            ));
        }
    }
    #[cfg(feature = "cover-flow")]
    {
        match initialize(hub) {
            Ok((configuration, info)) => {
                slint::BackendSelector::new()
                    .backend_name("winit".into())
                    .renderer_name("femtovg-wgpu".into())
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
        software(Some(
            "This build does not include the WGPU compositor".into(),
        ))
    }
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
    let info = GraphicsInfo {
        renderer: "femtovg-wgpu".into(),
        adapter: details.name,
        backend: format!("{:?}", details.backend),
        fallback: None,
        perspective: true,
        integrated: details.device_type == wgpu::DeviceType::IntegratedGpu,
    };
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
