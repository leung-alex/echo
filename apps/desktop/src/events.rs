//! Typed in-process delivery to the native UI thread.
use echo_engine::{ClipboardSettings, QuickInsertOutcome, QuickInsertPage};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Main,
    Favorites,
}
impl Role {
    pub const fn index(self) -> usize {
        match self {
            Self::Main => 0,
            Self::Favorites => 1,
        }
    }
}

pub enum Command {
    Query(Role, String),
    Select(Role, String),
    Action(Role, String, String),
    Batch(Role, String),
    Refresh(Role),
    More(Role),
    Previous(Role),
    Panel(Role, String),
    Route(String),
    Dismiss(Role),
    Drag(Role),
    Keyboard(Role, Intent),
    Thumbnail(Role, String),
    SaveSettings,
    SaveFavorite(Role),
    CancelEditor(Role),
    Clear(Role),
    Create(Role),
    Reorder(Role, RowKey, RowKey),
    Quit,
}
pub enum Event {
    Shell(ShellEvent),
    Command(Command),
    Ready(Result<ClipboardSettings, String>),
    Loaded(Role, LoadTicket, Result<QuickInsertPage, String>),
    Activated(u64, Context, Result<bool, String>),
    Executed(Role, Operation, Result<QuickInsertOutcome, String>),
    Mutated(Role, u64, Result<MutationResult, String>),
    Thumbnail(Role, u64, String, Result<PixelData, String>),
    Invalidated,
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
