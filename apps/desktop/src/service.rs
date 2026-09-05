//! A bounded worker owns domain calls. Database/image work never blocks the UI.
use crate::events::{Event, Hub, MutationResult, PixelData, Role};
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

pub enum Mutation {
    Favorite(i64),
    Pin(i64, bool),
    Delete(RowKey),
    BulkFavorite(Vec<i64>),
    BulkPin(Vec<i64>),
    BulkDelete(Vec<i64>),
    Clear,
    Create(FavoriteDraft),
    Update(i64, FavoriteUpdate),
    Settings(ClipboardSettings),
    Reorder {
        id: i64,
        before: Option<i64>,
        delta: i32,
    },
}
pub enum Work {
    List(Role, LoadTicket, QuickInsertView, String),
    Begin(u64, Context),
    Cancel,
    Execute(Role, Operation, RowKey),
    Mutate(Role, u64, Mutation),
    Thumbnail(Role, u64, String),
    Stop,
}
pub struct Worker {
    sender: SyncSender<Work>,
    thread: Option<JoinHandle<()>>,
    pub epoch: Arc<AtomicU64>,
}
impl Worker {
    pub fn start(path: PathBuf, hub: Arc<Hub>) -> Result<Self, String> {
        let (sender, receiver) = mpsc::sync_channel(32);
        let epoch = Arc::new(AtomicU64::new(0));
        let worker_epoch = epoch.clone();
        let thread = std::thread::Builder::new()
            .name("echo-domain-worker".into())
            .spawn(move || run(path, hub, receiver, worker_epoch))
            .map_err(|e| e.to_string())?;
        Ok(Self {
            sender,
            thread: Some(thread),
            epoch,
        })
    }
    pub fn send(&self, work: Work) -> Result<(), String> {
        self.sender
            .try_send(work)
            .map_err(|_| "Echo is busy; retry the operation".into())
    }
    pub fn stop(&mut self) {
        self.epoch.fetch_add(1, Ordering::AcqRel);
        let _ = self.sender.send(Work::Stop);
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
}
impl Services {
    fn open(path: &std::path::Path) -> Result<Self, String> {
        let store = Arc::new(SharedClipboardStore::open(path).map_err(|e| e.to_string())?);
        let platform: Arc<dyn ClipboardPlatform> = Arc::new(echo_windows::WindowsPlatform::new());
        let sink: Arc<dyn ClipboardSink> = store.clone();
        let clipboard = Arc::new(ClipboardService::new(platform.clone(), sink));
        let library = Library::new(store);
        let quick = QuickInsertService::new(library.clone(), clipboard.clone(), platform);
        Ok(Self {
            library,
            quick,
            clipboard,
        })
    }
}
fn run(path: PathBuf, hub: Arc<Hub>, receiver: mpsc::Receiver<Work>, epoch: Arc<AtomicU64>) {
    let services = match Services::open(&path) {
        Ok(service) => service,
        Err(error) => {
            hub.post(Event::Ready(Err(error)));
            return;
        }
    };
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
    hub.post(Event::Ready(
        services.library.settings().map_err(|e| e.to_string()),
    ));
    while let Ok(work) = receiver.recv() {
        match work {
            Work::Stop => break,
            Work::List(role, ticket, view, query) => {
                let request = QuickInsertRequest {
                    view,
                    query,
                    limit: 50,
                    cursor: ticket.cursor,
                };
                hub.post(Event::Loaded(
                    role,
                    ticket,
                    services.quick.list(&request).map_err(|e| e.to_string()),
                ));
            }
            Work::Begin(generation, context) => {
                if epoch.load(Ordering::Acquire) != generation {
                    continue;
                }
                services.quick.clear_session();
                let result = if context == Context::QuickInsert {
                    services.quick.begin_session().map_err(|e| e.to_string())
                } else {
                    Ok(false)
                };
                hub.post(Event::Activated(generation, context, result));
            }
            Work::Cancel => services.quick.clear_session(),
            Work::Execute(role, operation, key) => {
                let result = if epoch.load(Ordering::Acquire) != operation.epoch {
                    Err("Insertion session was cancelled".into())
                } else {
                    services
                        .quick
                        .execute(key.source, key.id, operation.action)
                        .map_err(|e| e.to_string())
                };
                hub.post(Event::Executed(role, operation, result));
            }
            Work::Mutate(role, serial, mutation) => {
                hub.post(Event::Mutated(role, serial, mutate(&services, mutation)))
            }
            Work::Thumbnail(role, generation, hash) => hub.post(Event::Thumbnail(
                role,
                generation,
                hash.clone(),
                thumbnail(&services, &hash),
            )),
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
    let mut editor_saved = false;
    let quick = &services.quick;
    let message = match mutation {
        Mutation::Favorite(id) => {
            quick
                .move_history_to_favorite(id)
                .map_err(|e| e.to_string())?;
            "Added to Favorites"
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
        Mutation::BulkFavorite(ids) => {
            quick
                .move_history_many_to_favorites(&ids)
                .map_err(|e| e.to_string())?;
            "Added to Favorites"
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
        Mutation::Create(draft) => {
            quick.create_favorite(draft).map_err(|e| e.to_string())?;
            editor_saved = true;
            "Favorite created"
        }
        Mutation::Update(id, update) => {
            quick
                .update_favorite(id, update)
                .map_err(|e| e.to_string())?;
            editor_saved = true;
            "Favorite updated"
        }
        Mutation::Settings(value) => {
            services
                .library
                .update_settings(&value)
                .map_err(|e| e.to_string())?;
            settings = Some(value);
            quick
                .refresh_capture_configuration()
                .map_err(|e| e.to_string())?;
            "Settings saved"
        }
        Mutation::Reorder { id, before, delta } => {
            let mut cursor = None;
            let mut ids = Vec::new();
            loop {
                let page = quick
                    .list(&QuickInsertRequest {
                        view: QuickInsertView::Favorites,
                        query: String::new(),
                        limit: 100,
                        cursor,
                    })
                    .map_err(|e| e.to_string())?;
                ids.extend(page.items.iter().map(|item| item.id));
                if ids.len() > 100_000 {
                    return Err("Favorites reorder exceeded the safe operation limit".into());
                }
                match page.next_cursor {
                    Some(next) if Some(next) != cursor => cursor = Some(next),
                    None => break,
                    _ => return Err("Favorites cursor did not advance".into()),
                }
            }
            reorder(&mut ids, id, before, delta)?;
            quick.reorder_favorites(&ids).map_err(|e| e.to_string())?;
            "Favorites reordered"
        }
    };
    Ok(MutationResult {
        message: message.into(),
        settings,
        editor_saved,
    })
}
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
