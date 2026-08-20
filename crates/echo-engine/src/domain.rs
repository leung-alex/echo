use std::sync::mpsc::{self, Receiver, RecvError, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlatformError(pub String);

impl std::fmt::Display for PlatformError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for PlatformError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlatformChange {
    Clipboard { sequence: u64 },
}

#[derive(Clone, Default)]
pub struct PlatformChangePublisher {
    subscribers: Arc<Mutex<Vec<Sender<PlatformChange>>>>,
}

impl PlatformChangePublisher {
    pub fn subscribe(&self) -> PlatformChangeSubscription {
        let (sender, receiver) = mpsc::channel();
        self.subscribers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(sender);
        PlatformChangeSubscription { receiver }
    }

    pub fn publish(&self, change: PlatformChange) {
        let mut subscribers = self
            .subscribers
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        subscribers.retain(|sender| sender.send(change.clone()).is_ok());
    }

    pub fn close(&self) {
        self.subscribers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
    }
}

pub struct PlatformChangeSubscription {
    receiver: Receiver<PlatformChange>,
}

impl PlatformChangeSubscription {
    pub fn recv(&self) -> Result<PlatformChange, RecvError> {
        self.receiver.recv()
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<PlatformChange, RecvTimeoutError> {
        self.receiver.recv_timeout(timeout)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardRepresentation {
    pub format: String,
    pub mime_type: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardSnapshot {
    pub sequence: u64,
    pub source: SourceContext,
    pub representations: Vec<ClipboardRepresentation>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SourceContext {
    pub app_name: Option<String>,
    pub executable: Option<String>,
    pub window_title: Option<String>,
    pub is_source_verified: bool,
    pub is_sensitivity_verified: bool,
    pub is_password_input: bool,
    pub is_private_window: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasteControlIdentity {
    NativeWindow { handle: isize, class_name: String },
    AutomationRuntimeId(Vec<i32>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PhysicalRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputTargetGeometry {
    pub target: PhysicalRect,
    pub work_area: PhysicalRect,
    pub dpi: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusSafety {
    Safe,
    Password,
    ReadOnly,
    Elevated,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusedTarget {
    pub native_window_id: u64,
    pub process_id: u32,
    pub process_instance_id: Option<u64>,
    pub identity: Option<PasteControlIdentity>,
    pub safety: FocusSafety,
    pub is_text_input: bool,
}

impl Default for FocusedTarget {
    fn default() -> Self {
        Self {
            native_window_id: 0,
            process_id: 0,
            process_instance_id: None,
            identity: None,
            safety: FocusSafety::Unknown,
            is_text_input: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PasteTarget {
    pub window_id: isize,
    pub window_class: String,
    pub process_id: u32,
    pub process_started_at: u64,
    pub focused_control: Option<PasteControlIdentity>,
    pub app_name: Option<String>,
    pub selected_text: Option<String>,
    pub is_single_line: Option<bool>,
    pub geometry: InputTargetGeometry,
}

impl PasteTarget {
    pub fn matches_window(&self, focus: &FocusedTarget) -> bool {
        self.window_id > 0
            && self.window_id as u64 == focus.native_window_id
            && self.process_id != 0
            && self.process_id == focus.process_id
            && self.process_started_at != 0
            && focus.process_instance_id == Some(self.process_started_at)
    }

    pub fn matches_focus(&self, focus: &FocusedTarget) -> bool {
        self.matches_window(focus)
            && self.focused_control.is_some()
            && self.focused_control == focus.identity
            && focus.safety == FocusSafety::Safe
            && focus.is_text_input
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteDeliveryFailure {
    OriginalWindowUnavailable,
    InputUnavailable,
    ElevatedTarget,
    ModifierKeysBusy,
    KeyInjectionFailed,
    KeyReleaseFailed,
    NativePasteFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasteDelivery {
    Pasted,
    Failed(PasteDeliveryFailure),
}

pub trait ClipboardPlatform: Send + Sync {
    fn subscribe_changes(&self) -> PlatformChangeSubscription;
    fn clipboard_sequence(&self) -> u64;
    fn read_clipboard(&self) -> Result<Option<ClipboardSnapshot>, PlatformError>;
    fn write_clipboard(
        &self,
        representations: &[ClipboardRepresentation],
    ) -> Result<u64, PlatformError>;
    fn capture_target(&self) -> Result<Option<PasteTarget>, PlatformError>;
    fn paste_to_target(&self, target: &PasteTarget) -> Result<PasteDelivery, PlatformError>;
    fn reset_paste_window_session(&self) {}
}
