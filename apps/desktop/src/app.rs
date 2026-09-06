//! One native window, one insertion session, and independently identified content spaces.
use crate::{
    events::{Command, Event, Hub, PixelData},
    formatting,
    service::{Mutation, Work, Worker},
    AppWindow, EntryRow,
};
use echo_engine::*;
use echo_presentation::{
    deck::{Deck, Phase},
    interaction::Intent,
    session::{Completion, Context, RecentActivations, Session},
    space_state::SpacePositions,
    RowKey, Surface,
};
use echo_windows::shell::{self, ShellEvent, WindowHook};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use slint::{ComponentHandle, Model, ModelRc, Timer, TimerMode, VecModel};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
mod bindings;
mod card_window;
mod deck_controller;
mod dialogs;
#[cfg(feature = "native-test")]
pub(crate) mod native_test;
mod settings_controller;
use dialogs::{Confirmation, Picker};
thread_local! { static APP: RefCell<Option<Rc<RefCell<App>>>> = const { RefCell::new(None) }; }
pub fn install(app: Rc<RefCell<App>>) {
    APP.with(|slot| *slot.borrow_mut() = Some(app));
}
pub fn uninstall() {
    APP.with(|slot| {
        slot.borrow_mut().take();
    });
}
pub fn deliver(event: Event) {
    let _ = APP.try_with(|slot| {
        if let Some(app) = slot.borrow().as_ref() {
            app.borrow_mut().handle(event);
        }
    });
}
pub fn key_intent(key: &str, ctrl: bool, shift: bool, target: &str) -> Option<Intent> {
    APP.try_with(|slot| {
        slot.borrow().as_ref().and_then(|app| {
            app.try_borrow()
                .ok()
                .map(|a| a.interpret_key(key, ctrl, shift, target))
        })
    })
    .ok()
    .flatten()
}
pub fn resolve_key(key: &str) -> Option<RowKey> {
    APP.try_with(|slot| {
        slot.borrow().as_ref().and_then(|app| {
            app.try_borrow()
                .ok()
                .and_then(|a| a.surface.resolve_key(key))
        })
    })
    .ok()
    .flatten()
}
pub fn is_composing() -> bool {
    // Native focus/IME teardown can arrive after the app TLS owner was destroyed.
    APP.try_with(|slot| {
        slot.borrow().as_ref().is_some_and(|app| {
            app.try_borrow().map_or(true, |a| {
                a.hook.as_ref().is_some_and(WindowHook::is_composing)
            })
        })
    })
    .unwrap_or(true)
}
#[derive(Default)]
struct Images {
    cache: HashMap<String, (slint::Image, usize)>,
    order: VecDeque<String>,
    pending: HashSet<String>,
    bytes: usize,
    epoch: u64,
}
impl Images {
    fn trim_to(&mut self, limit: usize) -> bool {
        let mut changed = false;
        while self.bytes > limit {
            let Some(key) = self.order.pop_front() else {
                break;
            };
            if let Some((_, size)) = self.cache.remove(&key) {
                self.bytes = self.bytes.saturating_sub(size);
                changed = true;
            }
        }
        changed
    }
}
struct Preview {
    items: Vec<QuickInsertItem>,
    revision: i64,
    total: u64,
}
pub struct App {
    window: AppWindow,
    surface: Surface,
    model: Rc<VecModel<EntryRow>>,
    images: Images,
    hook: Option<WindowHook>,
    hwnd: Option<isize>,
    hub: Arc<Hub>,
    worker: Worker,
    session: Session,
    recent: RecentActivations,
    settings: ClipboardSettings,
    ui: UiSettings,
    settings_revision: i64,
    ready: bool,
    pending_args: Option<Vec<String>>,
    mutation: Option<u64>,
    serial: u64,
    quitting: bool,
    restart: Option<bool>,
    clock: Instant,
    deck: Deck,
    spaces: Vec<Space>,
    positions: SpacePositions,
    search_timer: Timer,
    flow_timer: Timer,
    prewarm_timer: Timer,
    trim_timer: Timer,
    preview_timer: Timer,
    preview_started: Option<Instant>,
    preview_epoch: u64,
    previews: HashMap<SpaceId, Preview>,
    pending_previews: HashSet<SpaceId>,
    dirty_snapshots: HashSet<SpaceId>,
    navigation_us: Vec<u64>,
    window_shapes: Option<Option<Vec<shell::CardShape>>>,
    last_scroll_bits: u32,
    geometry: (u32, u32, u32),
    pending_scroll: Option<f32>,
    navigate_after_refresh: Option<SpaceId>,
    graphics: crate::graphics::GraphicsInfo,
    environment: shell::UiEnvironment,
    graphics_error: Option<String>,
    confirmation: Option<Confirmation>,
    picker: Picker,
    picker_generation: u64,
    picker_cursor: Option<PageCursor>,
    picker_items: Vec<QuickInsertItem>,
    editor_key: Option<RowKey>,
    editor_original: Option<(String, String, String, String)>,
    space_edit_id: Option<(SpaceId, i64)>,
    space_original: Option<(String, String, String, String)>,
    inspect_intent: Option<(u64, String, RowKey)>,
    edit_created_copy: bool,
    wheel_delta: f32,
    #[cfg(feature = "cover-flow")]
    flow: Option<crate::cover_flow::bridge::FlowBridge>,
}
impl App {
    pub fn new(
        hub: Arc<Hub>,
        worker: Worker,
        args: Vec<String>,
        graphics: crate::graphics::GraphicsInfo,
    ) -> Result<Rc<RefCell<Self>>, String> {
        let window = AppWindow::new().map_err(|e| e.to_string())?;
        window.window().set_size(slint::LogicalSize::new(
            echo_presentation::echo_tokens::WINDOW_WIDTH,
            echo_presentation::echo_tokens::WINDOW_HEIGHT,
        ));
        let model = Rc::new(VecModel::default());
        window.set_rows(ModelRc::from(model.clone()));
        let bootstrap = worker.bootstrap.clone();
        let mut surface = Surface::new(QuickInsertView::History);
        surface.row_limit = 400;
        let environment = shell::ui_environment(None);
        #[cfg(feature = "cover-flow")]
        let flow = if graphics.perspective {
            Some(crate::cover_flow::bridge::FlowBridge::install(
                &window,
                hub.clone(),
                graphics.integrated || bootstrap.ui.reduce_on_battery && environment.on_battery,
            )?)
        } else {
            None
        };
        let app = Rc::new(RefCell::new(Self {
            window,
            surface,
            model,
            images: Images::default(),
            hook: None,
            hwnd: None,
            hub,
            worker,
            session: Session::default(),
            recent: RecentActivations::default(),
            settings: bootstrap.clipboard,
            ui: bootstrap.ui,
            settings_revision: bootstrap.revision,
            ready: false,
            pending_args: Some(args),
            mutation: None,
            serial: 0,
            quitting: false,
            restart: None,
            clock: Instant::now(),
            deck: Deck::default(),
            spaces: Vec::new(),
            positions: SpacePositions::default(),
            search_timer: Timer::default(),
            flow_timer: Timer::default(),
            prewarm_timer: Timer::default(),
            trim_timer: Timer::default(),
            preview_timer: Timer::default(),
            preview_started: None,
            preview_epoch: 0,
            previews: HashMap::new(),
            pending_previews: HashSet::new(),
            dirty_snapshots: HashSet::new(),
            navigation_us: Vec::new(),
            window_shapes: None,
            last_scroll_bits: 0,
            geometry: (0, 0, 0),
            pending_scroll: None,
            navigate_after_refresh: None,
            graphics,
            environment,
            graphics_error: None,
            confirmation: None,
            picker: Picker::Closed,
            picker_generation: 0,
            picker_cursor: None,
            picker_items: Vec::new(),
            editor_key: None,
            editor_original: None,
            space_edit_id: None,
            space_original: None,
            inspect_intent: None,
            edit_created_copy: false,
            wheel_delta: 0.0,
            #[cfg(feature = "cover-flow")]
            flow,
        }));
        bindings::connect(&app.borrow());
        app.borrow().render_settings();
        app.borrow_mut().apply_theme();
        Ok(app)
    }
    fn now(&self) -> u64 {
        self.clock.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
    }
    fn send(&mut self, work: Work) -> bool {
        if let Err(e) = self.worker.send(work) {
            self.report(e, true);
            false
        } else {
            true
        }
    }
    fn report(&mut self, text: impl Into<String>, error: bool) {
        let text = text.into();
        if self.window.get_route().as_str() == "settings" {
            self.window.set_settings_notice(text.clone().into());
        }
        self.surface.report(text, error);
        self.window.set_status(self.surface.status.clone().into());
        self.window.set_status_error(error);
    }
    fn set_busy(&self) {
        self.window
            .set_busy(self.session.busy() || self.mutation.is_some());
    }
    fn handle(&mut self, event: Event) {
        if self.quitting {
            return;
        }
        match event {
            Event::Command(command) => self.command(command),
            Event::Shell(event) => self.shell_event(event),
            Event::Ready(result) => match result {
                Ok(value) => {
                    self.settings = value.clipboard;
                    self.ui = value.ui;
                    self.settings_revision = value.revision;
                    self.ready = true;
                    self.render_settings();
                    self.apply_theme();
                    self.send(Work::Spaces);
                }
                Err(e) => {
                    self.report(format!("Startup failed: {e}"), true);
                    let _ = self.show_window();
                }
            },
            Event::Spaces(result) => self.spaces_loaded(result),
            Event::Loaded(space, ticket, result) => {
                if space != self.surface.space {
                    return;
                }
                let metadata = result.as_ref().ok().map(|p| (p.revision, p.total));
                let retry = metadata.is_none()
                    && ticket.cursor.is_some()
                    && result
                        .as_ref()
                        .err()
                        .is_some_and(|e| e.contains("outdated"));
                if self.surface.finish_load(ticket, result.map(|p| p.page)) {
                    if let Some((revision, total)) = metadata {
                        self.surface.revision = revision;
                        self.surface.total = total;
                    }
                    if retry {
                        self.surface.refresh_top();
                        self.pending_scroll = None;
                        self.window.set_scroll_y(0.0);
                        self.load(false);
                    }
                    if self.deck.phase != Phase::Animating {
                        self.render();
                    }
                    self.content_ready();
                    self.schedule_prewarm();
                }
            }
            Event::Preview(space, epoch, result) => self.preview_loaded(space, epoch, result),
            Event::Inspected(generation, result) => self.inspected(generation, result),
            Event::Catalog(generation, result) => self.catalog_loaded(generation, result),
            Event::Activated(epoch, context, result) => {
                if epoch != self.session.epoch {
                    return;
                }
                match result {
                    Ok(target) => {
                        self.session.capture_finished(epoch, target);
                    }
                    Err(e) => {
                        self.session.capture_finished(epoch, false);
                        self.report(e, true);
                    }
                }
                self.window
                    .set_quick_insert(context == Context::QuickInsert);
                if let Err(e) = self.show_window() {
                    self.report(e, true);
                }
                self.load(false);
            }
            Event::Executed(operation, result) => self.executed(operation, result),
            Event::Mutated(serial, result) => self.mutated(serial, result),
            Event::Thumbnail(epoch, hash, result) => self.thumbnail_finished(epoch, hash, result),
            Event::Invalidated => self.history_invalidated(),
            Event::DiagnosticExported(result) => match result {
                Ok(path) => self.report(format!("Diagnostics saved: {path}"), false),
                Err(e) => {
                    self.window.set_settings_error(e.clone().into());
                    self.report(e, true);
                }
            },
            Event::GraphicsError(error) => {
                let lost = error.contains("device lost");
                self.graphics_error = Some(error.clone());
                self.flow_timer.stop();
                self.preview_timer.stop();
                self.deck.snap();
                self.render();
                self.content_ready();
                self.clear_flow_cache();
                self.window.set_navigation_busy(false);
                self.report(error, true);
                self.refresh_diagnostics();
                if lost && std::env::var("ECHO_GRAPHICS_RECOVERY").as_deref() != Ok("1") {
                    self.restart = Some(true);
                    self.quit();
                }
            }
        }
        self.update_card_region();
    }
    fn shell_event(&mut self, event: ShellEvent) {
        match event {
            ShellEvent::Open => self.activate_args(Vec::new()),
            ShellEvent::Favorites => self.activate_args(vec!["--favorites".into()]),
            ShellEvent::Settings => self.activate_args(vec!["--settings".into()]),
            ShellEvent::Quit => self.request_quit(),
            ShellEvent::Activation(args) => self.activate_args(args),
            ShellEvent::ThemeChanged => {
                self.environment = shell::ui_environment(self.hwnd);
                self.apply_theme();
                self.viewport_changed();
            }
            ShellEvent::GeometryChanged => {
                if let Some(hwnd) = self.hwnd {
                    let _ = shell::fit_window(hwnd, false, 16.0);
                }
                self.environment = shell::ui_environment(self.hwnd);
                self.viewport_changed();
            }
            ShellEvent::Error(e) => self.report(e, true),
        }
    }
    fn activate_args(&mut self, args: Vec<String>) {
        if args.first().map(String::as_str) == Some("--quit") {
            self.request_quit();
            return;
        }
        if !self.ready || self.spaces.is_empty() {
            self.pending_args = Some(args);
            return;
        }
        if args.first().map(String::as_str) == Some("--background") {
            return;
        }
        if self.window.get_modal() || self.window.get_settings_dirty() {
            let _ = self.show_window();
            self.report(
                "Finish or cancel the current edit before opening another space",
                false,
            );
            return;
        }
        let mut context = Context::Manager;
        let mut query = String::new();
        let mut route = "history";
        let mut id = if self.ui.startup_space == StartupSpace::Last {
            self.ui
                .resume_last_space_id
                .as_deref()
                .and_then(SpaceId::parse)
                .unwrap_or(SpaceId::HISTORY)
        } else {
            SpaceId::HISTORY
        };
        match args.first().map(String::as_str) {
            Some("--history") => id = SpaceId::HISTORY,
            Some("--favorites") => id = SpaceId::FAVORITES,
            Some("--settings") => route = "settings",
            _ => {}
        }
        if let Some(decoded) = echo_activation::decode_args(args.iter().map(String::as_str)) {
            let envelope = match decoded {
                Ok(v) => v,
                Err(e) => {
                    self.report(e.to_string(), true);
                    return;
                }
            };
            if !self.recent.admit(&envelope.request_id) {
                return;
            }
            match envelope.action.as_str() {
                "echo.settings" => route = "settings",
                "echo.quick_insert" => {
                    context = Context::QuickInsert;
                    match echo_activation::quick_insert_payload(&envelope) {
                        Ok(p) => query = p.query.unwrap_or_default(),
                        Err(e) => {
                            self.report(e.to_string(), true);
                            return;
                        }
                    }
                }
                _ => {}
            }
        }
        if query.len() > 16 * 1024 {
            self.report("Search text is too large", true);
            return;
        }
        if !self.spaces.iter().any(|s| s.id == id) {
            id = SpaceId::HISTORY;
        }
        self.remember_position();
        self.surface.hide();
        self.surface.set_space(id);
        self.surface.set_query(query.clone());
        self.pending_scroll = if self.ui.remember_position {
            Some(self.positions.restore(&mut self.surface))
        } else {
            Some(0.0)
        };
        self.deck.show(id, self.now());
        self.cancel_prewarm();
        self.clear_flow_cache();
        self.window.set_route(route.into());
        self.window.set_query(query.into());
        self.window.set_stale_rows(true);
        self.window.set_navigation_busy(false);
        self.window.set_control_focus_mode(false);
        self.window.set_editor_open(false);
        self.render_navigation();
        self.render_settings();
        let epoch = self.session.activate(context);
        self.worker.epoch.store(epoch, Ordering::Release);
        self.set_busy();
        // Capturing the external insertion target completes before showing the window.
        self.send(Work::Begin(epoch, context));
    }
    fn show_window(&mut self) -> Result<(), String> {
        let first = self.hwnd.is_none();
        self.trim_timer.stop();
        self.window.show().map_err(|e| e.to_string())?;
        if first {
            let handle = self.window.window().window_handle();
            let hwnd = match handle.window_handle().map_err(|e| e.to_string())?.as_raw() {
                RawWindowHandle::Win32(h) => h.hwnd.get(),
                _ => return Err("Echo requires a Windows window".into()),
            };
            let hub = self.hub.clone();
            self.hook = Some(shell::attach_window(
                hwnd,
                true,
                Arc::new(move |e| hub.post(Event::Shell(e))),
            )?);
            self.hwnd = Some(hwnd);
        }
        if let Some(hwnd) = self.hwnd {
            shell::fit_window(hwnd, first, 16.0)?;
            let _ = shell::focus_window(hwnd);
        }
        self.environment = shell::ui_environment(self.hwnd);
        self.surface.visible = true;
        self.apply_theme();
        if self.deck.phase == Phase::Suspended {
            self.deck.show(self.surface.space, self.now());
        }
        if self.window.get_route().as_str() == "history" {
            self.window.invoke_focus_search(false);
        } else {
            self.window.invoke_focus_controls();
        }
        Ok(())
    }
    fn hide_window(&mut self) {
        self.remember_position();
        self.search_timer.stop();
        self.flow_timer.stop();
        self.preview_timer.stop();
        self.preview_started = None;
        self.cancel_prewarm();
        self.deck.hide();
        self.surface.hide();
        self.inspect_intent = None;
        self.images.epoch = self.images.epoch.wrapping_add(1);
        self.images.pending.clear();
        if self.images.trim_to(1024 * 1024) {
            for index in 0..self.model.row_count() {
                if let Some(mut row) = self.model.row_data(index) {
                    row.thumbnail = Default::default();
                    self.model.set_row_data(index, row);
                }
            }
        }
        self.window.set_navigation_busy(false);
        let _ = self.window.hide();
        if self.ui.trim_when_hidden {
            let hub = self.hub.clone();
            self.trim_timer
                .start(TimerMode::SingleShot, Duration::from_secs(30), move || {
                    hub.post(Event::Command(Command::TrimHidden))
                });
        }
        self.send(Work::Resume(self.surface.space));
    }
    fn dismiss(&mut self) {
        self.hide_window();
        self.session.dismiss();
        self.worker
            .epoch
            .store(self.session.epoch, Ordering::Release);
        self.send(Work::Cancel);
        self.set_busy();
    }
    fn quit(&mut self) {
        self.quitting = true;
        self.search_timer.stop();
        self.flow_timer.stop();
        self.preview_timer.stop();
        self.prewarm_timer.stop();
        self.trim_timer.stop();
        let _ = self.window.hide();
        self.hub.close();
        let _ = slint::quit_event_loop();
    }
    pub fn shutdown(&mut self) {
        self.quitting = true;
        self.hub.close();
        self.window.set_stage_image(Default::default());
        self.window.set_preview_image(Default::default());
        #[cfg(feature = "cover-flow")]
        if let Some(flow) = self.flow.take() {
            flow.shutdown();
        }
        self.worker.stop();
    }
    pub fn take_restart(&mut self) -> Option<bool> {
        self.restart.take()
    }
    fn load(&mut self, more: bool) {
        if !self.ready || !self.surface.visible {
            return;
        }
        let Some(ticket) = self.surface.begin_load(more) else {
            return;
        };
        self.deck.block_content();
        self.window.set_loading(true);
        if !self.send(Work::List(
            ticket,
            self.surface.space,
            self.surface.query.clone(),
        )) {
            self.surface.loading = false;
            self.window.set_loading(false);
        }
    }
    fn render(&mut self) {
        let mut section = String::new();
        let rows = self
            .surface
            .items
            .iter()
            .map(|item| {
                let mut row = formatting::row(item, &mut section);
                row.selected = self.surface.selection == Some(RowKey::of(item));
                row.batch_selected = self.surface.selected_ids.contains(&item.id);
                if let Some(image) = item
                    .thumbnail
                    .as_ref()
                    .and_then(|t| self.images.cache.get(&t.content_hash))
                {
                    row.thumbnail = image.0.clone();
                }
                row
            })
            .collect();
        crate::native_model::reconcile(self.model.as_ref(), rows);
        self.window
            .set_stale_rows(!self.surface.ready && self.surface.items.is_empty());
        self.window.set_loading(self.surface.loading);
        self.window.set_has_more(self.surface.next_cursor.is_some());
        self.window.set_has_previous(self.surface.has_previous());
        self.window.set_batch_mode(self.surface.batch);
        self.window
            .set_selected_count(self.surface.selected_ids.len() as i32);
        self.render_selection();
        self.window.set_status(self.surface.status.clone().into());
        self.window.set_status_error(self.surface.error);
        self.render_navigation();
        self.dirty_snapshots.insert(self.surface.space);
        if self.surface.ready {
            if let Some(scroll) = self.pending_scroll.take() {
                self.window.set_scroll_y(scroll);
            }
        }
        if self.surface.dirty && !self.surface.loading {
            self.load(false);
        }
    }
    fn render_selection(&self) {
        for (index, item) in self.surface.items.iter().enumerate() {
            if let Some(mut row) = self.model.row_data(index) {
                let selected = self.surface.selection == Some(RowKey::of(item));
                let batch = self.surface.selected_ids.contains(&item.id);
                if row.selected != selected || row.batch_selected != batch {
                    row.selected = selected;
                    row.batch_selected = batch;
                    self.model.set_row_data(index, row);
                }
            }
        }
        self.window.set_batch_mode(self.surface.batch);
        self.window
            .set_selected_count(self.surface.selected_ids.len() as i32);
        self.window.set_selection_index(
            self.surface
                .items
                .iter()
                .position(|x| Some(RowKey::of(x)) == self.surface.selection)
                .map(|x| x as i32)
                .unwrap_or(-1),
        );
    }
    fn history_invalidated(&mut self) {
        self.previews.remove(&SpaceId::HISTORY);
        self.dirty_snapshots.insert(SpaceId::HISTORY);
        if self.surface.space == SpaceId::HISTORY {
            self.surface.invalidate();
            if self.surface.visible
                && self.window.get_route().as_str() == "history"
                && !self.surface.loading
                && self.mutation.is_none()
            {
                self.load(false);
            }
        }
        if self.surface.visible {
            self.send(Work::Spaces);
            self.schedule_prewarm();
        }
    }
    fn thumbnail_request(&mut self, key: String) {
        if !self.surface.visible || self.window.get_route().as_str() != "history" {
            return;
        }
        let Some(key) = self.surface.resolve_key(&key) else {
            return;
        };
        let Some(hash) = self
            .surface
            .items
            .iter()
            .find(|x| RowKey::of(x) == key)
            .and_then(|x| x.thumbnail.as_ref())
            .map(|t| t.content_hash.clone())
        else {
            return;
        };
        if self.images.cache.contains_key(&hash) || !self.images.pending.insert(hash.clone()) {
            return;
        }
        if !self.send(Work::Thumbnail(self.images.epoch, hash.clone())) {
            self.images.pending.remove(&hash);
        }
    }
    fn thumbnail_finished(&mut self, epoch: u64, hash: String, result: Result<PixelData, String>) {
        if epoch != self.images.epoch || !self.surface.visible {
            return;
        }
        self.images.pending.remove(&hash);
        let Ok(pixels) = result else {
            return;
        };
        let bytes = pixels.rgba.len();
        if bytes > 8 * 1024 * 1024 {
            return;
        }
        if self.images.cache.contains_key(&hash) {
            return;
        }
        if self.images.trim_to(8 * 1024 * 1024 - bytes) {
            for index in 0..self.model.row_count() {
                if let Some(mut row) = self.model.row_data(index) {
                    row.thumbnail = Default::default();
                    self.model.set_row_data(index, row);
                }
            }
            self.dirty_snapshots.extend(self.previews.keys().copied());
        }
        let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
            &pixels.rgba,
            pixels.width,
            pixels.height,
        );
        let image = slint::Image::from_rgba8(buffer);
        self.images.bytes += bytes;
        self.images.order.push_back(hash.clone());
        self.dirty_snapshots.extend(
            self.previews
                .iter()
                .filter(|(_, p)| {
                    p.items
                        .iter()
                        .any(|i| i.thumbnail.as_ref().is_some_and(|t| t.content_hash == hash))
                })
                .map(|(id, _)| *id),
        );
        self.images.cache.insert(hash, (image, bytes));
        if self.deck.phase != Phase::Animating {
            self.render();
        }
        self.schedule_prewarm();
    }
    fn command(&mut self, command: Command) {
        match command {
            Command::Quit => self.request_quit(),
            Command::Dismiss => self.request_hide(),
            Command::Drag => {
                if let Some(hwnd) = self.hwnd {
                    let _ = shell::start_drag(hwnd);
                }
            }
            Command::Query(query) => {
                if self.deck.phase == Phase::Animating {
                    self.finish_motion();
                }
                if query.len() > 16 * 1024 {
                    self.report("Search text is too large", true);
                    return;
                }
                self.surface.set_query(query);
                self.window.set_scroll_y(0.0);
                self.pending_scroll = None;
                self.deck.block_content();
                self.cancel_prewarm();
                self.previews.clear();
                self.dirty_snapshots.insert(self.surface.space);
                self.window.set_stale_rows(true);
                self.window.set_loading(true);
                let hub = self.hub.clone();
                self.search_timer.start(
                    TimerMode::SingleShot,
                    Duration::from_millis(75),
                    move || hub.post(Event::Command(Command::Refresh)),
                );
            }
            Command::Refresh => self.load(false),
            Command::More => self.load(true),
            Command::Previous => {
                if self.surface.previous_window() {
                    self.window.set_scroll_y(0.0);
                    self.load(false);
                }
            }
            Command::Select(key) => {
                if self.deck.can_insert(self.surface.space) && self.surface.ready {
                    if let Some(key) = self.surface.resolve_key(&key) {
                        self.surface.select(key);
                        self.render_selection();
                        self.dirty_snapshots.insert(self.surface.space);
                        self.schedule_prewarm();
                    }
                }
            }
            Command::Action(action, key) => self.action(&action, &key),
            Command::Batch(action) => self.batch(&action),
            Command::Panel(panel) => self.navigate_to(if panel == "favorites" {
                SpaceId::FAVORITES
            } else {
                SpaceId::HISTORY
            }),
            Command::Route(route) => self.request_route(&route),
            Command::Keyboard(intent) => self.keyboard(intent),
            Command::Thumbnail(key) => self.thumbnail_request(key),
            Command::SaveSettings => self.save_settings(),
            Command::SettingsEdited => self.settings_edited(),
            Command::SettingsAction(action) => self.settings_action(&action),
            Command::SpaceAction(action, key) => self.space_action(&action, &key),
            Command::PickerQuery(query) => self.picker_query(query),
            Command::PickerMore => self.picker_more(),
            Command::PickerSelect(key) => self.picker_select(&key),
            Command::Confirm(answer) => self.confirm(&answer),
            Command::FlowTick => self.flow_tick(),
            Command::Prewarm => self.prewarm(),
            Command::TrimHidden => {
                if !self.surface.visible {
                    self.clear_flow_cache();
                    self.previews.clear();
                    self.images.trim_to(0);
                }
            }
            Command::ViewportChanged => self.viewport_changed(),
            Command::StageClick(x, y) => self.stage_click(x, y),
            Command::StageScroll(delta) => self.stage_scroll(delta),
            Command::SaveFavorite => self.save_favorite(),
            Command::CancelEditor => self.cancel_editor(),
            Command::Clear => self.mutate(Mutation::Clear),
            Command::Create => self.new_item(),
            Command::Reorder(source, target) => {
                if self.surface.query.is_empty()
                    && self.surface.ready
                    && !self.window.get_modal()
                    && source.source == QuickInsertSource::Favorite
                    && target.source == QuickInsertSource::Favorite
                {
                    self.space_mutation(
                        self.surface.space,
                        SpaceAction::ReorderItem {
                            id: source.id,
                            before: Some(target.id),
                            delta: 0,
                        },
                    );
                }
            }
        }
    }
    fn action(&mut self, action: &str, value: &str) {
        if action == "retry" {
            self.load(false);
            return;
        }
        if !self.surface.visible
            || !self.surface.ready
            || self.surface.loading
            || self.session.busy()
            || self.mutation.is_some()
            || !self.deck.can_insert(self.surface.space)
            || self.window.get_modal()
        {
            return;
        }
        let Some(key) = self.surface.resolve_key(value) else {
            return;
        };
        match action {
            "copy" => self.execute(key, QuickInsertAction::Copy),
            "insert" => self.execute(key, QuickInsertAction::Insert),
            "favorite" if key.source == QuickInsertSource::History => {
                self.space_mutation(SpaceId::FAVORITES, SpaceAction::MoveHistory(vec![key.id]))
            }
            "pin" if key.source == QuickInsertSource::History => {
                let pinned = self
                    .surface
                    .items
                    .iter()
                    .find(|x| RowKey::of(x) == key)
                    .is_some_and(|x| x.pinned_at.is_some());
                self.mutate(Mutation::Pin(key.id, pinned));
            }
            "edit" if key.source == QuickInsertSource::Favorite => self.inspect_item("edit", key),
            "options" => self.item_options(key),
            "toggle-batch" => {
                self.surface.toggle_selected(key.id);
                self.render_selection();
            }
            "up" | "down"
                if key.source == QuickInsertSource::Favorite && self.surface.query.is_empty() =>
            {
                self.space_mutation(
                    self.surface.space,
                    SpaceAction::ReorderItem {
                        id: key.id,
                        before: None,
                        delta: if action == "up" { -1 } else { 1 },
                    },
                )
            }
            _ => {}
        }
    }
    fn execute(&mut self, key: RowKey, action: QuickInsertAction) {
        if !self.deck.can_insert(self.surface.space)
            || !self.surface.ready
            || self.mutation.is_some()
        {
            return;
        }
        if action == QuickInsertAction::Insert && self.session.context == Context::Manager {
            self.report("Selected · use Copy to copy this content", false);
            return;
        }
        let Some(operation) = self.session.begin(action) else {
            return;
        };
        self.set_busy();
        self.report(
            if action == QuickInsertAction::Copy {
                "Copying…"
            } else {
                "Inserting…"
            },
            false,
        );
        if action == QuickInsertAction::Insert {
            self.hide_window();
        }
        if !self.send(Work::Execute(operation, key)) {
            self.session.finish(operation, Err(()));
            self.set_busy();
            if action == QuickInsertAction::Insert {
                let _ = self.show_window();
                self.load(false);
            }
        }
    }
    fn executed(
        &mut self,
        operation: echo_presentation::session::Operation,
        result: Result<QuickInsertOutcome, String>,
    ) {
        let completion = self
            .session
            .finish(operation, result.as_ref().copied().map_err(|_| ()));
        self.set_busy();
        match completion {
            Completion::Stale => {}
            Completion::Inserted => self.report("Inserted", false),
            Completion::Copied => self.report("Copied", false),
            Completion::Staged => self.report("Copied to clipboard; no active target", false),
            Completion::Restore => {
                if operation.action == QuickInsertAction::Insert {
                    let _ = self.show_window();
                    self.load(false);
                }
                self.report(
                    result.err().unwrap_or_else(|| "Operation failed".into()),
                    true,
                );
            }
        }
    }
    fn mutate(&mut self, mutation: Mutation) {
        if self.mutation.is_some() || self.session.busy() {
            return;
        }
        self.serial = self.serial.wrapping_add(1);
        let serial = self.serial;
        self.mutation = Some(serial);
        self.set_busy();
        self.report("Saving…", false);
        if !self.send(Work::Mutate(serial, mutation)) {
            self.mutation = None;
            self.set_busy();
        }
    }
    fn mutated(&mut self, serial: u64, result: Result<crate::events::MutationResult, String>) {
        if self.mutation != Some(serial) {
            return;
        }
        self.mutation = None;
        self.set_busy();
        match result {
            Ok(result) => {
                if let Some(snapshot) = result.snapshot {
                    self.cancel_prewarm();
                    self.previews.clear();
                    self.positions.clear();
                    self.settings = snapshot.clipboard;
                    self.ui = snapshot.ui;
                    self.settings_revision = snapshot.revision;
                    self.render_settings();
                    self.apply_theme();
                    self.clear_flow_cache();
                    self.schedule_prewarm();
                } else if let Some(settings) = result.settings {
                    self.settings = settings;
                    self.render_settings();
                    self.apply_theme();
                }
                if result.editor_saved {
                    self.window.set_editor_open(false);
                    self.editor_key = None;
                    self.editor_original = None;
                }
                self.window.set_clear_confirm_open(false);
                self.window.set_space_dialog_open(false);
                self.space_edit_id = None;
                self.space_original = None;
                self.close_picker();
                if let Some(change) = result.space_result {
                    for id in &change.affected_spaces {
                        self.previews.remove(id);
                        self.positions.remove(*id);
                        self.dirty_snapshots.insert(*id);
                    }
                    if let Some(id) = change.created_space {
                        self.navigate_after_refresh = Some(id);
                    }
                    if self.edit_created_copy {
                        self.edit_created_copy = false;
                        if let Some(id) = change.created_item {
                            self.inspect_item(
                                "edit",
                                RowKey {
                                    source: QuickInsertSource::Favorite,
                                    id,
                                },
                            );
                        }
                    }
                    if change.migrated_count > 0 {
                        self.report(
                            format!(
                                "Space updated · {} exclusive items moved to Favorites",
                                change.migrated_count
                            ),
                            false,
                        );
                    } else {
                        self.report(result.message, false);
                    }
                } else {
                    self.previews.clear();
                    self.clear_flow_cache();
                    self.report(result.message, false);
                }
                self.surface.set_batch(false);
                self.surface.refresh_top();
                self.send(Work::Spaces);
                if self.surface.visible {
                    self.load(false);
                }
                self.schedule_prewarm();
                if !self.window.get_modal() && self.window.get_route().as_str() == "history" {
                    self.window.invoke_focus_search(false);
                }
            }
            Err(error) => {
                self.edit_created_copy = false;
                self.report(error.clone(), true);
                if self.window.get_route().as_str() == "settings" {
                    self.window.set_settings_error(error.into());
                }
                self.send(Work::Spaces);
            }
        }
    }
    fn batch(&mut self, action: &str) {
        if self.mutation.is_some()
            || self.session.busy()
            || self.surface.space != SpaceId::HISTORY
            || !self.surface.ready
        {
            return;
        }
        let ids = self
            .surface
            .selected_ids
            .iter()
            .copied()
            .collect::<Vec<_>>();
        match action {
            "begin" => self.surface.set_batch(true),
            "cancel" => self.surface.set_batch(false),
            "all" => self.surface.select_all(),
            "favorite" if !ids.is_empty() => {
                self.space_mutation(SpaceId::FAVORITES, SpaceAction::MoveHistory(ids))
            }
            "pin" if !ids.is_empty() => self.mutate(Mutation::BulkPin(ids)),
            "delete" if !ids.is_empty() => self.ask_confirmation(
                "Delete selected history?",
                "This removes the selected captures. Saved content is not affected.",
                "Delete",
                true,
                Confirmation::DeleteHistory(ids),
            ),
            _ => {}
        }
        self.render_selection();
        self.dirty_snapshots.insert(self.surface.space);
        self.schedule_prewarm();
    }
    fn interpret_key(&self, text: &str, ctrl: bool, shift: bool, target: &str) -> Intent {
        use echo_presentation::interaction::{self, Key, Target};
        use slint::platform::Key as NativeKey;
        let mut key = text;
        for (native, name) in [
            (NativeKey::Return, "Enter"),
            (NativeKey::Escape, "Escape"),
            (NativeKey::Tab, "Tab"),
            (NativeKey::UpArrow, "ArrowUp"),
            (NativeKey::DownArrow, "ArrowDown"),
            (NativeKey::LeftArrow, "ArrowLeft"),
            (NativeKey::RightArrow, "ArrowRight"),
            (NativeKey::Home, "Home"),
            (NativeKey::End, "End"),
            (NativeKey::F6, "F6"),
        ] {
            if slint::SharedString::from(native).as_str() == text {
                key = name;
                break;
            }
        }
        if key == "F6" && self.window.get_route().as_str() != "history" {
            return Intent::None;
        }
        let target = match target {
            "search" => Target::Search,
            "row" => Target::Row,
            "control" => Target::Control,
            _ => Target::Surface,
        };
        let intent = interaction::interpret_space(
            Key {
                text: key,
                ctrl,
                shift,
                composing: self.hook.as_ref().is_some_and(WindowHook::is_composing),
                target,
                text_edit: true,
                batch: self.surface.batch,
            },
            self.ui.switch_shortcut,
            !self.window.get_control_focus_mode(),
            self.window.get_modal(),
            self.window.get_route().as_str() != "history",
        );
        if (self.session.busy() || self.mutation.is_some())
            && !matches!(intent, Intent::Escape | Intent::None)
        {
            Intent::PreventDefault
        } else {
            intent
        }
    }
    fn keyboard(&mut self, intent: Intent) {
        match intent {
            Intent::None | Intent::PreventDefault => {}
            Intent::Escape => self.escape(),
            Intent::FocusSearch(select) => {
                self.window.set_control_focus_mode(false);
                self.window.invoke_focus_search(select);
            }
            Intent::FocusMode => {
                let value = !self.window.get_control_focus_mode();
                self.window.set_control_focus_mode(value);
                if value {
                    self.window.invoke_focus_controls();
                } else {
                    self.window.invoke_focus_search(false);
                }
            }
            Intent::SwitchSpace(delta) => self.navigate(delta),
            Intent::SwitchPanel => self.navigate(1),
            Intent::NewSpace => self.space_action("new", ""),
            Intent::NewItem => self.new_item(),
            Intent::Move(delta) => {
                if self.surface.ready {
                    self.surface.move_selection(delta);
                    self.render_selection();
                    self.window.invoke_reveal_selection();
                    self.dirty_snapshots.insert(self.surface.space);
                    self.schedule_prewarm();
                }
            }
            Intent::Select(index) => {
                if self.surface.ready {
                    self.surface.select_index(index);
                    self.render_selection();
                    self.window.invoke_reveal_selection();
                    self.dirty_snapshots.insert(self.surface.space);
                    self.schedule_prewarm();
                }
            }
            Intent::ToggleBatch => {
                if let Some(key) = self.surface.selection {
                    self.surface.toggle_selected(key.id);
                    self.render_selection();
                }
            }
            Intent::SelectAllBatch => {
                self.surface.select_all();
                self.render_selection();
            }
            Intent::Primary | Intent::Copy => {
                self.finish_motion();
                if self.surface.ready && self.deck.can_insert(self.surface.space) {
                    if let Some(key) = self.surface.selection {
                        self.execute(
                            key,
                            if intent == Intent::Copy {
                                QuickInsertAction::Copy
                            } else {
                                QuickInsertAction::Insert
                            },
                        );
                    }
                } else {
                    self.report(
                        "Content is still loading; press Enter again when ready",
                        false,
                    );
                }
            }
        }
    }
}
