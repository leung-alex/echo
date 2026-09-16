//! One native window, one insertion session, and independently identified content spaces.
use crate::{
    events::{Command, Event, Hub, PixelData},
    formatting,
    service::{Mutation, Work, Worker},
    AppWindow,
};
use echo_engine::*;
use echo_presentation::{
    interaction::Intent,
    navigation::{Navigation, Phase},
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
mod editor_validation;
mod inline_completion;
mod input_indicator;
#[cfg(feature = "native-test")]
pub(crate) mod native_test;
mod quick_insert_window;
mod settings_controller;
mod settings_geometry;
mod side_previews;
mod software_deck;
mod styles;
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
/// One presentation clock updates geometry immediately before the next frame.
/// No separate animation polling timer competes with the renderer's 60 Hz cap.
pub fn before_software_frame() {
    let _ = APP.try_with(|slot| {
        let slot = slot.borrow();
        let Some(app) = slot.as_ref() else { return };
        let Ok(mut app) = app.try_borrow_mut() else {
            return;
        };
        if !app.surface.visible {
            return;
        }
        if app.software.slide.moving() {
            app.software_tick();
        }
        if app.software.slide.moving() {
            app.window.window().request_redraw();
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
    quality: HashMap<String, crate::image_preview::PreviewSize>,
    main_sizes: HashMap<String, crate::image_preview::PreviewSize>,
    cache: HashMap<String, (slint::Image, usize)>,
    order: VecDeque<String>,
    // Bounded request identities only; hidden reclamation still releases pixels.
    recent_main: VecDeque<String>,
    pending: HashSet<String>,
    bytes: usize,
    epoch: u64,
}
impl Images {
    fn remember_main(&mut self, hash: &str) {
        self.recent_main.retain(|key| key != hash);
        self.recent_main.push_back(hash.to_owned());
        while self.recent_main.len() > 8 {
            if let Some(old) = self.recent_main.pop_front() {
                self.main_sizes.remove(&old);
            }
        }
    }
    fn trim_to(&mut self, limit: usize) -> bool {
        let mut changed = false;
        while self.bytes > limit {
            let Some(key) = self.order.pop_front() else {
                break;
            };
            self.quality.remove(&key);
            if let Some((_, size)) = self.cache.remove(&key) {
                self.bytes = self.bytes.saturating_sub(size);
                changed = true;
            }
        }
        changed
    }
}

#[cfg(test)]
mod image_reclamation_tests {
    use super::Images;

    #[test]
    fn reclaim_pixels_preserves_only_bounded_recent_main_requests() {
        let mut images = Images::default();
        for index in 0..12 {
            let key = index.to_string();
            images.remember_main(&key);
            images.cache.insert(key.clone(), (Default::default(), 4));
            images.order.push_back(key);
            images.bytes += 4;
        }
        images.remember_main("4");
        images.remember_main("12");
        assert!(images.trim_to(0));
        assert_eq!(images.bytes, 0);
        assert!(images.cache.is_empty());
        assert!(images.order.is_empty());
        assert_eq!(images.recent_main.len(), 8);
        assert!(!images.recent_main.iter().any(|key| key == "5"));
        assert!(images.recent_main.iter().any(|key| key == "4"));
        assert_eq!(images.recent_main.back().map(String::as_str), Some("12"));
    }
}

struct Preview {
    // Read-only neighbor results are bounded and matched to query/revision.
    query: String,
    items: Vec<QuickInsertItem>,
    revision: i64,
    total: u64,
}
pub struct App {
    styles: crate::style::StyleSnapshot,
    style_theme: Option<(bool, bool)>,
    window: AppWindow,
    surface: Surface,
    model: Rc<crate::native_model::EntryModel>,
    model_bytes: usize,
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
    pending_focus: Option<echo_windows::focus::FocusSnapshot>,
    activation_focus: Option<echo_windows::focus::FocusSnapshot>,
    popup_anchor: Option<echo_windows::focus::PopupAnchor>,
    popup_placement: Option<echo_windows::focus::PopupPlacement>,
    popup_side_right: Option<bool>,
    manager_geometry: Option<(slint::PhysicalPosition, slint::PhysicalSize)>,
    settings_geometry: settings_geometry::SettingsGeometry,
    quick_geometry_active: bool,
    capture_pending: bool,
    pending_reclaim: Option<(u64, u64)>,
    hidden_generation: u64,
    inline_ui: inline_completion::InlineUi,
    inline_timer: Timer,
    input_indicator: input_indicator::Indicator,
    compatibility_notice: Option<String>,
    mutation: Option<u64>,
    serial: u64,
    quitting: bool,
    restart: bool,
    clock: Instant,
    deck: Navigation,
    software: software_deck::SoftwareDeck,
    spaces: Vec<Space>,
    positions: SpacePositions,
    search_timer: Timer,
    trim_timer: Timer,
    preview_epoch: u64,
    previews: HashMap<SpaceId, Preview>,
    pending_previews: HashSet<SpaceId>,
    window_shapes: Option<Option<Vec<shell::CardShape>>>,
    geometry: (u32, u32, u32, u32, u32, u32),
    pending_card_region: Option<card_window::PendingCardFrame>,
    card_region_serial: u64,
    card_region_generation: Rc<std::cell::Cell<u64>>,
    popup_first_frame_pending: bool,
    pending_scroll: Option<f32>,
    navigate_after_refresh: Option<SpaceId>,
    graphics: crate::graphics::GraphicsInfo,
    environment: shell::UiEnvironment,
    tray: shell::TrayController,
    active_language: Option<Language>,
    native_theme: Option<(isize, bool)>,
    confirmation: Option<Confirmation>,
    picker: Picker,
    picker_generation: u64,
    picker_cursor: Option<PageCursor>,
    picker_items: Vec<QuickInsertItem>,
    editor_key: Option<RowKey>,
    editor_original: Option<(String, String, String)>,
    editor_tags: Vec<String>,
    space_edit_id: Option<(SpaceId, i64)>,
    space_original: Option<(String, String, String)>,
    inspect_intent: Option<(u64, String, RowKey)>,
    edit_created_copy: bool,
    wheel_delta: f32,
}
impl App {
    pub fn new(
        hub: Arc<Hub>,
        worker: Worker,
        args: Vec<String>,
        graphics: crate::graphics::GraphicsInfo,
        tray: shell::TrayController,
        styles: crate::style::StyleSnapshot,
    ) -> Result<Rc<RefCell<Self>>, String> {
        let window = AppWindow::new().map_err(|e| e.to_string())?;
        window
            .global::<crate::SelectEnvironment>()
            .on_filter(crate::select::filter);
        window
            .global::<crate::I18n>()
            .on_translate(|language, source| {
                crate::i18n::text(Language::parse(&language).unwrap_or_default(), &source).into()
            });
        window.set_software_deck(true);
        window.window().set_size(slint::LogicalSize::new(
            echo_presentation::echo_tokens::WINDOW_WIDTH,
            echo_presentation::echo_tokens::WINDOW_HEIGHT,
        ));
        let model = Rc::new(crate::native_model::EntryModel::default());
        window.set_rows(ModelRc::from(model.clone()));
        let bootstrap = worker.bootstrap.clone();
        let mut surface = Surface::new(QuickInsertView::History);
        surface.row_limit = 40;
        let environment = shell::ui_environment(None);
        let input_indicator =
            input_indicator::Indicator::new(bootstrap.ui.input_method_indicator, hub.clone())?;
        let app = Rc::new(RefCell::new(Self {
            styles,
            style_theme: None,
            window,
            surface,
            model,
            model_bytes: 0,
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
            pending_focus: None,
            activation_focus: None,
            popup_anchor: None,
            popup_placement: None,
            popup_side_right: None,
            manager_geometry: None,
            settings_geometry: Default::default(),
            quick_geometry_active: false,
            capture_pending: false,
            pending_reclaim: None,
            hidden_generation: 0,
            inline_ui: Default::default(),
            inline_timer: Timer::default(),
            input_indicator,
            compatibility_notice: None,
            mutation: None,
            serial: 0,
            quitting: false,
            restart: false,
            clock: Instant::now(),
            deck: Navigation::default(),
            software: Default::default(),
            spaces: Vec::new(),
            positions: SpacePositions::default(),
            search_timer: Timer::default(),
            trim_timer: Timer::default(),
            preview_epoch: 0,
            previews: HashMap::new(),
            pending_previews: HashSet::new(),
            window_shapes: None,
            geometry: (0, 0, 0, 0, 0, 0),
            pending_card_region: None,
            card_region_serial: 0,
            card_region_generation: Rc::new(std::cell::Cell::new(0)),
            popup_first_frame_pending: false,
            pending_scroll: None,
            navigate_after_refresh: None,
            graphics,
            environment,
            tray,
            active_language: None,
            native_theme: None,
            confirmation: None,
            picker: Picker::Closed,
            picker_generation: 0,
            picker_cursor: None,
            picker_items: Vec::new(),
            editor_key: None,
            editor_original: None,
            editor_tags: Vec::new(),
            space_edit_id: None,
            space_original: None,
            inspect_intent: None,
            edit_created_copy: false,
            wheel_delta: 0.0,
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
            .set_busy(self.capture_pending || self.session.busy() || self.mutation.is_some());
    }
    fn handle(&mut self, event: Event) {
        if self.quitting {
            return;
        }
        match event {
            #[cfg(debug_assertions)]
            Event::Styles(styles) => {
                self.styles = styles;
                self.style_theme = None;
                self.apply_styles();
                eprintln!("[Echo styles] applied");
            }
            Event::Inline(event) => self.inline_event(event),
            Event::InputIndicator(update) => self.input_indicator.update(update),
            Event::InputIndicatorExpired => {}
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
                let old_window = self.surface.window_start;
                if self.surface.finish_load(ticket, result.map(|p| p.page)) {
                    if self.surface.window_start != old_window {
                        self.pending_scroll = Some(0.0);
                    }
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
                    if self.inline_active() {
                        self.prepare_window_geometry();
                    }
                    self.content_ready();
                    self.inline_results_ready();
                    self.schedule_prewarm();
                }
            }
            Event::Preview(space, epoch, result) => self.preview_loaded(space, epoch, result),
            Event::SidePreviewCancelled(space, epoch) => {
                if epoch == self.preview_epoch {
                    self.pending_previews.remove(&space);
                    self.prepare_software_neighbors();
                }
            }
            Event::Inspected(generation, result) => self.inspected(generation, result),
            Event::Catalog(generation, result) => self.catalog_loaded(generation, result),
            Event::Activated(epoch, context, result) => {
                if epoch != self.session.epoch {
                    return;
                }
                if self.inline_ui.unavailable
                    && (self.worker.inline.readiness()[0] != epoch
                        || !self
                            .activation_focus
                            .as_ref()
                            .is_some_and(echo_windows::focus::FocusSnapshot::still_current))
                {
                    self.activation_focus = None;
                    self.dismiss();
                    return;
                }
                match result {
                    Ok(result) => {
                        self.popup_anchor = result.anchor;
                        if context == Context::QuickInsert
                            && self.activation_focus.as_ref().is_some_and(|snapshot| {
                                !snapshot.is_echo() && !snapshot.still_current()
                            })
                        {
                            self.activation_focus = None;
                            self.dismiss();
                            return;
                        }
                        let has_target = result.target.is_some();
                        if self.inline_ui.unavailable && has_target {
                            if !self.stop_inline() {
                                return;
                            }
                            self.inline_ui.plain_paste = true;
                        }
                        if !self.send(Work::Adopt(epoch, result.target)) {
                            self.capture_pending = false;
                            self.dismiss();
                            return;
                        }
                        self.session.capture_finished(epoch, has_target);
                    }
                    Err(e) => {
                        self.session.capture_finished(epoch, false);
                        self.report(e, true);
                    }
                }
                self.capture_pending = false;
                self.window
                    .set_paste_target_available(self.session.has_target);
                self.set_busy();
                self.window
                    .set_quick_insert(context == Context::QuickInsert);
                if context == Context::QuickInsert && !self.session.has_target {
                    self.report("Copy only: no safe paste target was captured", false);
                }
                if let Err(e) = self.show_window_loading(true) {
                    self.report(e, true);
                }
                if let Some(notice) = self.compatibility_notice.take() {
                    let action = if self.session.has_target {
                        "Select a history item to insert."
                    } else {
                        "Browse history and copy manually; your input is unchanged."
                    };
                    self.report(format!("{notice}. {action}"), false);
                }
                self.schedule_prewarm();
            }
            Event::Executed(operation, result) => self.executed(operation, result),
            Event::Mutated(serial, result) => self.mutated(serial, result),
            Event::Thumbnail(epoch, hash, result) => self.thumbnail_finished(epoch, hash, result),
            Event::Invalidated => self.history_invalidated(),
            Event::SearchCacheTrimmed(epoch, hidden_generation) => {
                if self.pending_reclaim == Some((epoch, hidden_generation))
                    && self.hidden_generation == hidden_generation
                    && self.can_reclaim_hidden(epoch)
                {
                    self.pending_reclaim = None;
                    crate::popup_timing::mark("hidden_reclaimed");
                    crate::memory_trace::record(
                        "hidden_reclaimed",
                        serde_json::json!({
                            "epoch":epoch, "hidden_generation":hidden_generation, "query_epoch":self.surface.query_epoch(),
                            "rows":self.model.row_count(), "thumbnail_bytes":self.images.bytes,
                            "model_bytes":self.model_bytes, "software_frame_bytes":crate::graphics::software_frame_bytes(), "main_backend_retained":true,
                        "worker_cache_acknowledged":true
                        ,"display_queue_high_water_bytes":self.hub.data_high_water()
                        }),
                    );
                } else if self.pending_reclaim == Some((epoch, hidden_generation)) {
                    self.retry_hidden_reclaim(epoch, hidden_generation);
                }
            }
            Event::DiagnosticExported(result) => match result {
                Ok(path) => self.report(format!("Diagnostics saved: {path}"), false),
                Err(e) => {
                    self.window.set_settings_error(e.clone().into());
                    self.report(e, true);
                }
            },
        }
        self.inline_results_ready();
        self.update_card_region();
        self.input_indicator.sync(
            self.ui.input_method_indicator,
            self.capture_pending
                || self.inline_ui.pending
                || (self.surface.visible
                    && self.quick_geometry_active
                    && !self.inline_ui.editor_focus),
            self.window.global::<crate::EchoTheme>(),
            self.environment,
        );
    }
    fn shell_event(&mut self, event: ShellEvent) {
        match event {
            ShellEvent::QuickInsert(snapshot) => self.hotkey_activate(snapshot),
            ShellEvent::HotkeyStatus(status) => {
                let previous = self.window.get_hotkey_status();
                self.window.set_hotkey_registration_failed(
                    !formatting::hotkey_status_is_informational(&status),
                );
                if !formatting::hotkey_status_is_informational(&status) {
                    self.window.set_settings_error(status.clone().into());
                } else if self.window.get_settings_error() == previous {
                    self.window.set_settings_error("".into());
                }
                self.window.set_hotkey_status(status.into());
            }
            ShellEvent::FocusLost => self.external_focus_lost(),
            ShellEvent::Open => self.activate_args(Vec::new()),
            ShellEvent::Favorites => self.activate_args(vec!["--favorites".into()]),
            ShellEvent::Settings => self.activate_args(vec!["--settings".into()]),
            ShellEvent::Quit => self.request_quit(),
            ShellEvent::Activation(args) => self.activate_args(args),
            ShellEvent::ThemeChanged => {
                self.native_theme = None;
                self.environment = {
                    let _timing = crate::popup_timing::span("environment_update");
                    shell::ui_environment(self.hwnd)
                };
                self.apply_theme();
                self.viewport_changed();
            }
            ShellEvent::GeometryChanged => {
                if let Some(hwnd) = self.hwnd.filter(|_| !self.quick_geometry_active) {
                    let _ = shell::fit_window(hwnd, false, 16.0);
                }
                self.environment = {
                    let _timing = crate::popup_timing::span("environment_update");
                    shell::ui_environment(self.hwnd)
                };
                self.viewport_changed();
                self.remember_settings_geometry();
            }
            ShellEvent::Error(e) => self.report(e, true),
        }
    }
    fn activate_args(&mut self, args: Vec<String>) {
        let snapshot = self
            .pending_focus
            .take()
            .unwrap_or_else(echo_windows::focus::FocusSnapshot::capture);
        self.activate_from(args, snapshot);
    }
    fn activate_from(&mut self, args: Vec<String>, snapshot: echo_windows::focus::FocusSnapshot) {
        if args.first().map(String::as_str) == Some("--quit") {
            self.request_quit();
            return;
        }
        if !self.ready || self.spaces.is_empty() {
            self.pending_args = Some(args);
            self.pending_focus = Some(snapshot);
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
        let mut context = if args.first().map(String::as_str) == Some("--quick-insert") {
            Context::QuickInsert
        } else {
            Context::Manager
        };
        let mut query = String::new();
        let mut route = "history";
        let mut id = self.ui.startup_space.resolve(
            self.ui.resume_last_space_id.as_deref(),
            self.spaces.iter().map(|space| space.id),
        );
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
        query.clear();
        if !self.spaces.iter().any(|s| s.id == id) {
            id = SpaceId::HISTORY;
        }
        if !self.stop_inline() {
            return;
        }
        self.compatibility_notice = None;
        self.remember_position();
        self.cancel_software_slide();
        self.surface.hide();
        self.surface.set_space(id);
        self.surface.set_query(query.clone());
        self.pending_scroll = if self.ui.remember_position {
            Some(self.positions.restore(&mut self.surface))
        } else {
            Some(0.0)
        };
        self.deck.show(id);
        self.cancel_prewarm();

        self.window.set_route(route.into());
        self.window.set_query(query.clone().into());
        self.window.set_stale_rows(true);
        self.window.set_navigation_busy(false);
        self.window.set_control_focus_mode(false);
        self.window.set_editor_open(false);
        self.render_navigation();
        self.render_settings();
        if context == Context::QuickInsert {
            let _ = self.window.hide();
        }
        self.activation_focus = (context == Context::QuickInsert).then_some(snapshot);
        self.popup_anchor = None;
        self.popup_placement = None;
        self.capture_pending = true;
        let epoch = self.session.activate(context);
        self.worker.epoch.store(epoch, Ordering::Release);
        self.set_busy();
        // Filtering is driven only by the original input, never a hidden local query.
        if context == Context::QuickInsert {
            if let Some(snapshot) = self.activation_focus.clone() {
                self.begin_inline(epoch, snapshot);
                return;
            }
        }
        // Capturing the external insertion target completes before showing the window.
        if !self.send(Work::Begin(epoch, context, self.activation_focus.clone())) {
            self.capture_pending = false;
            self.session.dismiss();
            self.worker
                .epoch
                .store(self.session.epoch, Ordering::Release);
            self.set_busy();
        }
    }
    fn show_window(&mut self) -> Result<(), String> {
        self.show_window_loading(false)
    }
    fn show_window_loading(&mut self, load_content: bool) -> Result<(), String> {
        let first = self.hwnd.is_none();
        if self.session.context == Context::QuickInsert && self.popup_anchor.is_some() {
            // ShowWindow can expose the previous swapchain before RedrawRequested.
            // Cloak before moving/resizing; the generation-bound frame commit below
            // reveals only the new layout, without a timer or a side-switch animation.
            if let Some(hwnd) = self.hwnd {
                shell::cloak_card_frame(hwnd, true)?;
            }
            self.popup_first_frame_pending = true;
            crate::popup_timing::mark("first_frame_hidden");
            self.window_shapes = None;
            self.pending_card_region = None;
            self.card_region_generation.set(0);
        }
        let center = self.prepare_window_geometry();
        if let Some(hook) = &self.hook {
            hook.set_inline_popup(self.popup_preserves_input_focus())?;
        }
        self.trim_timer.stop();
        self.hidden_generation = self.hidden_generation.wrapping_add(1);
        self.pending_reclaim = None;
        self.surface.visible = true;
        if load_content {
            // Let storage work overlap visible-side rendering and HWND setup.
            self.load(false);
        }
        if self.popup_first_frame_pending {
            self.request_recent_thumbnails();
        }
        if self.deck.phase == Phase::Suspended {
            self.deck.show(self.surface.space);
        }

        self.window.show().map_err(|e| e.to_string())?;
        crate::memory_trace::record(
            "window_shown",
            serde_json::json!({"epoch":self.session.epoch, "query_epoch":self.surface.query_epoch()}),
        );
        crate::popup_timing::mark("show_returned");
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
            if self.popup_first_frame_pending {
                shell::cloak_card_frame(hwnd, true)?;
            }
            // Propagate the redraw. The queued command runs after winit finishes
            // this software draw/present.
            use slint::winit_030::{EventResult, WinitWindowAccessor};
            let generation = self.card_region_generation.clone();
            let hub = self.hub.clone();
            crate::graphics::software_card_commits(generation, hub);
            self.window.window().on_winit_window_event(move |_, event| {
                if matches!(
                    event,
                    slint::winit_030::winit::event::WindowEvent::RedrawRequested
                ) {
                    crate::memory_trace::record("frame_redraw", serde_json::Value::Null);
                }
                EventResult::Propagate
            });
        }
        if let Some(hwnd) = self.hwnd {
            if !self.quick_geometry_active {
                shell::fit_window(hwnd, first || center, 16.0)?;
            }
            if let Some(hook) = &self.hook {
                hook.set_inline_popup(self.popup_preserves_input_focus())?;
            }
            if !self.popup_preserves_input_focus() {
                shell::focus_window(hwnd)?;
            }
        }
        self.environment = {
            let _timing = crate::popup_timing::span("environment_update");
            shell::ui_environment(self.hwnd)
        };
        self.apply_theme();
        if self.popup_preserves_input_focus() {
            // Keyboard focus remains in the original input.
        } else if self.window.get_route().as_str() == "history" {
            self.window.invoke_focus_content();
        } else {
            self.window.invoke_focus_controls();
        }
        Ok(())
    }
    fn can_reclaim_hidden(&self, epoch: u64) -> bool {
        crate::memory_lifecycle::ReclaimBarrier {
            epoch: self.session.epoch,
            visible: self.surface.visible,
            capture_pending: self.capture_pending,
            inserting: self.session.busy(),
            inline_pending: self.inline_ui.pending,
            inline_active: self.inline_active(),
            mutating: self.mutation.is_some(),
            modal: self.window.get_modal(),
        }
        .permits(epoch)
    }
    fn retry_hidden_reclaim(&mut self, epoch: u64, hidden_generation: u64) {
        // A transaction may outlive the initial timer. Retry only the same
        // hidden session; a new activation must never inherit this timer.
        if self.surface.visible
            || self.session.epoch != epoch
            || self.hidden_generation != hidden_generation
        {
            return;
        }
        let hub = self.hub.clone();
        self.trim_timer.start(
            TimerMode::SingleShot,
            Duration::from_millis(100),
            move || {
                hub.post(Event::Command(Command::TrimHidden(
                    epoch,
                    hidden_generation,
                )))
            },
        );
    }
    fn hide_window(&mut self) {
        self.remember_settings_geometry();
        self.cancel_software_slide();
        self.hidden_generation = self.hidden_generation.wrapping_add(1);
        self.pending_reclaim = None;
        crate::popup_timing::finish();
        self.remember_position();
        self.search_timer.stop();
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
                    self.model
                        .update_visual(index, |current| current.thumbnail = row.thumbnail);
                }
            }
        }
        self.window.set_navigation_busy(false);
        let _ = self.window.hide();
        crate::memory_trace::record(
            "hidden_warm",
            serde_json::json!({"epoch":self.session.epoch, "hidden_generation":self.hidden_generation, "query_epoch":self.surface.query_epoch()}),
        );
        {
            let hub = self.hub.clone();
            let epoch = self.session.epoch;
            let hidden_generation = self.hidden_generation;
            self.trim_timer
                .start(TimerMode::SingleShot, Duration::from_secs(30), move || {
                    hub.post(Event::Command(Command::TrimHidden(
                        epoch,
                        hidden_generation,
                    )))
                });
        }
        // The next activation resolves its home space from the in-process
        // settings. Persisting on the worker alone leaves "last" stale until
        // a restart or another settings snapshot arrives.
        self.ui.resume_last_space_id = Some(self.surface.space.to_string());
        self.send(Work::Resume(self.surface.space));
    }
    fn dismiss(&mut self) {
        if !self.stop_inline() {
            return;
        }
        self.capture_pending = false;
        // Invalidate the dismissed session before hide_window captures the
        // reclamation epoch. Otherwise every normal close invalidates its own
        // 30-second timer immediately after arming it.
        self.session.dismiss();
        self.worker
            .epoch
            .store(self.session.epoch, Ordering::Release);
        self.hide_window();
        if let Some(snapshot) = self.activation_focus.take() {
            echo_windows::focus::restore_after_dismiss(&snapshot);
        }
        self.window.set_paste_target_available(false);
        self.send(Work::Cancel);
        self.set_busy();
    }
    fn quit(&mut self) {
        self.stop_inline();
        self.quitting = true;
        self.search_timer.stop();
        self.trim_timer.stop();
        let _ = self.window.hide();
        self.hub.close();
        let _ = slint::quit_event_loop();
    }
    pub fn shutdown(&mut self) {
        self.input_indicator.stop();
        crate::graphics::release_software_frame();
        self.stop_inline();
        self.quitting = true;
        self.hub.close();

        self.worker.stop();
    }
    pub fn take_restart(&mut self) -> bool {
        std::mem::take(&mut self.restart)
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
            self.surface.retry_unqueued_load(ticket, more);
            let hub = self.hub.clone();
            self.search_timer.start(
                TimerMode::SingleShot,
                Duration::from_millis(100),
                move || {
                    hub.post(Event::Command(if more {
                        Command::More
                    } else {
                        Command::Refresh
                    }))
                },
            );
        }
    }
    fn render(&mut self) {
        // The settled render publishes any changes received during the slide.
        if self.software.slide.moving() {
            return;
        }
        let _timing = crate::popup_timing::span("main_model_update");
        if (self.surface.loading || self.surface.dirty)
            && (self.surface.presented_query.is_some() || self.model.row_count() > 0)
        {
            // Preserve complete visual rows, including highlight spans and action
            // strips, until the next query is ready. Safety is a separate gate.
            self.window.set_loading(true);
            if self.surface.dirty && !self.surface.loading && !self.search_timer.running() {
                self.load(false);
            }
            return;
        }
        let selected = if self.surface.loading || self.surface.dirty {
            self.model
                .iter()
                .find(|r| r.selected)
                .map(|r| r.key.to_string())
        } else {
            self.surface.selection.map(|key| key.to_string())
        };
        let rows = {
            let mut section = String::new();
            self.surface
                .items
                .iter()
                .map(|item| {
                    let label = formatting::row_section(item, &mut section);
                    let (row, bytes) = echo_windows::allocation::measure_owned(|| {
                        let mut row = formatting::row_content(item, &label);
                        if self.surface.ready && !self.surface.query.trim().is_empty() {
                            let mut matcher = FuzzyMatcher::new(&self.surface.query);
                            crate::match_highlight::apply(
                                &mut row,
                                &mut matcher,
                                &self.highlight_color(),
                            );
                        }
                        row.selected =
                            selected.as_deref() == Some(RowKey::of(item).to_string().as_str());
                        row.batch_selected = self.surface.selected_ids.contains(&item.id);
                        if let Some(image) = item
                            .thumbnail
                            .as_ref()
                            .and_then(|t| self.images.cache.get(&t.source_hash))
                        {
                            row.thumbnail = image.0.clone();
                        }
                        row
                    });
                    crate::native_model::OwnedRow { row, bytes }
                })
                .collect::<Vec<_>>()
        };
        self.model.reconcile(rows);
        self.window
            .set_drag_epoch(self.window.get_drag_epoch().wrapping_add(1));
        self.model_bytes = self.model.held_bytes();
        crate::memory_trace::record(
            "display_data",
            serde_json::json!({"page_bytes":self.surface.held_item_bytes(),"row_model_bytes":self.model_bytes,"side_bytes":self.software_side_bytes(),"outgoing_bytes":self.software.outgoing_bytes,"queued_bytes":self.hub.data_bytes(),"limit":4*1024*1024}),
        );
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
        if self.surface.ready || self.surface.presented_query.is_none() {
            let empty = if self.surface.loading {
                "Loading…"
            } else if self.surface.error {
                "Unable to load this space"
            } else if !self.surface.query.is_empty() {
                "No matches in this space"
            } else if self.surface.space == SpaceId::HISTORY {
                "Your clipboard history appears here"
            } else {
                "A space for things you use often"
            };
            self.window.set_empty_state_text(empty.into());
        }
        self.render_navigation();
        if self.surface.ready {
            if let Some(scroll) = self.pending_scroll.take() {
                self.window.set_scroll_y(scroll);
            }
        }
        if self.surface.dirty && !self.surface.loading && !self.search_timer.running() {
            self.load(false);
        }
    }
    fn render_selection(&self) {
        if self.surface.loading || self.surface.dirty {
            return;
        }
        for (index, item) in self.surface.items.iter().enumerate() {
            if let Some(mut row) = self.model.row_data(index) {
                let selected = self.surface.selection == Some(RowKey::of(item));
                let batch = self.surface.selected_ids.contains(&item.id);
                if row.selected != selected || row.batch_selected != batch {
                    row.selected = selected;
                    row.batch_selected = batch;
                    self.model.update_visual(index, |current| {
                        current.selected = selected;
                        current.batch_selected = batch;
                    });
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
    fn request_recent_thumbnails(&mut self) {
        if self.surface.presented_query.as_deref() != Some(self.surface.query.as_str()) {
            return;
        }
        // Re-request only thumbnails previously needed by the live viewport and
        // still present in this query. Queue reads before the first Slint render
        // so reclamation does not force a placeholder frame on activation.
        let keys: Vec<_> = self
            .surface
            .items
            .iter()
            .filter(|item| {
                item.thumbnail
                    .as_ref()
                    .is_some_and(|t| self.images.recent_main.contains(&t.source_hash))
            })
            .map(|item| RowKey::of(item).to_string())
            .collect();
        for key in keys {
            let size = self
                .surface
                .resolve_key(&key)
                .and_then(|k| self.surface.items.iter().find(|i| RowKey::of(i) == k))
                .and_then(|i| i.thumbnail.as_ref())
                .and_then(|t| self.images.main_sizes.get(&t.source_hash))
                .copied()
                .unwrap_or_default();
            self.thumbnail_request(key, size);
        }
    }
    fn thumbnail_request(&mut self, key: String, size: crate::image_preview::PreviewSize) {
        if !self.surface.visible || self.window.get_route().as_str() != "history" {
            return;
        }
        let Some(key) = self.surface.resolve_key(&key) else {
            return;
        };
        let Some(asset) = self
            .surface
            .items
            .iter()
            .find(|x| RowKey::of(x) == key)
            .and_then(|x| x.thumbnail.as_ref())
            .cloned()
        else {
            return;
        };
        let hash = asset.source_hash.clone();
        self.images.remember_main(&hash);
        self.images.main_sizes.insert(hash.clone(), size);
        // Frame readiness drained prior reads. Defer newly visible requests until
        // the slide settles, so no cache eviction can mutate its row models.
        if self.software.slide.moving() {
            return;
        }
        if (self.images.cache.contains_key(&hash)
            && self
                .images
                .quality
                .get(&hash)
                .is_some_and(|q| q.covers(size)))
            || !self.images.pending.insert(hash.clone())
        {
            return;
        }
        if !self.send(Work::Thumbnail(self.images.epoch, asset, size)) {
            self.images.pending.remove(&hash);
        } else {
            crate::popup_timing::mark("main_thumbnail_requested");
        }
    }
    fn thumbnail_finished(&mut self, epoch: u64, hash: String, result: Result<PixelData, String>) {
        if epoch != self.images.epoch || !self.surface.visible {
            return;
        }
        self.images.pending.remove(&hash);
        let Ok(pixels) = result else {
            {
                self.software_content_ready();
            }
            return;
        };
        let bytes = pixels.rgba.len();
        if bytes > crate::image_preview::CACHE_BYTES {
            return;
        }
        if self
            .images
            .quality
            .get(&hash)
            .is_some_and(|q| q.covers(pixels.requested))
            && self.images.cache.contains_key(&hash)
        {
            return;
        }
        if let Some((_, old_bytes)) = self.images.cache.remove(&hash) {
            self.images.bytes = self.images.bytes.saturating_sub(old_bytes);
        }
        self.images.order.retain(|key| key != &hash);
        self.images.quality.insert(hash.clone(), pixels.requested);
        if self
            .images
            .trim_to(crate::image_preview::CACHE_BYTES - bytes)
        {
            for index in 0..self.model.row_count() {
                if let Some(mut row) = self.model.row_data(index) {
                    row.thumbnail = Default::default();
                    self.model
                        .update_visual(index, |current| current.thumbnail = row.thumbnail);
                }
            }
        }
        let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
            &pixels.rgba,
            pixels.width,
            pixels.height,
        );
        let image = slint::Image::from_rgba8(buffer);
        self.images.bytes += bytes;
        self.images.order.push_back(hash.clone());
        self.images.cache.insert(hash, (image, bytes));
        self.software.image_version = self.software.image_version.wrapping_add(1);
        if !self.software.slide.loading() {
            self.render_software_side_previews();
        }
        crate::popup_timing::mark("main_thumbnail_ready");
        if self.deck.phase != Phase::Animating {
            self.render();
        }
        {
            self.software_content_ready();
        }
        self.schedule_prewarm();
    }
    fn command(&mut self, command: Command) {
        match command {
            Command::InlineTimeout(epoch) => {
                if self.session.epoch == epoch && self.inline_ui.pending {
                    // A timed-out provider may still publish late results. Do
                    // not expose a fallback without an acknowledged guard.
                    self.dismiss();
                    self.report(
                        "Input inspection timed out; invoke again in your input",
                        false,
                    );
                }
            }
            Command::Quit => self.request_quit(),
            Command::Dismiss => self.request_hide(),
            Command::Drag => {
                self.popup_anchor = None;
                self.popup_placement = None;
                if let Some(hwnd) = self.hwnd {
                    let _ = shell::start_drag(hwnd);
                }
            }
            Command::Query(query) => {
                if self.inline_active() {
                    self.worker.inline.invalidate_results();
                }
                if self.deck.phase == Phase::Animating {
                    self.finish_motion();
                }
                if query.len() > 16 * 1024 {
                    self.report("Search text is too large", true);
                    return;
                }
                self.surface.set_query(query);
                // Keep the last complete result set painted while the next query
                // is in flight. Stale results are not executable (epoch/readiness).
                let keep_rows = self.model.row_count() > 0;
                if !keep_rows {
                    self.window.set_scroll_y(0.0);
                }
                self.pending_scroll = Some(0.0);
                self.deck.block_content();
                self.cancel_prewarm();
                // Keep side-card content until the new query snapshot is ready.
                self.render_software_side_previews();
                self.window.set_stale_rows(!keep_rows);
                self.window.set_loading(true);
                let hub = self.hub.clone();
                self.search_timer.start(
                    TimerMode::SingleShot,
                    Duration::from_millis(if self.inline_active() { 25 } else { 75 }),
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
            Command::Thumbnail(key, size) => self.thumbnail_request(key, size),
            Command::SaveSettings => self.save_settings(),
            Command::SettingsEdited => self.settings_edited(),
            Command::SettingsAction(action) => self.settings_action(&action),
            Command::SpaceAction(action, key) => self.space_action(&action, &key),
            Command::PickerQuery(query) => self.picker_query(query),
            Command::PickerMore => self.picker_more(),
            Command::PickerSelect(key) => self.picker_select(&key),
            Command::Confirm(answer) => self.confirm(&answer),

            Command::TrimHidden(epoch, hidden_generation) => {
                if self.hidden_generation == hidden_generation && self.can_reclaim_hidden(epoch) {
                    if self
                        .worker
                        .send(Work::TrimSearchCache(epoch, hidden_generation))
                        .is_err()
                    {
                        self.retry_hidden_reclaim(epoch, hidden_generation);
                        return;
                    }
                    self.pending_reclaim = Some((epoch, hidden_generation));
                    self.cancel_prewarm();
                    self.preview_epoch = self.preview_epoch.wrapping_add(1);
                    self.pending_previews.clear();
                    self.images.epoch = self.images.epoch.wrapping_add(1);
                    self.images.pending.clear();

                    self.surface.reclaim_hidden();
                    self.model.clear();
                    self.model_bytes = 0;
                    self.previews.clear();
                    self.clear_software_side_models();
                    self.images.trim_to(0);
                    crate::graphics::release_software_frame();
                } else {
                    self.retry_hidden_reclaim(epoch, hidden_generation);
                }
            }
            Command::ViewportChanged => {
                self.sync_inline_editor_focus();
                self.viewport_changed();
                if self.inline_active() {
                    self.prepare_window_geometry();
                }
            }

            Command::CommitCardRegion(generation) => self.commit_card_region(generation),
            Command::SoftwareFrameReady(stamp) => self.software_frame_ready(stamp),
            Command::StageScroll(delta) => self.stage_scroll(delta),
            Command::SaveFavorite => self.save_favorite(),
            Command::CancelEditor => self.cancel_editor(),
            Command::Clear => self.mutate(Mutation::Clear),
            Command::Create => self.new_item(),
            Command::Reorder(origin, target) => {
                let source = origin.key;
                if self.surface.query.is_empty()
                    && self.surface.visible
                    && self.surface.ready
                    && !self.surface.loading
                    && !self.surface.dirty
                    && !self.session.busy()
                    && self.mutation.is_none()
                    && self.deck.can_insert(self.surface.space)
                    && origin.frame == self.software_frame_stamp()
                    && origin.binding == self.window.get_drag_epoch()
                    && self
                        .surface
                        .items
                        .iter()
                        .any(|item| RowKey::of(item) == source)
                    && self
                        .surface
                        .items
                        .iter()
                        .any(|item| RowKey::of(item) == target)
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
        if action == "clear-all" && self.surface.space == SpaceId::HISTORY {
            self.ask_confirmation(
                "Clear unpinned history?",
                "Pinned history, Favorites and custom spaces are kept. This cannot be undone.",
                "Clear",
                true,
                dialogs::Confirmation::ClearHistory,
            );
            return;
        }
        if action == "clear-all" && self.surface.space == SpaceId::FAVORITES {
            self.ask_confirmation(
                "Clear Favorites?",
                "Clears all Favorites, including hidden results. Other spaces and History are kept. This cannot be undone.",
                "Clear Favorites",
                true,
                dialogs::Confirmation::ClearFavorites(self.surface.revision),
            );
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
            "delete" => self.delete_item(key),
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
        if self.inline_active() && action == QuickInsertAction::Insert {
            self.execute_inline_item(key);
            return;
        }
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
        let action = if action == QuickInsertAction::Insert && !self.session.has_target {
            QuickInsertAction::Copy
        } else {
            action
        };
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
        result: Result<QuickInsertOutcome, QuickInsertError>,
    ) {
        let completion = self
            .session
            .finish(operation, result.as_ref().copied().map_err(|_| ()));
        self.set_busy();
        match completion {
            Completion::Stale => {}
            Completion::Inserted => {
                if self.inline_active() {
                    if !self.stop_inline() {
                        return;
                    }
                }
                self.activation_focus = None;
                self.hide_window();
                self.report("Inserted", false);
            }
            Completion::Copied => self.report(
                if self.session.context == Context::QuickInsert && !self.session.has_target {
                    "Copied to clipboard; no safe paste target"
                } else {
                    "Copied"
                },
                false,
            ),
            Completion::Staged => self.report("Copied to clipboard; no active target", false),
            Completion::Restore => {
                if self.inline_active() && operation.action == QuickInsertAction::Insert {
                    let error = result
                        .as_ref()
                        .err()
                        .map(ToString::to_string)
                        .unwrap_or_else(|| "Replacement failed".into());
                    if matches!(
                        result,
                        Err(QuickInsertError::DeliveryFailed(
                            PasteDeliveryFailure::ReplacementUnconfirmed
                                | PasteDeliveryFailure::SelectionUnconfirmed
                                | PasteDeliveryFailure::RangeChanged
                        ))
                    ) {
                        self.inline_ui.suspended = true;
                        self.worker.inline.invalidate_results();
                        self.report(format!("{error}. Enter stays protected. Check the input; Esc keeps it and F6 opens history for copying."), true);
                    } else {
                        self.report(error, true);
                        self.inline_results_ready();
                    }
                    return;
                }
                if operation.action == QuickInsertAction::Insert {
                    let _ = self.show_window();
                    self.load(false);
                }
                self.report(
                    result
                        .err()
                        .map(|e| e.to_string())
                        .unwrap_or_else(|| "Operation failed".into()),
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
        if !self.window.get_editor_open() {
            self.report("Saving…", false);
        }
        if let Err(error) = self.worker.send(Work::Mutate(serial, mutation)) {
            self.mutation = None;
            self.set_busy();
            if !editor_validation::report_save_error(&self.window, &error) {
                self.report(error, true);
            }
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
                    if self.ui.remember_position != snapshot.ui.remember_position {
                        self.positions.clear();
                    }
                    self.settings = snapshot.clipboard;
                    self.ui = snapshot.ui;
                    self.settings_revision = snapshot.revision;
                    self.render_settings();
                    self.apply_theme();

                    self.schedule_prewarm();
                } else if let Some(settings) = result.settings {
                    self.settings = settings;
                    self.render_settings();
                    self.apply_theme();
                }
                if result.editor_saved {
                    self.close_editor();
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

                    self.report(result.message, false);
                }
                if let Some(warning) = result.settings_warning {
                    self.window.set_settings_error(warning.clone().into());
                    self.report(warning, true);
                }
                self.surface.set_batch(false);
                self.surface.refresh_top();
                self.send(Work::Spaces);
                if self.surface.visible {
                    self.load(false);
                }
                self.schedule_prewarm();
                if !self.window.get_modal() && self.window.get_route().as_str() == "history" {
                    self.window.invoke_focus_content();
                }
            }
            Err(error) => {
                self.edit_created_copy = false;
                if !editor_validation::report_save_error(&self.window, &error) {
                    self.report(error.clone(), true);
                }
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
            || self.surface.loading
            || self.surface.dirty
            || !self.deck.can_insert(self.surface.space)
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
                "Deletes selected history. Saved content is kept. This cannot be undone.",
                "Delete",
                true,
                Confirmation::DeleteHistory(ids),
            ),
            _ => {}
        }
        self.render_selection();
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
            (NativeKey::F10, "F10"),
            (NativeKey::Menu, "ContextMenu"),
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
        if self.window.get_route().as_str() == "history"
            && !self.window.get_modal()
            && (!self.surface.ready
                || self.surface.loading
                || self.surface.dirty
                || !self.deck.can_insert(self.surface.space))
            && !matches!(
                intent,
                Intent::None
                    | Intent::PreventDefault
                    | Intent::Escape
                    | Intent::SwitchSpace(_)
                    | Intent::SwitchPanel
                    | Intent::FocusMode
            )
        {
            return;
        }
        match intent {
            Intent::None | Intent::PreventDefault => {}
            Intent::Escape => self.escape(),
            Intent::FocusMode => {
                let value = !self.window.get_control_focus_mode();
                self.window.set_control_focus_mode(value);
                if value {
                    self.window.invoke_focus_controls();
                } else {
                    self.window.invoke_focus_content();
                }
            }
            Intent::SwitchSpace(delta) => self.navigate(delta),
            Intent::SwitchPanel => self.navigate(1),
            Intent::NewSpace => self.space_action("new", ""),
            Intent::NewItem => self.new_item(),
            Intent::ItemOptions => {
                if let Some(key) = self.surface.selection {
                    self.action("options", &key.to_string());
                }
            }
            Intent::Move(delta) => {
                if self.surface.ready {
                    self.surface.move_selection(delta);
                    self.render_selection();
                    self.window.invoke_reveal_selection();
                    self.schedule_prewarm();
                }
            }
            Intent::Select(index) => {
                if self.surface.ready {
                    self.surface.select_index(index);
                    self.render_selection();
                    self.window.invoke_reveal_selection();
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
