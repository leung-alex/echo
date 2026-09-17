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
mod image_lane;
#[cfg(feature = "native-test")]
pub(crate) mod native_faults;
pub(crate) mod native_isolation;
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
    SidePreview(SpaceId, u64, String),
    Spaces,
    Inspect(u64, i64),
    Catalog(u64, String, Option<PageCursor>),
    Resume(SpaceId),
    Begin(u64, Context, Option<echo_windows::focus::FocusSnapshot>),
    BeginInline(u64, echo_windows::focus::FocusSnapshot),
    Adopt(u64, Option<PasteTarget>),
    Cancel,
    TrimSearchCache(u64, u64),
    RetryHotkey,
    Execute(Operation, RowKey),
    ExecuteInline(Operation, RowKey, echo_engine::InlineTicket),
    Mutate(u64, Mutation),
    Thumbnail(
        u64,
        echo_engine::Thumbnail,
        crate::image_preview::PreviewSize,
    ),
    Diagnostics(crate::events::DiagnosticReport),
    Wake,
    Stop,
}
// Invalidate synchronously, before routing to a possibly busy control/capture lane.
fn advance_search_generation(epoch: &AtomicU64, work: &Work) -> u64 {
    if matches!(
        work,
        Work::List(..)
            | Work::Cancel
            | Work::Begin(..)
            | Work::BeginInline(..)
            | Work::TrimSearchCache(..)
            | Work::Stop
    ) {
        epoch.fetch_add(1, Ordering::AcqRel).wrapping_add(1)
    } else {
        epoch.load(Ordering::Acquire)
    }
}
pub struct Worker {
    #[cfg(feature = "native-test")]
    pub capture_disabled: bool,
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
        let _capture_disabled = native_isolation::validated_root(&path)?.is_some();
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
        crate::indicator_trace::initialize(sender.clone());
        Ok(Self {
            #[cfg(feature = "native-test")]
            capture_disabled: _capture_disabled,
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
            Work::Adopt(..)
                | Work::Cancel
                | Work::Execute(..)
                | Work::ExecuteInline(..)
                | Work::TrimSearchCache(..)
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
        let _capture_disabled = native_isolation::validated_root(path)?.is_some();
        let store = Arc::new(SharedClipboardStore::open(path).map_err(|e| e.to_string())?);
        let platform = Arc::new(echo_windows::WindowsPlatform::new());
        platform.set_inline_controller(inline);
        let sink: Arc<dyn ClipboardSink> = store.clone();
        #[cfg(feature = "native-test")]
        let sink: Arc<dyn ClipboardSink> = if _capture_disabled {
            Arc::new(native_isolation::DisabledCaptureSink)
        } else {
            sink
        };
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
    let images = image_lane::Lane::start(services.library.store().clone(), hub.clone());
    hub.post(Event::Ready(Ok(settings)));
    let mut indicator_log = crate::indicator_trace::Writer::new(path.join("logs"));
    while let Ok((search_generation, work)) = control.try_recv().or_else(|_| receiver.recv()) {
        indicator_log.drain();
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

            Work::SidePreview(space, generation, query) => {
                let result = scope_page_limit(&services, space, &query, None, &cancelled, 4).map(
                    |mut data| {
                        data.page.items = data.page.items.into_iter().map(side_summary).collect();
                        data
                    },
                );
                if cancelled() {
                    hub.post(Event::SidePreviewCancelled(space, generation));
                } else {
                    hub.post(Event::Preview(space, generation, result));
                }
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
                            .map(|saved| {
                                let mut item = QuickInsertItem::from_saved(saved);
                                // List/catalog rows are display summaries. Editing uses
                                // Work::Inspect and execution rehydrates originals by ID.
                                item.editable_text = None;
                                item
                            })
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
            Work::TrimSearchCache(generation, hidden_generation) => {
                if epoch.load(Ordering::Acquire) == generation {
                    services.quick.release_search_cache();
                    hub.post(Event::SearchCacheTrimmed(generation, hidden_generation));
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
            Work::Thumbnail(generation, asset, size) => {
                if let Err(error) = images
                    .as_ref()
                    .map_err(Clone::clone)
                    .and_then(|lane| lane.submit(generation, asset.clone(), size))
                {
                    hub.post(Event::Thumbnail(generation, asset.source_hash, Err(error)));
                }
            }
        }
    }
    drop(images);
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
// Matching has already scanned the complete document. This read-only projection
// must not retain its original capacity, editor payload, tags or application data.
fn side_summary(item: QuickInsertItem) -> QuickInsertItem {
    QuickInsertItem {
        name: item.name.map(|s| s.chars().take(160).collect()),
        preview_text: item.preview_text.map(|s| s.chars().take(384).collect()),
        content_type: item.content_type.chars().take(80).collect(),
        editable_text: None,
        tags: Vec::new(),
        source_app: None,
        icon_key: None,
        ..item
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn side_projection_drops_large_document_capacity_after_full_query() {
        let item: QuickInsertItem = serde_json::from_value(serde_json::json!({
            "id": 17, "source": "favorite", "name": "名".repeat(500),
            "preview_text": "文".repeat(1_000_000), "content_type": "text/plain",
            "editable_text": "original".repeat(10000), "tags": ["tag".repeat(10000)],
            "source_app": "app".repeat(10000), "updated_at": 0, "pinned_at": null,
            "icon_key": null, "favorite_order": null, "thumbnail": null
        }))
        .unwrap();
        let summary = side_summary(item);
        assert_eq!(summary.id, 17);
        assert_eq!(summary.name.as_ref().unwrap().chars().count(), 160);
        assert_eq!(summary.preview_text.as_ref().unwrap().chars().count(), 384);
        assert!(summary.held_bytes() < 4096);
        assert!(summary.editable_text.is_none());
        assert!(summary.tags.is_empty());
        assert!(summary.source_app.is_none());
    }
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
    fn trim_invalidates_queued_searches_but_resume_does_not() {
        let epoch = AtomicU64::new(8);
        assert_eq!(
            advance_search_generation(&epoch, &Work::TrimSearchCache(1, 1)),
            9
        );
        assert_eq!(
            advance_search_generation(&epoch, &Work::Resume(SpaceId::HISTORY)),
            9
        );
        assert_eq!(epoch.load(Ordering::Acquire), 9);
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
    scope_page_limit(services, space, query, cursor, cancelled, 20)
}
fn scope_page_limit(
    services: &Services,
    space: SpaceId,
    query: &str,
    cursor: Option<PageCursor>,
    cancelled: &dyn Fn() -> bool,
    mut limit: u32,
) -> Result<crate::events::LoadedPage, String> {
    // Byte-bounded cursor batches keep large display pages off the UI queue.
    // Retry with a smaller batch; never remove rows from an already-issued cursor.
    loop {
        let result = services
            .quick
            .fuzzy_list_space_cancellable(space, query, limit, cursor, cancelled)
            .map(|(mut page, revision, total)| {
                // The empty-query path also includes Saved Items. Display pages
                // never need their full editor body; Inspect hydrates it by ID.
                for item in &mut page.items {
                    item.editable_text = None;
                }
                crate::events::LoadedPage {
                    page,
                    revision,
                    total,
                }
            })
            .map_err(|e| e.to_string());
        if let Ok(page) = &result {
            let bytes = page
                .page
                .items
                .iter()
                .map(QuickInsertItem::held_bytes)
                .sum::<usize>();
            if bytes > 128 * 1024 && page.page.items.len() > 1 && limit > 1 {
                limit = (limit / 2).max(1);
                continue;
            }
        }
        return result;
    }
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
    let path = directory.join(format!("software-{now}-{}.json", std::process::id()));
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
