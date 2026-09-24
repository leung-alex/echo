//! Content-free foreground observation. One bounded worker owns all native reads.
use crate::{
    focus::{AnchorSource, FocusSnapshot},
    ime_observer::Observer,
};
use echo_engine::{
    arbitrate_geometry, CompositionState, GeometryCandidate, GeometryConfidence, GeometryIdentity,
    GeometrySafety, GeometrySource, GeometryStamp, InputAnchor, InputMode, InputStatus,
};
use std::{
    cell::RefCell,
    ptr::null_mut,
    sync::{
        atomic::{AtomicBool, AtomicIsize, AtomicU32, AtomicU64, Ordering},
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

#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetTickCount64() -> u64;
}

const WAKE: u32 = WM_APP + 113;
const FOCUS: u32 = 1;
const GEOMETRY: u32 = 2;
const CANDIDATE: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CaretProvider {
    Shadow,
    Primary,
    Legacy,
}

impl CaretProvider {
    fn from_environment() -> Result<Self, String> {
        match std::env::var("ECHO_CARET_PROVIDER") {
            Ok(value) => match value.trim().to_ascii_lowercase().as_str() {
                "shadow" => Ok(Self::Shadow),
                "primary" => Ok(Self::Primary),
                "legacy" => Ok(Self::Legacy),
                _ => Err(format!("invalid ECHO_CARET_PROVIDER={value}")),
            },
            Err(std::env::VarError::NotPresent) => Ok(Self::Legacy),
            Err(std::env::VarError::NotUnicode(_)) => {
                Err("ECHO_CARET_PROVIDER is not valid UTF-8".into())
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidationTrigger {
    Foreground,
    ObjectFocus,
    Geometry,
    Candidate,
    TargetChanged,
    SettingsEnable,
    SettingsDisable,
    SnapshotSuperseded,
    HostedProbePending,
    Polling,
}
impl InvalidationTrigger {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Foreground => "foreground",
            Self::ObjectFocus => "object-focus",
            Self::Geometry => "geometry",
            Self::Candidate => "candidate",
            Self::TargetChanged => "target-changed",
            Self::SettingsEnable => "settings-enable",
            Self::SettingsDisable => "settings-disable",
            Self::SnapshotSuperseded => "snapshot-superseded",
            Self::HostedProbePending => "hosted-probe-pending",
            Self::Polling => "polling",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnavailableReason {
    SettingsDisabled,
    NoEditableTarget,
    HostedProbeDisconnected,
    ObserverUnavailable,
    ModeUnavailable,
    PointerAnchorUnavailable,
}
impl UnavailableReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SettingsDisabled => "settings-disabled",
            Self::NoEditableTarget => "no-editable-target",
            Self::HostedProbeDisconnected => "hosted-probe-disconnected",
            Self::ObserverUnavailable => "observer-unavailable",
            Self::ModeUnavailable => "mode-unavailable",
            Self::PointerAnchorUnavailable => "pointer-anchor-unavailable",
        }
    }
}

#[derive(Clone, Debug)]
pub enum Observation {
    Revalidating {
        trigger: InvalidationTrigger,
    },
    Observed {
        sample: InputStatus,
        process_name: Option<String>,
        trigger: InvalidationTrigger,
        elapsed_ms: u64,
    },
    Unavailable {
        reason: UnavailableReason,
        process_name: Option<String>,
        trigger: InvalidationTrigger,
        elapsed_ms: u64,
    },
}

#[derive(Clone, Debug)]
pub struct Update {
    pub generation: u64,
    pub observation: Observation,
}
struct Shared {
    enabled: AtomicBool,
    stopped: AtomicBool,
    thread: AtomicU32,
    generation: AtomicU64,
    samples: AtomicU64,
    probes: AtomicU64,
    dirty: AtomicU32,
    console_root: AtomicIsize,
    post: Arc<dyn Fn(Update) + Send + Sync>,
}
impl Shared {
    fn invalidate(&self, trigger: InvalidationTrigger) {
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        (self.post)(Update {
            generation,
            observation: if trigger == InvalidationTrigger::SettingsDisable {
                Observation::Unavailable {
                    reason: UnavailableReason::SettingsDisabled,
                    process_name: None,
                    trigger,
                    elapsed_ms: 0,
                }
            } else {
                Observation::Revalidating { trigger }
            },
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
        // The console provider emits new MSAA focus object IDs while reading
        // the same cursor. They do not denote a new input control; invalidating
        // here creates a query/focus/query loop. Foreground changes still retire it.
        if event == EVENT_OBJECT_FOCUS
            && !hwnd.is_null()
            && hwnd == GetForegroundWindow()
            && s.console_root.load(Ordering::Acquire) == hwnd as isize
        {
            return;
        }
        if event == EVENT_SYSTEM_FOREGROUND || event == EVENT_OBJECT_FOCUS {
            s.dirty.fetch_or(FOCUS, Ordering::Release);
            s.invalidate(if event == EVENT_SYSTEM_FOREGROUND {
                InvalidationTrigger::Foreground
            } else {
                InvalidationTrigger::ObjectFocus
            });
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
        let provider = CaretProvider::from_environment()?;
        let shared = Arc::new(Shared {
            enabled: AtomicBool::new(enabled),
            stopped: AtomicBool::new(false),
            thread: AtomicU32::new(0),
            generation: AtomicU64::new(1),
            samples: AtomicU64::new(0),
            probes: AtomicU64::new(0),
            dirty: AtomicU32::new(FOCUS),
            console_root: AtomicIsize::new(0),
            post,
        });
        let state = shared.clone();
        let worker = std::thread::Builder::new()
            .name("echo-input-indicator".into())
            .spawn(move || unsafe { run(state, provider) })
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
            self.shared.invalidate(if enabled {
                InvalidationTrigger::SettingsEnable
            } else {
                InvalidationTrigger::SettingsDisable
            });
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

fn caret_endpoint(
    snapshot: &FocusSnapshot,
    endpoint: Option<crate::focus::InputStatusEndpoint>,
    generation: u64,
) -> Option<crate::caret::CaretEndpoint> {
    let endpoint = endpoint.or_else(|| snapshot.input_endpoint())?;
    if endpoint.input == 0 || endpoint.process == 0 || endpoint.started == 0 || endpoint.thread == 0
    {
        return None;
    }
    Some(crate::caret::CaretEndpoint {
        root_window: snapshot.window_id,
        root_process: snapshot.process_id,
        root_started: snapshot.process_started_at,
        input_window: endpoint.input,
        input_process: endpoint.process,
        input_started: endpoint.started,
        input_thread: endpoint.thread,
        focus_generation: generation,
    })
}

fn geometry_identity(
    generation: u64,
    snapshot: &FocusSnapshot,
    endpoint: Option<crate::focus::InputStatusEndpoint>,
) -> Option<GeometryIdentity> {
    let endpoint = endpoint.or_else(|| snapshot.input_endpoint())?;
    Some(GeometryIdentity {
        generation,
        process: endpoint.process,
        process_started: endpoint.started,
        input_thread: endpoint.thread,
        root_window: snapshot.window_id,
        focused_window: snapshot.focused_handle,
    })
}

fn candidate_from_anchor(
    identity: GeometryIdentity,
    anchor: crate::focus::PopupAnchor,
    observed_at: Instant,
    sequence: u64,
) -> GeometryCandidate {
    let source = match anchor.source {
        AnchorSource::NativeCaret => GeometrySource::NativeCaret,
        AnchorSource::AutomationCaret => GeometrySource::UiaCaret,
        AnchorSource::AccessibleCaret => GeometrySource::MsaaCaret,
        AnchorSource::AdjacentCharacter => GeometrySource::AdjacentCharacter,
        AnchorSource::InputMethodCaret => GeometrySource::ImmExclusion,
        AnchorSource::InputControl | AnchorSource::Window => GeometrySource::Control,
        AnchorSource::Pointer => GeometrySource::Pointer,
    };
    let confidence = if source == GeometrySource::AdjacentCharacter {
        GeometryConfidence::Estimated
    } else if matches!(source, GeometrySource::Control | GeometrySource::Pointer) {
        GeometryConfidence::Fallback
    } else {
        GeometryConfidence::Exact
    };
    GeometryCandidate {
        identity,
        source,
        confidence,
        geometry: anchor.geometry,
        observed_at,
        sequence,
        context_epoch: 0,
        safety: GeometrySafety::Allowed,
        clipped: false,
        interim_character: false,
        noncollapsed_selection: false,
        view_verified: !matches!(source, GeometrySource::Control | GeometrySource::Pointer),
        control: None,
    }
}

fn candidate_from_tsf(
    reply: crate::caret::GeometryReply,
    identity: GeometryIdentity,
    fallback_geometry: echo_engine::InputTargetGeometry,
    now_tick: u64,
) -> Option<GeometryCandidate> {
    if reply.status != crate::caret::ReplyStatus::Ready {
        return None;
    }
    let words = reply.response_words;
    let observed_tick = u64::from(words[6]) | (u64::from(words[7]) << 32);
    if observed_tick > now_tick {
        return None;
    }
    let observed_at =
        Instant::now().checked_sub(Duration::from_millis(now_tick - observed_tick))?;
    let rect = echo_engine::PhysicalRect {
        x: words[2] as i32,
        y: words[3] as i32,
        width: (words[4] as i32).checked_sub(words[2] as i32)?,
        height: (words[5] as i32).checked_sub(words[3] as i32)?,
    };
    (rect.width >= 0 && rect.height > 0).then_some(GeometryCandidate::exact(
        identity,
        GeometrySource::TsfCaret,
        echo_engine::InputTargetGeometry {
            target: rect,
            work_area: fallback_geometry.work_area,
            dpi: fallback_geometry.dpi,
        },
        observed_at,
        u64::from(reply.request_sequence),
        u64::from(words[10]) | (u64::from(words[11]) << 32),
    ))
}

unsafe fn run(s: Arc<Shared>, provider: CaretProvider) {
    let mut msg: MSG = std::mem::zeroed();
    PeekMessageW(&mut msg, null_mut(), 0, 0, PM_NOREMOVE);
    s.thread.store(GetCurrentThreadId(), Ordering::Release);
    EVENTS.with(|slot| *slot.borrow_mut() = Some(s.clone()));
    let mut hooks = None;
    let mut cached: Option<(
        FocusSnapshot,
        (
            Option<echo_engine::PasteControlIdentity>,
            Option<crate::focus::InputStatusEndpoint>,
        ),
        crate::focus::PopupAnchor,
    )> = None;
    let mut observer = None;
    let mut caret_observer: Option<crate::caret::CaretObserver> = None;
    let mut tsf_candidate: Option<GeometryCandidate> = None;
    // Hosted XAML providers can block for a second between calls. Geometry
    // refresh must not block the independent, fresh IME mode samples.
    let mut pending: Option<(
        FocusSnapshot,
        std::sync::mpsc::Receiver<Option<crate::focus::automation::Probe>>,
    )> = None;
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
            s.console_root.store(0, Ordering::Release);
            cached = None;
            observer = None;
            caret_observer = None;
            tsf_candidate = None;
            pending = None;
            next = None;
        } else if hooks.is_none() {
            hooks = Some(Hooks::install());
            s.dirty.fetch_or(FOCUS, Ordering::Release);
        }
        let dirty = s.dirty.swap(0, Ordering::AcqRel);
        if enabled && dirty & FOCUS != 0 {
            s.console_root.store(0, Ordering::Release);
            cached = None;
            observer = None;
            caret_observer = None;
            tsf_candidate = None;
            pending = None;
            failures = 0;
            next = Some(Instant::now());
        }
        if enabled && (dirty != 0 && cached.is_some() || next.is_some_and(|t| Instant::now() >= t))
        {
            let generation = s.generation.load(Ordering::Acquire);
            let probe_trigger = if dirty & FOCUS != 0 {
                InvalidationTrigger::ObjectFocus
            } else if dirty & GEOMETRY != 0 {
                InvalidationTrigger::Geometry
            } else if dirty & CANDIDATE != 0 {
                InvalidationTrigger::Candidate
            } else {
                InvalidationTrigger::Polling
            };
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
                caret_observer = None;
                tsf_candidate = None;
            }
            if pending.as_ref().is_some_and(|(old, _)| {
                !old.current()
                    || old.focused_handle != snapshot.focused_handle
                    || old.window_id != snapshot.window_id
            }) {
                pending = None;
            }
            let mut hosted_disconnected = false;
            let completed = pending
                .as_ref()
                .and_then(|(_, response)| match response.try_recv() {
                    Ok(value) => Some(value),
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        hosted_disconnected = true;
                        Some(None)
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => None,
                });
            let hosted = snapshot.is_hosted_input();
            let needs_probe = cached.is_none()
                || dirty & GEOMETRY != 0
                || refreshed.elapsed() >= Duration::from_millis(500);
            if hosted && needs_probe && pending.is_none() && completed.is_none() {
                s.probes.fetch_add(1, Ordering::Relaxed);
                pending = crate::focus::automation::begin_query(snapshot.clone(), true)
                    .map(|response| (snapshot.clone(), response));
            }
            if completed.is_some() || !hosted && needs_probe {
                if !hosted {
                    s.probes.fetch_add(1, Ordering::Relaxed);
                }
                let observed = if hosted {
                    pending = None;
                    completed.flatten().and_then(|probe| {
                        let (rect, source) = probe.anchor?;
                        (source != AnchorSource::Window).then(|| {
                            (
                                (Some(probe.identity), None),
                                crate::focus::PopupAnchor {
                                    geometry: crate::focus::geometry(rect),
                                    source,
                                },
                            )
                        })
                    })
                } else {
                    snapshot
                        .console_indicator()
                        .or_else(|| snapshot.terminal_tsf_indicator())
                        .map(|(endpoint, anchor)| ((None, Some(endpoint)), anchor))
                        .or_else(|| {
                            let found = snapshot.capture_target();
                            found
                                .target
                                .filter(|_| found.anchor.source != AnchorSource::Window)
                                .map(|target| ((target.focused_control, None), found.anchor))
                        })
                        .or_else(|| {
                            snapshot
                                .warp_pointer_indicator()
                                .map(|(endpoint, anchor)| ((None, Some(endpoint)), anchor))
                        })
                };
                refreshed = Instant::now();
                if let Some((target, anchor)) = observed {
                    if cached.as_ref().is_some_and(|(_, old, _)| old != &target) {
                        observer = None;
                        caret_observer = None;
                        tsf_candidate = None;
                        s.invalidate(InvalidationTrigger::TargetChanged);
                    }
                    s.console_root.store(
                        if target.1.is_some_and(|t| t.window != t.input) {
                            snapshot.window_id
                        } else {
                            0
                        },
                        Ordering::Release,
                    );
                    cached = Some((snapshot.clone(), target, anchor));
                } else {
                    cached = None;
                    observer = None;
                    caret_observer = None;
                    tsf_candidate = None;
                }
            }
            if needs_probe || dirty & CANDIDATE != 0 {
                candidate_visible = cached.as_ref().is_some_and(|(_, _, anchor)| {
                    !matches!(
                        anchor.source,
                        AnchorSource::Window | AnchorSource::InputControl | AnchorSource::Pointer
                    ) && crate::inline::ime_window::visible_near(
                        snapshot.focused_handle as HWND,
                        anchor.geometry.target,
                        anchor.geometry.dpi,
                    )
                    .is_some()
                });
            }
            if provider != CaretProvider::Legacy {
                if let Some((cached_snapshot, target, anchor)) = cached.as_ref() {
                    let endpoint = caret_endpoint(cached_snapshot, target.1, generation);
                    let endpoint_matches = endpoint
                        .as_ref()
                        .zip(caret_observer.as_ref())
                        .is_some_and(|(expected, current)| {
                            let actual = current.endpoint();
                            actual.input_window == expected.input_window
                                && actual.input_process == expected.input_process
                                && actual.input_started == expected.input_started
                                && actual.focus_generation == expected.focus_generation
                        });
                    if !endpoint_matches {
                        caret_observer = endpoint
                            .and_then(|value| crate::caret::CaretObserver::start(value).ok());
                        tsf_candidate = None;
                    }
                    if let Some(caret) = caret_observer.as_mut() {
                        let now_tick = GetTickCount64();
                        caret.heartbeat(now_tick);
                        if let Some(reply) = caret.try_read(now_tick) {
                            tsf_candidate = if reply.status == crate::caret::ReplyStatus::Ready {
                                geometry_identity(generation, cached_snapshot, target.1).and_then(
                                    |identity| {
                                        candidate_from_tsf(
                                            reply,
                                            identity,
                                            anchor.geometry,
                                            now_tick,
                                        )
                                    },
                                )
                            } else {
                                None
                            };
                        }
                        let _ = caret.try_request(now_tick, dirty);
                    }
                }
            }
            let probe_started = Instant::now();
            let process_name = process_basename(snapshot.process_id);
            let mut unavailable_reason = UnavailableReason::NoEditableTarget;
            let sample = cached.as_ref().and_then(|(_, target, anchor)| {
                let thread = GetWindowThreadProcessId(
                    target.1.map_or(snapshot.focused_handle, |t| t.window) as HWND,
                    null_mut(),
                );
                let layout = GetKeyboardLayout(thread);
                let english_keyboard = layout as usize & 0x3ff == 0x09 && ImmIsIME(layout) == 0;
                let state = if english_keyboard {
                    Some((InputMode::English, CompositionState::Idle))
                } else {
                    if observer.is_none() {
                        observer = if let Some(endpoint) = target.1 {
                            Observer::status_at(endpoint)
                        } else {
                            snapshot
                                .input_endpoint()
                                .ok_or_else(|| "Input owner unavailable".to_string())
                                .and_then(|input| {
                                    Observer::status_only(input.window, input.process, input.started)
                                })
                        }
                        .ok();
                    }
                    observer.as_ref().and_then(Observer::input_state)
                };
                let Some((mode, composition)) = state else {
                    unavailable_reason = UnavailableReason::ObserverUnavailable;
                    return None;
                };
                if mode == InputMode::Unknown {
                    unavailable_reason = UnavailableReason::ModeUnavailable;
                    return None;
                }
                #[cfg(feature = "native-test")]
                if std::env::var_os("ECHO_INPUT_STATUS_TRACE").is_some() {
                    eprintln!(
                        "input-state pid={} mode={mode:?} raw={composition:?} candidate={candidate_visible}",
                        snapshot.process_id
                    );
                }
                if !snapshot.current() || s.generation.load(Ordering::Acquire) != generation {
                    return None;
                }
                let legacy_anchor = if anchor.source == AnchorSource::Pointer {
                    let Some((_, anchor)) = snapshot.warp_pointer_indicator() else {
                        unavailable_reason = UnavailableReason::PointerAnchorUnavailable;
                        return None;
                    };
                    anchor
                } else {
                    *anchor
                };
                let now = Instant::now();
                let sequence = s.samples.load(Ordering::Relaxed);
                let (anchor, geometry_stamp) = if provider == CaretProvider::Primary {
                    let Some(identity) = geometry_identity(generation, &snapshot, target.1) else {
                        unavailable_reason = UnavailableReason::NoEditableTarget;
                        return None;
                    };
                    let fallback = candidate_from_anchor(identity, legacy_anchor, now, sequence);
                    let selected = if let Some(tsf) = tsf_candidate {
                        arbitrate_geometry(identity, [fallback, tsf], now, tsf.context_epoch)
                    } else {
                        arbitrate_geometry(identity, [fallback], now, 0)
                    };
                    let Some(selected) = selected else {
                        unavailable_reason = UnavailableReason::NoEditableTarget;
                        return None;
                    };
                    let source = match selected.source {
                        GeometrySource::Control => AnchorSource::InputControl,
                        GeometrySource::Pointer => AnchorSource::Pointer,
                        GeometrySource::NativeCaret => AnchorSource::NativeCaret,
                        GeometrySource::UiaCaret => AnchorSource::AutomationCaret,
                        GeometrySource::MsaaCaret => AnchorSource::AccessibleCaret,
                        GeometrySource::AdjacentCharacter => AnchorSource::AdjacentCharacter,
                        GeometrySource::ImmExclusion => AnchorSource::InputMethodCaret,
                        GeometrySource::TsfCaret => AnchorSource::NativeCaret,
                    };
                    (
                        crate::focus::PopupAnchor {
                            geometry: selected.geometry,
                            source,
                        },
                        GeometryStamp {
                            source: selected.source,
                            confidence: selected.confidence,
                            observed_at: selected.observed_at,
                            sequence: selected.sequence,
                            context_epoch: selected.context_epoch,
                        },
                    )
                } else {
                    (
                        legacy_anchor,
                        GeometryStamp {
                            source: match legacy_anchor.source {
                                AnchorSource::NativeCaret => GeometrySource::NativeCaret,
                                AnchorSource::AutomationCaret => GeometrySource::UiaCaret,
                                AnchorSource::AccessibleCaret => GeometrySource::MsaaCaret,
                                AnchorSource::AdjacentCharacter => GeometrySource::AdjacentCharacter,
                                AnchorSource::InputMethodCaret => GeometrySource::ImmExclusion,
                                AnchorSource::InputControl | AnchorSource::Window => GeometrySource::Control,
                                AnchorSource::Pointer => GeometrySource::Pointer,
                            },
                            confidence: match legacy_anchor.source {
                                AnchorSource::AdjacentCharacter => GeometryConfidence::Estimated,
                                AnchorSource::InputControl
                                | AnchorSource::Window
                                | AnchorSource::Pointer => GeometryConfidence::Fallback,
                                _ => GeometryConfidence::Exact,
                            },
                            observed_at: now,
                            sequence,
                            context_epoch: 0,
                        },
                    )
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
                    anchor: match anchor.source {
                        AnchorSource::InputControl => InputAnchor::Control,
                        AnchorSource::Pointer => InputAnchor::Pointer,
                        _ => InputAnchor::Caret,
                    },
                    geometry: anchor.geometry,
                    geometry_stamp,
                    sampled_at: Instant::now(),
                })
            });
            let current_generation = s.generation.load(Ordering::Acquire);
            let valid = sample.is_some();
            if current_generation == generation {
                let elapsed_ms = probe_started.elapsed().as_millis().min(u64::MAX as u128) as u64;
                let observation = if let Some(sample) = sample {
                    Observation::Observed {
                        sample,
                        process_name,
                        trigger: probe_trigger,
                        elapsed_ms,
                    }
                } else if !snapshot.current() {
                    Observation::Revalidating {
                        trigger: InvalidationTrigger::SnapshotSuperseded,
                    }
                } else if pending.is_some() {
                    Observation::Revalidating {
                        trigger: InvalidationTrigger::HostedProbePending,
                    }
                } else {
                    Observation::Unavailable {
                        reason: if hosted_disconnected {
                            UnavailableReason::HostedProbeDisconnected
                        } else {
                            unavailable_reason
                        },
                        process_name,
                        trigger: probe_trigger,
                        elapsed_ms,
                    }
                };
                (s.post)(Update {
                    generation,
                    observation,
                });
            }
            if valid || pending.is_some() || hosted && cached.is_some() {
                failures = 0;
                next = Some(Instant::now() + Duration::from_millis(100));
            } else {
                failures = failures.saturating_add(1);
                // Hosted XAML may still be connecting on the bounded UIA broker
                // after the normal retry window. Keep a finite, slower warm-up
                // budget; every attempt captures and validates fresh focus.
                let attempts = if snapshot
                    .input_endpoint()
                    .is_some_and(|input| input.process != snapshot.process_id)
                {
                    8
                } else {
                    4
                };
                next = (cached.is_some() || failures < attempts)
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

/// Content-free executable basename used by diagnostics. Full paths are never exposed.
pub fn process_basename(process_id: u32) -> Option<String> {
    crate::windows_impl::process_path(process_id).and_then(|path| {
        std::path::Path::new(&path)
            .file_name()
            .and_then(|name| name.to_str())
            .map(str::to_owned)
    })
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
                console_root: AtomicIsize::new(0),
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
