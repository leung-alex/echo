//! Content-free foreground observation. One bounded worker owns all native reads.
use crate::{
    focus::{AnchorSource, FocusSnapshot},
    ime_observer::Observer,
};
use echo_engine::{CompositionState, InputAnchor, InputMode, InputStatus};
use std::{
    cell::RefCell,
    ptr::null_mut,
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        Arc,
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    System::Threading::GetCurrentThreadId,
    UI::{
        Accessibility::*,
        Input::{Ime::ImmIsIME, KeyboardAndMouse::GetKeyboardLayout},
        WindowsAndMessaging::*,
    },
};

const WAKE: u32 = WM_APP + 113;
const FOCUS: u32 = 1;
const GEOMETRY: u32 = 2;
const CANDIDATE: u32 = 4;
pub struct Update {
    pub generation: u64,
    pub sample: Option<InputStatus>,
}
struct Shared {
    enabled: AtomicBool,
    stopped: AtomicBool,
    thread: AtomicU32,
    generation: AtomicU64,
    samples: AtomicU64,
    probes: AtomicU64,
    dirty: AtomicU32,
    post: Arc<dyn Fn(Update) + Send + Sync>,
}
impl Shared {
    fn invalidate(&self) {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        (self.post)(Update {
            generation,
            sample: None,
        });
    }
    fn wake(&self) {
        unsafe {
            PostThreadMessageW(self.thread.load(Ordering::Acquire), WAKE, 0, 0);
        }
    }
}
thread_local! { static EVENTS: RefCell<Option<Arc<Shared>>> = const { RefCell::new(None) }; }
unsafe extern "system" fn event(
    _: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    object: i32,
    _: i32,
    _: u32,
    _: u32,
) {
    EVENTS.with(|slot| {
        let state = slot.borrow();
        let Some(s) = state.as_ref().filter(|s| s.enabled.load(Ordering::Acquire)) else {
            return;
        };
        if event == EVENT_SYSTEM_FOREGROUND || event == EVENT_OBJECT_FOCUS {
            s.dirty.fetch_or(FOCUS, Ordering::Release);
            s.invalidate();
        } else if (EVENT_OBJECT_IME_SHOW..=EVENT_OBJECT_IME_CHANGE).contains(&event) {
            s.dirty.fetch_or(CANDIDATE, Ordering::Release);
        } else if !hwnd.is_null()
            && GetAncestor(hwnd, GA_ROOT) == GetForegroundWindow()
            && matches!(object, OBJID_CARET | OBJID_CLIENT | OBJID_WINDOW)
        {
            s.dirty.fetch_or(GEOMETRY, Ordering::Release);
        }
    });
}
struct Hooks(Vec<HWINEVENTHOOK>);
impl Hooks {
    unsafe fn install() -> Self {
        let mut hooks = Vec::new();
        for (first, last) in [
            (EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND),
            (EVENT_OBJECT_FOCUS, EVENT_OBJECT_FOCUS),
            (EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_LOCATIONCHANGE),
            (EVENT_OBJECT_IME_SHOW, EVENT_OBJECT_IME_CHANGE),
            (EVENT_OBJECT_SHOW, EVENT_OBJECT_HIDE),
        ] {
            let h = SetWinEventHook(
                first,
                last,
                null_mut(),
                Some(event),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            );
            if !h.is_null() {
                hooks.push(h);
            }
        }
        Self(hooks)
    }
}
impl Drop for Hooks {
    fn drop(&mut self) {
        for h in self.0.drain(..) {
            unsafe {
                UnhookWinEvent(h);
            }
        }
    }
}

