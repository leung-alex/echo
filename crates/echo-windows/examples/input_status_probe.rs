//! Read-only diagnostic for physical input-mode acceptance. No text is captured.
#[cfg(windows)]
fn main() {
    use std::{
        sync::{mpsc, Arc},
        time::{Duration, Instant},
    };
    let seconds = std::env::args()
        .nth(1)
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(15)
        .clamp(1, 300);
    let (sender, receiver) = mpsc::channel();
    let _monitor = echo_windows::input_indicator::Monitor::start(
        true,
        Arc::new(move |update| {
            let _ = sender.send(update);
        }),
    )
    .unwrap();
    let start = Instant::now();
    let mut previous = String::new();
    while start.elapsed() < Duration::from_secs(seconds) {
        let Ok(update) = receiver.recv_timeout(Duration::from_millis(250)) else {
            continue;
        };
        let state = update
            .sample
            .map(|s| {
                format!(
                    "pid={} window={} mode={:?} composition={:?} anchor={:?} geometry={:?}",
                    s.process, s.window, s.mode, s.composition, s.anchor, s.geometry
                )
            })
            .unwrap_or_else(|| "hidden".into());
        if state != previous {
            println!(
                "{}ms generation={} {state}",
                start.elapsed().as_millis(),
                update.generation
            );
            previous = state;
        }
    }
}
#[cfg(not(windows))]
fn main() {}
