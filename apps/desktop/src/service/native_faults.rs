//! Bounded worker faults for capture-disabled native acceptance only. This
//! module is absent from distribution builds and never handles platform input.
use crate::events::{Event, Hub};
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};

static DELAY_MS: AtomicU64 = AtomicU64::new(0);
static FAIL_NEXT: AtomicBool = AtomicBool::new(false);
static HOLD_NEXT: AtomicBool = AtomicBool::new(false);
static HELD: Mutex<Option<Event>> = Mutex::new(None);
static DELAYED: AtomicU64 = AtomicU64::new(0);
static FAILED: AtomicU64 = AtomicU64::new(0);
static HELD_COUNT: AtomicU64 = AtomicU64::new(0);
static LATE_COUNT: AtomicU64 = AtomicU64::new(0);
static HOLD_THUMBNAIL: AtomicBool = AtomicBool::new(false);
static HELD_THUMBNAIL: Mutex<Option<Event>> = Mutex::new(None);
static THUMBNAILS_HELD: AtomicU64 = AtomicU64::new(0);
static THUMBNAILS_RELEASED: AtomicU64 = AtomicU64::new(0);

pub(crate) fn configure_thumbnail(hub: &Arc<Hub>, hold: bool) -> Result<(), String> {
    let mut held = HELD_THUMBNAIL
        .lock()
        .map_err(|_| "Thumbnail fault state unavailable")?;
    if hold && (held.is_some() || HOLD_THUMBNAIL.load(Ordering::Acquire)) {
        return Err("A thumbnail fault is already pending".into());
    }
    HOLD_THUMBNAIL.store(hold, Ordering::Release);
    if !hold {
        if let Some(event) = held.take() {
            hub.post(event);
            THUMBNAILS_RELEASED.fetch_add(1, Ordering::Release);
        }
    }
    Ok(())
}

pub(super) fn publish_thumbnail(hub: &Arc<Hub>, event: Event) {
    if HOLD_THUMBNAIL.swap(false, Ordering::AcqRel) {
        *HELD_THUMBNAIL.lock().unwrap() = Some(event);
        THUMBNAILS_HELD.fetch_add(1, Ordering::Release);
    } else {
        hub.post(event);
    }
}

pub(crate) fn configure(delay_ms: u32, fail: bool, hold: bool) -> Result<(), String> {
    if delay_ms > 2_000
        || HELD
            .lock()
            .map_err(|_| "Fault state unavailable")?
            .is_some()
    {
        return Err(
            "Fault delay exceeds its bound or a held result still needs a newer query".into(),
        );
    }
    DELAY_MS.store(u64::from(delay_ms), Ordering::Release);
    FAIL_NEXT.store(fail, Ordering::Release);
    HOLD_NEXT.store(hold, Ordering::Release);
    Ok(())
}

pub(super) fn before_list() -> bool {
    let delay = DELAY_MS.swap(0, Ordering::AcqRel);
    if delay != 0 {
        DELAYED.fetch_add(1, Ordering::Relaxed);
        std::thread::sleep(std::time::Duration::from_millis(delay));
    }
    let fail = FAIL_NEXT.swap(false, Ordering::AcqRel);
    if fail {
        FAILED.fetch_add(1, Ordering::Relaxed);
    }
    fail
}

pub(super) fn publish_loaded(hub: &Arc<Hub>, event: Event) {
    if HOLD_NEXT.swap(false, Ordering::AcqRel) {
        *HELD.lock().unwrap() = Some(event);
        HELD_COUNT.fetch_add(1, Ordering::Release);
        return;
    }
    hub.post(event);
    // Deliberately reverse two real worker responses. Their original tickets
    // and results stay intact, so presentation must reject the older one.
    if let Some(older) = HELD.lock().unwrap().take() {
        hub.post(older);
        LATE_COUNT.fetch_add(1, Ordering::Release);
    }
}

pub(crate) fn metrics() -> serde_json::Value {
    serde_json::json!({
        "delayed": DELAYED.load(Ordering::Acquire),
        "failed": FAILED.load(Ordering::Acquire),
        "held": HELD_COUNT.load(Ordering::Acquire),
        "late": LATE_COUNT.load(Ordering::Acquire),
        "thumbnails_held": THUMBNAILS_HELD.load(Ordering::Acquire),
        "thumbnails_released": THUMBNAILS_RELEASED.load(Ordering::Acquire),
    })
}
