//! Native Slint shell for Echo. Engine, persistence and clipboard formats are unchanged.
slint::include_modules!();
#[cfg(windows)]
mod app;
#[cfg(windows)]
mod events;
mod formatting;
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
    // Secondary invocations returned before any renderer or database initialization.
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name(std::env::var("ECHO_RENDERER").unwrap_or_else(|_| "software".into()))
        .select()
        .map_err(|e| e.to_string())?;
    let worker = service::Worker::start(data_dir, hub.clone())?;
    let application = app::App::new(hub.clone(), worker, args)?;
    app::install(application.clone());
    hub.activate();
    let result = slint::run_event_loop_until_quit().map_err(|e| e.to_string());
    application.borrow_mut().shutdown();
    app::uninstall();
    drop(application);
    drop(shell);
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
