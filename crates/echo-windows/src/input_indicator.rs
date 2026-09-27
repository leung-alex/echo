//! Content-free foreground observation. One bounded worker owns all native reads.
use crate::{
    focus::{AnchorSource, FocusSnapshot},
    ime_observer::Observer,
};
use echo_engine::{
    arbitrate_geometry, geometry_is_fresh, validate_geometry, CompositionState, GeometryCandidate,
    GeometryConfidence, GeometryIdentity, GeometrySafety, GeometrySource, GeometryStamp,
    InputAnchor, InputMode, InputStatus, GEOMETRY_REVALIDATION_INTERVAL,
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

fn bootstrap_reason_code(error: &str) -> u32 {
    if let Some(reason) = error
        .split("reason=")
        .nth(1)
        .and_then(|value| value.split(|ch: char| !ch.is_ascii_digit()).next())
        .and_then(|value| value.parse::<u32>().ok())
    {
        return reason;
    }
    if error.contains("target identity") {
        32
    } else if error.contains("module") || error.contains("entry point") {
        33
    } else if error.contains("bootstrap") || error.contains("scheduler") {
        34
    } else if error.contains("hook") {
        35
    } else {
        31
    }
}

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

/// Native acceptance supplies an exact fixture PID/root HWND so a transient
/// focus loss cannot make the observer attach to an unrelated desktop app.
/// Production processes do not set `ECHO_NATIVE_TEST_ROOT`, so this guard is
/// inactive for ordinary use.
fn native_test_target() -> Option<(u32, isize)> {
    if std::env::var_os("ECHO_NATIVE_TEST_ROOT").is_none() {
        return None;
    }
    let pid = std::env::var("ECHO_NATIVE_TEST_TARGET_PID")
        .ok()
        .and_then(|value| value.parse::<u32>().ok());
    let hwnd = std::env::var("ECHO_NATIVE_TEST_TARGET_HWND")
        .ok()
        .and_then(|value| value.parse::<isize>().ok());
    match (pid, hwnd) {
        (Some(pid), Some(hwnd)) if pid != 0 && hwnd != 0 => Some((pid, hwnd)),
        // A partially configured acceptance environment must fail closed.
        _ => Some((u32::MAX, isize::MIN)),
    }
}

fn next_poll_delay(
    provider: CaretProvider,
    valid: bool,
    hosted_pending: bool,
    hosted_admitted: bool,
    tsf_active: bool,
    tsf_backoff: bool,
    admitted: bool,
    failures: u32,
    cross_process: bool,
) -> Option<Duration> {
    if valid || hosted_pending || hosted_admitted || tsf_active {
        return Some(Duration::from_millis(
            if provider == CaretProvider::Legacy {
                100
            } else if tsf_backoff {
                100
            } else {
                50
            },
        ));
    }
    let attempts = if cross_process { 8 } else { 4 };
    (admitted || failures < attempts).then(|| Duration::from_millis(250 * (1 << failures.min(3))))
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
    /// A provider reported that the current editor is sensitive, read-only,
    /// or otherwise unsafe for a passive caret badge.  This is terminal for
    /// the current target until a fresh allowed geometry is proven.
    UnsafeGeometry,
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
            Self::UnsafeGeometry => "unsafe-geometry",
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
    pub tsf: Option<TsfDiagnostic>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TsfDiagnosticState {
    Pending,
    Ready,
    Unavailable,
    Closed,
    BootstrapFailed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TsfDiagnostic {
    pub state: TsfDiagnosticState,
    pub api_hresult: Option<i32>,
    pub session_hresult: Option<i32>,
    pub reason_code: Option<u32>,
    pub sequence: u32,
    pub context_epoch: u64,
    pub raw_rect: Option<[i32; 4]>,
    pub normalized_rect: Option<[i32; 4]>,
    pub age_ms: Option<u64>,
    pub pending_callbacks: u32,
    pub outstanding_callbacks: u32,
    pub created_callbacks: u64,
    pub released_callbacks: u64,
    pub callback_high_water: u32,
    pub pending_high_water: u32,
    pub api_requests: u64,
    pub request_edit_calls: u64,
    pub accepted_sessions: u64,
    pub callback_entered: u64,
    pub callback_completed: u64,
    pub final_released: u64,
    pub cancelled: u64,
    pub timed_out: u64,
    pub ready_results: u64,
    pub mock_sessions: u64,
    pub mock_callback_entered: u64,
    pub mock_callback_completed: u64,
    pub mock_final_released: u64,
    pub final_source: Option<GeometrySource>,
    pub fallback_reason: Option<u32>,
}

impl TsfDiagnostic {
    fn from_reply(reply: crate::caret::GeometryReply, now_tick: u64) -> Self {
        let words = reply.response_words;
        let epoch = u64::from(words[10]) | (u64::from(words[11]) << 32);
        let tick = u64::from(words[8]) | (u64::from(words[9]) << 32);
        let age_ms = (tick <= now_tick).then_some(now_tick - tick);
        let raw = [
            words[14] as i32,
            words[15] as i32,
            words[16] as i32,
            words[17] as i32,
        ];
        let normalized = [
            words[18] as i32,
            words[19] as i32,
            words[20] as i32,
            words[21] as i32,
        ];
        Self {
            state: match reply.status {
                crate::caret::ReplyStatus::Ready => TsfDiagnosticState::Ready,
                crate::caret::ReplyStatus::Unavailable => TsfDiagnosticState::Unavailable,
                crate::caret::ReplyStatus::Pending => TsfDiagnosticState::Pending,
                crate::caret::ReplyStatus::Closed => TsfDiagnosticState::Closed,
            },
            api_hresult: Some(words[5] as i32),
            session_hresult: Some(words[6] as i32),
            reason_code: (words[4] != 0).then_some(words[4]),
            sequence: reply.request_sequence,
            context_epoch: epoch,
            raw_rect: (words[14..18] != [0; 4]).then_some(raw),
            normalized_rect: (words[18..22] != [0; 4]).then_some(normalized),
            age_ms,
            pending_callbacks: words[29],
            outstanding_callbacks: words[30],
            created_callbacks: u64::from(words[32]) | (u64::from(words[33]) << 32),
            released_callbacks: u64::from(words[34]) | (u64::from(words[35]) << 32),
            callback_high_water: words[50],
            pending_high_water: words[51],
            api_requests: u64::from(words[36]) | (u64::from(words[37]) << 32),
            request_edit_calls: u64::from(words[54]) | (u64::from(words[55]) << 32),
            accepted_sessions: u64::from(words[38]) | (u64::from(words[39]) << 32),
            callback_entered: u64::from(words[40]) | (u64::from(words[41]) << 32),
            callback_completed: u64::from(words[42]) | (u64::from(words[43]) << 32),
            final_released: u64::from(words[44]) | (u64::from(words[45]) << 32),
            cancelled: u64::from(words[46]) | (u64::from(words[47]) << 32),
            timed_out: u64::from(words[48]) | (u64::from(words[49]) << 32),
            ready_results: u64::from(words[52]) | (u64::from(words[53]) << 32),
            mock_sessions: u64::from(words[56]) | (u64::from(words[57]) << 32),
            mock_callback_entered: u64::from(words[58]) | (u64::from(words[59]) << 32),
            mock_callback_completed: u64::from(words[60]) | (u64::from(words[61]) << 32),
            mock_final_released: u64::from(words[62]) | (u64::from(words[63]) << 32),
            final_source: None,
            fallback_reason: (!reply.accepted).then_some(words[4]),
        }
    }
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
            tsf: None,
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
    let endpoint = endpoint
        .or_else(|| snapshot.indicator_endpoint())
        .or_else(|| snapshot.input_endpoint())?;
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

fn input_identity(
    snapshot: &FocusSnapshot,
    target: &(
        Option<echo_engine::PasteControlIdentity>,
        Option<crate::focus::InputStatusEndpoint>,
    ),
) -> Option<(isize, u32, u64, u32)> {
    target
        .1
        .or_else(|| snapshot.indicator_endpoint())
        .or_else(|| snapshot.input_endpoint())
        .map(|endpoint| {
            (
                endpoint.input,
                endpoint.process,
                endpoint.started,
                endpoint.thread,
            )
        })
}

fn target_endpoint_changed(
    old_snapshot: &FocusSnapshot,
    old_target: &(
        Option<echo_engine::PasteControlIdentity>,
        Option<crate::focus::InputStatusEndpoint>,
    ),
    snapshot: &FocusSnapshot,
    target: &(
        Option<echo_engine::PasteControlIdentity>,
        Option<crate::focus::InputStatusEndpoint>,
    ),
) -> bool {
    old_snapshot.window_id != snapshot.window_id
        || old_snapshot.process_id != snapshot.process_id
        || old_snapshot.process_started_at != snapshot.process_started_at
        || old_snapshot.focused_handle != snapshot.focused_handle
        || input_identity(old_snapshot, old_target) != input_identity(snapshot, target)
}

fn target_identity_changed(
    old_snapshot: &FocusSnapshot,
    old_target: &(
        Option<echo_engine::PasteControlIdentity>,
        Option<crate::focus::InputStatusEndpoint>,
    ),
    snapshot: &FocusSnapshot,
    target: &(
        Option<echo_engine::PasteControlIdentity>,
        Option<crate::focus::InputStatusEndpoint>,
    ),
) -> bool {
    target_endpoint_changed(old_snapshot, old_target, snapshot, target) || old_target.0 != target.0
}

fn geometry_identity(
    generation: u64,
    snapshot: &FocusSnapshot,
    endpoint: Option<crate::focus::InputStatusEndpoint>,
) -> Option<GeometryIdentity> {
    let endpoint = endpoint
        .or_else(|| snapshot.indicator_endpoint())
        .or_else(|| snapshot.input_endpoint())?;
    Some(GeometryIdentity {
        generation,
        process: endpoint.process,
        process_started: endpoint.started,
        input_thread: endpoint.thread,
        root_process: snapshot.process_id,
        root_started: snapshot.process_started_at,
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
    let source = geometry_source(anchor.source);
    let confidence = geometry_confidence(source);
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

#[derive(Clone, Copy, Debug)]
struct GeometryProbeState {
    last_attempt: Option<Instant>,
}

impl GeometryProbeState {
    const fn new() -> Self {
        Self { last_attempt: None }
    }

    fn reset(&mut self) {
        self.last_attempt = None;
    }

    fn due(&self, now: Instant, force: bool) -> bool {
        force
            || self.last_attempt.is_none_or(|last| {
                now.saturating_duration_since(last) >= GEOMETRY_REVALIDATION_INTERVAL
            })
    }

    fn mark_attempt(&mut self, now: Instant) {
        self.last_attempt = Some(now);
    }
}

/// Legacy geometry may keep the explicit IME caret path, but a generic
/// control/window or pointer rectangle is not the insertion caret. Showing
/// either would place the passive badge at an unrelated edge (for example,
/// the far right of a WeChat composer or wherever the mouse happens to be).
fn legacy_geometry_allowed(source: GeometrySource) -> bool {
    !matches!(source, GeometrySource::Control | GeometrySource::Pointer)
}

fn anchor_source(source: GeometrySource) -> AnchorSource {
    match source {
        GeometrySource::Control => AnchorSource::InputControl,
        GeometrySource::Pointer => AnchorSource::Pointer,
        GeometrySource::NativeCaret | GeometrySource::TsfCaret => AnchorSource::NativeCaret,
        GeometrySource::UiaCaret => AnchorSource::AutomationCaret,
        GeometrySource::MsaaCaret => AnchorSource::AccessibleCaret,
        GeometrySource::AdjacentCharacter => AnchorSource::AdjacentCharacter,
        GeometrySource::EditorLeadingEdge => AnchorSource::EditorLeadingEdge,
        GeometrySource::ImmExclusion => AnchorSource::InputMethodCaret,
    }
}

fn geometry_source(source: AnchorSource) -> GeometrySource {
    match source {
        AnchorSource::NativeCaret => GeometrySource::NativeCaret,
        AnchorSource::AutomationCaret => GeometrySource::UiaCaret,
        AnchorSource::AccessibleCaret => GeometrySource::MsaaCaret,
        AnchorSource::AdjacentCharacter => GeometrySource::AdjacentCharacter,
        AnchorSource::EditorLeadingEdge => GeometrySource::EditorLeadingEdge,
        AnchorSource::InputMethodCaret => GeometrySource::ImmExclusion,
        AnchorSource::InputControl | AnchorSource::Window => GeometrySource::Control,
        AnchorSource::Pointer => GeometrySource::Pointer,
    }
}

fn geometry_confidence(source: GeometrySource) -> GeometryConfidence {
    match source {
        GeometrySource::AdjacentCharacter | GeometrySource::EditorLeadingEdge => {
            GeometryConfidence::Estimated
        }
        GeometrySource::Control | GeometrySource::Pointer => GeometryConfidence::Fallback,
        _ => GeometryConfidence::Exact,
    }
}

fn geometry_stamp(
    anchor: crate::focus::PopupAnchor,
    observed_at: Instant,
    sequence: u64,
) -> GeometryStamp {
    let source = geometry_source(anchor.source);
    GeometryStamp {
        source,
        confidence: geometry_confidence(source),
        observed_at,
        sequence,
        context_epoch: 0,
    }
}

fn tsf_unavailable_invalidates_candidate(reason: u32) -> bool {
    // Loading, late delivery, edit-lock and callback-capacity failures are
    // transient. All other unavailable reasons prove that the previous caret
    // is no longer safe to display for the current target/context.
    !matches!(reason, 13 | 21 | 22 | 23 | 24 | 28)
}

fn tsf_reason_blocks_geometry(reason: u32) -> bool {
    // TSF explicitly reports sensitivity and editability before it can
    // expose a caret.  Legacy/UIA rectangles must not be used as a fallback
    // after one of these safety decisions for the same target.
    matches!(reason, 10 | 11 | 12)
}

fn tsf_view_is_live(identity: GeometryIdentity, view_window: u64) -> bool {
    let view = view_window as usize as HWND;
    if view.is_null() {
        return false;
    }
    unsafe {
        let mut process = 0_u32;
        let thread = GetWindowThreadProcessId(view, &mut process);
        if thread == 0
            || process != identity.process
            || thread != identity.input_thread
            || crate::windows_impl::process_started_at(process) != Some(identity.process_started)
            || GetAncestor(view, GA_ROOT) as isize != identity.root_window
            || GetForegroundWindow() as isize != identity.root_window
        {
            return false;
        }
        let mut root_process = 0_u32;
        if GetWindowThreadProcessId(identity.root_window as HWND, &mut root_process) == 0
            || root_process == 0
            || root_process != identity.root_process
            || crate::windows_impl::process_started_at(root_process) != Some(identity.root_started)
        {
            return false;
        }
        let mut gui = std::mem::zeroed::<GUITHREADINFO>();
        gui.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        GetGUIThreadInfo(identity.input_thread, &mut gui) != 0 && gui.hwndFocus == view
    }
}

fn select_legacy_geometry(
    identity: GeometryIdentity,
    legacy: Option<(crate::focus::PopupAnchor, GeometryStamp)>,
    now: Instant,
) -> Option<(crate::focus::PopupAnchor, GeometryStamp)> {
    let (legacy_anchor, legacy_stamp) = legacy?;
    if !legacy_geometry_allowed(legacy_stamp.source)
        || !geometry_is_fresh(legacy_stamp.observed_at, now)
        || geometry_source(legacy_anchor.source) != legacy_stamp.source
    {
        return None;
    }

    // Pointer and explicit IME exclusion anchors are intentionally outside
    // the normal source arbitration order. They still share the same
    // freshness/identity gate; all caret-like providers go through the full
    // geometry validator so a whole Control rectangle cannot leak through.
    if legacy_stamp.source == GeometrySource::ImmExclusion {
        let candidate = candidate_from_anchor(
            identity,
            legacy_anchor,
            legacy_stamp.observed_at,
            legacy_stamp.sequence,
        );
        if validate_geometry(&candidate, identity, now, legacy_stamp.context_epoch).is_some() {
            return None;
        }
        return Some((legacy_anchor, legacy_stamp));
    }

    let selected = arbitrate_geometry(
        identity,
        [candidate_from_anchor(
            identity,
            legacy_anchor,
            legacy_stamp.observed_at,
            legacy_stamp.sequence,
        )],
        now,
        legacy_stamp.context_epoch,
    )?;
    Some((
        crate::focus::PopupAnchor {
            geometry: selected.geometry,
            source: anchor_source(selected.source),
        },
        GeometryStamp {
            source: selected.source,
            confidence: selected.confidence,
            observed_at: selected.observed_at,
            sequence: selected.sequence,
            context_epoch: selected.context_epoch,
        },
    ))
}

fn candidate_from_tsf(
    reply: crate::caret::GeometryReply,
    identity: GeometryIdentity,
    _fallback_geometry: echo_engine::InputTargetGeometry,
    now_tick: u64,
) -> Option<GeometryCandidate> {
    if reply.status != crate::caret::ReplyStatus::Ready || !reply.accepted {
        return None;
    }
    let words = reply.response_words;
    if words[2] != 1 || words[3] != 1 || words[27] != 2 {
        return None;
    }
    let view_window = u64::from(words[12]) | (u64::from(words[13]) << 32);
    if !tsf_view_is_live(identity, view_window) || words[28] != 1 {
        return None;
    }
    let observed_tick = u64::from(words[8]) | (u64::from(words[9]) << 32);
    if observed_tick > now_tick {
        return None;
    }
    let observed_at =
        Instant::now().checked_sub(Duration::from_millis(now_tick - observed_tick))?;
    let left = words[18] as i32;
    let top = words[19] as i32;
    let right = words[20] as i32;
    let bottom = words[21] as i32;
    let rect = echo_engine::PhysicalRect {
        x: left,
        y: top,
        width: right.checked_sub(left)?,
        height: bottom.checked_sub(top)?,
    };
    // The TSF observer publishes a physical screen rectangle.  Its request
    // runs in the target thread's DPI context, so a target supplied DPI is
    // not a reliable badge scale (system-aware and unaware targets are
    // virtualized differently).  Resolve both the monitor work-area and the
    // effective DPI on Echo's per-monitor-aware host from the new caret
    // rectangle itself.  This also handles a caret crossing onto another
    // monitor without retaining the legacy anchor's monitor metadata.
    let host_geometry = crate::focus::geometry(rect);
    (rect.width >= 0 && rect.height > 0).then_some(GeometryCandidate::exact(
        identity,
        GeometrySource::TsfCaret,
        echo_engine::InputTargetGeometry {
            target: rect,
            work_area: host_geometry.work_area,
            dpi: host_geometry.dpi,
        },
        observed_at,
        u64::from(reply.request_sequence),
        u64::from(words[10]) | (u64::from(words[11]) << 32),
    ))
}

fn select_primary_geometry(
    identity: GeometryIdentity,
    legacy: Option<(crate::focus::PopupAnchor, GeometryStamp)>,
    tsf: Option<GeometryCandidate>,
    now: Instant,
) -> Option<GeometryCandidate> {
    match (legacy, tsf) {
        (Some((legacy_anchor, legacy_stamp)), Some(tsf)) => arbitrate_geometry(
            identity,
            [
                candidate_from_anchor(
                    identity,
                    legacy_anchor,
                    legacy_stamp.observed_at,
                    legacy_stamp.sequence,
                ),
                tsf,
            ],
            now,
            tsf.context_epoch,
        ),
        (Some((legacy_anchor, legacy_stamp)), None) => arbitrate_geometry(
            identity,
            [candidate_from_anchor(
                identity,
                legacy_anchor,
                legacy_stamp.observed_at,
                legacy_stamp.sequence,
            )],
            now,
            0,
        ),
        // TSF-only admission is valid. A missing legacy rectangle must not
        // turn a current, positively admitted target into no target.
        (None, Some(tsf)) => arbitrate_geometry(identity, [tsf], now, tsf.context_epoch),
        (None, None) => None,
    }
}

#[derive(Clone)]
struct AdmittedTarget {
    snapshot: FocusSnapshot,
    target: (
        Option<echo_engine::PasteControlIdentity>,
        Option<crate::focus::InputStatusEndpoint>,
    ),
}

fn close_observer_until_drained(observer: &mut crate::caret::CaretObserver, now_tick: u64) -> bool {
    observer.close();
    observer.try_read(now_tick).is_some_and(|reply| {
        reply.status == crate::caret::ReplyStatus::Closed && reply.response_words[30] == 0
    }) || observer.target_exited()
}

fn retire_observer(
    observer: &mut Option<crate::caret::CaretObserver>,
    retired: &mut Vec<crate::caret::CaretObserver>,
) {
    if let Some(mut observer) = observer.take() {
        observer.close();
        retired.push(observer);
    }
}

fn drain_retired_observers(retired: &mut Vec<crate::caret::CaretObserver>, now_tick: u64) {
    retired.retain_mut(|observer| !close_observer_until_drained(observer, now_tick));
}

/// Close every remaining TSF observer before the worker thread releases its
/// mailbox mappings.  A target publishes CLOSED before its final callback
/// Release, so teardown must pump the scheduler messages until the same
/// snapshot reports `outstanding_callbacks == 0`.
unsafe fn shutdown_caret_observers(
    current: &mut Option<crate::caret::CaretObserver>,
    retired: &mut Vec<crate::caret::CaretObserver>,
    msg: &mut MSG,
) {
    if let Some(observer) = current.as_mut() {
        observer.close();
    }
    for observer in retired.iter_mut() {
        observer.close();
    }
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        while PeekMessageW(msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(msg);
            DispatchMessageW(msg);
        }
        let now_tick = GetTickCount64();
        if current
            .as_mut()
            .is_some_and(|observer| close_observer_until_drained(observer, now_tick))
        {
            *current = None;
        }
        drain_retired_observers(retired, now_tick);
        if current.is_none() && retired.is_empty() {
            break;
        }
        if Instant::now() >= deadline {
            // Do not hang Quit on an unresponsive target. Runtime owns a
            // separate target-side mapping and the observer module is pinned;
            // dropping the host mapping/hook cannot invalidate an in-flight
            // callback's view. No successful drain is inferred on this path.
            break;
        }
        MsgWaitForMultipleObjectsEx(0, null_mut(), 10, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
    }
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
    // Admission is the security/identity decision. It survives a transient
    // legacy/UIA geometry miss so the independent TSF source can continue to
    // prove a caret for the same verified target.
    let mut admitted_target: Option<AdmittedTarget> = None;
    let mut observer = None;
    let mut caret_observer: Option<crate::caret::CaretObserver> = None;
    let mut retired_caret_observers = Vec::new();
    let mut tsf_candidate: Option<GeometryCandidate> = None;
    let mut tsf_safety_blocked = false;
    let mut tsf_diagnostic: Option<TsfDiagnostic> = None;
    let mut tsf_backoff = false;
    let mut tsf_request_pending = false;
    // The legacy/native/UIA geometry clock is updated only when that source
    // actually returns a new rectangle. Mode ticks reuse this stamp verbatim.
    let mut legacy_geometry: Option<(crate::focus::PopupAnchor, GeometryStamp)> = None;
    // Hosted XAML providers can block for a second between calls. Geometry
    // refresh must not block the independent, fresh IME mode samples.
    let mut pending: Option<(
        FocusSnapshot,
        Option<crate::focus::InputStatusEndpoint>,
        std::sync::mpsc::Receiver<Option<crate::focus::automation::Probe>>,
    )> = None;
    let mut geometry_probe = GeometryProbeState::new();
    let mut next = Some(Instant::now());
    let mut failures = 0u32;
    let mut candidate_visible = false;
    let owned_test_target = native_test_target();
    while !s.stopped.load(Ordering::Acquire) {
        while PeekMessageW(&mut msg, null_mut(), 0, 0, PM_REMOVE) != 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        drain_retired_observers(&mut retired_caret_observers, GetTickCount64());
        let enabled = s.enabled.load(Ordering::Acquire);
        if !enabled {
            hooks = None;
            s.console_root.store(0, Ordering::Release);
            cached = None;
            admitted_target = None;
            observer = None;
            tsf_candidate = None;
            tsf_safety_blocked = false;
            legacy_geometry = None;
            tsf_backoff = false;
            tsf_request_pending = false;
            pending = None;
            geometry_probe.reset();
            next = (caret_observer.is_some() || !retired_caret_observers.is_empty())
                .then(|| Instant::now() + Duration::from_millis(50));
            // Keep a closed observer alive while its target-side callback
            // leases drain.  `CLOSED` is published before the final COM
            // Release, so dropping the mapping here would lose the only
            // post-stop outstanding=0 evidence.
            if let Some(caret) = caret_observer.as_mut() {
                let now_tick = GetTickCount64();
                caret.close();
                if let Some(reply) = caret.try_read(now_tick) {
                    let terminal = reply.status == crate::caret::ReplyStatus::Closed;
                    let drained = reply.response_words[30] == 0;
                    tsf_diagnostic = Some(TsfDiagnostic::from_reply(reply, now_tick));
                    let generation = s.generation.load(Ordering::Acquire);
                    (s.post)(Update {
                        generation,
                        observation: Observation::Unavailable {
                            reason: UnavailableReason::SettingsDisabled,
                            process_name: None,
                            trigger: InvalidationTrigger::SettingsDisable,
                            elapsed_ms: 0,
                        },
                        tsf: tsf_diagnostic,
                    });
                    if terminal && drained {
                        caret_observer = None;
                    }
                }
            }
        } else if hooks.is_none() {
            hooks = Some(Hooks::install());
            s.dirty.fetch_or(FOCUS, Ordering::Release);
        }
        let dirty = s.dirty.swap(0, Ordering::AcqRel);
        if enabled && dirty & FOCUS != 0 {
            s.console_root.store(0, Ordering::Release);
            cached = None;
            // Keep the last verified target identity across a focus signal.
            // The signal can be emitted while the same editor is processing
            // its focus notification; clearing admission here makes the
            // following sample look like an unrelated target and causes a
            // needless hide/show cycle.  Geometry is still discarded and
            // must be revalidated below before it is published again.
            observer = None;
            retire_observer(&mut caret_observer, &mut retired_caret_observers);
            tsf_candidate = None;
            tsf_safety_blocked = false;
            tsf_diagnostic = None;
            legacy_geometry = None;
            tsf_backoff = false;
            tsf_request_pending = false;
            pending = None;
            geometry_probe.reset();
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
            if let Some((owned_pid, owned_hwnd)) = owned_test_target {
                if snapshot.process_id != owned_pid || snapshot.window_id != owned_hwnd {
                    // Never let the acceptance observer follow the foreground
                    // into a browser/editor after the fixture loses focus.
                    cached = None;
                    admitted_target = None;
                    observer = None;
                    pending = None;
                    tsf_candidate = None;
                    tsf_safety_blocked = false;
                    legacy_geometry = None;
                    tsf_backoff = false;
                    tsf_request_pending = false;
                    geometry_probe.reset();
                    if let Some(caret) = caret_observer.as_mut() {
                        caret.close();
                        if let Some(reply) = caret.try_read(GetTickCount64()) {
                            if reply.status == crate::caret::ReplyStatus::Closed
                                && reply.response_words[30] == 0
                            {
                                caret_observer = None;
                            }
                        }
                    }
                    let generation = s.generation.load(Ordering::Acquire);
                    (s.post)(Update {
                        generation,
                        observation: Observation::Unavailable {
                            reason: UnavailableReason::NoEditableTarget,
                            process_name: None,
                            trigger: InvalidationTrigger::Foreground,
                            elapsed_ms: 0,
                        },
                        tsf: None,
                    });
                    next = Some(Instant::now() + Duration::from_millis(100));
                    continue;
                }
            }
            let current_endpoint = (
                None,
                snapshot
                    .indicator_endpoint()
                    .or_else(|| snapshot.input_endpoint()),
            );
            let mut target_changed_this_sample = false;
            let had_target_identity =
                admitted_target.is_some() || cached.is_some() || pending.is_some();
            let same_admitted = admitted_target.as_ref().is_some_and(|admitted| {
                // `current()` can be false for one GUI-thread sample while a
                // native editor is processing the same focus notification.
                // The new foreground snapshot and endpoint below are the
                // authoritative identity check; requiring the old snapshot
                // to revalidate first turns that transient into a hide/show
                // cycle for every ordinary input box.
                !target_endpoint_changed(
                    &admitted.snapshot,
                    &admitted.target,
                    &snapshot,
                    &current_endpoint,
                )
            });
            // A hosted query can be pending before it has produced the first
            // positive admission. Treat its frozen snapshot/endpoint as the
            // current identity so the 50ms polling loop does not retire and
            // recreate TSF observers while that one query is still in flight.
            let same_pending = pending.as_ref().is_some_and(|(old, endpoint, _)| {
                !target_endpoint_changed(old, &(None, *endpoint), &snapshot, &current_endpoint)
            });
            let same = same_admitted || (admitted_target.is_none() && same_pending);
            if !same && had_target_identity {
                target_changed_this_sample = true;
                cached = None;
                admitted_target = None;
                observer = None;
                retire_observer(&mut caret_observer, &mut retired_caret_observers);
                tsf_candidate = None;
                tsf_safety_blocked = false;
                tsf_diagnostic = None;
                legacy_geometry = None;
                tsf_backoff = false;
                tsf_request_pending = false;
                geometry_probe.reset();
            }
            if pending.as_ref().is_some_and(|(old, endpoint, _)| {
                !old.current()
                    || target_endpoint_changed(
                        old,
                        &(None, *endpoint),
                        &snapshot,
                        &current_endpoint,
                    )
            }) {
                pending = None;
            }
            let mut hosted_disconnected = false;
            let completed =
                pending
                    .as_ref()
                    .and_then(|(_, _, response)| match response.try_recv() {
                        Ok(value) => Some(value),
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            hosted_disconnected = true;
                            Some(None)
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => None,
                    });
            let hosted = snapshot.is_hosted_input();
            let needs_probe =
                geometry_probe.due(Instant::now(), cached.is_none() || dirty & GEOMETRY != 0);
            if hosted && needs_probe && pending.is_none() && completed.is_none() {
                s.probes.fetch_add(1, Ordering::Relaxed);
                geometry_probe.mark_attempt(Instant::now());
                pending = crate::focus::automation::begin_query(snapshot.clone(), true)
                    .map(|response| (snapshot.clone(), current_endpoint.1, response));
            }
            if completed.is_some() || !hosted && needs_probe {
                geometry_probe.mark_attempt(Instant::now());
                if !hosted {
                    s.probes.fetch_add(1, Ordering::Relaxed);
                }
                let observed = if hosted {
                    let endpoint = pending.take().and_then(|(_, endpoint, _)| endpoint);
                    completed.flatten().and_then(|probe| {
                        let (rect, source) = probe.anchor?;
                        (source != AnchorSource::Window).then(|| {
                            (
                                (Some(probe.identity), endpoint),
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
                };
                if let Some((target, anchor)) = observed {
                    let target_changed = cached.as_ref().is_some_and(|(old_snapshot, old, _)| {
                        target_identity_changed(old_snapshot, old, &snapshot, &target)
                    }) || admitted_target.as_ref().is_some_and(|admitted| {
                        target_identity_changed(
                            &admitted.snapshot,
                            &admitted.target,
                            &snapshot,
                            &target,
                        )
                    });
                    if target_changed {
                        target_changed_this_sample = true;
                        observer = None;
                        retire_observer(&mut caret_observer, &mut retired_caret_observers);
                        tsf_candidate = None;
                        tsf_safety_blocked = false;
                        tsf_request_pending = false;
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
                    admitted_target = Some(AdmittedTarget {
                        snapshot: snapshot.clone(),
                        target: target.clone(),
                    });
                    let sequence = s.samples.load(Ordering::Relaxed);
                    let observed_at = Instant::now();
                    let stamp = geometry_stamp(anchor, observed_at, sequence);
                    let validated =
                        geometry_identity(generation, &snapshot, target.1).and_then(|identity| {
                            select_legacy_geometry(identity, Some((anchor, stamp)), observed_at)
                        });
                    if let Some((validated_anchor, validated_stamp)) = validated {
                        cached = Some((snapshot.clone(), target.clone(), validated_anchor));
                        legacy_geometry = Some((validated_anchor, validated_stamp));
                    } else {
                        // A failed probe is a current safety result.  Keep
                        // target admission for the independent TSF path, but
                        // never fall back to the previous caret/control
                        // rectangle for this same target.
                        cached = None;
                        legacy_geometry = None;
                    }
                } else {
                    cached = None;
                    legacy_geometry = None;
                    if provider == CaretProvider::Legacy {
                        admitted_target = None;
                        observer = None;
                        retire_observer(&mut caret_observer, &mut retired_caret_observers);
                        tsf_candidate = None;
                        tsf_safety_blocked = false;
                        tsf_diagnostic = None;
                        tsf_request_pending = false;
                    } else if let Some(endpoint) = snapshot
                        .indicator_endpoint()
                        .or_else(|| snapshot.input_endpoint())
                    {
                        // A positive, current input endpoint is enough to keep
                        // passive TSF admission alive when legacy geometry is
                        // unavailable. Sensitivity and target ownership are
                        // still rechecked by the TSF callback itself.
                        admitted_target = Some(AdmittedTarget {
                            snapshot: snapshot.clone(),
                            target: (None, Some(endpoint)),
                        });
                    } else {
                        admitted_target = None;
                        observer = None;
                        retire_observer(&mut caret_observer, &mut retired_caret_observers);
                        tsf_candidate = None;
                        tsf_safety_blocked = false;
                        tsf_diagnostic = None;
                        tsf_request_pending = false;
                    }
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
            let mut tsf_candidate_refreshed = false;
            if provider != CaretProvider::Legacy {
                // Passive target admission is independent of a successful
                // legacy/UIA rectangle. This lets the new TSF source prove a
                // caret even when the old geometry provider is unavailable.
                let admission_snapshot = admitted_target
                    .as_ref()
                    .map(|admitted| &admitted.snapshot)
                    .unwrap_or(&snapshot);
                let admission_endpoint = admitted_target
                    .as_ref()
                    .and_then(|admitted| admitted.target.1)
                    .or_else(|| snapshot.indicator_endpoint())
                    .or_else(|| snapshot.input_endpoint());
                if let Some(endpoint) =
                    caret_endpoint(admission_snapshot, admission_endpoint, generation)
                {
                    let endpoint_matches = caret_observer.as_ref().is_some_and(|current| {
                        let actual = current.endpoint();
                        actual.input_window == endpoint.input_window
                            && actual.input_process == endpoint.input_process
                            && actual.input_started == endpoint.input_started
                    });
                    if crate::caret::CaretObserver::endpoint_is_blocked(endpoint) {
                        // WeChat/Codex/ChatGPT can expose a safe passive
                        // provider while their TSF threads remain unsafe for
                        // an injected observer.  Keep the provider geometry
                        // alive and do not repeatedly construct a blocked
                        // mapping/hook attempt.
                        if caret_observer.is_some() {
                            retire_observer(&mut caret_observer, &mut retired_caret_observers);
                        }
                        tsf_backoff = true;
                        tsf_request_pending = false;
                    } else if !endpoint_matches {
                        retire_observer(&mut caret_observer, &mut retired_caret_observers);
                        // Keep at most one retired observer alive.  A new
                        // target must wait for CLOSED+outstanding=0 before
                        // another injected mapping/hook is created.
                        if retired_caret_observers.is_empty() {
                            caret_observer = match crate::caret::CaretObserver::start(endpoint) {
                                Ok(observer) => {
                                    tsf_diagnostic = None;
                                    tsf_backoff = false;
                                    Some(observer)
                                }
                                Err(error) => {
                                    tsf_diagnostic = Some(TsfDiagnostic {
                                        state: TsfDiagnosticState::BootstrapFailed,
                                        api_hresult: None,
                                        session_hresult: None,
                                        reason_code: Some(bootstrap_reason_code(&error)),
                                        sequence: 0,
                                        context_epoch: 0,
                                        raw_rect: None,
                                        normalized_rect: None,
                                        age_ms: None,
                                        pending_callbacks: 0,
                                        outstanding_callbacks: 0,
                                        created_callbacks: 0,
                                        released_callbacks: 0,
                                        callback_high_water: 0,
                                        pending_high_water: 0,
                                        api_requests: 0,
                                        request_edit_calls: 0,
                                        accepted_sessions: 0,
                                        callback_entered: 0,
                                        callback_completed: 0,
                                        final_released: 0,
                                        cancelled: 0,
                                        timed_out: 0,
                                        ready_results: 0,
                                        mock_sessions: 0,
                                        mock_callback_entered: 0,
                                        mock_callback_completed: 0,
                                        mock_final_released: 0,
                                        final_source: None,
                                        fallback_reason: Some(bootstrap_reason_code(&error)),
                                    });
                                    None
                                }
                            };
                        }
                        tsf_candidate = None;
                        tsf_safety_blocked = false;
                        tsf_request_pending = false;
                    } else if let Some(caret) = caret_observer.as_mut() {
                        caret.rebind_focus_generation(endpoint.focus_generation);
                    }
                    let mut observer_closed = false;
                    let mut target_closed = false;
                    let mut observer_drained = false;
                    if let Some(caret) = caret_observer.as_mut() {
                        let now_tick = GetTickCount64();
                        caret.heartbeat(now_tick);
                        if let Some(reply) = caret.try_read(now_tick) {
                            tsf_diagnostic = Some(TsfDiagnostic::from_reply(reply, now_tick));
                            tsf_request_pending = false;
                            if reply.response_words[30]
                                >= crate::caret::scheduler::SessionState::MAX_CALLBACKS
                            {
                                tsf_backoff = true;
                            } else {
                                tsf_backoff = false;
                            }
                            if reply.status == crate::caret::ReplyStatus::Ready {
                                if !reply.accepted {
                                    // A normal deadline/late response is not
                                    // an identity change. Keep the candidate
                                    // until its own geometry TTL expires;
                                    // clear it only when the observer proves a
                                    // target/context/session invalidation.
                                    if reply.identity_invalidated {
                                        tsf_candidate = None;
                                    }
                                } else {
                                    let candidate_geometry = cached
                                        .as_ref()
                                        .map(|(_, _, anchor)| anchor.geometry)
                                        .unwrap_or(snapshot.anchor.geometry);
                                    let candidate = geometry_identity(
                                        generation,
                                        admission_snapshot,
                                        admission_endpoint,
                                    )
                                    .and_then(|identity| {
                                        candidate_from_tsf(
                                            reply,
                                            identity,
                                            candidate_geometry,
                                            now_tick,
                                        )
                                    });
                                    // A malformed or stale Ready reply is not a
                                    // target/context/session change. Keep the last
                                    // valid candidate until its normal TTL expires
                                    // instead of replacing it with invalid data.
                                    if let Some(candidate) = candidate.filter(|candidate| {
                                        geometry_is_fresh(candidate.observed_at, Instant::now())
                                    }) {
                                        tsf_candidate = Some(candidate);
                                        tsf_safety_blocked = false;
                                        tsf_candidate_refreshed = true;
                                    }
                                }
                            } else if reply.status == crate::caret::ReplyStatus::Closed {
                                // CLOSED is published before the target-side
                                // callback leases are finally released. Keep
                                // the mapping alive until the same snapshot
                                // reports outstanding=0, but never acquire a
                                // new request after the target has closed.
                                target_closed = true;
                                observer_closed = reply.response_words[30] == 0;
                                observer_drained = observer_closed;
                                tsf_candidate = None;
                                tsf_request_pending = false;
                            } else if reply.status == crate::caret::ReplyStatus::Unavailable {
                                let reason = reply.response_words[4];
                                if reply.identity_invalidated
                                    || tsf_unavailable_invalidates_candidate(reason)
                                {
                                    // Safety, view, selection, DPI, context
                                    // and session failures invalidate the old
                                    // caret. Only explicitly transient
                                    // reasons retain it for its own TTL.
                                    tsf_candidate = None;
                                }
                                if tsf_reason_blocks_geometry(reason) {
                                    tsf_safety_blocked = true;
                                    tsf_candidate = None;
                                }
                            }
                        }
                        if !observer_closed && !target_closed {
                            let request = caret.try_request(now_tick, dirty);
                            if matches!(request, crate::caret::scheduler::RequestDecision::Backoff)
                            {
                                // A cap or request-rate backoff is a transient
                                // scheduler condition. Keep the last ready
                                // candidate and slow polling until the target
                                // reports that callback capacity is available.
                                tsf_backoff = true;
                            }
                            if matches!(
                                request,
                                crate::caret::scheduler::RequestDecision::Sent(_)
                                    | crate::caret::scheduler::RequestDecision::Pending
                                    | crate::caret::scheduler::RequestDecision::Coalesced
                            ) {
                                tsf_request_pending = true;
                            } else if matches!(
                                request,
                                crate::caret::scheduler::RequestDecision::Backoff
                                    | crate::caret::scheduler::RequestDecision::Closed
                                    | crate::caret::scheduler::RequestDecision::Unavailable
                            ) {
                                tsf_request_pending = false;
                            }
                            if matches!(
                                request,
                                crate::caret::scheduler::RequestDecision::Closed
                                    | crate::caret::scheduler::RequestDecision::Unavailable
                            ) {
                                observer_closed = true;
                                tsf_candidate = None;
                                tsf_request_pending = false;
                            }
                            if matches!(
                                request,
                                crate::caret::scheduler::RequestDecision::Sent(_)
                                    | crate::caret::scheduler::RequestDecision::Pending
                            ) && tsf_diagnostic.is_none()
                            {
                                tsf_diagnostic = Some(TsfDiagnostic {
                                    state: TsfDiagnosticState::Pending,
                                    api_hresult: None,
                                    session_hresult: None,
                                    reason_code: None,
                                    sequence: 0,
                                    context_epoch: 0,
                                    raw_rect: None,
                                    normalized_rect: None,
                                    age_ms: None,
                                    pending_callbacks: 0,
                                    outstanding_callbacks: 0,
                                    created_callbacks: 0,
                                    released_callbacks: 0,
                                    callback_high_water: 0,
                                    pending_high_water: 0,
                                    api_requests: 0,
                                    request_edit_calls: 0,
                                    accepted_sessions: 0,
                                    callback_entered: 0,
                                    callback_completed: 0,
                                    final_released: 0,
                                    cancelled: 0,
                                    timed_out: 0,
                                    ready_results: 0,
                                    mock_sessions: 0,
                                    mock_callback_entered: 0,
                                    mock_callback_completed: 0,
                                    mock_final_released: 0,
                                    final_source: None,
                                    fallback_reason: None,
                                });
                            }
                        }
                    }
                    if observer_closed {
                        if observer_drained {
                            caret_observer = None;
                        } else {
                            retire_observer(&mut caret_observer, &mut retired_caret_observers);
                        }
                    }
                }
            }
            let probe_started = Instant::now();
            let process_name = process_basename(snapshot.process_id);
            let update_trigger = if target_changed_this_sample {
                InvalidationTrigger::TargetChanged
            } else {
                probe_trigger
            };
            let mut unavailable_reason = UnavailableReason::NoEditableTarget;
            let sample = admitted_target.as_ref().and_then(|admitted| {
                if tsf_safety_blocked {
                    unavailable_reason = UnavailableReason::UnsafeGeometry;
                    return None;
                }
                if pending.is_some() || (tsf_request_pending && !tsf_candidate_refreshed) {
                    unavailable_reason = UnavailableReason::NoEditableTarget;
                    return None;
                }
                let target = &admitted.target;
                let status_endpoint = target
                    .1
                    .or_else(|| admitted.snapshot.indicator_endpoint())
                    .or_else(|| admitted.snapshot.input_endpoint());
                let thread = GetWindowThreadProcessId(
                    status_endpoint.map_or(snapshot.focused_handle, |t| t.window) as HWND,
                    null_mut(),
                );
                let layout = GetKeyboardLayout(thread);
                let english_keyboard = layout as usize & 0x3ff == 0x09 && ImmIsIME(layout) == 0;
                let state = if english_keyboard {
                    Some((InputMode::English, CompositionState::Idle))
                } else {
                    if observer.is_none() {
                        observer = if let Some(endpoint) = status_endpoint {
                            Observer::status_at(endpoint)
                        } else {
                            Err("Input owner unavailable".to_string())
                        }
                        .ok();
                    }
                    observer
                        .as_ref()
                        .and_then(Observer::input_state)
                        .or_else(|| {
                            status_endpoint
                                .and_then(crate::ime_observer::Observer::status_from_ime_window)
                        })
                };
                let Some((mode, composition)) = state else {
                    unavailable_reason = UnavailableReason::ObserverUnavailable;
                    return None;
                };
                if mode == InputMode::Unknown {
                    unavailable_reason = UnavailableReason::ModeUnavailable;
                    return None;
                }
                if !snapshot.current() || s.generation.load(Ordering::Acquire) != generation {
                    return None;
                }
                let now = Instant::now();
                let legacy = legacy_geometry
                    .as_ref()
                    .map(|(anchor, stamp)| (*anchor, *stamp));
                let (anchor, geometry_stamp) = if provider == CaretProvider::Primary {
                    let Some(identity) = geometry_identity(generation, &snapshot, target.1) else {
                        unavailable_reason = UnavailableReason::NoEditableTarget;
                        return None;
                    };
                    let selected = select_primary_geometry(identity, legacy, tsf_candidate, now);
                    let Some(selected) = selected else {
                        unavailable_reason = UnavailableReason::NoEditableTarget;
                        return None;
                    };
                    let source = anchor_source(selected.source);
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
                    let Some((legacy_anchor, legacy_stamp)) =
                        geometry_identity(generation, &snapshot, target.1)
                            .and_then(|identity| select_legacy_geometry(identity, legacy, now))
                    else {
                        unavailable_reason = UnavailableReason::NoEditableTarget;
                        return None;
                    };
                    (legacy_anchor, legacy_stamp)
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
                if let Some(diagnostic) = tsf_diagnostic.as_mut() {
                    diagnostic.final_source =
                        sample.as_ref().map(|value| value.geometry_stamp.source);
                    if diagnostic.final_source != Some(GeometrySource::TsfCaret)
                        && diagnostic.state == TsfDiagnosticState::Ready
                    {
                        diagnostic.fallback_reason = diagnostic.reason_code.or(Some(0));
                    }
                }
                let elapsed_ms = probe_started.elapsed().as_millis().min(u64::MAX as u128) as u64;
                let observation = if let Some(sample) = sample {
                    Observation::Observed {
                        sample,
                        process_name,
                        trigger: update_trigger,
                        elapsed_ms,
                    }
                } else if tsf_safety_blocked {
                    Observation::Unavailable {
                        reason: UnavailableReason::UnsafeGeometry,
                        process_name,
                        trigger: update_trigger,
                        elapsed_ms,
                    }
                } else if !snapshot.current() {
                    Observation::Revalidating {
                        trigger: InvalidationTrigger::SnapshotSuperseded,
                    }
                } else if pending.is_some() {
                    Observation::Revalidating {
                        trigger: if target_changed_this_sample {
                            InvalidationTrigger::TargetChanged
                        } else {
                            InvalidationTrigger::HostedProbePending
                        },
                    }
                } else if tsf_request_pending {
                    Observation::Revalidating {
                        trigger: update_trigger,
                    }
                } else {
                    Observation::Unavailable {
                        reason: if hosted_disconnected {
                            UnavailableReason::HostedProbeDisconnected
                        } else {
                            unavailable_reason
                        },
                        process_name,
                        trigger: update_trigger,
                        elapsed_ms,
                    }
                };
                (s.post)(Update {
                    generation,
                    observation,
                    tsf: tsf_diagnostic,
                });
            }
            let cross_process = snapshot
                .indicator_endpoint()
                .or_else(|| snapshot.input_endpoint())
                .is_some_and(|input| input.process != snapshot.process_id);
            let tsf_active = provider != CaretProvider::Legacy
                && admitted_target.is_some()
                && caret_observer.is_some();
            let continuation =
                valid || pending.is_some() || hosted && admitted_target.is_some() || tsf_active;
            if continuation {
                failures = 0;
            } else {
                failures = failures.saturating_add(1);
            }
            next = next_poll_delay(
                provider,
                valid,
                pending.is_some(),
                hosted && admitted_target.is_some(),
                tsf_active,
                tsf_backoff,
                admitted_target.is_some(),
                failures,
                cross_process,
            )
            .map(|delay| Instant::now() + delay);
            if !retired_caret_observers.is_empty() {
                next = Some(Instant::now() + Duration::from_millis(50));
            }
            if next.is_none() {
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
    shutdown_caret_observers(&mut caret_observer, &mut retired_caret_observers, &mut msg);
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

/// Fast UI-thread guard; no COM or process-open calls. The worker validates
/// the sample's process instance before publishing it.
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

    #[test]
    fn primary_keeps_tsf_candidate_when_legacy_geometry_is_absent() {
        let now = Instant::now();
        let identity = GeometryIdentity {
            generation: 4,
            process: 10,
            process_started: 20,
            input_thread: 30,
            root_process: 10,
            root_started: 20,
            root_window: 40,
            focused_window: 41,
        };
        let geometry = echo_engine::InputTargetGeometry {
            target: echo_engine::PhysicalRect {
                x: 100,
                y: 200,
                width: 1,
                height: 20,
            },
            work_area: echo_engine::PhysicalRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            dpi: 96,
        };
        let candidate =
            GeometryCandidate::exact(identity, GeometrySource::TsfCaret, geometry, now, 1, 2);
        let selected = select_primary_geometry(identity, None, Some(candidate), now)
            .expect("TSF-only admission must remain selectable");
        assert_eq!(selected.source, GeometrySource::TsfCaret);
    }

    #[test]
    fn legacy_rejects_whole_control_fallback_but_keeps_explicit_special_paths() {
        assert!(!legacy_geometry_allowed(GeometrySource::Control));
        assert!(!legacy_geometry_allowed(GeometrySource::Pointer));
        assert!(legacy_geometry_allowed(GeometrySource::ImmExclusion));
        assert!(legacy_geometry_allowed(GeometrySource::UiaCaret));
    }

    #[test]
    fn wechat_editor_leading_edge_is_estimated_but_not_a_control_fallback() {
        let now = Instant::now();
        let identity = GeometryIdentity {
            generation: 4,
            process: 10,
            process_started: 20,
            input_thread: 30,
            root_process: 10,
            root_started: 20,
            root_window: 40,
            focused_window: 41,
        };
        let geometry = echo_engine::InputTargetGeometry {
            target: echo_engine::PhysicalRect {
                x: 100,
                y: 200,
                width: 1,
                height: 24,
            },
            work_area: echo_engine::PhysicalRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            dpi: 96,
        };
        let anchor = crate::focus::PopupAnchor {
            geometry,
            source: AnchorSource::EditorLeadingEdge,
        };
        let stamp = GeometryStamp {
            source: GeometrySource::EditorLeadingEdge,
            confidence: GeometryConfidence::Estimated,
            observed_at: now,
            sequence: 1,
            context_epoch: 0,
        };
        assert!(select_legacy_geometry(identity, Some((anchor, stamp)), now).is_some());
        assert_eq!(
            geometry_source(anchor.source),
            GeometrySource::EditorLeadingEdge
        );
        assert_eq!(
            geometry_confidence(GeometrySource::EditorLeadingEdge),
            GeometryConfidence::Estimated
        );
        assert!(legacy_geometry_allowed(GeometrySource::EditorLeadingEdge));
        assert!(!legacy_geometry_allowed(GeometrySource::Control));
    }

    #[test]
    fn safety_reasons_block_legacy_fallback_until_allowed_tsf_geometry() {
        for reason in [10, 11, 12] {
            assert!(tsf_reason_blocks_geometry(reason));
        }
        for reason in [13, 21, 22, 23, 24, 28] {
            assert!(!tsf_reason_blocks_geometry(reason));
        }
    }

    #[test]
    fn geometry_probe_is_due_before_the_geometry_ttl_expires() {
        let now = Instant::now();
        let mut probe = GeometryProbeState::new();
        assert!(probe.due(now, false));
        probe.mark_attempt(now);
        assert!(!probe.due(now + Duration::from_millis(99), false));
        assert!(probe.due(now + GEOMETRY_REVALIDATION_INTERVAL, false));
        assert!(probe.due(now, true));
    }

    #[test]
    fn legacy_selection_rejects_stale_and_control_geometry() {
        let now = Instant::now();
        let identity = GeometryIdentity {
            generation: 4,
            process: 10,
            process_started: 20,
            input_thread: 30,
            root_process: 10,
            root_started: 20,
            root_window: 40,
            focused_window: 41,
        };
        let geometry = echo_engine::InputTargetGeometry {
            target: echo_engine::PhysicalRect {
                x: 100,
                y: 200,
                width: 1,
                height: 20,
            },
            work_area: echo_engine::PhysicalRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            dpi: 96,
        };
        let anchor = crate::focus::PopupAnchor {
            geometry,
            source: AnchorSource::AutomationCaret,
        };
        let fresh = GeometryStamp {
            source: GeometrySource::UiaCaret,
            confidence: GeometryConfidence::Exact,
            observed_at: now,
            sequence: 1,
            context_epoch: 0,
        };
        assert!(select_legacy_geometry(identity, Some((anchor, fresh)), now).is_some());

        let stale = GeometryStamp {
            observed_at: now - echo_engine::GEOMETRY_TTL - Duration::from_millis(1),
            ..fresh
        };
        assert!(select_legacy_geometry(identity, Some((anchor, stale)), now).is_none());

        let control = GeometryStamp {
            source: GeometrySource::Control,
            ..fresh
        };
        assert!(select_legacy_geometry(identity, Some((anchor, control)), now).is_none());

        let mut wrong_dpi = anchor;
        wrong_dpi.geometry.dpi = 769;
        assert!(select_legacy_geometry(identity, Some((wrong_dpi, fresh)), now).is_none());

        let mut offscreen = anchor;
        offscreen.geometry.target.x = 1920;
        assert!(select_legacy_geometry(identity, Some((offscreen, fresh)), now).is_none());

        let window_anchor = crate::focus::PopupAnchor {
            source: AnchorSource::Window,
            ..anchor
        };
        assert!(select_legacy_geometry(identity, Some((window_anchor, control)), now).is_none());

        let pointer = crate::focus::PopupAnchor {
            source: AnchorSource::Pointer,
            ..anchor
        };
        let pointer_stamp = GeometryStamp {
            source: GeometrySource::Pointer,
            confidence: GeometryConfidence::Fallback,
            ..fresh
        };
        assert!(select_legacy_geometry(identity, Some((pointer, pointer_stamp)), now).is_none());

        let ime_anchor = crate::focus::PopupAnchor {
            source: AnchorSource::InputMethodCaret,
            ..anchor
        };
        let ime_stamp = GeometryStamp {
            source: GeometrySource::ImmExclusion,
            ..fresh
        };
        assert!(select_legacy_geometry(identity, Some((ime_anchor, ime_stamp)), now).is_some());
    }

    #[test]
    fn tsf_only_cold_start_polls_before_the_request_deadline() {
        let delay = next_poll_delay(
            CaretProvider::Primary,
            false,
            false,
            false,
            true,
            false,
            true,
            1,
            false,
        )
        .expect("an admitted TSF observer must remain scheduled");
        assert!(delay <= Duration::from_millis(50));
        assert!(Duration::from_millis(20) + delay < Duration::from_millis(150));
    }

    #[test]
    fn tsf_callback_backoff_slows_polling_without_retiring_the_observer() {
        let delay = next_poll_delay(
            CaretProvider::Primary,
            true,
            false,
            false,
            true,
            true,
            true,
            0,
            true,
        )
        .expect("an admitted observer remains scheduled while callbacks drain");
        assert_eq!(delay, Duration::from_millis(100));
    }
}
