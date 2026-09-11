//! Native Slint shell for Echo. Engine, persistence and clipboard formats are unchanged.
#[cfg(windows)]
#[global_allocator]
static ALLOCATOR: echo_windows::allocation::AccountedSystem =
    echo_windows::allocation::AccountedSystem;
mod favorite_icons;
mod match_highlight;
mod memory_lifecycle;
mod memory_trace;
mod native_model;
mod popup_timing;
slint::include_modules!();
#[cfg(windows)]
mod app;
#[cfg(windows)]
mod events;
mod formatting;
#[cfg(windows)]
mod graphics;
#[cfg(windows)]
mod service;

pub fn run() -> Result<(), String> {
    #[cfg(windows)]
    {
        run_windows()
    }
    #[cfg(not(windows))]
    {
        Err("Echo's clipboard runtime requires Windows".into())
    }
}
#[cfg(windows)]
fn run_windows() -> Result<(), String> {
    use echo_windows::shell::{Instance, NativeShell};
    use events::{Event, Hub};
    use std::sync::Arc;
    #[cfg(feature = "native-test")]
    if std::env::var("ECHO_DEBUG_EXIT_BACKTRACE").as_deref() == Ok("1") {
        // Preload the Windows symbolizer before any loader-lock/TLS teardown callback.
        let _ = std::backtrace::Backtrace::force_capture().to_string();
    }
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args == ["--help"] || args == ["-h"] {
        println!("Echo native clipboard history\n--background  --history  --favorites  --settings  --quit\n--echo-activate <base64url-envelope>");
        return Ok(());
    }
    if args == ["--version"] {
        println!("Echo {} (Slint)", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    validate_args(&args)?;
    let data_dir = std::env::var_os("ECHO_DATA_DIR")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("LOCALAPPDATA").map(|p| std::path::PathBuf::from(p).join("Echo"))
        })
        .ok_or("LOCALAPPDATA is unavailable; specify ECHO_DATA_DIR")?;
    let hub = Arc::new(Hub::default());
    let callback_hub = hub.clone();
    let shell = match NativeShell::start(
        &data_dir,
        &args,
        Arc::new(move |e| callback_hub.post(Event::Shell(e))),
    )? {
        Instance::Forwarded => return Ok(()),
        Instance::Primary(shell) => shell,
    };
    if args == ["--quit"] {
        drop(shell);
        return Ok(());
    }
    // Secondary invocations return before the single storage owner or GPU is started.
    let worker = service::Worker::start(data_dir, hub.clone(), shell.hotkeys())?;
    memory_trace::record(
        "core_ready",
        serde_json::json!({"background":args == ["--background"]}),
    );
    if args == ["--background"] && !hub.wait_for_activation() {
        hub.close();
        drop(worker);
        drop(shell);
        memory_trace::record("stopped", serde_json::Value::Null);
        return Ok(());
    }
    memory_trace::record("ui_starting", serde_json::Value::Null);
    let graphics = graphics::select()?;
    memory_trace::record(
        "graphics_selected",
        serde_json::json!({
            "renderer":graphics.renderer, "adapter":graphics.adapter, "backend":graphics.backend,
            "perspective":false, "fallback":graphics.fallback
        }),
    );
    let application = app::App::new(hub.clone(), worker, args, graphics)?;
    app::install(application.clone());
    hub.activate();
    #[cfg(feature = "native-test")]
    let _native_test = app::native_test::Controller::start(&application)?;
    let result = slint::run_event_loop_until_quit().map_err(|e| e.to_string());
    let restart = application.borrow_mut().take_restart();
    application.borrow_mut().shutdown();
    app::uninstall();
    drop(application);
    drop(shell);
    memory_trace::record("stopped", serde_json::Value::Null);
    if restart {
        restart_application()?;
    }
    result
}
#[cfg(windows)]
fn validate_args(args: &[String]) -> Result<(), String> {
    match args {
        [] => Ok(()),
        [flag]
            if matches!(
                flag.as_str(),
                "--background" | "--history" | "--favorites" | "--settings" | "--quit"
            ) =>
        {
            Ok(())
        }
        [flag, value] if flag == echo_activation::ACTIVATION_FLAG && value.len() <= 60 * 1024 => {
            echo_activation::decode(value)
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
        _ => Err("Invalid Echo arguments; use --help".into()),
    }
}

// Regression-test the exact patched accessibility mapping in ordinary CI.
#[cfg(test)]
#[path = "../../../vendor/i-slint-backend-winit/echo_accessibility_value.rs"]
mod accessibility_value_regression;

#[cfg(windows)]
fn restart_application() -> Result<(), String> {
    // Only restart this executable, after all old window/storage/instance owners were dropped.
    // Never preserve an activation envelope or replay an insertion operation.
    let executable = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut command = std::process::Command::new(executable);
    command.arg("--history");
    command
        .spawn()
        .map_err(|e| format!("Could not restart Echo: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod row_click_tests;
