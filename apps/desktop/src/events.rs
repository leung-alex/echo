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
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Condvar, Mutex,
    },
};

pub enum Command {
    InlineTimeout(u64),
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
    StageScroll(f32),
    SoftwareFrameReady(echo_presentation::slide::ContentFrame),
    TrimHidden(u64, u64),
    ViewportChanged,
    CommitCardRegion(u64),
    SaveFavorite,
    CancelEditor,
    Clear,
    Create,
    Reorder(DragOrigin, RowKey),
    Quit,
}
#[derive(Clone, Copy)]
pub struct DragOrigin {
    pub frame: echo_presentation::slide::ContentFrame,
    pub binding: i32,
    pub key: RowKey,
}
pub enum Event {
    Inline(echo_windows::inline::InlineEvent),
    Shell(ShellEvent),
    Command(Command),
    Ready(Result<SettingsSnapshot, String>),
    Loaded(SpaceId, LoadTicket, Result<LoadedPage, String>),
    Preview(SpaceId, u64, Result<LoadedPage, String>),
    SidePreviewCancelled(SpaceId, u64),
    Spaces(Result<Vec<Space>, String>),
    Inspected(u64, Result<ItemDetails, String>),
    Catalog(u64, Result<QuickInsertPage, String>),
    DiagnosticExported(Result<String, String>),
    Activated(u64, Context, Result<ActivationResult, String>),
    Executed(
        Operation,
        Result<QuickInsertOutcome, echo_engine::QuickInsertError>,
    ),
    Mutated(u64, Result<MutationResult, String>),
    Thumbnail(u64, String, Result<PixelData, String>),
    Invalidated,
    SearchCacheTrimmed(u64, u64),
}
pub struct ActivationResult {
    pub target: Option<echo_engine::PasteTarget>,
    pub anchor: Option<echo_windows::focus::PopupAnchor>,
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
    pub settings_warning: Option<String>,
    pub settings: Option<ClipboardSettings>,
    pub editor_saved: bool,
    pub snapshot: Option<SettingsSnapshot>,
    pub space_result: Option<SpaceMutationResult>,
}
// 1 MiB transport (including the single producer's 128 KiB batch); the remaining
// 3 MiB is reserved for page and Slint model ownership on the UI thread.
const DISPLAY_QUEUE_BYTES: usize = 896 * 1024;

