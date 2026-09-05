#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() {
    if let Err(error) = echo_desktop::run() {
        eprintln!("Echo: {error}");
        std::process::exit(1);
    }
}
