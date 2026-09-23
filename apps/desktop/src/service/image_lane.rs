//! One bounded decoder keeps expensive images out of the domain request queue.
use super::*;
use crate::image_preview::PreviewSize;
use std::sync::atomic::AtomicBool;
type Request = (u64, Thumbnail, PreviewSize);
pub(super) struct Lane {
    sender: Option<SyncSender<Request>>,
    stopped: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Lane {
    pub(super) fn start(store: Arc<SharedClipboardStore>, hub: Arc<Hub>) -> Result<Self, String> {
        let (sender, receiver) = mpsc::sync_channel::<Request>(32);
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = stopped.clone();
        let thread = std::thread::Builder::new()
            .name("echo-image-preview".into())
            .spawn(move || {
                while let Ok((generation, asset, size)) = receiver.recv() {
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    let result = thumbnail(&store, &asset, size);
                    if stop.load(Ordering::Acquire) {
                        break;
                    }
                    let event = Event::Thumbnail(generation, asset.source_hash, size, result);
                    #[cfg(feature = "native-test")]
                    native_faults::publish_thumbnail(&hub, event);
                    #[cfg(not(feature = "native-test"))]
                    hub.post(event);
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            sender: Some(sender),
            stopped,
            thread: Some(thread),
        })
    }
    pub(super) fn submit(
        &self,
        generation: u64,
        asset: Thumbnail,
        size: PreviewSize,
    ) -> Result<(), String> {
        self.sender
            .as_ref()
            .ok_or("Image decoder stopped")?
            .try_send((generation, asset, size))
            .map_err(|_| "Image preview queue is busy".into())
    }
}
impl Drop for Lane {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        self.sender.take();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
fn thumbnail(
    store: &SharedClipboardStore,
    asset: &echo_engine::Thumbnail,
    size: crate::image_preview::PreviewSize,
) -> Result<PixelData, String> {
    let hash = &asset.content_hash;
    if hash.len() != 64 || !hash.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err("Invalid thumbnail identity".into());
    }
    let started = std::time::Instant::now();
    if size.width > 0 && size.height > 0 {
        if let Ok(Some(source)) = store.read_preview_source(&asset.source_hash) {
            if let Ok(pixels) = crate::image_preview::decode(&source.bytes, size) {
                crate::memory_trace::record(
                    "image_preview_decoded",
                    serde_json::json!({
                        "elapsed_us":started.elapsed().as_micros(),
                        "source_bytes":source.bytes.len(),
                        "width":pixels.width,
                        "height":pixels.height,
                        "requested_width":size.width,
                        "requested_height":size.height,
                        "source_kind":"original",
                    }),
                );
                return Ok(pixels);
            }
        }
    }
    let asset = store
        .read_thumbnail(hash)
        .map_err(|e| e.to_string())?
        .ok_or("Thumbnail is unavailable")?;
    let source_bytes = asset.bytes.len();
    if source_bytes > 8 * 1024 * 1024 {
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
    // A persisted thumbnail is bounded to the storage thumbnail size. Keep
    // that effective quality separate from the display size that triggered
    // this request so the UI does not treat a low-resolution fallback as a
    // full-size cache hit.
    let pixels = PixelData {
        requested: crate::image_preview::PreviewSize {
            width: rgba.width(),
            height: rgba.height(),
        },
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
    };
    crate::memory_trace::record(
        "image_preview_decoded",
        serde_json::json!({
            "elapsed_us":started.elapsed().as_micros(),
            "source_bytes":source_bytes,
            "width":pixels.width,
            "height":pixels.height,
            "requested_width":size.width,
            "requested_height":size.height,
            "source_kind":"thumbnail",
        }),
    );
    Ok(pixels)
}