fn item_bytes(item: &QuickInsertItem) -> usize {
    item.held_bytes()
}
fn page_bytes(page: &QuickInsertPage) -> usize {
    (page.items.capacity() - page.items.len()) * std::mem::size_of::<QuickInsertItem>()
        + page.items.iter().map(item_bytes).sum::<usize>()
}
fn event_data_bytes(event: &Event) -> usize {
    let payload = match event {
        Event::Loaded(_, _, result) | Event::Preview(_, _, result) => result
            .as_ref()
            .map_or_else(|e| e.capacity(), |p| page_bytes(&p.page)),
        Event::Catalog(_, result) => result.as_ref().map_or_else(|e| e.capacity(), page_bytes),
        Event::Inspected(_, result) => result.as_ref().map_or_else(
            |e| e.capacity(),
            |d| item_bytes(&d.item) + d.spaces.capacity() * std::mem::size_of::<SpaceId>(),
        ),
        Event::Thumbnail(_, hash, result) => {
            hash.capacity()
                + result
                    .as_ref()
                    .map_or_else(|e| e.capacity(), |p| p.rgba.capacity())
        }
        _ => return 0,
    };
    std::mem::size_of::<Event>().saturating_add(payload)
}
#[derive(Default)]
pub struct Hub {
    queue: Mutex<VecDeque<Event>>,
    wake: Condvar,
    data_bytes: AtomicUsize,
    data_high_water: AtomicUsize,
    ready: AtomicBool,
    scheduled: AtomicBool,
    closed: AtomicBool,
}
impl Hub {
    #[cfg(test)]
    pub(crate) fn take_test_events(&self) -> Vec<Event> {
        let events: Vec<_> = self
            .queue
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
            .collect();
        self.release_data(events.iter().map(event_data_bytes).sum());
        events
    }
    /// All inline events use one proxy path; hook callbacks never wait on the UI queue.
    pub fn post_inline(self: &Arc<Self>, event: echo_windows::inline::InlineEvent) {
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        let hub = self.clone();
        let _ = slint::invoke_from_event_loop(move || hub.post(Event::Inline(event)));
    }
    pub fn post(self: &Arc<Self>, event: Event) {
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        let bytes = event_data_bytes(&event);
        // Include this producer's already materialized result as well as queued
        // and currently delivered results. Production display data has one worker.
        self.data_high_water.fetch_max(
            self.data_bytes
                .load(Ordering::Relaxed)
                .saturating_add(bytes),
            Ordering::Relaxed,
        );
        {
            let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
            // Only worker-owned display results carry credits. UI/control and
            // shutdown events never wait behind data. Credits remain held while
            // the UI batch is delivered, not merely until dequeue.
            while bytes > 0 && !self.closed.load(Ordering::Acquire) {
                let used = self.data_bytes.load(Ordering::Relaxed);
                if used == 0 || used.saturating_add(bytes) <= DISPLAY_QUEUE_BYTES {
                    break;
                }
                queue = self.wake.wait(queue).unwrap_or_else(|e| e.into_inner());
            }
            if self.closed.load(Ordering::Acquire) {
                return;
            }
            // Never coalesce across Confirm, Cancel or other control-event barriers.
            if let Event::Inline(echo_windows::inline::InlineEvent::Changed { ticket, .. }) = &event
            {
                if let Some(Event::Inline(echo_windows::inline::InlineEvent::Changed {
                    ticket: previous,
                    ..
                })) = queue.back()
                {
                    if ticket.session == previous.session {
                        if ticket.revision < previous.revision
                            || ticket.input_serial < previous.input_serial
                        {
                            return;
                        }
                        queue.pop_back();
                    }
                }
            }
            let duplicate = match &event {
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
                        ShellEvent::QuickInsert(_)
                            | ShellEvent::Activation(_)
                            | ShellEvent::Open
                            | ShellEvent::Favorites
                            | ShellEvent::Settings
                    )
                )
            {
                return;
            }
            let used = self
                .data_bytes
                .fetch_add(bytes, Ordering::Relaxed)
                .saturating_add(bytes);
            self.data_high_water.fetch_max(used, Ordering::Relaxed);
            // An indivisible explicit editor payload may exceed the display
            // budget; deliver it alone, without truncating durable user content.
            // This exception is observable and is not a P50 pass.
            if bytes > DISPLAY_QUEUE_BYTES {
                crate::memory_trace::record(
                    "oversized_display_result",
                    serde_json::json!({"bytes":bytes,"budget":DISPLAY_QUEUE_BYTES}),
                );
            }
            queue.push_back(event);
        }
        self.wake.notify_all();
        self.kick();
    }
    /// Before the first activation there is no Slint backend or UI event loop.
    /// Bootstrap/settings and coalesced invalidations remain queued for the UI;
    /// the worker, capture and native shell continue on their existing threads.
    pub fn wait_for_activation(&self) -> bool {
        let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if self.closed.load(Ordering::Acquire)
                || queue.iter().any(|event| {
                    matches!(
                        event,
                        Event::Shell(ShellEvent::Quit) | Event::Command(Command::Quit)
                    )
                })
                || queue.iter().any(|event| {
                    matches!(event,
                    Event::Shell(ShellEvent::Activation(args)) if args == &["--quit"])
                })
            {
                return false;
            }
            if queue.iter().any(|event| matches!(event,
                Event::Shell(ShellEvent::Open | ShellEvent::Favorites | ShellEvent::Settings | ShellEvent::QuickInsert(_)))
                || matches!(event, Event::Shell(ShellEvent::Activation(args)) if args != &["--background"]))
            {
                return true;
            }
            queue = self.wake.wait(queue).unwrap_or_else(|e| e.into_inner());
        }
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
    fn take_batch(&self) -> Vec<Event> {
        let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        let count = queue.len().min(32);
        queue.drain(..count).collect()
    }
    fn drain(self: Arc<Self>) {
        let events = self.take_batch();
        for event in events {
            if self.closed.load(Ordering::Acquire) {
                break;
            }
            let bytes = event_data_bytes(&event);
            crate::app::deliver(event);
            self.release_data(bytes);
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
        self.data_bytes.store(0, Ordering::Relaxed);
        self.wake.notify_all();
    }
    fn release_data(&self, bytes: usize) {
        let _queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        let used = self.data_bytes.load(Ordering::Relaxed);
        self.data_bytes
            .store(used.saturating_sub(bytes), Ordering::Relaxed);
        self.wake.notify_all();
    }
    pub fn data_high_water(&self) -> usize {
        self.data_high_water.load(Ordering::Relaxed)
    }
    pub fn data_bytes(&self) -> usize {
        self.data_bytes.load(Ordering::Relaxed)
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
    pub model_bytes: usize,
    pub outgoing_bytes: usize,
    pub side_bytes: usize,
    pub cached_thumbnail_bytes: usize,
    pub queued_bytes: usize,
    pub software_frame_bytes: usize,
    pub scale_factor: f32,
    pub high_contrast: bool,
    pub system_animations: bool,
    pub on_battery: bool,
}

#[cfg(test)]
mod tests;