pub struct Monitor {
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
}
impl Monitor {
    pub fn start(enabled: bool, post: Arc<dyn Fn(Update) + Send + Sync>) -> Result<Self, String> {
        let shared = Arc::new(Shared {
            enabled: AtomicBool::new(enabled),
            stopped: AtomicBool::new(false),
            thread: AtomicU32::new(0),
            generation: AtomicU64::new(1),
            samples: AtomicU64::new(0),
            probes: AtomicU64::new(0),
            dirty: AtomicU32::new(FOCUS),
            post,
        });
        let state = shared.clone();
        let worker = std::thread::Builder::new()
            .name("echo-input-indicator".into())
            .spawn(move || unsafe { run(state) })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            shared,
            worker: Some(worker),
        })
    }
    pub fn generation(&self) -> u64 {
        self.shared.generation.load(Ordering::Acquire)
    }
    /// Content-free counters for acceptance and performance diagnostics.
    pub fn observation_counts(&self) -> (u64, u64) {
        (
            self.shared.samples.load(Ordering::Relaxed),
            self.shared.probes.load(Ordering::Relaxed),
        )
    }
    pub fn set_enabled(&self, enabled: bool) {
        if self.shared.enabled.swap(enabled, Ordering::AcqRel) != enabled {
            self.shared.invalidate();
            self.shared.dirty.fetch_or(FOCUS, Ordering::Release);
            self.shared.wake();
        }
    }
}
impl Drop for Monitor {
    fn drop(&mut self) {
        self.shared.stopped.store(true, Ordering::Release);
        self.shared.wake();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

unsafe fn run(s: Arc<Shared>) {
    let mut msg: MSG = std::mem::zeroed();
    PeekMessageW(&mut msg, null_mut(), 0, 0, PM_NOREMOVE);
    s.thread.store(GetCurrentThreadId(), Ordering::Release);
    EVENTS.with(|slot| *slot.borrow_mut() = Some(s.clone()));
    let mut hooks = None;
    let mut cached: Option<(
        FocusSnapshot,
        echo_engine::PasteTarget,
        crate::focus::PopupAnchor,
    )> = None;
    let mut observer = None;
    let mut refreshed = Instant::now();
    let mut next = Some(Instant::now());
    let mut failures = 0u32;
    let mut candidate_visible = false;
    while !s.stopped.load(Ordering::Acquire) {
        while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let enabled = s.enabled.load(Ordering::Acquire);
        if !enabled {
            hooks = None;
            cached = None;
            observer = None;
            next = None;
        } else if hooks.is_none() {
            hooks = Some(Hooks::install());
            s.dirty.fetch_or(FOCUS, Ordering::Release);
        }
        let dirty = s.dirty.swap(0, Ordering::AcqRel);
        if enabled && dirty & FOCUS != 0 {
            cached = None;
            observer = None;
            failures = 0;
            next = Some(Instant::now());
        }
        if enabled && (dirty != 0 && cached.is_some() || next.is_some_and(|t| Instant::now() >= t))
        {
            let generation = s.generation.load(Ordering::Acquire);
            s.samples.fetch_add(1, Ordering::Relaxed);
            let snapshot = FocusSnapshot::capture_for_indicator();
            let same = cached.as_ref().is_some_and(|(old, _, _)| {
                old.current()
                    && old.focused_handle == snapshot.focused_handle
                    && old.process_started_at == snapshot.process_started_at
            });
            if !same {
                cached = None;
                observer = None;
            }
            let needs_probe = cached.is_none()
                || dirty & GEOMETRY != 0
                || refreshed.elapsed() >= Duration::from_millis(500);
            if needs_probe {
                s.probes.fetch_add(1, Ordering::Relaxed);
                let found = snapshot.capture_target();
                refreshed = Instant::now();
                if let Some(target) = found
                    .target
                    .filter(|_| found.anchor.source != AnchorSource::Window)
                {
                    if cached
                        .as_ref()
                        .is_some_and(|(_, old, _)| old.focused_control != target.focused_control)
                    {
                        observer = None;
                        s.invalidate();
                    }
                    cached = Some((snapshot.clone(), target, found.anchor));
                } else {
                    cached = None;
                    observer = None;
                }
            }
            if needs_probe || dirty & CANDIDATE != 0 {
                candidate_visible = cached.as_ref().is_some_and(|(_, _, anchor)| {
                    !matches!(
                        anchor.source,
                        AnchorSource::Window | AnchorSource::InputControl
                    ) && crate::inline::ime_window::visible_near(
                        snapshot.focused_handle as HWND,
                        anchor.geometry.target,
                        anchor.geometry.dpi,
                    )
                    .is_some()
                });
            }
            let sample = cached.as_ref().and_then(|(_, _, anchor)| {
                let thread = GetWindowThreadProcessId(snapshot.focused_handle as HWND, null_mut());
                let layout = GetKeyboardLayout(thread);
                let english_keyboard = layout as usize & 0x3ff == 0x09 && ImmIsIME(layout) == 0;
                let state = if english_keyboard {
                    Some((InputMode::English, CompositionState::Idle))
                } else {
                    if observer.is_none() {
                        observer = Observer::status_only(
                            snapshot.focused_handle,
                            snapshot.process_id,
                            snapshot.process_started_at,
                        )
                        .ok();
                    }
                    observer.as_ref().and_then(Observer::input_state)
                };
                let (mode, composition) = state?;
                #[cfg(feature = "native-test")]
                if std::env::var_os("ECHO_INPUT_STATUS_TRACE").is_some() {
                    eprintln!("input-state pid={} mode={mode:?} raw={composition:?} candidate={candidate_visible}", snapshot.process_id);
                }
                if !snapshot.current() || s.generation.load(Ordering::Acquire) != generation {
                    return None;
                }
                let anchor = if snapshot.anchor.source == AnchorSource::NativeCaret {
                    snapshot.anchor
                } else {
                    *anchor
                };
                Some(InputStatus {
                    generation,
                    window: snapshot.window_id,
                    focused_window: snapshot.focused_handle,
                    process: snapshot.process_id,
                    process_started: snapshot.process_started_at,
                    mode,
                    composition: if candidate_visible {
                        CompositionState::Composing
                    } else {
                        composition
                    },
                    anchor: if anchor.source == AnchorSource::InputControl {
                        InputAnchor::Control
                    } else {
                        InputAnchor::Caret
                    },
                    geometry: anchor.geometry,
                    sampled_at: Instant::now(),
                })
            });
            let valid = sample.as_ref().is_some_and(|s| {
                s.mode != InputMode::Unknown && s.composition != CompositionState::Unknown
            });
            (s.post)(Update {
                generation: s.generation.load(Ordering::Acquire),
                sample,
            });
            if valid {
                failures = 0;
                next = Some(Instant::now() + Duration::from_millis(100));
            } else {
                failures = failures.saturating_add(1);
                next = (cached.is_some() || failures < 4)
                    .then(|| Instant::now() + Duration::from_millis(250 * (1 << failures.min(3))));
                observer = None;
            }
        }
        let timeout = next.map_or(u32::MAX, |t| {
            t.saturating_duration_since(Instant::now())
                .as_millis()
                .min(u32::MAX as u128 - 1) as u32
        });
        MsgWaitForMultipleObjectsEx(0, null_mut(), timeout, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
    }
    drop(observer);
    drop(hooks);
    EVENTS.with(|slot| *slot.borrow_mut() = None);
}

/// Fast UI-thread guard; no COM or cross-process messages.
pub fn foreground_matches(sample: &InputStatus) -> bool {
    unsafe {
        let hwnd = GetForegroundWindow();
        let mut pid = 0;
        let thread = GetWindowThreadProcessId(hwnd, &mut pid);
        let mut info: GUITHREADINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        hwnd as isize == sample.window
            && pid == sample.process
            && IsIconic(hwnd) == 0
            && GetGUIThreadInfo(thread, &mut info) != 0
            && info.hwndFocus as isize == sample.focused_window
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_monitor_stops_without_touching_foreground_or_waiting_for_a_timer() {
        let start = Instant::now();
        let monitor = Monitor::start(
            false,
            Arc::new(|_| panic!("disabled monitor must not sample")),
        )
        .unwrap();
        drop(monitor);
        assert!(start.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn enable_changes_invalidate_queued_samples_once() {
        let updates = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = updates.clone();
        let monitor = Monitor {
            shared: Arc::new(Shared {
                enabled: AtomicBool::new(true),
                stopped: AtomicBool::new(false),
                thread: AtomicU32::new(0),
                generation: AtomicU64::new(7),
                samples: AtomicU64::new(0),
                probes: AtomicU64::new(0),
                dirty: AtomicU32::new(0),
                post: Arc::new(move |u| observed.lock().unwrap().push(u.generation)),
            }),
            worker: None,
        };
        monitor.set_enabled(false);
        monitor.set_enabled(false);
        monitor.set_enabled(true);
        assert_eq!(*updates.lock().unwrap(), [8, 9]);
        assert_eq!(monitor.generation(), 9);
    }
}
