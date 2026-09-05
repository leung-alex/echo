//! Native two-window controller. Slint handles stay on this thread.
use crate::{
    events::{Command, Event, Hub, PixelData, Role},
    formatting,
    service::{Mutation, Work, Worker},
    AppWindow, EntryRow,
};
use echo_engine::{
    ClipboardSettings, QuickInsertAction, QuickInsertSource, QuickInsertView, ThemeMode,
};
use echo_presentation::{
    interaction::Intent,
    session::{Completion, Context, RecentActivations, Session},
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
    time::Duration,
};
mod bindings;
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
    APP.with(|slot| {
        if let Some(app) = slot.borrow().as_ref() {
            app.borrow_mut().handle(event);
        }
    });
}
pub fn key_intent(role: Role, key: &str, ctrl: bool, shift: bool, target: &str) -> Option<Intent> {
    APP.with(|slot| {
        slot.borrow().as_ref().and_then(|app| {
            app.try_borrow()
                .ok()
                .map(|a| a.interpret_key(role, key, ctrl, shift, target))
        })
    })
}
struct Images {
    cache: HashMap<String, (slint::Image, usize)>,
    order: VecDeque<String>,
    pending: HashSet<String>,
    bytes: usize,
    epoch: u64,
}
impl Default for Images {
    fn default() -> Self {
        Self {
            cache: HashMap::new(),
            order: VecDeque::new(),
            pending: HashSet::new(),
            bytes: 0,
            epoch: 0,
        }
    }
}
pub struct App {
    windows: [AppWindow; 2],
    surfaces: [Surface; 2],
    models: [Rc<VecModel<EntryRow>>; 2],
    images: [Images; 2],
    hooks: [Option<WindowHook>; 2],
    hwnds: [Option<isize>; 2],
    timers: [Timer; 2],
    hub: Arc<Hub>,
    worker: Worker,
    session: Session,
    recent: RecentActivations,
    settings: ClipboardSettings,
    ready: bool,
    pending_args: Option<Vec<String>>,
    mutation: Option<(Role, u64)>,
    serial: u64,
    restore_favorites: bool,
    quitting: bool,
}
impl App {
    pub fn new(
        hub: Arc<Hub>,
        worker: Worker,
        args: Vec<String>,
    ) -> Result<Rc<RefCell<Self>>, String> {
        let main = AppWindow::new().map_err(|e| e.to_string())?;
        let favorites = AppWindow::new().map_err(|e| e.to_string())?;
        main.window()
            .set_size(slint::LogicalSize::new(824.0, 814.0));
        favorites
            .window()
            .set_size(slint::LogicalSize::new(310.0, 575.0));
        favorites.set_is_favorites_window(true);
        favorites.set_active_view("favorites".into());
        let models = [Rc::new(VecModel::default()), Rc::new(VecModel::default())];
        main.set_rows(ModelRc::from(models[0].clone()));
        favorites.set_rows(ModelRc::from(models[1].clone()));
        let app = Rc::new(RefCell::new(Self {
            windows: [main, favorites],
            surfaces: [
                Surface::new(QuickInsertView::History),
                Surface::new(QuickInsertView::Favorites),
            ],
            models,
            images: Default::default(),
            hooks: [None, None],
            hwnds: [None, None],
            timers: Default::default(),
            hub,
            worker,
            session: Default::default(),
            recent: Default::default(),
            settings: Default::default(),
            ready: false,
            pending_args: Some(args),
            mutation: None,
            serial: 0,
            restore_favorites: true,
            quitting: false,
        }));
        bindings::connect(&app.borrow());
        Ok(app)
    }
    fn send(&mut self, role: Role, work: Work) -> bool {
        if let Err(error) = self.worker.send(work) {
            self.report(role, error, true);
            false
        } else {
            true
        }
    }
    fn report(&mut self, role: Role, text: impl Into<String>, error: bool) {
        let i = role.index();
        self.surfaces[i].report(text, error);
        self.windows[i].set_status(self.surfaces[i].status.clone().into());
        self.windows[i].set_status_error(error);
    }
    fn set_busy(&self) {
        let busy = self.session.busy() || self.mutation.is_some();
        for window in &self.windows {
            window.set_busy(busy);
        }
    }
    fn handle(&mut self, event: Event) {
        if self.quitting {
            return;
        }
        match event {
            Event::Shell(event) => self.shell_event(event),
            Event::Command(command) => self.command(command),
            Event::Ready(result) => match result {
                Ok(settings) => {
                    self.settings = settings;
                    self.ready = true;
                    self.render_settings();
                    self.apply_theme();
                    if let Some(args) = self.pending_args.take() {
                        self.activate_args(args);
                    }
                }
                Err(error) => {
                    self.report(Role::Main, format!("Startup failed: {error}"), true);
                    let _ = self.show_windows();
                }
            },
            Event::Loaded(role, ticket, result) => {
                if self.surfaces[role.index()].finish_load(ticket, result) {
                    self.render(role);
                }
            }
            Event::Activated(epoch, context, result) => {
                if epoch != self.session.epoch {
                    return;
                }
                match result {
                    Ok(target) => {
                        self.session.capture_finished(epoch, target);
                    }
                    Err(error) => {
                        self.session.capture_finished(epoch, false);
                        self.report(Role::Main, error, true);
                    }
                }
                for w in &self.windows {
                    w.set_quick_insert(context == Context::QuickInsert);
                }
                if let Err(error) = self.show_windows() {
                    self.report(Role::Main, error, true);
                }
                self.load(Role::Main, false);
                self.load(Role::Favorites, false);
            }
            Event::Executed(role, operation, result) => self.executed(role, operation, result),
            Event::Mutated(role, serial, result) => self.mutated(role, serial, result),
            Event::Thumbnail(role, epoch, hash, result) => {
                self.thumbnail_finished(role, epoch, hash, result)
            }
            Event::Invalidated => self.invalidate(),
        }
    }
    fn shell_event(&mut self, event: ShellEvent) {
        match event {
            ShellEvent::Open => self.activate_args(Vec::new()),
            ShellEvent::Favorites => self.activate_args(vec!["--favorites".into()]),
            ShellEvent::Settings => self.activate_args(vec!["--settings".into()]),
            ShellEvent::Quit => self.quit(),
            ShellEvent::Activation(args) => self.activate_args(args),
            ShellEvent::ThemeChanged => self.apply_theme(),
            ShellEvent::GeometryChanged => {
                if let [Some(main), Some(fav)] = self.hwnds {
                    let _ = shell::reposition_favorites(main, fav);
                }
            }
            ShellEvent::Error(error) => self.report(Role::Main, error, true),
        }
    }
    fn activate_args(&mut self, args: Vec<String>) {
        if args.first().map(String::as_str) == Some("--quit") {
            self.quit();
            return;
        }
        if !self.ready {
            self.pending_args = Some(args);
            return;
        }
        if args.first().map(String::as_str) == Some("--background") {
            return;
        }
        let mut context = Context::Manager;
        let mut query = String::new();
        let mut route = "history";
        let mut view = QuickInsertView::History;
        if args.first().map(String::as_str) == Some("--favorites") {
            view = QuickInsertView::Favorites;
        }
        if args.first().map(String::as_str) == Some("--settings") {
            route = "settings";
        }
        if let Some(decoded) = echo_activation::decode_args(args.iter().map(String::as_str)) {
            let envelope = match decoded {
                Ok(e) => e,
                Err(e) => {
                    self.report(Role::Main, e.to_string(), true);
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
                            self.report(Role::Main, e.to_string(), true);
                            return;
                        }
                    }
                }
                _ => {}
            }
        }
        if query.len() > 16 * 1024 {
            self.report(Role::Main, "Search text is too large", true);
            return;
        }
        let epoch = self.session.activate(context);
        self.worker.epoch.store(epoch, Ordering::Release);
        self.surfaces[0].set_view(view);
        self.surfaces[0].set_query(query.clone());
        self.windows[0].set_active_view(
            if view == QuickInsertView::History {
                "history"
            } else {
                "favorites"
            }
            .into(),
        );
        self.windows[0].set_route(route.into());
        self.windows[0].set_query(query.into());
        self.windows[0].set_text_edit_mode(false);
        self.windows[0].invoke_reset_scroll();
        for window in &self.windows {
            window.set_editor_open(false);
            window.set_clear_confirm_open(false);
        }
        self.set_busy();
        // Target capture completes on the native worker BEFORE any window is shown.
        self.send(Role::Main, Work::Begin(epoch, context));
    }
    fn initialize_windows(&mut self) -> Result<(), String> {
        for i in 0..2 {
            if self.hwnds[i].is_some() {
                continue;
            }
            let handle = self.windows[i].window().window_handle();
            let raw = handle.window_handle().map_err(|e| e.to_string())?.as_raw();
            let hwnd = match raw {
                RawWindowHandle::Win32(h) => h.hwnd.get(),
                _ => return Err("Echo requires a Windows native window".into()),
            };
            let hub = self.hub.clone();
            self.hooks[i] = Some(shell::attach_window(
                hwnd,
                i == 0,
                Arc::new(move |e| hub.post(Event::Shell(e))),
            )?);
            self.hwnds[i] = Some(hwnd);
        }
        let [Some(main), Some(fav)] = self.hwnds else {
            return Err("Native window handle is unavailable".into());
        };
        shell::set_owner(fav, main)?;
        shell::reposition_favorites(main, fav)?;
        self.apply_theme();
        Ok(())
    }
    fn show_windows(&mut self) -> Result<(), String> {
        let first = self.hwnds[0].is_none();
        self.windows[0].show().map_err(|e| e.to_string())?;
        self.windows[1].show().map_err(|e| e.to_string())?;
        self.initialize_windows()?;
        if first {
            if let [Some(main), Some(favorite)] = self.hwnds {
                shell::center_composition(main, favorite)?;
            }
        }
        if let [Some(main), Some(fav)] = self.hwnds {
            shell::reposition_favorites(main, fav)?;
            let _ = shell::focus_window(main);
        }
        for surface in &mut self.surfaces {
            surface.visible = true;
        }
        self.windows[0].invoke_focus_search(false);
        Ok(())
    }
    fn hide_surface(&mut self, role: Role) {
        let i = role.index();
        self.timers[i].stop();
        self.surfaces[i].hide();
        let _ = self.windows[i].hide();
        self.models[i].set_vec(Vec::new());
        self.images[i].epoch = self.images[i].epoch.wrapping_add(1);
        self.images[i].cache.clear();
        self.images[i].order.clear();
        self.images[i].pending.clear();
        self.images[i].bytes = 0;
    }
    fn dismiss(&mut self, role: Role) {
        self.hide_surface(role);
        if role == Role::Main {
            self.hide_surface(Role::Favorites);
            self.session.dismiss();
            self.worker
                .epoch
                .store(self.session.epoch, Ordering::Release);
            self.send(role, Work::Cancel);
            self.set_busy();
        }
    }
    fn quit(&mut self) {
        self.quitting = true;
        for timer in &self.timers {
            timer.stop();
        }
        for window in &self.windows {
            let _ = window.hide();
        }
        self.hub.close();
        let _ = slint::quit_event_loop();
    }
    fn load(&mut self, role: Role, more: bool) {
        let i = role.index();
        if !self.ready {
            return;
        }
        let Some(ticket) = self.surfaces[i].begin_load(more) else {
            return;
        };
        let view = self.surfaces[i].view;
        let query = self.surfaces[i].query.clone();
        self.windows[i].set_loading(true);
        if !self.send(role, Work::List(role, ticket, view, query)) {
            self.surfaces[i].loading = false;
            self.windows[i].set_loading(false);
        }
    }
    fn invalidate(&mut self) {
        for role in [Role::Main, Role::Favorites] {
            let i = role.index();
            self.surfaces[i].invalidate();
            if self.surfaces[i].visible && !self.surfaces[i].loading && self.mutation.is_none() {
                self.load(role, false);
            }
        }
    }
    fn render(&mut self, role: Role) {
        let i = role.index();
        let surface = &self.surfaces[i];
        let window = &self.windows[i];
        let mut section = String::new();
        let rows = surface
            .items
            .iter()
            .map(|item| {
                let mut row = formatting::row(item, &mut section);
                row.selected = surface.selection == Some(RowKey::of(item));
                row.batch_selected = surface.selected_ids.contains(&item.id);
                if let Some(thumb) = &item.thumbnail {
                    if let Some((image, _)) = self.images[i].cache.get(&thumb.content_hash) {
                        row.thumbnail = image.clone();
                    }
                }
                row
            })
            .collect::<Vec<_>>();
        self.models[i].set_vec(rows);
        window.set_loading(surface.loading);
        window.set_has_more(surface.next_cursor.is_some());
        window.set_has_previous(surface.has_previous());
        window.set_batch_mode(surface.batch);
        window.set_selected_count(surface.selected_ids.len() as i32);
        window.set_selection_index(
            surface
                .items
                .iter()
                .position(|x| Some(RowKey::of(x)) == surface.selection)
                .map(|n| n as i32)
                .unwrap_or(-1),
        );
        window.set_status(surface.status.clone().into());
        window.set_status_error(surface.error);
        if surface.dirty && !surface.loading {
            self.load(role, false);
        }
    }
    fn thumbnail_request(&mut self, role: Role, key: String) {
        let i = role.index();
        if !self.surfaces[i].visible {
            return;
        }
        let Ok(key) = key.parse::<RowKey>() else {
            return;
        };
        let Some(hash) = self.surfaces[i]
            .items
            .iter()
            .find(|x| RowKey::of(x) == key)
            .and_then(|x| x.thumbnail.as_ref())
            .map(|x| x.content_hash.clone())
        else {
            return;
        };
        if self.images[i].cache.contains_key(&hash) || !self.images[i].pending.insert(hash.clone())
        {
            return;
        }
        let epoch = self.images[i].epoch;
        if !self.send(role, Work::Thumbnail(role, epoch, hash.clone())) {
            self.images[i].pending.remove(&hash);
        }
    }
    fn thumbnail_finished(
        &mut self,
        role: Role,
        epoch: u64,
        hash: String,
        result: Result<PixelData, String>,
    ) {
        let i = role.index();
        if epoch != self.images[i].epoch || !self.surfaces[i].visible {
            return;
        }
        self.images[i].pending.remove(&hash);
        let pixels = match result {
            Ok(p) => p,
            Err(_) => return,
        };
        let bytes = pixels.rgba.len();
        if bytes > 8 * 1024 * 1024 {
            return;
        }
        while self.images[i].bytes + bytes > 8 * 1024 * 1024 {
            let Some(old) = self.images[i].order.pop_front() else {
                break;
            };
            if let Some((_, size)) = self.images[i].cache.remove(&old) {
                self.images[i].bytes = self.images[i].bytes.saturating_sub(size);
            }
            for (index, item) in self.surfaces[i].items.iter().enumerate() {
                if item
                    .thumbnail
                    .as_ref()
                    .is_some_and(|t| t.content_hash == old)
                {
                    if let Some(mut row) = self.models[i].row_data(index) {
                        row.thumbnail = Default::default();
                        self.models[i].set_row_data(index, row);
                    }
                }
            }
        }
        let buffer = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
            &pixels.rgba,
            pixels.width,
            pixels.height,
        );
        let image = slint::Image::from_rgba8(buffer);
        self.images[i].bytes += bytes;
        self.images[i].order.push_back(hash.clone());
        self.images[i]
            .cache
            .insert(hash.clone(), (image.clone(), bytes));
        for (index, item) in self.surfaces[i].items.iter().enumerate() {
            if item
                .thumbnail
                .as_ref()
                .is_some_and(|t| t.content_hash == hash)
            {
                if let Some(mut row) = self.models[i].row_data(index) {
                    row.thumbnail = image.clone();
                    self.models[i].set_row_data(index, row);
                }
            }
        }
    }
    fn command(&mut self, command: Command) {
        match command {
            Command::Quit => self.quit(),
            Command::Dismiss(role) => self.dismiss(role),
            Command::Drag(role) => {
                if let Some(hwnd) = self.hwnds[role.index()] {
                    let _ = shell::start_drag(hwnd);
                }
            }
            Command::Query(role, query) => {
                let i = role.index();
                self.surfaces[i].set_query(query);
                self.windows[i].invoke_reset_scroll();
                self.models[i].set_vec(Vec::new());
                self.windows[i].set_loading(true);
                let hub = self.hub.clone();
                self.timers[i].start(
                    TimerMode::SingleShot,
                    Duration::from_millis(75),
                    move || hub.post(Event::Command(Command::Refresh(role))),
                );
            }
            Command::Refresh(role) => self.load(role, false),
            Command::More(role) => self.load(role, true),
            Command::Previous(role) => {
                if self.surfaces[role.index()].previous_window() {
                    self.windows[role.index()].invoke_reset_scroll();
                    self.load(role, false);
                }
            }
            Command::Select(role, key) => {
                if let Ok(key) = key.parse() {
                    self.surfaces[role.index()].select(key);
                    self.render_selection(role);
                }
            }
            Command::Action(role, action, key) => self.action(role, &action, &key),
            Command::Batch(role, action) => self.batch(role, &action),
            Command::Panel(role, panel) => {
                if role == Role::Favorites {
                    return;
                }
                let view = if panel == "favorites" {
                    QuickInsertView::Favorites
                } else {
                    QuickInsertView::History
                };
                self.surfaces[0].set_view(view);
                self.windows[0].set_active_view(panel.into());
                self.windows[0].set_query("".into());
                self.windows[0].set_route("history".into());
                self.windows[0].invoke_reset_scroll();
                self.load(role, false);
                self.windows[0].invoke_focus_search(false);
            }
            Command::Route(route) => {
                if route != "history" {
                    self.session.dismiss();
                    self.worker
                        .epoch
                        .store(self.session.epoch, Ordering::Release);
                    self.send(Role::Main, Work::Cancel);
                }
                self.windows[0].set_route(route.into());
                self.render_settings();
            }
            Command::Thumbnail(role, key) => self.thumbnail_request(role, key),
            Command::Keyboard(role, intent) => self.keyboard(role, intent),
            Command::SaveSettings => self.save_settings(),
            Command::Create(role) => self.open_editor(role, None),
            Command::SaveFavorite(role) => self.save_favorite(role),
            Command::CancelEditor(role) => {
                self.windows[role.index()].set_editor_open(false);
                self.windows[role.index()].invoke_focus_search(false);
            }
            Command::Clear(role) => self.mutate(role, Mutation::Clear),
            Command::Reorder(role, source, target) => {
                if source.source == QuickInsertSource::Favorite
                    && target.source == QuickInsertSource::Favorite
                    && self.surfaces[role.index()].query.is_empty()
                {
                    self.mutate(
                        role,
                        Mutation::Reorder {
                            id: source.id,
                            before: Some(target.id),
                            delta: 0,
                        },
                    );
                }
            }
        }
    }
    fn action(&mut self, role: Role, action: &str, key: &str) {
        let i = role.index();
        if !self.surfaces[i].visible
            || self.surfaces[i].loading
            || self.session.busy()
            || self.mutation.is_some()
        {
            return;
        }
        let Ok(key) = key.parse::<RowKey>() else {
            return;
        };
        let Some(item) = self.surfaces[i]
            .items
            .iter()
            .find(|x| RowKey::of(x) == key)
            .cloned()
        else {
            return;
        };
        match action {
            "copy" => self.execute(role, key, QuickInsertAction::Copy),
            "insert" => self.execute(role, key, QuickInsertAction::Insert),
            "favorite" if key.source == QuickInsertSource::History => {
                self.mutate(role, Mutation::Favorite(key.id))
            }
            "pin" if key.source == QuickInsertSource::History => {
                self.mutate(role, Mutation::Pin(key.id, item.pinned_at.is_some()))
            }
            "delete" => self.mutate(role, Mutation::Delete(key)),
            "edit" if key.source == QuickInsertSource::Favorite => {
                self.open_editor(role, Some(item))
            }
            "toggle-batch" => {
                self.surfaces[i].toggle_selected(key.id);
                self.render_selection(role);
            }
            "up" | "down"
                if key.source == QuickInsertSource::Favorite
                    && self.surfaces[i].query.is_empty() =>
            {
                self.mutate(
                    role,
                    Mutation::Reorder {
                        id: key.id,
                        before: None,
                        delta: if action == "up" { -1 } else { 1 },
                    },
                )
            }
            _ => {}
        }
    }
    fn render_selection(&self, role: Role) {
        let i = role.index();
        let surface = &self.surfaces[i];
        for (index, item) in surface.items.iter().enumerate() {
            if let Some(mut row) = self.models[i].row_data(index) {
                row.selected = surface.selection == Some(RowKey::of(item));
                row.batch_selected = surface.selected_ids.contains(&item.id);
                self.models[i].set_row_data(index, row);
            }
        }
        self.windows[i].set_batch_mode(surface.batch);
        self.windows[i].set_selected_count(surface.selected_ids.len() as i32);
        self.windows[i].set_selection_index(
            surface
                .items
                .iter()
                .position(|x| Some(RowKey::of(x)) == surface.selection)
                .map(|n| n as i32)
                .unwrap_or(-1),
        );
    }
    fn execute(&mut self, role: Role, key: RowKey, action: QuickInsertAction) {
        if action == QuickInsertAction::Insert && self.session.context == Context::Manager {
            self.report(role, "Selected", false);
            return;
        }
        let Some(operation) = self.session.begin(action) else {
            return;
        };
        self.set_busy();
        self.report(
            role,
            if action == QuickInsertAction::Copy {
                "Copying…"
            } else {
                "Inserting…"
            },
            false,
        );
        if action == QuickInsertAction::Insert {
            self.restore_favorites = self.surfaces[1].visible;
            // A temporary hide for insertion must NOT clear the engine's captured target.
            self.hide_surface(Role::Favorites);
            self.hide_surface(Role::Main);
        }
        if !self.send(role, Work::Execute(role, operation, key)) {
            self.session.finish(operation, Err(()));
            self.set_busy();
            if action == QuickInsertAction::Insert {
                let _ = self.show_windows();
                self.load(Role::Main, false);
                self.load(Role::Favorites, false);
            }
        }
    }
    fn executed(
        &mut self,
        role: Role,
        operation: echo_presentation::session::Operation,
        result: Result<echo_engine::QuickInsertOutcome, String>,
    ) {
        let completion = self
            .session
            .finish(operation, result.as_ref().copied().map_err(|_| ()));
        self.set_busy();
        match completion {
            Completion::Stale => {}
            Completion::Inserted => self.report(role, "Inserted", false),
            Completion::Copied => self.report(role, "Copied", false),
            Completion::Staged => self.report(role, "Copied to clipboard; no active target", false),
            Completion::Restore => {
                if operation.action == QuickInsertAction::Insert {
                    let _ = self.show_windows();
                    self.load(Role::Main, false);
                    if self.restore_favorites {
                        self.load(Role::Favorites, false);
                    } else {
                        self.hide_surface(Role::Favorites);
                    }
                    self.windows[role.index()].invoke_focus_search(false);
                }
                self.report(
                    role,
                    result.err().unwrap_or_else(|| "Operation failed".into()),
                    true,
                );
            }
        }
    }
    fn mutate(&mut self, role: Role, mutation: Mutation) {
        if self.mutation.is_some() || self.session.busy() {
            return;
        }
        self.serial = self.serial.wrapping_add(1);
        let serial = self.serial;
        self.mutation = Some((role, serial));
        self.set_busy();
        self.report(role, "Saving…", false);
        if !self.send(role, Work::Mutate(role, serial, mutation)) {
            self.mutation = None;
            self.set_busy();
        }
    }
    fn mutated(
        &mut self,
        role: Role,
        serial: u64,
        result: Result<crate::events::MutationResult, String>,
    ) {
        if self.mutation != Some((role, serial)) {
            return;
        }
        self.mutation = None;
        self.set_busy();
        match result {
            Ok(result) => {
                if let Some(settings) = result.settings {
                    self.settings = settings;
                    self.render_settings();
                    self.apply_theme();
                }
                if result.editor_saved {
                    self.windows[role.index()].set_editor_open(false);
                    self.windows[role.index()].invoke_focus_search(false);
                }
                self.windows[role.index()].set_clear_confirm_open(false);
                self.surfaces[role.index()].set_batch(false);
                self.report(role, result.message, false);
                self.invalidate();
            }
            Err(error) => self.report(role, error, true),
        }
    }
    fn batch(&mut self, role: Role, action: &str) {
        let i = role.index();
        if self.mutation.is_some()
            || self.session.busy()
            || self.surfaces[i].view != QuickInsertView::History
        {
            return;
        }
        let ids = self.surfaces[i]
            .selected_ids
            .iter()
            .copied()
            .collect::<Vec<_>>();
        match action {
            "begin" => self.surfaces[i].set_batch(true),
            "cancel" => self.surfaces[i].set_batch(false),
            "all" => self.surfaces[i].select_all(),
            "favorite" if !ids.is_empty() => self.mutate(role, Mutation::BulkFavorite(ids)),
            "pin" if !ids.is_empty() => self.mutate(role, Mutation::BulkPin(ids)),
            "delete" if !ids.is_empty() => self.mutate(role, Mutation::BulkDelete(ids)),
            _ => {}
        }
        self.render_selection(role);
    }
    fn open_editor(&mut self, role: Role, item: Option<echo_engine::QuickInsertItem>) {
        // Use the full-size host for editing; the compact Favorites surface cannot contain an accessible form.
        if role == Role::Favorites {
            if let Some(hwnd) = self.hwnds[0] {
                let _ = shell::focus_window(hwnd);
            }
            self.open_editor(Role::Main, item);
            return;
        }
        let i = role.index();
        if self.mutation.is_some() || self.session.busy() {
            return;
        }
        let window = &self.windows[i];
        window.set_editor_new(item.is_none());
        window.set_editing_key(
            item.as_ref()
                .map(|x| RowKey::of(x).to_string())
                .unwrap_or_default()
                .into(),
        );
        window.set_draft_name(
            item.as_ref()
                .and_then(|x| x.name.clone())
                .unwrap_or_default()
                .into(),
        );
        window.set_draft_content(
            item.as_ref()
                .and_then(|x| x.editable_text.clone().or_else(|| x.preview_text.clone()))
                .unwrap_or_default()
                .into(),
        );
        window.set_draft_tags(
            item.as_ref()
                .map(|x| x.tags.join(", "))
                .unwrap_or_default()
                .into(),
        );
        window.set_draft_icon(
            item.as_ref()
                .and_then(|x| x.icon_key.clone())
                .unwrap_or_default()
                .into(),
        );
        window.set_content_editable(item.as_ref().is_none_or(|x| x.editable_text.is_some()));
        self.report(role, "", false);
        self.windows[i].set_editor_open(true);
    }
    fn save_favorite(&mut self, role: Role) {
        let window = &self.windows[role.index()];
        let name = formatting::optional(window.get_draft_name().as_str());
        let icon_key = formatting::optional(window.get_draft_icon().as_str());
        let tags = formatting::tags(window.get_draft_tags().as_str());
        let content = window.get_draft_content().to_string();
        if window.get_editor_new() {
            self.mutate(
                role,
                Mutation::Create(echo_engine::FavoriteDraft {
                    content,
                    name,
                    icon_key,
                    tags,
                }),
            );
        } else if let Ok(key) = window.get_editing_key().as_str().parse::<RowKey>() {
            if key.source != QuickInsertSource::Favorite {
                return;
            }
            let editable_text = window.get_content_editable().then_some(content);
            self.mutate(
                role,
                Mutation::Update(
                    key.id,
                    echo_engine::FavoriteUpdate {
                        name,
                        icon_key,
                        tags,
                        editable_text,
                    },
                ),
            );
        }
    }
    fn save_settings(&mut self) {
        let w = &self.windows[0];
        match formatting::settings(
            w.get_max_entries_text().as_str(),
            w.get_max_total_mib_text().as_str(),
            w.get_max_item_mib_text().as_str(),
            w.get_theme_mode().as_str(),
            w.get_history_enabled(),
            w.get_record_sensitive(),
            w.get_store_window_titles(),
        ) {
            Ok(settings) => self.mutate(Role::Main, Mutation::Settings(settings)),
            Err(e) => self.report(Role::Main, e, true),
        }
    }
    fn render_settings(&self) {
        let settings = &self.settings;
        let window = &self.windows[0];
        window.set_history_enabled(settings.history_enabled);
        window.set_record_sensitive(settings.record_sensitive);
        window.set_store_window_titles(settings.store_window_titles);
        window.set_max_entries_text(settings.max_entries.to_string().into());
        window.set_max_total_mib_text(
            (settings.max_total_bytes / (1024 * 1024))
                .to_string()
                .into(),
        );
        window.set_max_item_mib_text((settings.max_item_bytes / (1024 * 1024)).to_string().into());
        window.set_theme_mode(settings.theme.as_str().into());
    }
    fn apply_theme(&self) {
        let dark = match self.settings.theme {
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
            ThemeMode::System => shell::system_dark(),
        };
        let opaque_renderer = std::env::var("ECHO_RENDERER")
            .as_deref()
            .unwrap_or("software")
            == "software";
        let fallback = std::env::var("ECHO_WINDOWS_ACCEPTANCE").as_deref() == Ok("1")
            && std::env::var("ECHO_ACCEPTANCE_FORCE_MICA_FALLBACK").as_deref() == Ok("1");
        for i in 0..2 {
            self.windows[i].set_dark(dark);
            self.windows[i].set_native_mica(
                self.hwnds[i]
                    .is_some_and(|h| shell::apply_theme(h, dark, !fallback && !opaque_renderer)),
            );
        }
    }
    fn interpret_key(
        &self,
        role: Role,
        text: &str,
        ctrl: bool,
        shift: bool,
        target: &str,
    ) -> Intent {
        use echo_presentation::interaction::{self, Key, Target};
        use slint::platform::Key as NativeKey;
        let i = role.index();
        let mut key = text;
        let keys = [
            (NativeKey::Return, "Enter"),
            (NativeKey::Escape, "Escape"),
            (NativeKey::Tab, "Tab"),
            (NativeKey::UpArrow, "ArrowUp"),
            (NativeKey::DownArrow, "ArrowDown"),
            (NativeKey::LeftArrow, "ArrowLeft"),
            (NativeKey::RightArrow, "ArrowRight"),
            (NativeKey::Home, "Home"),
            (NativeKey::End, "End"),
        ];
        for (native, name) in keys {
            if slint::SharedString::from(native).as_str() == text {
                key = name;
                break;
            }
        }
        let target = match target {
            "search" => Target::Search,
            "row" => Target::Row,
            "control" => Target::Control,
            _ => Target::Surface,
        };
        let modal = self.windows[i].get_editor_open() || self.windows[i].get_clear_confirm_open();
        if modal && key != "Escape" {
            return Intent::None;
        }
        interaction::interpret(Key {
            text: key,
            ctrl,
            shift,
            composing: self.hooks[i].as_ref().is_some_and(WindowHook::is_composing),
            target,
            text_edit: self.windows[i].get_text_edit_mode(),
            batch: self.surfaces[i].batch,
        })
    }
    fn keyboard(&mut self, role: Role, intent: Intent) {
        let i = role.index();
        match intent {
            Intent::None | Intent::PreventDefault => {}
            Intent::Escape => {
                if self.windows[i].get_editor_open() {
                    if self.mutation.is_none() {
                        self.windows[i].set_editor_open(false);
                        self.windows[i].invoke_focus_search(false);
                    }
                } else if self.windows[i].get_clear_confirm_open() {
                    if self.mutation.is_none() {
                        self.windows[i].set_clear_confirm_open(false);
                        self.windows[i].invoke_focus_search(false);
                    }
                } else {
                    self.surfaces[i].set_batch(false);
                    self.dismiss(role);
                }
            }
            Intent::FocusSearch(select) => {
                self.windows[i].set_text_edit_mode(select);
                self.windows[i].invoke_focus_search(select);
            }
            Intent::SwitchPanel => {
                if role == Role::Main {
                    let panel = if self.surfaces[i].view == QuickInsertView::History {
                        "favorites"
                    } else {
                        "history"
                    };
                    self.command(Command::Panel(role, panel.into()));
                }
            }
            Intent::Move(delta) => {
                self.surfaces[i].move_selection(delta);
                self.render_selection(role);
                self.windows[i].invoke_reveal_selection();
            }
            Intent::Select(index) => {
                self.surfaces[i].select_index(index);
                self.render_selection(role);
                self.windows[i].invoke_reveal_selection();
            }
            Intent::ToggleBatch => {
                if let Some(key) = self.surfaces[i].selection {
                    self.surfaces[i].toggle_selected(key.id);
                    self.render_selection(role);
                }
            }
            Intent::SelectAllBatch => {
                self.surfaces[i].select_all();
                self.render_selection(role);
            }
            Intent::Primary | Intent::Copy => {
                if let Some(key) = self.surfaces[i].selection {
                    self.action(
                        role,
                        if intent == Intent::Copy {
                            "copy"
                        } else {
                            "insert"
                        },
                        &key.to_string(),
                    );
                }
            }
        }
    }
    pub fn shutdown(&mut self) {
        self.quitting = true;
        self.hub.close();
        self.worker.stop();
    }
}
