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
static THUMBNAIL_RELEASING: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn releasing_a_held_thumbnail_never_waits_for_ui_delivery() {
        let hub = Arc::new(Hub::default());
        let thumbnail = || {
            Event::Thumbnail(
                1,
                "synthetic".into(),
                Default::default(),
                Ok(crate::events::PixelData {
                    requested: Default::default(),
                    width: 1,
                    height: 1,
                    rgba: vec![0; 9 * 1024 * 1024],
                }),
            )
        };
        hub.post(thumbnail());
        configure_thumbnail(&hub, true).unwrap();
        publish_thumbnail(&hub, thumbnail());
        let (sent, received) = std::sync::mpsc::channel();
        let releasing = hub.clone();
        let ui = std::thread::spawn(move || {
            let _ = sent.send(configure_thumbnail(&releasing, false));
        });
        let returned = received.recv_timeout(std::time::Duration::from_secs(2));
        // Always unblock the worker even if the regression reappears.
        hub.close();
        ui.join().unwrap();
        returned
            .expect("UI release waited on display credits")
            .unwrap();
    }
}

pub(crate) fn configure_thumbnail(hub: &Arc<Hub>, hold: bool) -> Result<(), String> {
    let mut held = HELD_THUMBNAIL
        .lock()
        .map_err(|_| "Thumbnail fault state unavailable")?;
    if hold
        && (held.is_some()
            || HOLD_THUMBNAIL.load(Ordering::Acquire)
            || THUMBNAIL_RELEASING.load(Ordering::Acquire))
    {
        return Err("A thumbnail fault is already pending".into());
    }
    HOLD_THUMBNAIL.store(hold, Ordering::Release);
    if !hold {
        if let Some(event) = held.take() {
            // Called by the UI timer: it must remain able to drain credits.
            // At most one diagnostic release may wait on the hub. close()
            // wakes it during shutdown; no fault mutex crosses the wait.
            THUMBNAIL_RELEASING.store(true, Ordering::Release);
            drop(held);
            let hub = hub.clone();
            std::thread::spawn(move || {
                hub.post(event);
                THUMBNAILS_RELEASED.fetch_add(1, Ordering::Release);
                THUMBNAIL_RELEASING.store(false, Ordering::Release);
            });
        }
    }
    Ok(())
}

pub(super) fn publish_thumbnail(hub: &Arc<Hub>, event: Event) {
    let mut held = HELD_THUMBNAIL.lock().unwrap();
    if HOLD_THUMBNAIL.swap(false, Ordering::AcqRel) {
        *held = Some(event);
        THUMBNAILS_HELD.fetch_add(1, Ordering::Release);
    } else {
        drop(held);
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
