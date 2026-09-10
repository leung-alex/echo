//! Test-only bridge: input is dispatched to this Slint window, never SendInput.
//! Captures read this window's renderer, never desktop pixels or other HWNDs.
//! This module is absent from normal release/distribution builds.
use super::*;
use serde::Deserialize;
use std::path::{Path, PathBuf};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    id: String,
    pid: u32,
    verb: String,
    #[serde(default)]
    key: u32,
    #[serde(default)]
    ctrl: bool,
    #[serde(default)]
    shift: bool,
    #[serde(default)]
    paused: bool,
    #[serde(default)]
    file: String,
}
pub(crate) struct Controller {
    _timer: Timer,
}
impl Controller {
    pub fn start(app: &Rc<RefCell<App>>) -> Result<Option<Self>, String> {
        let Some(root) = std::env::var_os("ECHO_NATIVE_TEST_ROOT") else {
            return Ok(None);
        };
        if std::env::var("ECHO_WINDOWS_ACCEPTANCE").as_deref() != Ok("1") {
            return Err("Native test bridge requires explicit acceptance authorization".into());
        }
        let root = PathBuf::from(root)
            .canonicalize()
            .map_err(|e| e.to_string())?;
        let data =
            PathBuf::from(std::env::var_os("ECHO_DATA_DIR").ok_or("Missing isolated data path")?)
                .canonicalize()
                .map_err(|e| e.to_string())?;
        if data
            != root
                .join("data")
                .canonicalize()
                .map_err(|e| e.to_string())?
            || !data.starts_with(&root)
        {
            return Err("Native test data must be inside the evidence root".into());
        }
        let marker: serde_json::Value = serde_json::from_slice(
            &std::fs::read(data.join("synthetic-fixture.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        if marker["synthetic"] != true
            || marker["capture_enabled"] != false
            || app.borrow().settings.history_enabled
        {
            return Err(
                "Native test bridge only accepts capture-disabled synthetic fixtures".into(),
            );
        }
        let control = root.join("native-control");
        std::fs::create_dir_all(&control).map_err(|e| e.to_string())?;
        let request = control.join("request.json");
        let mut last = std::fs::read(&request)
            .ok()
            .and_then(|v| serde_json::from_slice::<Request>(&v).ok())
            .map(|r| r.id)
            .unwrap_or_default();
        let weak = Rc::downgrade(app);
        let mut pending_response: Option<Vec<u8>> = None;
        let timer = Timer::default();
        timer.start(TimerMode::Repeated, Duration::from_millis(20), move || {
            if let Some(app) = weak.upgrade() {
                trace_tick(&app);
            }
            if let Some(bytes) = pending_response.as_ref() {
                if publish_response(&control, bytes) {
                    pending_response = None;
                } else {
                    return;
                }
            }
            let Ok(meta) = std::fs::metadata(&request) else {
                return;
            };
            if meta.len() > 16 * 1024 {
                return;
            }
            let Ok(bytes) = std::fs::read(&request) else {
                return;
            };
            let Ok(request) = serde_json::from_slice::<Request>(&bytes) else {
                return;
            };
            if request.id == last || request.id.is_empty() || request.id.len() > 80 {
                return;
            }
            last = request.id.clone();
            let result = weak
                .upgrade()
                .ok_or_else(|| "Test application is closed".into())
                .and_then(|app| execute(&app, &root, &request));
            let response = match result {
                Ok(value) => serde_json::json!({"id":request.id,"status":"PASS","value":value}),
                Err(error) => serde_json::json!({"id":request.id,"status":"FAIL","error":error}),
            };
            let response = serde_json::to_vec(&response).unwrap_or_default();
            if !publish_response(&control, &response) {
                // A Windows reader can temporarily deny replacement. Retry
                // publication on the next tick, never execute the request twice.
                pending_response = Some(response);
            }
        });
        Ok(Some(Self { _timer: timer }))
    }
}
fn publish_response(control: &Path, response: &[u8]) -> bool {
    let temporary = control.join("response.pending");
    std::fs::write(&temporary, response).is_ok()
        && std::fs::rename(&temporary, control.join("response.json")).is_ok()
}

#[cfg(test)]
mod bridge_tests {
    use super::*;
    #[test]
    fn response_replacement_preserves_last_good_data_while_reader_holds_it() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/test-tmp/echo-desktop")
            .join(format!(
                "bridge-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
        std::fs::create_dir_all(&root).unwrap();
        assert!(publish_response(&root, b"old"));
        let reader = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(root.join("response.json"))
            .unwrap();
        assert!(!publish_response(&root, b"new"));
        assert_eq!(std::fs::read(root.join("response.json")).unwrap(), b"old");
        drop(reader);
        assert!(publish_response(&root, b"new"));
        assert_eq!(std::fs::read(root.join("response.json")).unwrap(), b"new");
        std::fs::remove_file(root.join("response.json")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}

fn execute(
    app: &Rc<RefCell<App>>,
    root: &Path,
    request: &Request,
) -> Result<serde_json::Value, String> {
    if request.pid != std::process::id() {
        return Err("Test command targets a different process".into());
    }
    let window = app
        .borrow()
        .window
        .as_weak()
        .upgrade()
        .ok_or("Window is unavailable")?;
    match request.verb.as_str() {
        "ping" => Ok(serde_json::json!({"native_test":true,"pid":std::process::id()})),
        "query" => {
            window.set_query(request.file.clone().into());
            app.borrow_mut()
                .command(Command::Query(request.file.clone()));
            Ok(serde_json::json!({"query_requested":true}))
        }
        "pause_inline_window_events" => {
            echo_windows::inline::diagnostics::pause_window_events(request.paused)?;
            Ok(
                serde_json::json!({"window_events_paused":request.paused,"keyboard_hook_unchanged":true}),
            )
        }
        "search_fault" => {
            crate::service::native_faults::configure(request.key, request.shift, request.ctrl)?;
            Ok(crate::service::native_faults::metrics())
        }
        "provider_fault" => {
            echo_windows::inline::diagnostics::configure_provider_fault(
                request.key,
                request.paused,
                if request.ctrl {
                    3
                } else if request.shift {
                    2
                } else if request.file == "refuse-selection" {
                    1
                } else {
                    0
                },
            )?;
            Ok(echo_windows::inline::diagnostics::provider_fault_metrics())
        }
        "thumbnail_fault" => {
            crate::service::native_faults::configure_thumbnail(&app.borrow().hub, request.paused)?;
            Ok(crate::service::native_faults::metrics())
        }
        "key" => {
            if app.borrow().session.context != Context::Manager {
                return Err(
                    "Test input is restricted to manager mode; clipboard insertion is disabled"
                        .into(),
                );
            }
            use slint::platform::{Key, WindowEvent};
            let key: slint::SharedString = match request.key {
                9 => Key::Tab.into(),
                13 => Key::Return.into(),
                27 => Key::Escape.into(),
                33 => Key::PageUp.into(),
                34 => Key::PageDown.into(),
                35 => Key::End.into(),
                36 => Key::Home.into(),
                37 => Key::LeftArrow.into(),
                38 => Key::UpArrow.into(),
                39 => Key::RightArrow.into(),
                40 => Key::DownArrow.into(),
                117 => Key::F6.into(),
                70 if request.ctrl => "f".into(),
                78 if request.ctrl => "n".into(),
                _ => return Err("Key is not allowed by the no-clipboard native test bridge".into()),
            };
            // These are framework events delivered to the owned window, not OS keyboard events.
            if request.ctrl {
                window.window().dispatch_event(WindowEvent::KeyPressed {
                    text: Key::Control.into(),
                });
            }
            if request.shift {
                window.window().dispatch_event(WindowEvent::KeyPressed {
                    text: Key::Shift.into(),
                });
            }
            window
                .window()
                .dispatch_event(WindowEvent::KeyPressed { text: key.clone() });
            window
                .window()
                .dispatch_event(WindowEvent::KeyReleased { text: key });
            if request.shift {
                window.window().dispatch_event(WindowEvent::KeyReleased {
                    text: Key::Shift.into(),
                });
            }
            if request.ctrl {
                window.window().dispatch_event(WindowEvent::KeyReleased {
                    text: Key::Control.into(),
                });
            }
            Ok(serde_json::json!({"owned_window_input":true,"global_input":false}))
        }
        "scroll" => {
            if request.key == 0
                || request.key > 4096
                || !app.borrow().surface.visible
                || app.borrow().window.get_modal()
            {
                return Err(
                    "Scroll requires a visible non-modal owned window and a bounded distance"
                        .into(),
                );
            }
            use slint::platform::WindowEvent;
            let position = if window.get_route().as_str() == "settings" {
                slint::LogicalPosition::new(
                    window.get_settings_scroll_x(),
                    window.get_settings_scroll_y(),
                )
            } else {
                slint::LogicalPosition::new(
                    window.get_panel_left() + window.get_panel_width() / 2.0,
                    window.get_panel_top() + window.get_panel_height() / 2.0,
                )
            };
            window
                .window()
                .dispatch_event(WindowEvent::PointerScrolled {
                    position,
                    delta_x: 0.0,
                    delta_y: request.key as f32 * if request.shift { 1.0 } else { -1.0 },
                });
            Ok(serde_json::json!({"owned_window_scroll":true,"global_input":false}))
        }
        "trace_begin" => {
            if request.file.is_empty()
                || request.file.len() > 64
                || !request
                    .file
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            {
                return Err("Trace requires an evidence-directory basename".into());
            }
            let directory = root.join(&request.file);
            std::fs::create_dir(&directory).map_err(|e| e.to_string())?;
            TRACE.with(|t| {
                *t.borrow_mut() = Some(FrameTrace {
                    directory,
                    started: Instant::now(),
                    frames: Vec::new(),
                })
            });
            Ok(serde_json::json!({"started":true,"source":"owned Slint renderer only"}))
        }
        "trace_end" => finish_trace(),
        "capture" => {
            if request.file.is_empty()
                || request.file.len() > 120
                || !request.file.ends_with(".png")
                || request.file.contains(['/', '\\', ':'])
                || request.file.contains("..")
            {
                return Err("Capture name must be a PNG basename inside the evidence root".into());
            }
            let path = root.join(&request.file);
            if path.exists() {
                return Err("Evidence already exists".into());
            }
            let snapshot = window.window().take_snapshot().map_err(|e| e.to_string())?;
            image::save_buffer_with_format(
                &path,
                snapshot.as_bytes(),
                snapshot.width(),
                snapshot.height(),
                image::ColorType::Rgba8,
                image::ImageFormat::Png,
            )
            .map_err(|e| e.to_string())?;
            Ok(
                serde_json::json!({"width":snapshot.width(),"height":snapshot.height(),"source":"owned Slint Window::take_snapshot","desktop_pixels":false}),
            )
        }
        "metrics" => {
            let a = app.borrow();
            #[cfg(feature = "cover-flow")]
            let graphics = a.flow.as_ref().map(|f| f.metrics());
            #[cfg(not(feature = "cover-flow"))]
            let graphics: Option<serde_json::Value> = None;
            let mut metrics = serde_json::json!({"space":a.surface.space.0,"phase":format!("{:?}",a.deck.phase),
                "ready":a.surface.ready,"loading":a.surface.loading,"visible":a.surface.visible,
                "spaces":a.spaces.iter().map(|s|serde_json::json!({"id":s.id.0,"title":s.title,"icon":s.icon_key,"accent":s.accent_key,"count":s.item_count})).collect::<Vec<_>>(),
                "renderer":a.graphics.renderer,"adapter":a.graphics.adapter,"backend":a.graphics.backend,"actual":a.window.get_actual_mode().as_str(),
                "graphics":graphics,"navigation_us":a.navigation_us,"snapshot_model_count":a.model.row_count(),"highlighted_rows":a.model.iter().filter(|r|r.match_count>0).count(),"match_spans":a.model.iter().map(|r|r.match_count).collect::<Vec<_>>(),"scroll_y":a.window.get_scroll_y(),"query":a.surface.query,"route":a.window.get_route().to_string(),
                "requested":a.deck.requested.to_string(),"presented":a.deck.presented.to_string(),
                "interaction":a.deck.interaction.map(|id|id.to_string()),
                "inline_safety":a.worker.inline.safety_status(),
                "search_faults":crate::service::native_faults::metrics(),
                "selection":a.surface.selection.map(|key|key.to_string()),
                "target_capabilities":a.worker.inline.capabilities().map(|c| serde_json::json!({"backend":c.backend,"can_read_query":c.can_read_query,"can_observe_selection":c.can_observe_selection,"advertises_exact_selection":c.advertises_exact_selection,"has_text_edit_pattern":c.has_text_edit_pattern,"exact_selection_verified":c.exact_selection_verified})),
                "inline_trace":a.worker.inline.diagnostics().iter().map(|e| serde_json::json!({"us":e.elapsed_us,"kind":e.kind,"detail":e.detail,"session":e.session,"input_serial":e.input_serial,"observed_serial":e.observed_serial})).collect::<Vec<_>>(),
                "inline":{"active":a.inline_ui.ticket.is_some(),"popup":a.inline_active(),"unavailable":a.inline_ui.unavailable,"pending":a.inline_ui.pending,"composing":a.inline_ui.composing,"suspended":a.inline_ui.suspended,"provider":a.inline_ui.backend,"readiness":a.worker.inline.readiness(),"ticket":a.inline_ui.ticket.map(|t|[t.session,t.revision,t.input_serial]),"natural_height":a.window.get_inline_content_height(),"status":a.surface.status},
                "quick_insert":{"active":a.session.context == Context::QuickInsert,"has_target":a.session.has_target,"capture_pending":a.capture_pending,"anchor_source":a.popup_anchor.map(|anchor|anchor.source.label()),"hotkey_status":a.window.get_hotkey_status().to_string()},
                "settings":{"dirty":a.window.get_settings_dirty(),"valid":a.window.get_settings_valid(),"error":a.window.get_settings_error().to_string(),"ui":a.ui},
                "flow_timer":a.flow_timer.running(),"preview_timer":a.preview_timer.running(),
                "thumbnails_bytes":a.images.bytes+a.software.outgoing_image_bytes,"native_region":a.window_shapes.as_ref().is_some_and(|s|s.is_some()),
                "panel":[a.window.get_panel_left(),a.window.get_panel_top(),a.window.get_panel_width(),a.window.get_panel_height()],
                "stage":[a.window.get_stage_width(),a.window.get_stage_height()],"scale_factor":a.window.window().scale_factor()});
            metrics["error"] = a.surface.error.into();
            metrics["display_bytes"] = serde_json::json!({"page":a.surface.held_item_bytes(),"model":a.model_bytes,"outgoing":a.software.outgoing_bytes,"sides":a.software_side_bytes(),"queued":a.hub.data_bytes(),"total":a.surface.held_item_bytes()+a.model_bytes+a.software_side_bytes()+a.software.outgoing_bytes+a.hub.data_bytes()});
            metrics["software_slide"] = serde_json::json!({"outgoing":a.window.get_outgoing_present(),"outgoing_rows":a.window.get_outgoing().rows.row_count(),"moving":a.software.slide.moving(),"loading":a.software.slide.loading(),"pending_target":a.software.slide.intent().map(|id|id.0),"incoming_x":a.window.get_incoming_x(),"outgoing_x":a.window.get_outgoing_x(),"left_visible":a.window.get_left_side_visible(),"right_visible":a.window.get_right_side_visible(),"frame_bytes":crate::graphics::software_frame_bytes()});
            metrics["status"] = a.surface.status.clone().into();
            metrics["software_slide"]["side_width"] = a.window.get_side_width().into();
            metrics["software_slide"]["progress"] = a.window.get_carousel_progress().into();
            metrics["software_slide"]["direction"] = a.window.get_carousel_direction().into();
            metrics["query_epoch"] = a.surface.query_epoch().into();
            metrics["side_previews"] = [
                (a.window.get_left_space(), a.window.get_left_preview(), a.window.get_left_side_visible()),
                (a.window.get_right_space(), a.window.get_right_preview(), a.window.get_right_side_visible()),
            ].into_iter().map(|(space, preview, visible)| {
                let id = a.spaces.iter().find(|s| s.id.to_string() == space.key.as_str()).map(|s| s.id);
                let source = id.and_then(|id| a.previews.get(&id));
                serde_json::json!({"space":space.key.as_str(),"visible":visible,"ready":id.is_some_and(|id|a.has_current_preview(id)),"query":source.map(|p|p.query.as_str()),"loading":preview.loading,"rows":preview.rows.row_count(),"titles":preview.rows.iter().map(|r|r.title.to_string()).collect::<Vec<_>>(),"bodies":preview.rows.iter().map(|r|r.body.to_string()).collect::<Vec<_>>()})
            }).collect::<Vec<_>>().into();
            metrics["provider_faults"] =
                echo_windows::inline::diagnostics::provider_fault_metrics();
            Ok(metrics)
        }
        "reset_metrics" => {
            let mut a = app.borrow_mut();
            a.navigation_us.clear();
            #[cfg(feature = "cover-flow")]
            if let Some(flow) = &a.flow {
                flow.reset_metrics();
            }
            Ok(serde_json::Value::Null)
        }
        "step" => {
            if app.borrow().session.context != Context::Manager {
                return Err("Only manager-mode tests may navigate".into());
            }
            window.invoke_navigate_space(if request.shift { -1 } else { 1 });
            Ok(serde_json::Value::Null)
        }
        _ => Err("Unknown native test operation".into()),
    }
}

struct FrameTrace {
    directory: PathBuf,
    started: Instant,
    frames: Vec<serde_json::Value>,
}
thread_local! { static TRACE: RefCell<Option<FrameTrace>> = const { RefCell::new(None) }; }
fn trace_tick(app: &Rc<RefCell<App>>) {
    TRACE.with(|slot| {
        let mut slot = slot.borrow_mut();
        let Some(trace) = slot.as_mut() else { return; };
        // Bounded diagnostics, not a benchmark or a production rendering path.
        if trace.frames.len() >= 180 || trace.started.elapsed() > Duration::from_secs(12) { return; }
        let a = app.borrow();
        let i = trace.frames.len();
        let mut frame = serde_json::json!({"ms":trace.started.elapsed().as_millis(),
            "visible":a.surface.visible,"rows":a.model.row_count(),"stale":a.window.get_stale_rows(),
            "loading":a.surface.loading,"flow_enabled":a.window.get_flow_enabled(),
            "navigation_busy":a.window.get_navigation_busy(),"query_units":a.surface.query.chars().count(),
            "height":a.window.get_panel_height(),"width":a.window.get_panel_width(),"selected_rows":a.model.iter().filter(|r|r.selected).count(),"highlighted_rows":a.model.iter().filter(|r|r.match_count>0).count(),"busy":a.window.get_busy()});
        frame["query_epoch"] = a.surface.query_epoch().into();
        frame["selection"] = serde_json::json!(a.surface.selection.map(|key|key.to_string()));
        frame["row_keys"] = serde_json::json!(a.model.iter().map(|row|row.key.to_string()).collect::<Vec<_>>());
        frame["panel_origin"] = serde_json::json!([a.window.get_panel_left(),a.window.get_panel_top()]);
        frame["stage_size"] = serde_json::json!([a.window.get_stage_width(),a.window.get_stage_height()]);
        frame["error"] = a.surface.error.into();
        frame["thumbnails_bytes"] = (a.images.bytes+a.software.outgoing_image_bytes).into();
        if i % 2 == 0 && a.surface.visible {
            match a.window.window().take_snapshot() {
                Ok(image) => {
                    let name = format!("frame-{i:03}.png");
                    if let Err(error) = image::save_buffer_with_format(trace.directory.join(&name),image.as_bytes(),image.width(),image.height(),image::ColorType::Rgba8,image::ImageFormat::Png) {
                        frame["capture_error"] = error.to_string().into();
                    } else { frame["png"] = name.into(); }
                }
                Err(error) => frame["capture_error"] = error.to_string().into(),
            }
        }
        trace.frames.push(frame);
    });
}
fn finish_trace() -> Result<serde_json::Value, String> {
    let trace = TRACE
        .with(|t| t.borrow_mut().take())
        .ok_or("No active frame trace")?;
    std::fs::write(
        trace.directory.join("frames.json"),
        serde_json::to_vec_pretty(&trace.frames).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    Ok(
        serde_json::json!({"frames":trace.frames.len(),"directory":trace.directory.file_name().unwrap().to_string_lossy()}),
    )
}
