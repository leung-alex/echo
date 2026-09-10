//! A bounded worker owns domain calls. Database/image work never blocks the UI.
use crate::events::{Event, Hub, MutationResult, PixelData};
use echo_engine::*;
use echo_presentation::{
    session::{Context, Operation},
    LoadTicket, RowKey,
};
use echo_storage::SharedClipboardStore;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, SyncSender},
        Arc,
    },
    thread::JoinHandle,
};

mod capture_lane;
#[cfg(feature = "native-test")]
pub(crate) mod native_faults;
mod settings_commit;

pub enum Mutation {
    Space(SpaceCommand),
    SettingsPatch(SettingsPatch),
    Pin(i64, bool),
    Delete(RowKey),
    BulkPin(Vec<i64>),
    BulkDelete(Vec<i64>),
    Clear,
    Update(i64, FavoriteUpdate),
}
pub enum Work {
    List(LoadTicket, SpaceId, String),
    Preview(SpaceId, u64, String),
    Spaces,
    Inspect(u64, i64),
    Catalog(u64, String, Option<PageCursor>),
    Resume(SpaceId),
    Begin(u64, Context, Option<echo_windows::focus::FocusSnapshot>),
    BeginInline(u64, echo_windows::focus::FocusSnapshot),
    Adopt(u64, Option<PasteTarget>),
    Cancel,
    TrimSearchCache(u64),
    RetryHotkey,
    Execute(Operation, RowKey),
    ExecuteInline(Operation, RowKey, echo_engine::InlineTicket),
    Mutate(u64, Mutation),
    Thumbnail(u64, String),
    Diagnostics(crate::events::DiagnosticReport),
    Wake,
    Stop,
}
// Invalidate synchronously, before routing to a possibly busy control/capture lane.
fn advance_search_generation(epoch: &AtomicU64, work: &Work) -> u64 {
    if matches!(
        work,
        Work::List(..) | Work::Cancel | Work::Begin(..) | Work::BeginInline(..) | Work::Stop
    ) {
        epoch.fetch_add(1, Ordering::AcqRel).wrapping_add(1)
    } else {
        epoch.load(Ordering::Acquire)
    }
}
pub struct Worker {
    sender: SyncSender<(u64, Work)>,
    control: SyncSender<(u64, Work)>,
    search_epoch: Arc<AtomicU64>,
    capture: capture_lane::CaptureLane,
    thread: Option<JoinHandle<()>>,
    pub epoch: Arc<AtomicU64>,
    pub bootstrap: SettingsSnapshot,
    pub inline: echo_windows::inline::InlineController,
}
impl Worker {
    pub fn start(
        path: PathBuf,
        hub: Arc<Hub>,
        hotkeys: echo_windows::shell::HotkeyController,
    ) -> Result<Self, String> {
        let inline_hub = hub.clone();
        let inline = echo_windows::inline::InlineController::start(Arc::new(move |event| {
            inline_hub.post_inline(event)
        }))?;
        let worker_inline = inline.clone();
        let (sender, receiver) = mpsc::sync_channel(32);
        let (control, control_rx) = mpsc::sync_channel(8);
        let epoch = Arc::new(AtomicU64::new(0));
        let capture = capture_lane::CaptureLane::start(hub.clone(), epoch.clone())?;
        let worker_epoch = epoch.clone();
        let search_epoch = Arc::new(AtomicU64::new(0));
        let worker_search_epoch = search_epoch.clone();
        let (boot_tx, boot_rx) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("echo-domain-worker".into())
            .spawn(move || {
                run(
                    path,
                    hub,
                    receiver,
                    control_rx,
                    worker_epoch,
                    boot_tx,
                    hotkeys,
                    worker_inline,
                    worker_search_epoch,
                )
            })
            .map_err(|e| e.to_string())?;
        let bootstrap = match boot_rx.recv() {
            Ok(Ok(snapshot)) => snapshot,
            Ok(Err(error)) => {
                let _ = thread.join();
                return Err(error);
            }
            Err(error) => {
                let _ = thread.join();
                return Err(error.to_string());
            }
        };
        Ok(Self {
            inline,
            bootstrap,
            sender,
            control,
            capture,
            thread: Some(thread),
            epoch,
            search_epoch,
        })
    }
    pub fn send(&self, work: Work) -> Result<(), String> {
        let search_generation = advance_search_generation(&self.search_epoch, &work);
        let work = match work {
            Work::BeginInline(epoch, snapshot) => return self.inline.begin(epoch, snapshot),
            Work::Begin(epoch, context, snapshot) => {
                return self.capture.submit(epoch, context, snapshot)
            }
            work => work,
        };
        if matches!(
            work,
            Work::Adopt(..) | Work::Cancel | Work::Execute(..) | Work::ExecuteInline(..)
        ) {
            self.control
                .try_send((search_generation, work))
                .map_err(|_| "Echo activation queue is busy".to_string())?;
            // Wake an idle worker; a full ordinary queue already guarantees a wake.
            let _ = self.sender.try_send((search_generation, Work::Wake));
            return Ok(());
        }
        self.sender
            .try_send((search_generation, work))
            .map_err(|_| "Echo is busy; retry the operation".into())
    }
    pub fn stop(&mut self) {
        let epoch = self.epoch.fetch_add(1, Ordering::AcqRel);
        self.inline.cancel(epoch);
        self.capture.stop();
        self.search_epoch.fetch_add(1, Ordering::AcqRel);
        let _ = self.sender.send((0, Work::Stop));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        if self.thread.is_some() {
            self.stop();
        }
    }
}
struct Services {
    library: Library<SharedClipboardStore>,
    quick: QuickInsertService<SharedClipboardStore>,
    clipboard: Arc<ClipboardService>,
    platform: Arc<echo_windows::WindowsPlatform>,
    hotkeys: echo_windows::shell::HotkeyController,
}
impl Services {
    fn open(
        path: &std::path::Path,
        hotkeys: echo_windows::shell::HotkeyController,
        inline: echo_windows::inline::InlineController,
    ) -> Result<Self, String> {
        let store = Arc::new(SharedClipboardStore::open(path).map_err(|e| e.to_string())?);
        let platform = Arc::new(echo_windows::WindowsPlatform::new());
        platform.set_inline_controller(inline);
        let sink: Arc<dyn ClipboardSink> = store.clone();
        let clipboard = Arc::new(ClipboardService::new(platform.clone(), sink));
        let library = Library::new(store);
        let quick = QuickInsertService::new(library.clone(), clipboard.clone(), platform.clone());
        Ok(Self {
            library,
            quick,
            clipboard,
            platform,
            hotkeys,
        })
    }
}
fn run(
    path: PathBuf,
    hub: Arc<Hub>,
    receiver: mpsc::Receiver<(u64, Work)>,
    control: mpsc::Receiver<(u64, Work)>,
    epoch: Arc<AtomicU64>,
    bootstrap: SyncSender<Result<SettingsSnapshot, String>>,
    hotkeys: echo_windows::shell::HotkeyController,
    inline: echo_windows::inline::InlineController,
    search_epoch: Arc<AtomicU64>,
) {
    let services = match Services::open(&path, hotkeys, inline) {
        Ok(service) => service,
        Err(error) => {
            let _ = bootstrap.send(Err(error.clone()));
            hub.post(Event::Ready(Err(error)));
            return;
        }
    };
    let settings = match services.library.store().settings_snapshot() {
        Ok(settings) => settings,
        Err(error) => {
            let _ = bootstrap.send(Err(error.to_string()));
            return;
        }
    };
    if let Err(error) = services.hotkeys.apply(&settings.ui) {
        hub.post(Event::Shell(echo_windows::shell::ShellEvent::HotkeyStatus(
            error,
        )));
    }
    echo_windows::focus::warm_accessibility();
    let _ = bootstrap.send(Ok(settings.clone()));
    let events = services.clipboard.subscribe_events();
    let bridge_hub = hub.clone();
    let bridge = std::thread::Builder::new()
        .name("echo-library-events".into())
        .spawn(move || {
            while events.recv().is_ok() {
                bridge_hub.post(Event::Invalidated);
            }
        })
        .ok();
    hub.post(Event::Ready(Ok(settings)));
    while let Ok((search_generation, work)) = control.try_recv().or_else(|_| receiver.recv()) {
        let cancelled = || search_epoch.load(Ordering::Acquire) != search_generation;
        match work {
            Work::Diagnostics(report) => {
                hub.post(Event::DiagnosticExported(export_diagnostics(&path, report)))
            }
            Work::Wake => {}
            Work::Stop => break,
            Work::List(ticket, space, query) => {
                #[cfg(feature = "native-test")]
                let result = if native_faults::before_list() {
                    Err("Synthetic query failure for isolated acceptance".into())
                } else {
                    scope_page(&services, space, &query, ticket.cursor, &cancelled)
                };
                #[cfg(not(feature = "native-test"))]
                let result = scope_page(&services, space, &query, ticket.cursor, &cancelled);
                // Every accepted request completes, including cancelled scans.
                // Presentation rejects old tickets; a failed newer enqueue must
                // never leave the existing request permanently marked loading.
                let event = Event::Loaded(space, ticket, result);
                #[cfg(feature = "native-test")]
                native_faults::publish_loaded(&hub, event);
                #[cfg(not(feature = "native-test"))]
                hub.post(event);
            }
            Work::Preview(space, generation, query) => {
                let result = scope_page(&services, space, &query, None, &cancelled);
                hub.post(Event::Preview(space, generation, result));
            }
            Work::Spaces => hub.post(Event::Spaces(
                services
                    .library
                    .store()
                    .list_spaces()
                    .map_err(|e| e.to_string()),
            )),
            Work::Inspect(generation, id) => {
                let result = (|| {
                    let stored = services
                        .library
                        .store()
                        .saved_item(id)
                        .map_err(|e| e.to_string())?
                        .ok_or("Item no longer exists")?;
                    let spaces = services
                        .library
                        .store()
                        .spaces_for_item(id)
                        .map_err(|e| e.to_string())?;
                    Ok(crate::events::ItemDetails {
                        item: QuickInsertItem::from_saved(stored.item),
                        spaces,
                    })
                })();
                hub.post(Event::Inspected(generation, result));
            }
            Work::Catalog(generation, query, cursor) => {
                let result = services
                    .library
                    .store()
                    .list_saved_items_page(&query, 50, cursor)
                    .map(|page| QuickInsertPage {
                        items: page
                            .items
                            .into_iter()
                            .map(QuickInsertItem::from_saved)
                            .collect(),
                        next_cursor: page.next_cursor,
                    })
                    .map_err(|e| e.to_string());
                hub.post(Event::Catalog(generation, result));
            }
            Work::Resume(id) => {
                let _ = services.library.store().save_resume_space(id);
            }
            Work::BeginInline(..) | Work::Begin(..) => {
                unreachable!("Begin is intercepted by the capture lane")
            }
            Work::Adopt(generation, target) => {
                if epoch.load(Ordering::Acquire) != generation {
                    continue;
                }
                services.quick.clear_session();
                services.platform.adopt_captured_target(target.clone());
                services.quick.begin_captured_session(target);
            }
            Work::Cancel => {
                services.quick.clear_session();
                services.quick.release_search_results();
            }
            Work::TrimSearchCache(generation) => {
                if epoch.load(Ordering::Acquire) == generation {
                    services.quick.release_search_cache();
                }
            }
            Work::RetryHotkey => {
                let result = services
                    .library
                    .store()
                    .settings_snapshot()
                    .map_err(|e| e.to_string())
                    .and_then(|s| services.hotkeys.apply(&s.ui));
                if let Err(error) = result {
                    hub.post(Event::Shell(echo_windows::shell::ShellEvent::HotkeyStatus(
                        error,
                    )));
                }
            }
            Work::ExecuteInline(operation, key, ticket) => {
                let result = if epoch.load(Ordering::Acquire) != operation.epoch
                    || operation.epoch != ticket.session
                {
                    Err(QuickInsertError::InvalidTarget)
                } else {
                    services.quick.execute_inline(key.source, key.id, ticket)
                };
                hub.post(Event::Executed(operation, result));
            }
            Work::Execute(operation, key) => {
                let result = if epoch.load(Ordering::Acquire) != operation.epoch {
                    Err(QuickInsertError::InvalidTarget)
                } else {
                    services.quick.execute(key.source, key.id, operation.action)
                };
                hub.post(Event::Executed(operation, result));
            }
            Work::Mutate(serial, mutation) => {
                hub.post(Event::Mutated(serial, mutate(&services, mutation)))
            }
            Work::Thumbnail(generation, hash) => {
                let event = Event::Thumbnail(generation, hash.clone(), thumbnail(&services, &hash));
                #[cfg(feature = "native-test")]
                native_faults::publish_thumbnail(&hub, event);
                #[cfg(not(feature = "native-test"))]
                hub.post(event);
            }
        }
    }
    services.clipboard.shutdown();
    // Dropping the domain owners closes the publisher before joining its subscriber.
    drop(services);
    if let Some(bridge) = bridge {
        let _ = bridge.join();
    }
}
fn mutate(services: &Services, mutation: Mutation) -> Result<MutationResult, String> {
    let mut settings = None;
    let mut settings_warning = None;
    let mut editor_saved = false;
    let mut snapshot = None;
    let mut space_result = None;
    let quick = &services.quick;
    let message = match mutation {
        Mutation::Space(command) => {
            editor_saved = matches!(command.action, SpaceAction::CreateItem(_));
            let cleared_favorites = matches!(command.action, SpaceAction::ClearFavorites);
            space_result = Some(
                services
                    .library
                    .store()
                    .apply_space_command(command)
                    .map_err(|e| e.to_string())?,
            );
            if cleared_favorites {
                "Favorites cleared; content in other spaces kept"
            } else {
                "Space updated"
            }
        }
        Mutation::SettingsPatch(patch) => {
            let previous = services
                .library
                .store()
                .settings_snapshot()
                .map_err(|e| e.to_string())?;
            let reservation = services.hotkeys.prepare(&patch.ui)?;
            let mut value = services
                .library
                .store()
                .save_settings_patch(patch)
                .map_err(|e| e.to_string())?;
            if let Err(error) = reservation.commit() {
                let (reconciled, warning) = settings_commit::reconcile(
                    &previous,
                    value,
                    &error,
                    |patch| {
                        services
                            .library
                            .store()
                            .save_settings_patch(patch)
                            .map_err(|e| e.to_string())
                    },
                    || {
                        services
                            .library
                            .store()
                            .settings_snapshot()
                            .map_err(|e| e.to_string())
                    },
                );
                value = reconciled;
                settings_warning = Some(warning);
            }
            settings = Some(value.clipboard.clone());
            snapshot = Some(value);
            if let Err(error) = quick.refresh_capture_configuration() {
                let warning = settings_warning.get_or_insert_with(String::new);
                warning.push_str(&format!(
                    " Settings were stored, but capture could not be refreshed: {error}"
                ));
            }
            "Settings saved"
        }
        Mutation::Pin(id, was_pinned) => {
            if was_pinned {
                quick.unpin_history(id)
            } else {
                quick.pin_history(id)
            }
            .map_err(|e| e.to_string())?;
            "History pin updated"
        }
        Mutation::Delete(key) => {
            if key.source == QuickInsertSource::History {
                quick
                    .delete_history_many(&[key.id])
                    .map_err(|e| e.to_string())?;
            } else {
                quick.delete_favorite(key.id).map_err(|e| e.to_string())?;
            }
            "Item deleted"
        }
        Mutation::BulkPin(ids) => {
            quick.pin_history_many(&ids).map_err(|e| e.to_string())?;
            "History pinned"
        }
        Mutation::BulkDelete(ids) => {
            quick.delete_history_many(&ids).map_err(|e| e.to_string())?;
            "History deleted"
        }
        Mutation::Clear => {
            quick.clear_unpinned_history().map_err(|e| e.to_string())?;
            "Unpinned history cleared"
        }
        Mutation::Update(id, update) => {
            quick
                .update_favorite(id, update)
                .map_err(|e| e.to_string())?;
            editor_saved = true;
            "Favorite updated"
        }
    };
    Ok(MutationResult {
        message: message.into(),
        settings_warning,
        settings,
        editor_saved,
        snapshot,
        space_result,
    })
}
#[cfg(test)]
fn reorder(ids: &mut Vec<i64>, id: i64, before: Option<i64>, delta: i32) -> Result<(), String> {
    let source = ids
        .iter()
        .position(|candidate| *candidate == id)
        .ok_or("Favorite no longer exists")?;
    if let Some(target) = before {
        if id == target {
            return Ok(());
        }
        if !ids.contains(&target) {
            return Err("Drop target no longer exists".into());
        }
        ids.remove(source);
        let destination = ids
            .iter()
            .position(|candidate| *candidate == target)
            .ok_or("Invalid drop target")?;
        ids.insert(destination, id);
    } else {
        let destination = (source as i64 + i64::from(delta))
            .clamp(0, ids.len().saturating_sub(1) as i64) as usize;
        ids.swap(source, destination);
    }
    Ok(())
}
fn thumbnail(services: &Services, hash: &str) -> Result<PixelData, String> {
    if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Invalid thumbnail identity".into());
    }
    let asset = services
        .library
        .store()
        .read_thumbnail(hash)
        .map_err(|e| e.to_string())?
        .ok_or("Thumbnail is unavailable")?;
    if asset.bytes.len() > 8 * 1024 * 1024 {
        return Err("Thumbnail exceeds the decode budget".into());
    }
    let mut reader =
        image::io::Reader::with_format(std::io::Cursor::new(asset.bytes), image::ImageFormat::Png);
    let mut limits = image::io::Limits::default();
    limits.max_image_width = Some(2048);
    limits.max_image_height = Some(2048);
    limits.max_alloc = Some(16 * 1024 * 1024);
    reader.limits(limits);
    let rgba = reader.decode().map_err(|e| e.to_string())?.into_rgba8();
    Ok(PixelData {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dismissal_and_activation_invalidate_running_search_before_queue_delivery() {
        let epoch = AtomicU64::new(4);
        assert_eq!(advance_search_generation(&epoch, &Work::Cancel), 5);
        assert_ne!(epoch.load(Ordering::Acquire), 4);
        assert_eq!(
            advance_search_generation(&epoch, &Work::Begin(9, Context::QuickInsert, None)),
            6
        );
        assert_eq!(advance_search_generation(&epoch, &Work::Stop), 7);
    }
    #[test]
    fn maintenance_does_not_cancel_a_new_search() {
        let epoch = AtomicU64::new(8);
        assert_eq!(
            advance_search_generation(&epoch, &Work::TrimSearchCache(1)),
            8
        );
        assert_eq!(
            advance_search_generation(&epoch, &Work::Resume(SpaceId::HISTORY)),
            8
        );
        assert_eq!(epoch.load(Ordering::Acquire), 8);
    }
    #[test]
    fn reordering_keeps_every_favorite_including_unloaded_pages() {
        let mut ids = (1..=1000).collect::<Vec<_>>();
        reorder(&mut ids, 900, Some(1), 0).unwrap();
        assert_eq!(ids[0], 900);
        assert_eq!(ids.len(), 1000);
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(sorted, (1..=1000).collect::<Vec<_>>());
    }
    #[test]
    fn move_boundaries_and_missing_ids_are_safe() {
        let mut ids = vec![1, 2, 3];
        reorder(&mut ids, 1, None, -1).unwrap();
        assert_eq!(ids, [1, 2, 3]);
        reorder(&mut ids, 3, None, 1).unwrap();
        assert_eq!(ids, [1, 2, 3]);
        reorder(&mut ids, 2, None, -1).unwrap();
        assert_eq!(ids, [2, 1, 3]);
        assert!(reorder(&mut ids, 9, None, 1).is_err());
        assert!(reorder(&mut ids, 2, Some(99), 0).is_err());
    }
}

fn scope_page(
    services: &Services,
    space: SpaceId,
    query: &str,
    cursor: Option<PageCursor>,
    cancelled: &dyn Fn() -> bool,
) -> Result<crate::events::LoadedPage, String> {
    services
        .quick
        .fuzzy_list_space_cancellable(space, query, 50, cursor, cancelled)
        .map(|(page, revision, total)| crate::events::LoadedPage {
            page,
            revision,
            total,
        })
        .map_err(|e| e.to_string())
}

fn export_diagnostics(
    root: &std::path::Path,
    report: crate::events::DiagnosticReport,
) -> Result<String, String> {
    use std::io::Write;
    let directory = root.join("diagnostics");
    std::fs::create_dir_all(&directory).map_err(|e| e.to_string())?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis();
    let path = directory.join(format!("cover-flow-{now}-{}.json", std::process::id()));
    let bytes = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}
