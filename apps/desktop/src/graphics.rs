//! Renderer selection happens once, after single-instance admission and settings bootstrap.
use crate::events::Hub;
use std::sync::Arc;
thread_local! { static FRAME_BEGIN: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) }; }
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
}
fn software(reason: Option<String>) -> Result<GraphicsInfo, String> {
    let renderer = "software";
    if renderer == "software" {
        i_slint_backend_winit::echo_software::install_before_frame(std::rc::Rc::new(|| {
            FRAME_BEGIN.with(|start| start.set(Some(std::time::Instant::now())));
            crate::app::before_software_frame();
        }));
        i_slint_backend_winit::echo_software::install(std::rc::Rc::new(
            |hwnd, width, height, draw| {
                let started = std::time::Instant::now();
                let update_us = FRAME_BEGIN.with(|start| {
                    start
                        .get()
                        .map(|start| start.elapsed().as_micros())
                        .unwrap_or_default()
                });
                let commit = CARD_COMMITS.with(|commits| {
                    commits
                        .borrow()
                        .as_ref()
                        .map(|(generation, hub)| (generation.get(), hub.clone()))
                });
                let mut draw_us = 0;
                let mut measured_draw = |pixels: &mut [u32], fresh: bool| {
                    let start = std::time::Instant::now();
                    let changed = draw(pixels, fresh);
                    draw_us += start.elapsed().as_micros();
                    changed
                };
                let outcome = match SOFTWARE_FRAME.with(|frame| {
                    frame
                        .borrow_mut()
                        .render(hwnd, width, height, &mut measured_draw)
                }) {
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
                        serde_json::json!({"render_present_us":started.elapsed().as_micros(),"draw_us":draw_us,"update_us":update_us,"width":width,"height":height}),
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
    })
}
pub fn select() -> Result<GraphicsInfo, String> {
    let forced = std::env::var("ECHO_RENDERER").ok();
    validate_renderer(forced.as_deref())?;
    software(forced.map(|_| "Software renderer selected by ECHO_RENDERER".into()))
}
fn validate_renderer(value: Option<&str>) -> Result<(), String> {
    match value {
        None | Some("software") => Ok(()),
        Some(value) => Err(format!(
            "Unsupported ECHO_RENDERER '{value}'; GPU/Skia diagnostics are retired; use software"
        )),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_software_is_available() {
        assert!(validate_renderer(None).is_ok());
        assert!(validate_renderer(Some("software")).is_ok());
        for value in ["femtovg-wgpu", "skia-wgpu", "skia-software", "unknown", ""] {
            assert!(validate_renderer(Some(value))
                .unwrap_err()
                .contains("use software"));
        }
    }
}
