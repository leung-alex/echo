//! Observations and bounded provider faults for isolated native acceptance.
//! No input, clipboard access or additional hook is installed by this module.
use super::{target, FocusSnapshot};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

pub(super) static PAUSE_WINDOW_EVENTS: AtomicBool = AtomicBool::new(false);
static ACQUISITION_DELAY_MS: AtomicU32 = AtomicU32::new(0);
pub(super) static COMPOSITION_UNAVAILABLE: AtomicBool = AtomicBool::new(false);
static SELECTION_FAULT: AtomicU32 = AtomicU32::new(0);
static ACQUISITION_HITS: AtomicU32 = AtomicU32::new(0);
static SELECTION_HITS: AtomicU32 = AtomicU32::new(0);

pub fn provider_fault_metrics() -> serde_json::Value {
    serde_json::json!({"acquisitions": ACQUISITION_HITS.load(Ordering::Acquire),
        "selections": SELECTION_HITS.load(Ordering::Acquire),
        "composition_unavailable": COMPOSITION_UNAVAILABLE.load(Ordering::Acquire)})
}

pub fn configure_provider_fault(
    delay_ms: u32,
    composition_unavailable: bool,
    selection: u32,
) -> Result<(), String> {
    if std::env::var("ECHO_WINDOWS_ACCEPTANCE").as_deref() != Ok("1")
        || delay_ms > 500
        || selection > 3
    {
        return Err("Provider faults require explicit native acceptance and bounded values".into());
    }
    ACQUISITION_DELAY_MS.store(delay_ms, Ordering::Release);
    COMPOSITION_UNAVAILABLE.store(composition_unavailable, Ordering::Release);
    SELECTION_FAULT.store(selection, Ordering::Release);
    Ok(())
}

pub(super) fn acquisition_delay() -> u32 {
    let delay = ACQUISITION_DELAY_MS.swap(0, Ordering::AcqRel);
    if delay != 0 {
        ACQUISITION_HITS.fetch_add(1, Ordering::Release);
    }
    delay
}

pub(super) fn selection_fault() -> u32 {
    let fault = SELECTION_FAULT.swap(0, Ordering::AcqRel);
    if fault != 0 {
        SELECTION_HITS.fetch_add(1, Ordering::Release);
    }
    fault
}

/// Fault injection for the isolated native-test executable only. Keyboard
/// protection remains installed; no external Enter interceptor is introduced.
pub fn pause_window_events(paused: bool) -> Result<(), String> {
    if std::env::var("ECHO_WINDOWS_ACCEPTANCE").as_deref() != Ok("1") {
        return Err("Explicit native acceptance is required".into());
    }
    PAUSE_WINDOW_EVENTS.store(paused, Ordering::Release);
    Ok(())
}

pub fn inspect_composer(window: isize, expected_value: &str) -> Result<serde_json::Value, String> {
    if std::env::var("ECHO_WINDOWS_ACCEPTANCE").as_deref() != Ok("1") {
        return Err("Explicit native acceptance is required".into());
    }
    unsafe {
        CoInitializeEx(None, COINIT_MULTITHREADED)
            .ok()
            .map_err(|_| "COM initialization failed")?;
    }
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe { CoUninitialize() }
        }
    }
    let _apartment = Apartment;
    let snapshot = FocusSnapshot::capture();
    if snapshot.window_id != window {
        return Err("Foreground is not the explicitly selected synthetic composer window".into());
    }
    let uia = target::create_automation();
    let target = target::Target::open(&snapshot, uia.as_ref())?;
    target.inspect_synthetic_ranges(expected_value)
}
