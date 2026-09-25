//! Content-free caret provider probe. It never edits text, clipboard or focus.

#[cfg(windows)]
fn main() {
    use std::{
        fs::OpenOptions,
        io::Write,
        sync::{mpsc, Arc},
        time::{Duration, Instant},
    };

    let args: Vec<String> = std::env::args().collect();
    let seconds = args
        .windows(2)
        .find(|pair| pair[0] == "--seconds")
        .and_then(|pair| pair[1].parse::<u64>().ok())
        .unwrap_or(30)
        .clamp(1, 300);
    let provider = args
        .windows(2)
        .find(|pair| pair[0] == "--provider")
        .map(|pair| pair[1].to_ascii_lowercase())
        .unwrap_or_else(|| "shadow".into());
    if !matches!(provider.as_str(), "shadow" | "primary" | "legacy") {
        eprintln!("--provider must be shadow, primary or legacy");
        std::process::exit(2);
    }
    let Some(output) = args
        .windows(2)
        .find(|pair| pair[0] == "--output")
        .map(|pair| std::path::PathBuf::from(&pair[1]))
    else {
        eprintln!("--output is required");
        std::process::exit(2);
    };
    let mut file = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output)
    {
        Ok(file) => file,
        Err(error) => {
            eprintln!("cannot create new output: {error}");
            std::process::exit(3);
        }
    };
    std::env::set_var("ECHO_CARET_PROVIDER", &provider);
    let (sender, receiver) = mpsc::channel();
    let monitor = match echo_windows::input_indicator::Monitor::start(
        true,
        Arc::new(move |update| {
            let _ = sender.send(update);
        }),
    ) {
        Ok(monitor) => monitor,
        Err(error) => {
            eprintln!("monitor unavailable: {error}");
            std::process::exit(4);
        }
    };
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(seconds) {
        let Ok(update) = receiver.recv_timeout(Duration::from_millis(250)) else {
            continue;
        };
        let tsf = update.tsf.map(|diagnostic| {
            serde_json::json!({
                "state":format!("{:?}", diagnostic.state),
                "api_hresult":diagnostic.api_hresult,
                "session_hresult":diagnostic.session_hresult,
                "reason_code":diagnostic.reason_code,
                "sequence":diagnostic.sequence,
                "context_epoch":diagnostic.context_epoch,
                "raw_rect":diagnostic.raw_rect,
                "normalized_rect":diagnostic.normalized_rect,
                "age_ms":diagnostic.age_ms,
                "pending_callbacks":diagnostic.pending_callbacks,
                "outstanding_callbacks":diagnostic.outstanding_callbacks,
                "created_callbacks":diagnostic.created_callbacks,
                "released_callbacks":diagnostic.released_callbacks,
                "final_source":diagnostic.final_source.map(|source| format!("{:?}", source)),
                "fallback_reason":diagnostic.fallback_reason,
            })
        });
        let line = match &update.observation {
            echo_windows::input_indicator::Observation::Observed { sample, .. } => {
                serde_json::json!({
                    "event":"observed",
                    "elapsed_ms":started.elapsed().as_millis(),
                    "generation":update.generation,
                    "mode":format!("{:?}", sample.mode),
                    "source":format!("{:?}", sample.geometry_stamp.source),
                    "confidence":format!("{:?}", sample.geometry_stamp.confidence),
                    "geometry_sequence":sample.geometry_stamp.sequence,
                    "context_epoch":sample.geometry_stamp.context_epoch,
                    "rect":[sample.geometry.target.x,sample.geometry.target.y,sample.geometry.target.width,sample.geometry.target.height],
                    "tsf":tsf
                })
            }
            echo_windows::input_indicator::Observation::Revalidating { trigger } => {
                serde_json::json!({"event":"revalidating","elapsed_ms":started.elapsed().as_millis(),"generation":update.generation,"trigger":trigger.as_str(),"tsf":tsf})
            }
            echo_windows::input_indicator::Observation::Unavailable {
                reason, trigger, ..
            } => {
                serde_json::json!({"event":"unavailable","elapsed_ms":started.elapsed().as_millis(),"generation":update.generation,"reason":reason.as_str(),"trigger":trigger.as_str(),"tsf":tsf})
            }
        };
        let _ = writeln!(file, "{line}");
        let _ = file.flush();
    }
    drop(monitor);
}

#[cfg(not(windows))]
fn main() {}
