#[cfg(all(windows, feature = "native-test"))]
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let result = args
        .get(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| "Explicit target HWND required".to_string())
        .and_then(|hwnd| {
            echo_windows::inline::diagnostics::inspect_composer(
                hwnd,
                args.get(2).map(String::as_str).unwrap_or(""),
            )
        });
    match result {
        Ok(value) => println!(
            "{}",
            serde_json::json!({"status":"OBSERVED", "range":value})
        ),
        Err(error) => {
            println!("{}", serde_json::json!({"status":"FAIL", "error":error}));
            std::process::exit(1);
        }
    }
}

#[cfg(not(all(windows, feature = "native-test")))]
fn main() {
    eprintln!("This read-only probe requires Windows and native-test.");
    std::process::exit(1);
}
