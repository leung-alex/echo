//! Typed in-process delivery to the native UI thread.
use echo_engine::{
    ClipboardSettings, QuickInsertItem, QuickInsertOutcome, QuickInsertPage, SettingsSnapshot,
    Space, SpaceId, SpaceMutationResult,
};
use echo_presentation::{
    interaction::Intent,
    session::{Context, Operation},
    LoadTicket, RowKey,
};
use echo_windows::shell::ShellEvent;
use std::{
    collections::VecDeque,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

pub enum Command {
    Query(String),
    Select(String),
    Action(String, String),
    Batch(String),
    Refresh,
    More,
    Previous,
    Panel(String),
    Route(String),
    Dismiss,
    Drag,
    Keyboard(Intent),
    Thumbnail(String),
    SaveSettings,
    SettingsEdited,
    SettingsAction(String),
    SpaceAction(String, String),
    PickerQuery(String),
    PickerMore,
    PickerSelect(String),
    Confirm(String),
    StageClick(f32, f32),
    StageScroll(f32),
    FlowTick,
    Prewarm,
    TrimHidden,
    ViewportChanged,
    SaveFavorite,
    CancelEditor,
    Clear,
    Create,
    Reorder(RowKey, RowKey),
    Quit,
}
pub enum Event {
    Shell(ShellEvent),
    Command(Command),
    Ready(Result<SettingsSnapshot, String>),
    Loaded(SpaceId, LoadTicket, Result<LoadedPage, String>),
    Preview(SpaceId, u64, Result<LoadedPage, String>),
    Spaces(Result<Vec<Space>, String>),
    Inspected(u64, Result<ItemDetails, String>),
    Catalog(u64, Result<QuickInsertPage, String>),
    GraphicsError(String),
    DiagnosticExported(Result<String, String>),
    Activated(u64, Context, Result<bool, String>),
    Executed(Operation, Result<QuickInsertOutcome, String>),
    Mutated(u64, Result<MutationResult, String>),
    Thumbnail(u64, String, Result<PixelData, String>),
    Invalidated,
}
pub struct LoadedPage {
    pub page: QuickInsertPage,
    pub revision: i64,
    pub total: u64,
}
pub struct ItemDetails {
    pub item: QuickInsertItem,
    pub spaces: Vec<SpaceId>,
}
pub struct PixelData {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
pub struct MutationResult {
    pub message: String,
    pub settings: Option<ClipboardSettings>,
    pub editor_saved: bool,
    pub snapshot: Option<SettingsSnapshot>,
    pub space_result: Option<SpaceMutationResult>,
}
#[derive(Default)]
pub struct Hub {
    queue: Mutex<VecDeque<Event>>,
    ready: AtomicBool,
    scheduled: AtomicBool,
    closed: AtomicBool,
}
impl Hub {
    pub fn post(self: &Arc<Self>, event: Event) {
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        {
            let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
            let duplicate = match &event {
                Event::Command(Command::FlowTick) => queue
                    .iter()
                    .any(|e| matches!(e, Event::Command(Command::FlowTick))),
                Event::Command(Command::ViewportChanged) => queue
                    .iter()
                    .any(|e| matches!(e, Event::Command(Command::ViewportChanged))),
                Event::Invalidated => queue.iter().any(|e| matches!(e, Event::Invalidated)),
                Event::Shell(ShellEvent::GeometryChanged) => queue
                    .iter()
                    .any(|e| matches!(e, Event::Shell(ShellEvent::GeometryChanged))),
                Event::Shell(ShellEvent::ThemeChanged) => queue
                    .iter()
                    .any(|e| matches!(e, Event::Shell(ShellEvent::ThemeChanged))),
                _ => false,
            };
            if duplicate {
                return;
            }
            // Work results and Quit are never discarded. Unsolicited activation is bounded.
            if queue.len() >= 128
                && matches!(
                    &event,
                    Event::Shell(
                        ShellEvent::Activation(_)
                            | ShellEvent::Open
                            | ShellEvent::Favorites
                            | ShellEvent::Settings
                    )
                )
            {
                return;
            }
            queue.push_back(event);
        }
        self.kick();
    }
    pub fn activate(self: &Arc<Self>) {
        self.ready.store(true, Ordering::Release);
        self.kick();
    }
    fn kick(self: &Arc<Self>) {
        if !self.ready.load(Ordering::Acquire) || self.closed.load(Ordering::Acquire) {
            return;
        }
        if self.scheduled.swap(true, Ordering::AcqRel) {
            return;
        }
        let hub = self.clone();
        if slint::invoke_from_event_loop(move || hub.drain()).is_err() {
            self.scheduled.store(false, Ordering::Release);
        }
    }
    fn drain(self: Arc<Self>) {
        let events = {
            let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
            queue.drain(..).collect::<Vec<_>>()
        };
        for event in events {
            crate::app::deliver(event);
        }
        self.scheduled.store(false, Ordering::Release);
        let pending = !self
            .queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty();
        if pending {
            self.kick();
        }
    }
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.queue.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }
}

/// Explicit allowlist: no query, clipboard payload, item/space title, or window handle.
#[derive(serde::Serialize)]
pub struct DiagnosticReport {
    pub schema: &'static str,
    pub version: &'static str,
    pub renderer: String,
    pub backend: String,
    pub adapter: String,
    pub actual_mode: String,
    pub raster_policy: &'static str,
    pub panel_texture_limit: u64,
    pub motion_scale_cap: f32,
    pub motion_frame_cap: u32,
    pub panel_texture_bytes: u64,
    pub resident_panels: usize,
    pub draw_count: u64,
    pub upload_count: u64,
    pub scale_factor: f32,
    pub high_contrast: bool,
    pub system_animations: bool,
    pub on_battery: bool,
}
