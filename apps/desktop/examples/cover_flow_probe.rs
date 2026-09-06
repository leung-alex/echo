//! Synthetic native rendering test. No clipboard or database access.
use echo_desktop::cover_flow::compositor::{Compositor, PanelDraw};
use sha2::{Digest, Sha256};
use slint::{ComponentHandle, GraphicsAPI, RenderingState, Timer, TimerMode};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};
slint::slint! {
    export component Probe inherits Window {
        preferred-width: 1120px; preferred-height: 800px;
        title: "Echo Cover Flow G0 - synthetic data"; background: #181920;
        in property <bool> capture-mode;
        in property <bool> gpu-mode;
        in property <string> panel-title: "History";
        in property <image> stage-image;
        in property <image> fixture-image;
        out property <float> stage-width: root.width / 1px;
        out property <float> stage-height: (root.height - 160px) / 1px;
        out property <float> panel-width: min(740px,root.width - 180px) / 1px;
        out property <bool> search-focused: search.has-focus;
        public function focus-search() { search.focus(); }
        search := TextInput { x: 32px; y: 20px; width: parent.width - 64px; height: 32px;
            text: "Synthetic input - keep focus during capture"; color: #ffffff; }
        Image { y: 80px; width: parent.width; height: parent.height - 160px;
            visible: root.gpu-mode && !root.capture-mode;
            source: root.stage-image; image-fit: fill; }
        Rectangle { x: (parent.width - self.width)/2; y: 80px;
            width: root.panel-width * 1px; height: parent.height - 160px;
            visible: !root.gpu-mode || root.capture-mode;
            background: #fafafc; border-radius: 20px;
            Text { x: 24px; y: 16px; text: root.panel-title;
                font-family: "Segoe UI"; font-size: 24px; font-weight: 600; color: #20242c; }
            for n in 20: Rectangle {
                x: 24px; y: 64px + n * 24px; width: parent.width - 48px; height: 24px;
                Rectangle { y: 23px; height: 1px; background: #d9dce2; }
                Text { text: "Synthetic item " + (n + 1) + " · 中文内容 · email@example.test";
                    color: #30333a; font-size: 12px; }
            }
            Image { x: 24px; y: parent.height - 76px; width: 64px; height: 48px; source: root.fixture-image; }
            Image { x: 100px; y: parent.height - 76px; width: 64px; height: 48px; source: root.fixture-image; }
        }
        Text { x: 32px; y: parent.height - 48px; text: "Synthetic graphics probe - no clipboard access"; color: #c4c7d0; }
    }
}
#[derive(Default)]
struct State {
    compositor: Option<Compositor>,
    poses: Vec<PanelDraw>,
    width: f32,
    height: f32,
    dpi: f32,
    dirty: bool,
}
fn save(
    path: &std::path::Path,
    pixels: &slint::SharedPixelBuffer<slint::Rgba8Pixel>,
) -> Result<(), String> {
    image::save_buffer(
        path,
        pixels.as_bytes(),
        pixels.width(),
        pixels.height(),
        image::ColorType::Rgba8,
    )
    .map_err(|e| e.to_string())
}
fn crop(
    pixels: &slint::SharedPixelBuffer<slint::Rgba8Pixel>,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
) -> Vec<u8> {
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for row in y..y + h {
        let start = ((row * pixels.width() + x) * 4) as usize;
        out.extend_from_slice(&pixels.as_bytes()[start..start + (w * 4) as usize]);
    }
    out
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = PathBuf::from(
        std::env::var_os("ECHO_PROBE_OUTPUT_DIR")
            .ok_or("Set ECHO_PROBE_OUTPUT_DIR to an isolated evidence directory")?,
    );
    std::fs::create_dir_all(&output)?;
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("femtovg-wgpu".into())
        .require_wgpu_29(slint::wgpu_29::WGPUConfiguration::default())
        .select()?;
    let app = Probe::new()?;
    let mut fixture = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(64, 48);
    for (i, p) in fixture.make_mut_slice().iter_mut().enumerate() {
        *p = slint::Rgba8Pixel {
            r: (i % 64 * 4) as u8,
            g: 140,
            b: 220,
            a: 255,
        };
    }
    app.set_fixture_image(slint::Image::from_rgba8(fixture));
    let state = Rc::new(RefCell::new(State::default()));
    let capture_guard = Rc::new(Cell::new(false));
    let render_state = state.clone();
    let guard = capture_guard.clone();
    app.window().set_rendering_notifier(move |phase, api| {
        if guard.get() {
            return;
        }
        let mut state = render_state.borrow_mut();
        if let (RenderingState::RenderingSetup, GraphicsAPI::WGPU29 { device, queue, .. }) =
            (&phase, api)
        {
            state.compositor = Some(Compositor::new(device.clone(), queue.clone()));
        }
        if matches!(phase, RenderingState::BeforeRendering) && state.dirty {
            let (w, h, dpi, poses) = (state.width, state.height, state.dpi, state.poses.clone());
            if let Some(c) = state.compositor.as_mut() {
                c.draw(w, h, dpi, &poses).expect("probe draw");
            }
            state.dirty = false;
        }
    })?;
    app.show()?;
    app.invoke_focus_search();
    let weak = app.as_weak();
    let step = Rc::new(Cell::new(0u32));
    let failed = Rc::new(RefCell::new(None::<String>));
    let errors = failed.clone();
    let hashes = Rc::new(RefCell::new(Vec::<String>::new()));
    let metrics = Rc::new(RefCell::new(Vec::<serde_json::Value>::new()));
    let timer = Timer::default();
    timer.start(TimerMode::Repeated, Duration::from_millis(180), move || {
        let Some(app) = weak.upgrade() else { return; };
        let result = (|| -> Result<(), String> {
            let n = step.get();
            let dpi = app.window().scale_factor();
            let (w,h,pw) = (app.get_stage_width(),app.get_stage_height(),app.get_panel_width());
            if n == 0 {
                if state.borrow().compositor.is_none() { return Err("WGPU setup was not delivered".into()); }
                let focused = app.get_search_focused();
                let before = app.window().take_snapshot().map_err(|e|e.to_string())?;
                save(&output.join("g0-live.png"), &before)?;
                let x = (((w-pw)/2.0)*dpi).round() as u32;
                let y = (80.0*dpi).round() as u32;
                let (px,py) = ((pw*dpi).round() as u32,(h*dpi).round() as u32);
                let original = crop(&before,x,y,px,py);
                drop(before);
                for (id,title) in [(1,"History"),(2,"Favorites")] {
                    let start = Instant::now();
                    capture_guard.set(true); app.set_capture_mode(true); app.set_panel_title(title.into());
                    let snapshot = app.window().take_snapshot().map_err(|e|e.to_string());
                    app.set_panel_title("History".into()); app.set_capture_mode(false); capture_guard.set(false);
                    let snapshot = snapshot?;
                    let snapshot_us = start.elapsed().as_micros();
                    let pixels = crop(&snapshot,x,y,px,py); drop(snapshot);
                    let upload = Instant::now();
                    state.borrow_mut().compositor.as_mut().unwrap().upload(id,px,py,&pixels)?;
                    metrics.borrow_mut().push(serde_json::json!({"panel":id,"snapshot_crop_us":snapshot_us,
                        "upload_submit_us":upload.elapsed().as_micros(),"width_px":px,"height_px":py,"dpi":dpi}));
                }
                let restored = app.window().take_snapshot().map_err(|e|e.to_string())?;
                if crop(&restored,x,y,px,py) != original { return Err("Capture restoration changed the live panel".into()); }
                if focused != app.get_search_focused() { return Err("Capture changed keyboard focus".into()); }
                let output_image = state.borrow_mut().compositor.as_mut().unwrap()
                    .resize((w*dpi).round() as u32,(h*dpi).round() as u32)?;
                app.set_stage_image(output_image); app.set_gpu_mode(true);
            } else {
                let snapshot = app.window().take_snapshot().map_err(|e|e.to_string())?;
                let hash = format!("{:x}",Sha256::digest(snapshot.as_bytes()));
                hashes.borrow_mut().push(hash);
                if n == 1 || n == 6 || n == 11 { save(&output.join(format!("g0-frame-{n:02}.png")),&snapshot)?; }
            }
            if n == 11 {
                let unique = hashes.borrow().iter().cloned().collect::<std::collections::HashSet<_>>().len();
                let s = state.borrow(); let c = s.compositor.as_ref().unwrap();
                let report = serde_json::json!({"status":if unique>=10 {"PASS"}else{"FAIL"},
                    "synthetic":true,"renderer":"femtovg-wgpu","same_device":true,"dpi":dpi,
                    "capture_restore":true,"focus_preserved":true,"unique_frames":unique,
                    "gpu_texture_bytes":c.bytes(),"frames":c.frames,"uploads":c.uploads,
                    "samples":*metrics.borrow(),"physical_ime":"NOT_RUN"});
                std::fs::write(output.join("g0-report.json"),serde_json::to_vec_pretty(&report).unwrap())
                    .map_err(|e|e.to_string())?;
                println!("{report}");
                if unique < 10 { return Err("GPU texture updates were cached or not presented".into()); }
                slint::quit_event_loop().map_err(|e|e.to_string())?;
                return Ok(());
            }
            let angle = (n as f32 * 5.4).to_radians();
            let mut s = state.borrow_mut();
            s.width=w; s.height=h; s.dpi=dpi;
            s.poses=vec![PanelDraw { id:1,width:pw,height:h,x:-(n as f32)*12.0,y:0.0,z:0.0,
                yaw:-angle,scale:1.0,opacity:1.0,shade:0.0 },
                PanelDraw { id:2,width:pw,height:h,x:pw*0.70,y:12.0,z:-pw*0.23,
                    yaw:54.0f32.to_radians(),scale:0.90,opacity:0.96,shade:0.10 }];
            s.dirty=true;
            step.set(n+1); app.window().request_redraw();
            Ok(())
        })();
        if let Err(error) = result {
            eprintln!("G0 FAIL: {error}"); *errors.borrow_mut()=Some(error);
            let _ = slint::quit_event_loop();
        }
    });
    slint::run_event_loop_until_quit()?;
    timer.stop();
    if let Some(error) = failed.borrow_mut().take() {
        return Err(error.into());
    }
    Ok(())
}
