use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::domain::{
    ClipboardPlatform, ClipboardRepresentation, ClipboardSnapshot, PlatformChange,
    PlatformChangeSubscription, PlatformError, SourceContext,
};
use crate::settings::ClipboardSettings;
use crate::OperationMetrics;
use ammonia::Builder;
use sha2::{Digest, Sha256};
use thiserror::Error;

const PREVIEW_CHARACTERS: usize = 600;
pub const INGESTION_QUEUE_CAPACITY: usize = 8;
pub const DEFAULT_CAPTURE_LIMIT_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturePolicy {
    pub max_representation_bytes: u64,
    pub max_total_bytes: u64,
    pub supported_formats: Vec<String>,
}

impl Default for CapturePolicy {
    fn default() -> Self {
        Self {
            max_representation_bytes: DEFAULT_CAPTURE_LIMIT_BYTES,
            max_total_bytes: DEFAULT_CAPTURE_LIMIT_BYTES,
            supported_formats: ["text", "html", "rtf", "image", "files"]
                .into_iter()
                .map(str::to_owned)
                .collect(),
        }
    }
}

impl CapturePolicy {
    pub fn from_max_item_bytes(max_item_bytes: u64) -> Self {
        Self {
            max_representation_bytes: max_item_bytes,
            max_total_bytes: max_item_bytes,
            ..Self::default()
        }
    }

    pub fn supports_format(&self, format: &str) -> bool {
        self.supported_formats
            .iter()
            .any(|supported| supported == format)
    }

    pub fn accepts_size(&self, format: &str, representation_bytes: u64, total_bytes: u64) -> bool {
        self.supports_format(format)
            && representation_bytes <= self.max_representation_bytes
            && total_bytes
                .checked_add(representation_bytes)
                .is_some_and(|total| total <= self.max_total_bytes)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentType {
    Text,
    Html,
    Rtf,
    Image,
    Files,
}

impl ContentType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Html => "html",
            Self::Rtf => "rtf",
            Self::Image => "image",
            Self::Files => "files",
        }
    }
}

#[derive(Debug, Clone)]
pub struct CaptureSettings {
    pub history_enabled: bool,
    pub record_sensitive: bool,
    pub store_window_titles: bool,
}

impl Default for CaptureSettings {
    fn default() -> Self {
        Self {
            history_enabled: true,
            record_sensitive: false,
            store_window_titles: false,
        }
    }
}

impl From<&ClipboardSettings> for CaptureSettings {
    fn from(settings: &ClipboardSettings) -> Self {
        Self {
            history_enabled: settings.history_enabled,
            record_sensitive: settings.record_sensitive,
            store_window_titles: settings.store_window_titles,
        }
    }
}

#[derive(Debug, Clone)]
pub struct NormalizedCapture {
    pub sequence: u64,
    pub source: SourceContext,
    pub content_type: ContentType,
    pub preview_text: Option<String>,
    pub searchable_text: Option<String>,
    pub sanitized_html: Option<String>,
    pub fingerprint: String,
    pub representations: Vec<ClipboardRepresentation>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepresentationIdentity {
    pub format: String,
    pub byte_size: u64,
    pub hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentIdentity {
    pub fingerprint: String,
    pub total_byte_size: u64,
    pub representations: Vec<RepresentationIdentity>,
}

impl ContentIdentity {
    pub fn from_representations(representations: &[ClipboardRepresentation]) -> Self {
        let mut fingerprint_hasher = Sha256::new();
        let mut identities = Vec::with_capacity(representations.len());
        let mut total_byte_size = 0_u64;
        for representation in representations {
            let byte_size = representation.bytes.len() as u64;
            total_byte_size = total_byte_size.saturating_add(byte_size);
            fingerprint_hasher.update((representation.format.len() as u64).to_le_bytes());
            fingerprint_hasher.update(representation.format.as_bytes());
            fingerprint_hasher.update(byte_size.to_le_bytes());

            let mut representation_hasher = Sha256::new();
            fingerprint_hasher.update(&representation.bytes);
            representation_hasher.update(&representation.bytes);
            identities.push(RepresentationIdentity {
                format: representation.format.clone(),
                byte_size,
                hash: format!("{:x}", representation_hasher.finalize()),
            });
        }
        Self {
            fingerprint: format!("{:x}", fingerprint_hasher.finalize()),
            total_byte_size,
            representations: identities,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CapturedCapture {
    pub capture: NormalizedCapture,
    pub identity: ContentIdentity,
    pub preview: Option<crate::PreviewAsset>,
}

impl CapturedCapture {
    pub fn from_capture(mut capture: NormalizedCapture) -> Self {
        let identity = ContentIdentity::from_representations(&capture.representations);
        capture.fingerprint = identity.fingerprint.clone();
        Self {
            capture,
            identity,
            preview: None,
        }
    }

    /// Runs image decode/resize/encode unconditionally for a previewable capture.
    /// Ingestion must call this only after the sink requests generation.
    pub fn prepare_preview(mut self) -> Self {
        self.preview =
            crate::preview::thumbnail_for_capture(&self.capture.representations, &self.identity);
        self
    }

    pub fn preview_source(&self) -> Option<&RepresentationIdentity> {
        crate::preview::preview_source(&self.capture.representations, &self.identity)
            .map(|(_, identity)| identity)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptureEvent {
    HistoryChanged { id: i64, duplicate: bool },
    HistoryInvalidated { id: Option<i64> },
}

#[derive(Clone, Default)]
struct CaptureEventPublisher {
    subscribers: Arc<Mutex<Vec<mpsc::Sender<CaptureEvent>>>>,
}

impl CaptureEventPublisher {
    fn subscribe(&self) -> CaptureEventSubscription {
        let (sender, receiver) = mpsc::channel();
        self.subscribers
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(sender);
        CaptureEventSubscription { receiver }
    }

    fn publish(&self, event: CaptureEvent) {
        let mut subscribers = self
            .subscribers
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        subscribers.retain(|sender| sender.send(event.clone()).is_ok());
    }
}

pub struct CaptureEventSubscription {
    receiver: Receiver<CaptureEvent>,
}

impl CaptureEventSubscription {
    pub fn recv(&self) -> Result<CaptureEvent> {
        self.receiver
            .recv()
            .map_err(|_| ClipboardError::StateUnavailable)
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<CaptureEvent> {
        self.receiver
            .recv_timeout(timeout)
            .map_err(|_| ClipboardError::StateUnavailable)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureCommit {
    pub outcome: CaptureOutcome,
    pub event: Option<CaptureEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordResult {
    pub id: i64,
    pub duplicate: bool,
}

pub trait ClipboardSink: Send + Sync {
    fn settings(&self) -> std::result::Result<CaptureSettings, String>;
    fn record(&self, capture: NormalizedCapture) -> std::result::Result<RecordResult, String>;
    fn capture_policy(&self) -> std::result::Result<CapturePolicy, String> {
        Ok(CapturePolicy::default())
    }
    fn preview_disposition(
        &self,
        _source: &RepresentationIdentity,
    ) -> std::result::Result<crate::PreviewDisposition, String> {
        Ok(crate::PreviewDisposition::Generate)
    }
    fn record_captured(
        &self,
        captured: CapturedCapture,
    ) -> std::result::Result<RecordResult, String> {
        self.record(captured.capture)
    }
    fn request_maintenance(&self) {}
    fn reconcile(&self) -> std::result::Result<(), String> {
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum ClipboardError {
    #[error("platform error: {0}")]
    Platform(#[from] PlatformError),
    #[error("clipboard sink error: {0}")]
    Sink(String),
    #[error("clipboard service state is unavailable")]
    StateUnavailable,
    #[error("clipboard changed while it was being read")]
    ChangedDuringRead,
}

pub type Result<T> = std::result::Result<T, ClipboardError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureOutcome {
    Recorded(i64),
    Duplicate(i64),
    Disabled,
    SelfWriteIgnored,
    Unsupported,
}

struct Shared {
    platform: Arc<dyn ClipboardPlatform>,
    sink: Arc<dyn ClipboardSink>,
    processor: Arc<dyn CaptureProcessor>,
    configuration: Mutex<CaptureConfiguration>,
    queue: Mutex<Option<SyncSender<IngestionJob>>>,
    events: CaptureEventPublisher,
    write_capture: Mutex<()>,
    last_sequence: AtomicU64,
    ignored_sequence: AtomicU64,
    last_error: Mutex<Option<String>>,
    metrics: Arc<OperationMetrics>,
}

#[derive(Clone)]
struct CaptureConfiguration {
    settings: CaptureSettings,
    policy: CapturePolicy,
}

trait CaptureProcessor: Send + Sync {
    fn normalize(
        &self,
        snapshot: ClipboardSnapshot,
        settings: &CaptureSettings,
        policy: &CapturePolicy,
    ) -> Result<Option<CapturedCapture>>;

    fn prepare_preview(&self, captured: CapturedCapture) -> CapturedCapture;
}

struct DefaultCaptureProcessor;

impl CaptureProcessor for DefaultCaptureProcessor {
    fn normalize(
        &self,
        snapshot: ClipboardSnapshot,
        settings: &CaptureSettings,
        policy: &CapturePolicy,
    ) -> Result<Option<CapturedCapture>> {
        normalize_with_policy(snapshot, settings, policy)
    }

    fn prepare_preview(&self, captured: CapturedCapture) -> CapturedCapture {
        captured.prepare_preview()
    }
}

struct ClipboardListener {
    stop: Arc<std::sync::atomic::AtomicBool>,
    handle: JoinHandle<()>,
}

pub struct ClipboardService {
    shared: Arc<Shared>,
    listener: Mutex<Option<ClipboardListener>>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

struct IngestionJob {
    snapshot: ClipboardSnapshot,
    configuration: CaptureConfiguration,
    completion: Option<SyncSender<std::result::Result<CaptureCommit, String>>>,
}

impl ClipboardService {
    pub fn new(platform: Arc<dyn ClipboardPlatform>, sink: Arc<dyn ClipboardSink>) -> Self {
        Self::new_with_processor(platform, sink, Arc::new(DefaultCaptureProcessor))
    }

    fn new_with_processor(
        platform: Arc<dyn ClipboardPlatform>,
        sink: Arc<dyn ClipboardSink>,
        processor: Arc<dyn CaptureProcessor>,
    ) -> Self {
        let configuration = CaptureConfiguration {
            settings: sink.settings().unwrap_or_default(),
            policy: sink.capture_policy().unwrap_or_default(),
        };
        let baseline_sequence = platform.clipboard_sequence();
        let changes = platform.subscribe_changes();
        let post_subscribe_sequence = platform.clipboard_sequence();
        let (queue_sender, queue_receiver) = mpsc::sync_channel(INGESTION_QUEUE_CAPACITY);
        let shared = Arc::new(Shared {
            platform,
            sink,
            processor,
            configuration: Mutex::new(configuration),
            queue: Mutex::new(Some(queue_sender)),
            events: CaptureEventPublisher::default(),
            write_capture: Mutex::new(()),
            last_sequence: AtomicU64::new(baseline_sequence),
            ignored_sequence: AtomicU64::new(0),
            last_error: Mutex::new(None),
            metrics: Arc::new(OperationMetrics::default()),
        });
        let startup_sequence =
            (post_subscribe_sequence != baseline_sequence).then_some(post_subscribe_sequence);
        let worker_shared = Arc::clone(&shared);
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let handle = thread::Builder::new()
            .name("echo-clipboard-listener".to_owned())
            .spawn(move || listen(worker_shared, changes, startup_sequence, worker_stop))
            .expect("Echo clipboard listener thread");
        let ingestion_shared = Arc::clone(&shared);
        let ingestion_worker = thread::Builder::new()
            .name("echo-clipboard-ingestion".to_owned())
            .spawn(move || ingest(ingestion_shared, queue_receiver))
            .expect("Echo clipboard ingestion thread");
        Self {
            shared,
            listener: Mutex::new(Some(ClipboardListener { stop, handle })),
            worker: Mutex::new(Some(ingestion_worker)),
        }
    }

    pub fn capture_now(&self) -> Result<CaptureOutcome> {
        Ok(self.capture_now_commit()?.outcome)
    }

    pub fn capture_now_commit(&self) -> Result<CaptureCommit> {
        self.refresh_configuration()?;
        let sequence = self.shared.platform.clipboard_sequence();
        capture_sequence(&self.shared, sequence, true)?.ok_or(ClipboardError::StateUnavailable)
    }

    pub fn refresh_configuration(&self) -> Result<()> {
        let configuration = CaptureConfiguration {
            settings: self.shared.sink.settings().map_err(ClipboardError::Sink)?,
            policy: self
                .shared
                .sink
                .capture_policy()
                .map_err(ClipboardError::Sink)?,
        };
        *self
            .shared
            .configuration
            .lock()
            .map_err(|_| ClipboardError::StateUnavailable)? = configuration;
        Ok(())
    }

    pub fn subscribe_events(&self) -> CaptureEventSubscription {
        self.shared.events.subscribe()
    }

    pub fn publish_history_invalidation(&self, id: Option<i64>) {
        self.shared
            .events
            .publish(CaptureEvent::HistoryInvalidated { id });
    }

    pub fn copy_representations(&self, representations: &[ClipboardRepresentation]) -> Result<u64> {
        let _gate = lock(&self.shared.write_capture)?;
        let sequence = self.shared.platform.write_clipboard(representations)?;
        self.shared
            .ignored_sequence
            .store(sequence, Ordering::Release);
        self.shared.last_sequence.store(sequence, Ordering::Release);
        Ok(sequence)
    }

    pub fn metrics_snapshot(&self) -> Vec<crate::OperationMetric> {
        self.shared.metrics.snapshot()
    }

    pub fn request_maintenance(&self) {
        self.shared.sink.request_maintenance();
    }

    pub fn last_error(&self) -> Option<String> {
        self.shared
            .last_error
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn shutdown(&self) {
        let listener = self
            .listener
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        if let Some(listener) = listener {
            listener.stop.store(true, Ordering::Release);
            let _ = listener.handle.join();
        }
        if let Ok(mut queue) = self.shared.queue.lock() {
            queue.take();
        }
        if let Some(worker) = self
            .worker
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            let _ = worker.join();
        }
    }
}

impl Drop for ClipboardService {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn listen(
    shared: Arc<Shared>,
    changes: PlatformChangeSubscription,
    startup_sequence: Option<u64>,
    stop: Arc<std::sync::atomic::AtomicBool>,
) {
    if let Some(sequence) = startup_sequence {
        let _ = capture_sequence(&shared, sequence, false);
    }
    loop {
        match changes.recv_timeout(Duration::from_millis(100)) {
            Ok(PlatformChange::Clipboard { sequence }) => {
                let _ = capture_sequence(&shared, sequence, false);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) if !stop.load(Ordering::Acquire) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn capture_sequence(
    shared: &Arc<Shared>,
    sequence: u64,
    wait_for_commit: bool,
) -> Result<Option<CaptureCommit>> {
    let (snapshot, configuration, previous_sequence, read_elapsed, read_count) = {
        let _gate = lock(&shared.write_capture)?;
        if shared.ignored_sequence.load(Ordering::Acquire) == sequence {
            shared.last_sequence.store(sequence, Ordering::Release);
            return Ok(Some(CaptureCommit {
                outcome: CaptureOutcome::SelfWriteIgnored,
                event: None,
            }));
        }
        let previous_sequence = shared.last_sequence.load(Ordering::Acquire);
        if previous_sequence == sequence {
            return Ok(Some(CaptureCommit {
                outcome: CaptureOutcome::Unsupported,
                event: None,
            }));
        }
        let configuration = shared
            .configuration
            .lock()
            .map_err(|_| ClipboardError::StateUnavailable)?
            .clone();
        if !configuration.settings.history_enabled {
            shared.last_sequence.store(sequence, Ordering::Release);
            return Ok(Some(CaptureCommit {
                outcome: CaptureOutcome::Disabled,
                event: None,
            }));
        }
        let read_started = Instant::now();
        let snapshot_result = shared.platform.read_clipboard(&configuration.policy);
        let read_count = snapshot_result
            .as_ref()
            .ok()
            .and_then(Option::as_ref)
            .map(|snapshot| snapshot.representations.len() as u64)
            .unwrap_or(0);
        let read_elapsed = read_started.elapsed();
        let snapshot_result = match snapshot_result {
            Ok(snapshot) => snapshot,
            Err(error) => {
                drop(_gate);
                shared
                    .metrics
                    .record("capture_read", read_elapsed, read_count);
                return Err(error.into());
            }
        };
        let Some(snapshot) = snapshot_result else {
            shared.last_sequence.store(sequence, Ordering::Release);
            drop(_gate);
            shared
                .metrics
                .record("capture_read", read_elapsed, read_count);
            return Ok(Some(CaptureCommit {
                outcome: CaptureOutcome::Unsupported,
                event: None,
            }));
        };
        if snapshot.sequence != sequence || shared.platform.clipboard_sequence() != sequence {
            drop(_gate);
            shared
                .metrics
                .record("capture_read", read_elapsed, read_count);
            return Err(ClipboardError::ChangedDuringRead);
        }
        shared.last_sequence.store(sequence, Ordering::Release);
        (
            snapshot,
            configuration,
            previous_sequence,
            read_elapsed,
            read_count,
        )
    };
    shared
        .metrics
        .record("capture_read", read_elapsed, read_count);

    let (completion_sender, completion_receiver) = if wait_for_commit {
        let (sender, receiver) = mpsc::sync_channel(1);
        (Some(sender), Some(receiver))
    } else {
        (None, None)
    };
    if let Err(error) = enqueue(
        shared,
        IngestionJob {
            snapshot,
            configuration,
            completion: completion_sender,
        },
    ) {
        let _ = shared.last_sequence.compare_exchange(
            sequence,
            previous_sequence,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        return Err(error);
    }
    let Some(receiver) = completion_receiver else {
        return Ok(None);
    };
    receiver
        .recv()
        .map_err(|_| ClipboardError::StateUnavailable)?
        .map(Some)
        .map_err(ClipboardError::Sink)
}

fn enqueue(shared: &Arc<Shared>, job: IngestionJob) -> Result<()> {
    let sender = shared
        .queue
        .lock()
        .map_err(|_| ClipboardError::StateUnavailable)?
        .as_ref()
        .cloned()
        .ok_or(ClipboardError::StateUnavailable)?;
    let started = Instant::now();
    let result = sender
        .send(job)
        .map_err(|_| ClipboardError::StateUnavailable);
    shared
        .metrics
        .record("queue_wait", started.elapsed(), u64::from(result.is_ok()));
    result
}

fn ingest(shared: Arc<Shared>, receiver: Receiver<IngestionJob>) {
    while let Ok(job) = receiver.recv() {
        let IngestionJob {
            snapshot,
            configuration,
            completion,
        } = job;
        let captured = match shared.processor.normalize(
            snapshot,
            &configuration.settings,
            &configuration.policy,
        ) {
            Ok(Some(captured)) => captured,
            Ok(None) => {
                if let Some(completion) = completion {
                    let _ = completion.send(Ok(CaptureCommit {
                        outcome: CaptureOutcome::Unsupported,
                        event: None,
                    }));
                }
                continue;
            }
            Err(error) => {
                let error = error.to_string();
                if let Ok(mut last_error) = shared.last_error.lock() {
                    *last_error = Some(error.clone());
                }
                if let Some(completion) = completion {
                    let _ = completion.send(Err(error));
                }
                continue;
            }
        };
        let preview_disposition = captured
            .preview_source()
            .map(|source| shared.sink.preview_disposition(source))
            .transpose();
        let captured = match preview_disposition {
            Ok(Some(crate::PreviewDisposition::Generate)) => {
                let preview_started = Instant::now();
                let captured = shared.processor.prepare_preview(captured);
                shared.metrics.record(
                    "thumbnail_generation",
                    preview_started.elapsed(),
                    u64::from(captured.preview.is_some()),
                );
                captured
            }
            Ok(Some(crate::PreviewDisposition::ReuseExisting)) | Ok(None) => captured,
            Err(error) => {
                if let Ok(mut last_error) = shared.last_error.lock() {
                    *last_error = Some(error.clone());
                }
                if let Some(completion) = completion {
                    let _ = completion.send(Err(error));
                }
                continue;
            }
        };
        let result = shared.sink.record_captured(captured);
        match result {
            Ok(record) => {
                let event = CaptureEvent::HistoryChanged {
                    id: record.id,
                    duplicate: record.duplicate,
                };
                shared.events.publish(event.clone());
                let commit = CaptureCommit {
                    outcome: if record.duplicate {
                        CaptureOutcome::Duplicate(record.id)
                    } else {
                        CaptureOutcome::Recorded(record.id)
                    },
                    event: Some(event),
                };
                if let Some(completion) = completion {
                    let _ = completion.send(Ok(commit));
                }
            }
            Err(error) => {
                if let Ok(mut last_error) = shared.last_error.lock() {
                    *last_error = Some(error.clone());
                }
                if let Some(completion) = completion {
                    let _ = completion.send(Err(error));
                }
            }
        }
    }
}

#[cfg(test)]
fn normalize(snapshot: ClipboardSnapshot, settings: &CaptureSettings) -> Result<NormalizedCapture> {
    let sequence = snapshot.sequence;
    let fingerprint = ContentIdentity::from_representations(&snapshot.representations).fingerprint;
    Ok(
        normalize_with_policy(snapshot, settings, &CapturePolicy::default())?
            .map(|captured| captured.capture)
            .unwrap_or_else(|| NormalizedCapture {
                sequence,
                source: SourceContext::default(),
                content_type: ContentType::Text,
                preview_text: None,
                searchable_text: None,
                sanitized_html: None,
                fingerprint,
                representations: Vec::new(),
            }),
    )
}

fn normalize_with_policy(
    snapshot: ClipboardSnapshot,
    settings: &CaptureSettings,
    policy: &CapturePolicy,
) -> Result<Option<CapturedCapture>> {
    let sensitive = snapshot.source.is_password_input || snapshot.source.is_private_window;
    if sensitive && !settings.record_sensitive {
        return Ok(None);
    }
    let sequence = snapshot.sequence;
    let mut source = snapshot.source;
    if !source.is_source_verified {
        source.app_name = None;
        source.executable = None;
        source.window_title = None;
    } else if !settings.store_window_titles {
        source.window_title = None;
    }
    let mut plain_text = None;
    let mut sanitized_html = None;
    let mut preview = None;
    let mut representations = Vec::new();
    let mut total_bytes = 0_u64;
    for representation in snapshot.representations {
        let byte_size = representation.bytes.len() as u64;
        if !policy.accepts_size(&representation.format, byte_size, total_bytes) {
            continue;
        }
        match representation.format.as_str() {
            "text" | "files" => {
                if plain_text.is_none() {
                    plain_text = std::str::from_utf8(&representation.bytes)
                        .ok()
                        .map(str::to_owned);
                }
            }
            "html" => {
                let raw = String::from_utf8_lossy(&representation.bytes);
                let fragment = extract_html_fragment(&raw);
                let clean = sanitize_html(&fragment);
                if !clean.is_empty() {
                    preview = Some(strip_html(&clean));
                    sanitized_html = Some(clean);
                }
            }
            _ => {}
        }
        total_bytes = total_bytes.saturating_add(byte_size);
        representations.push(representation);
    }
    if representations.is_empty() {
        return Ok(None);
    }
    let content_type = if representations.iter().any(|item| item.format == "image") {
        ContentType::Image
    } else if representations.iter().any(|item| item.format == "files") {
        ContentType::Files
    } else if sanitized_html.is_some() {
        ContentType::Html
    } else if representations.iter().any(|item| item.format == "rtf") {
        ContentType::Rtf
    } else {
        ContentType::Text
    };
    let searchable_text = plain_text.clone().or_else(|| preview.clone());
    let preview_text = plain_text
        .or(preview)
        .map(|text| text.chars().take(PREVIEW_CHARACTERS).collect());
    let identity = ContentIdentity::from_representations(&representations);
    let capture = NormalizedCapture {
        sequence,
        source,
        content_type,
        preview_text,
        searchable_text,
        sanitized_html,
        fingerprint: identity.fingerprint.clone(),
        representations,
    };
    Ok(Some(CapturedCapture {
        capture,
        identity,
        preview: None,
    }))
}

fn extract_html_fragment(html: &str) -> String {
    if let (Some(start), Some(end)) = (
        html.find("<!--StartFragment-->")
            .map(|offset| offset + "<!--StartFragment-->".len()),
        html.find("<!--EndFragment-->"),
    ) {
        if start < end {
            return html[start..end].to_owned();
        }
    }
    let lower = html.to_ascii_lowercase();
    let Some(start) = lower.find("startfragment:") else {
        return html.to_owned();
    };
    let Some(end) = lower.find("endfragment:") else {
        return html.to_owned();
    };
    let start_offset = lower[start..]
        .find(|character: char| character.is_ascii_digit())
        .map(|offset| start + offset);
    let end_offset = lower[end..]
        .find(|character: char| character.is_ascii_digit())
        .map(|offset| end + offset);
    let (Some(start_offset), Some(end_offset)) = (start_offset, end_offset) else {
        return html.to_owned();
    };
    let start_number_end = html[start_offset..]
        .find(|character: char| !character.is_ascii_digit())
        .map(|offset| start_offset + offset)
        .unwrap_or(html.len());
    let end_number_end = html[end_offset..]
        .find(|character: char| !character.is_ascii_digit())
        .map(|offset| end_offset + offset)
        .unwrap_or(html.len());
    let Ok(start_index) = html[start_offset..start_number_end].parse::<usize>() else {
        return html.to_owned();
    };
    let Ok(end_index) = html[end_offset..end_number_end].parse::<usize>() else {
        return html.to_owned();
    };
    if start_index < end_index && end_index <= html.len() {
        return html[start_index..end_index].to_owned();
    }
    let marker_start = html
        .find("<!--StartFragment-->")
        .map(|offset| offset + "<!--StartFragment-->".len());
    let marker_end = html.find("<!--EndFragment-->");
    match (marker_start, marker_end) {
        (Some(start), Some(end)) if start < end => html[start..end].to_owned(),
        _ => html.to_owned(),
    }
}

fn sanitize_html(html: &str) -> String {
    Builder::default()
        .tags(
            [
                "p",
                "br",
                "b",
                "strong",
                "i",
                "em",
                "u",
                "s",
                "code",
                "pre",
                "blockquote",
                "ul",
                "ol",
                "li",
                "table",
                "thead",
                "tbody",
                "tr",
                "th",
                "td",
                "span",
                "div",
                "h1",
                "h2",
                "h3",
                "h4",
                "h5",
                "h6",
            ]
            .into(),
        )
        .clean(html)
        .to_string()
}

fn strip_html(html: &str) -> String {
    let mut text = String::with_capacity(html.len());
    let mut inside = false;
    for character in html.chars() {
        match character {
            '<' => inside = true,
            '>' => inside = false,
            character if !inside => text.push(character),
            _ => {}
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn fingerprint(representations: &[ClipboardRepresentation]) -> String {
    ContentIdentity::from_representations(representations).fingerprint
}

fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>> {
    mutex.lock().map_err(|_| ClipboardError::StateUnavailable)
}

#[cfg(test)]
#[derive(Default)]
struct MemorySink {
    settings: Mutex<CaptureSettings>,
    records: Mutex<Vec<NormalizedCapture>>,
}

#[cfg(test)]
impl MemorySink {
    fn records(&self) -> Vec<NormalizedCapture> {
        self.records
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    fn set_settings(&self, settings: CaptureSettings) {
        *self
            .settings
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = settings;
    }
}

#[cfg(test)]
impl ClipboardSink for MemorySink {
    fn settings(&self) -> std::result::Result<CaptureSettings, String> {
        Ok(self
            .settings
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone())
    }

    fn record(&self, capture: NormalizedCapture) -> std::result::Result<RecordResult, String> {
        let mut records = self
            .records
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if let Some((index, _)) = records
            .iter()
            .enumerate()
            .find(|(_, item)| item.fingerprint == capture.fingerprint)
        {
            records[index] = capture;
            return Ok(RecordResult {
                id: index as i64 + 1,
                duplicate: true,
            });
        }
        records.push(capture);
        Ok(RecordResult {
            id: records.len() as i64,
            duplicate: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        FocusSafety, InputTargetGeometry, PasteDelivery, PasteTarget, PhysicalRect,
        PlatformChangePublisher,
    };
    use std::collections::VecDeque;
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    struct TestPlatform {
        publisher: PlatformChangePublisher,
        sequence: AtomicU64,
        snapshots: Mutex<VecDeque<ClipboardSnapshot>>,
        clipboard_open: AtomicBool,
        writes: Mutex<Vec<Vec<ClipboardRepresentation>>>,
        read_failures: AtomicUsize,
        read_count: AtomicUsize,
        publish_during_write: AtomicBool,
        startup_snapshot: Mutex<Option<ClipboardSnapshot>>,
    }

    impl TestPlatform {
        fn new(snapshot: ClipboardSnapshot) -> Self {
            Self {
                publisher: PlatformChangePublisher::default(),
                sequence: AtomicU64::new(snapshot.sequence),
                snapshots: Mutex::new(VecDeque::from([snapshot])),
                clipboard_open: AtomicBool::new(false),
                writes: Mutex::new(Vec::new()),
                read_failures: AtomicUsize::new(0),
                read_count: AtomicUsize::new(0),
                publish_during_write: AtomicBool::new(false),
                startup_snapshot: Mutex::new(None),
            }
        }

        fn fail_next_read(&self) {
            self.read_failures.store(1, Ordering::Release);
        }

        fn publish_startup_snapshot(&self, snapshot: ClipboardSnapshot) {
            self.snapshots.lock().unwrap().clear();
            *self.startup_snapshot.lock().unwrap() = Some(snapshot);
        }

        fn set_snapshot(&self, snapshot: ClipboardSnapshot) {
            self.sequence.store(snapshot.sequence, Ordering::Release);
            self.snapshots.lock().unwrap().clear();
            self.snapshots.lock().unwrap().push_back(snapshot);
        }

        fn publish(&self, sequence: u64) {
            self.sequence.store(sequence, Ordering::Release);
            self.publisher
                .publish(PlatformChange::Clipboard { sequence });
        }

        fn clipboard_is_open(&self) -> bool {
            self.clipboard_open.load(Ordering::Acquire)
        }

        fn read_count(&self) -> usize {
            self.read_count.load(Ordering::Acquire)
        }
    }

    impl ClipboardPlatform for TestPlatform {
        fn subscribe_changes(&self) -> PlatformChangeSubscription {
            let subscription = self.publisher.subscribe();
            if let Some(snapshot) = self.startup_snapshot.lock().unwrap().take() {
                self.snapshots.lock().unwrap().push_back(snapshot.clone());
                self.publish(snapshot.sequence);
            }
            subscription
        }

        fn clipboard_sequence(&self) -> u64 {
            self.sequence.load(Ordering::Acquire)
        }

        fn read_clipboard(
            &self,
            _policy: &CapturePolicy,
        ) -> std::result::Result<Option<ClipboardSnapshot>, PlatformError> {
            if self.read_failures.load(Ordering::Acquire) > 0 {
                self.read_failures.fetch_sub(1, Ordering::AcqRel);
                return Err(PlatformError("transient clipboard read failure".to_owned()));
            }
            self.clipboard_open.store(true, Ordering::Release);
            let snapshot = self
                .snapshots
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .pop_front();
            self.clipboard_open.store(false, Ordering::Release);
            self.read_count.fetch_add(1, Ordering::AcqRel);
            Ok(snapshot)
        }

        fn write_clipboard(
            &self,
            representations: &[ClipboardRepresentation],
        ) -> std::result::Result<u64, PlatformError> {
            self.writes
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(representations.to_vec());
            let sequence = self.sequence.fetch_add(1, Ordering::AcqRel) + 1;
            if self.publish_during_write.load(Ordering::Acquire) {
                self.publish(sequence);
            }
            Ok(sequence)
        }

        fn capture_target(&self) -> std::result::Result<Option<PasteTarget>, PlatformError> {
            Ok(Some(PasteTarget {
                window_id: 1,
                window_class: "Edit".to_owned(),
                process_id: 1,
                process_started_at: 1,
                focused_control: None,
                app_name: None,
                selected_text: None,
                is_single_line: Some(true),
                geometry: InputTargetGeometry {
                    target: PhysicalRect {
                        x: 0,
                        y: 0,
                        width: 1,
                        height: 1,
                    },
                    work_area: PhysicalRect {
                        x: 0,
                        y: 0,
                        width: 1,
                        height: 1,
                    },
                    dpi: 96,
                },
            }))
        }

        fn paste_to_target(
            &self,
            _target: &PasteTarget,
        ) -> std::result::Result<PasteDelivery, PlatformError> {
            Ok(PasteDelivery::Pasted)
        }
    }

    struct BlockingSink {
        entered: std::sync::mpsc::Sender<()>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
        records: Mutex<Vec<NormalizedCapture>>,
    }

    impl BlockingSink {
        fn new() -> (
            Arc<Self>,
            std::sync::mpsc::Receiver<()>,
            std::sync::mpsc::Sender<()>,
        ) {
            let (entered, entered_receiver) = std::sync::mpsc::channel();
            let (release, release_receiver) = std::sync::mpsc::channel();
            (
                Arc::new(Self {
                    entered,
                    release: Mutex::new(release_receiver),
                    records: Mutex::new(Vec::new()),
                }),
                entered_receiver,
                release,
            )
        }
    }

    impl ClipboardSink for BlockingSink {
        fn settings(&self) -> std::result::Result<CaptureSettings, String> {
            Ok(CaptureSettings::default())
        }

        fn record(&self, capture: NormalizedCapture) -> std::result::Result<RecordResult, String> {
            self.entered.send(()).map_err(|error| error.to_string())?;
            self.release
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .recv()
                .map_err(|error| error.to_string())?;
            self.records
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(capture);
            Ok(RecordResult {
                id: 1,
                duplicate: false,
            })
        }
    }

    struct BlockingProcessor {
        entered: std::sync::mpsc::Sender<String>,
        release: Mutex<std::sync::mpsc::Receiver<()>>,
    }

    impl BlockingProcessor {
        fn new() -> (
            Arc<Self>,
            std::sync::mpsc::Receiver<String>,
            std::sync::mpsc::Sender<()>,
        ) {
            let (entered, entered_receiver) = std::sync::mpsc::channel();
            let (release, release_receiver) = std::sync::mpsc::channel();
            (
                Arc::new(Self {
                    entered,
                    release: Mutex::new(release_receiver),
                }),
                entered_receiver,
                release,
            )
        }
    }

    impl CaptureProcessor for BlockingProcessor {
        fn normalize(
            &self,
            snapshot: ClipboardSnapshot,
            settings: &CaptureSettings,
            policy: &CapturePolicy,
        ) -> Result<Option<CapturedCapture>> {
            let thread_name = std::thread::current().name().unwrap_or_default().to_owned();
            self.entered
                .send(thread_name)
                .map_err(|_| ClipboardError::StateUnavailable)?;
            self.release
                .lock()
                .map_err(|_| ClipboardError::StateUnavailable)?
                .recv()
                .map_err(|_| ClipboardError::StateUnavailable)?;
            DefaultCaptureProcessor.normalize(snapshot, settings, policy)
        }

        fn prepare_preview(&self, captured: CapturedCapture) -> CapturedCapture {
            DefaultCaptureProcessor.prepare_preview(captured)
        }
    }

    fn text_snapshot(sequence: u64, text: &str) -> ClipboardSnapshot {
        ClipboardSnapshot {
            sequence,
            source: SourceContext {
                app_name: Some("Editor".to_owned()),
                is_source_verified: true,
                is_sensitivity_verified: true,
                ..SourceContext::default()
            },
            representations: vec![ClipboardRepresentation {
                format: "text".to_owned(),
                mime_type: "text/plain;charset=utf-8".to_owned(),
                bytes: text.as_bytes().to_vec(),
            }],
        }
    }

    fn ingestion_job(snapshot: ClipboardSnapshot) -> IngestionJob {
        IngestionJob {
            snapshot,
            configuration: CaptureConfiguration {
                settings: CaptureSettings::default(),
                policy: CapturePolicy::default(),
            },
            completion: None,
        }
    }

    #[test]
    fn listener_records_and_deduplicates_by_fingerprint() {
        let platform = Arc::new(TestPlatform::new(text_snapshot(1, "hello")));
        let sink = Arc::new(MemorySink::default());
        let service = ClipboardService::new(platform.clone(), sink.clone());
        platform.snapshots.lock().unwrap().pop_front();
        platform
            .snapshots
            .lock()
            .unwrap()
            .push_back(text_snapshot(2, "hello"));
        platform.publish(2);
        std::thread::sleep(std::time::Duration::from_millis(30));
        assert_eq!(sink.records().len(), 1);
        platform
            .snapshots
            .lock()
            .unwrap()
            .push_back(text_snapshot(3, "world"));
        platform.publish(3);
        std::thread::sleep(std::time::Duration::from_millis(30));
        assert_eq!(sink.records().len(), 2);
        service.shutdown();
    }

    #[test]
    fn clipboard_is_released_before_slow_ingestion() {
        let platform = Arc::new(TestPlatform::new(text_snapshot(1, "initial")));
        let sink = BlockingSink::new();
        let service = Arc::new(ClipboardService::new(platform.clone(), sink.0.clone()));
        platform.set_snapshot(text_snapshot(2, "owned by echo"));
        let capture_service = Arc::clone(&service);
        let capture = std::thread::spawn(move || capture_service.capture_now());

        sink.1.recv().unwrap();
        assert!(!platform.clipboard_is_open());
        sink.2.send(()).unwrap();
        assert!(matches!(
            capture.join().unwrap().unwrap(),
            CaptureOutcome::Recorded(1)
        ));
        service.shutdown();
    }

    #[test]
    fn normalization_runs_on_the_ingestion_worker_after_the_capture_gate_is_released() {
        let platform = Arc::new(TestPlatform::new(text_snapshot(1, "initial")));
        let sink = Arc::new(MemorySink::default());
        let processor = BlockingProcessor::new();
        let service = Arc::new(ClipboardService::new_with_processor(
            platform.clone(),
            sink.clone(),
            processor.0,
        ));
        platform.set_snapshot(text_snapshot(2, "owned before normalization"));

        let capture_service = Arc::clone(&service);
        let capture = std::thread::spawn(move || capture_service.capture_now());
        assert_eq!(processor.1.recv().unwrap(), "echo-clipboard-ingestion");
        assert!(!platform.clipboard_is_open());

        let copy_service = Arc::clone(&service);
        let (copy_done, copy_receiver) = std::sync::mpsc::channel();
        let copy = std::thread::spawn(move || {
            let result = copy_service.copy_representations(&[ClipboardRepresentation {
                format: "text".to_owned(),
                mime_type: "text/plain".to_owned(),
                bytes: b"self write".to_vec(),
            }]);
            copy_done.send(result).unwrap();
        });
        assert!(copy_receiver
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap()
            .is_ok());

        processor.2.send(()).unwrap();
        assert!(matches!(
            capture.join().unwrap().unwrap(),
            CaptureOutcome::Recorded(1)
        ));
        copy.join().unwrap();
        let records = sink.records();
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].representations[0].bytes,
            b"owned before normalization"
        );
        assert_eq!(
            records[0].fingerprint,
            ContentIdentity::from_representations(&records[0].representations).fingerprint
        );
        service.shutdown();
    }

    #[test]
    fn ingestion_queue_applies_bounded_backpressure() {
        let platform = Arc::new(TestPlatform::new(text_snapshot(1, "initial")));
        let sink = BlockingSink::new();
        let service = Arc::new(ClipboardService::new(platform.clone(), sink.0.clone()));
        let sender = service
            .shared
            .queue
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .cloned()
            .unwrap();
        sender
            .send(ingestion_job(text_snapshot(10, "queued")))
            .unwrap();
        sink.1.recv().unwrap();
        for sequence in 0..INGESTION_QUEUE_CAPACITY {
            sender
                .try_send(ingestion_job(text_snapshot(sequence as u64 + 11, "queued")))
                .unwrap();
        }
        assert!(matches!(
            sender.try_send(ingestion_job(text_snapshot(20, "queued"))),
            Err(std::sync::mpsc::TrySendError::Full(_))
        ));

        platform.set_snapshot(text_snapshot(2, "backpressured raw bytes"));
        let capture_service = Arc::clone(&service);
        let (capture_done, capture_receiver) = std::sync::mpsc::channel();
        let capture = std::thread::spawn(move || {
            capture_done.send(capture_service.capture_now()).unwrap();
        });
        let deadline = Instant::now() + std::time::Duration::from_secs(1);
        while platform.read_count() == 0 {
            assert!(
                Instant::now() < deadline,
                "capture did not finish its OS read"
            );
            std::thread::yield_now();
        }

        let copy_service = Arc::clone(&service);
        let (copy_done, copy_receiver) = std::sync::mpsc::channel();
        let copy = std::thread::spawn(move || {
            copy_done
                .send(
                    copy_service.copy_representations(&[ClipboardRepresentation {
                        format: "text".to_owned(),
                        mime_type: "text/plain".to_owned(),
                        bytes: b"write during backpressure".to_vec(),
                    }]),
                )
                .unwrap();
        });
        assert!(copy_receiver
            .recv_timeout(std::time::Duration::from_secs(1))
            .unwrap()
            .is_ok());
        assert!(!platform.clipboard_is_open());
        assert!(matches!(
            capture_receiver.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        platform.set_snapshot(text_snapshot(30, "replacement"));

        for _ in 0..INGESTION_QUEUE_CAPACITY + 2 {
            sink.2.send(()).unwrap();
        }
        assert!(matches!(
            capture_receiver
                .recv_timeout(std::time::Duration::from_secs(1))
                .unwrap()
                .unwrap(),
            CaptureOutcome::Recorded(_)
        ));
        capture.join().unwrap();
        copy.join().unwrap();
        assert!(sink
            .0
            .records
            .lock()
            .unwrap()
            .iter()
            .any(|capture| { capture.representations[0].bytes == b"backpressured raw bytes" }));
        drop(sender);
        service.shutdown();
    }

    #[test]
    fn commit_event_is_published_after_ingestion() {
        let platform = Arc::new(TestPlatform::new(text_snapshot(1, "initial")));
        let sink = Arc::new(MemorySink::default());
        let service = ClipboardService::new(platform.clone(), sink);
        let events = service.subscribe_events();
        platform.set_snapshot(text_snapshot(2, "event"));

        let commit = service.capture_now_commit().unwrap();
        assert_eq!(commit.outcome, CaptureOutcome::Recorded(1));
        assert_eq!(
            commit.event,
            Some(CaptureEvent::HistoryChanged {
                id: 1,
                duplicate: false
            })
        );
        assert_eq!(events.recv().unwrap(), commit.event.unwrap());
        service.shutdown();
    }

    #[test]
    fn capture_policy_skips_oversized_supported_representations() {
        let snapshot = ClipboardSnapshot {
            sequence: 1,
            source: SourceContext::default(),
            representations: vec![
                ClipboardRepresentation {
                    format: "text".to_owned(),
                    mime_type: "text/plain".to_owned(),
                    bytes: b"too-large".to_vec(),
                },
                ClipboardRepresentation {
                    format: "html".to_owned(),
                    mime_type: "text/html".to_owned(),
                    bytes: b"<b>x</b>".to_vec(),
                },
            ],
        };
        let policy = CapturePolicy {
            max_representation_bytes: 8,
            max_total_bytes: 16,
            supported_formats: vec!["text".to_owned(), "html".to_owned()],
        };
        let captured = normalize_with_policy(snapshot, &CaptureSettings::default(), &policy)
            .unwrap()
            .unwrap();
        assert_eq!(captured.capture.representations.len(), 1);
        assert_eq!(captured.capture.content_type, ContentType::Html);
        assert_eq!(captured.identity.total_byte_size, 8);
    }

    #[test]
    fn content_identity_reuses_the_capture_fingerprint_and_representation_hashes() {
        let representations = vec![ClipboardRepresentation {
            format: "image".to_owned(),
            mime_type: "image/bmp".to_owned(),
            bytes: vec![1, 2, 3, 4],
        }];
        let identity = ContentIdentity::from_representations(&representations);
        assert_eq!(identity.fingerprint, fingerprint(&representations));
        assert_eq!(identity.total_byte_size, 4);
        assert_eq!(identity.representations[0].format, "image");
        assert_eq!(identity.representations[0].byte_size, 4);
        let mut hasher = Sha256::new();
        hasher.update(&representations[0].bytes);
        assert_eq!(
            identity.representations[0].hash,
            format!("{:x}", hasher.finalize())
        );
    }

    #[test]
    fn self_write_is_suppressed() {
        let platform = Arc::new(TestPlatform::new(text_snapshot(1, "hello")));
        let sink = Arc::new(MemorySink::default());
        let service = ClipboardService::new(platform.clone(), sink.clone());
        service
            .copy_representations(&[ClipboardRepresentation {
                format: "text".to_owned(),
                mime_type: "text/plain".to_owned(),
                bytes: b"copied".to_vec(),
            }])
            .unwrap();
        assert!(sink.records().is_empty());
        service.shutdown();
    }

    #[test]
    fn self_write_event_published_inside_platform_write_is_ignored() {
        let platform = Arc::new(TestPlatform::new(text_snapshot(1, "hello")));
        let sink = Arc::new(MemorySink::default());
        let service = ClipboardService::new(platform.clone(), sink.clone());
        platform.publish_during_write.store(true, Ordering::Release);
        service
            .copy_representations(&[ClipboardRepresentation {
                format: "text".to_owned(),
                mime_type: "text/plain".to_owned(),
                bytes: b"copied".to_vec(),
            }])
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(30));
        assert!(sink.records().is_empty());
        service.shutdown();
    }

    #[test]
    fn history_switch_disables_capture() {
        let platform = Arc::new(TestPlatform::new(text_snapshot(1, "disabled")));
        let sink = Arc::new(MemorySink::default());
        sink.set_settings(CaptureSettings {
            history_enabled: false,
            ..CaptureSettings::default()
        });
        let service = ClipboardService::new(platform.clone(), sink.clone());
        platform.set_snapshot(text_snapshot(2, "disabled"));
        assert_eq!(service.capture_now().unwrap(), CaptureOutcome::Disabled);
        assert!(sink.records().is_empty());
        service.shutdown();
    }

    #[test]
    fn retries_the_same_sequence_after_a_transient_read_error() {
        let platform = Arc::new(TestPlatform::new(text_snapshot(1, "eventually captured")));
        let sink = Arc::new(MemorySink::default());
        let service = ClipboardService::new(platform.clone(), sink.clone());
        platform.set_snapshot(text_snapshot(2, "eventually captured"));
        platform.fail_next_read();
        assert!(service.capture_now().is_err());
        assert_eq!(sink.records().len(), 0);
        assert!(matches!(
            service.capture_now().unwrap(),
            CaptureOutcome::Recorded(_)
        ));
        service.shutdown();
    }

    #[test]
    fn enqueue_failure_releases_the_claimed_sequence_for_retry() {
        let platform = Arc::new(TestPlatform::new(text_snapshot(1, "initial")));
        let sink = Arc::new(MemorySink::default());
        let service = ClipboardService::new(platform.clone(), sink);
        service.shared.queue.lock().unwrap().take();
        platform.set_snapshot(text_snapshot(2, "retry after queue failure"));

        assert!(service.capture_now().is_err());
        assert_eq!(service.shared.last_sequence.load(Ordering::Acquire), 1);
        service.shutdown();
    }

    #[test]
    fn startup_recheck_captures_a_change_that_races_subscription() {
        let platform = Arc::new(TestPlatform::new(text_snapshot(1, "before")));
        platform.publish_startup_snapshot(text_snapshot(2, "startup race"));
        let sink = Arc::new(MemorySink::default());
        let service = ClipboardService::new(platform, sink.clone());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        loop {
            if sink
                .records()
                .iter()
                .any(|capture| capture.preview_text.as_deref() == Some("startup race"))
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "startup clipboard change was lost"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        service.shutdown();
    }

    #[test]
    fn sensitive_source_is_not_recorded_by_default() {
        let mut snapshot = text_snapshot(1, "secret");
        snapshot.source.is_password_input = true;
        let platform = Arc::new(TestPlatform::new(snapshot));
        let sink = Arc::new(MemorySink::default());
        let service = ClipboardService::new(platform, sink.clone());
        assert_eq!(service.capture_now().unwrap(), CaptureOutcome::Unsupported);
        assert!(sink.records().is_empty());
        service.shutdown();
    }

    #[test]
    fn html_is_sanitized_and_fragment_is_used() {
        let snapshot = ClipboardSnapshot {
            sequence: 1,
            source: SourceContext::default(),
            representations: vec![ClipboardRepresentation {
                format: "html".to_owned(),
                mime_type: "text/html".to_owned(),
                bytes: b"Version:1.0\r\nStartHTML:00000000\r\nEndHTML:00000080\r\nStartFragment:00000040\r\nEndFragment:00000070\r\n<html><body><!--StartFragment--><b>ok</b><script>x</script><!--EndFragment--></body></html>".to_vec(),
            }],
        };
        let normalized = normalize(snapshot, &CaptureSettings::default()).unwrap();
        assert_eq!(normalized.content_type, ContentType::Html);
        assert!(normalized.sanitized_html.unwrap().contains("<b>ok</b>"));
        assert!(!normalized.preview_text.unwrap().contains("script"));
    }

    #[test]
    fn fingerprint_changes_when_representation_order_or_bytes_change() {
        let text = ClipboardRepresentation {
            format: "text".to_owned(),
            mime_type: "text/plain".to_owned(),
            bytes: b"a".to_vec(),
        };
        let html = ClipboardRepresentation {
            format: "html".to_owned(),
            mime_type: "text/html".to_owned(),
            bytes: b"<b>a</b>".to_vec(),
        };
        assert_ne!(
            fingerprint(&[text.clone(), html.clone()]),
            fingerprint(&[html, text.clone()])
        );
        assert_ne!(
            fingerprint(&[text]),
            fingerprint(&[ClipboardRepresentation {
                format: "text".to_owned(),
                mime_type: "text/plain".to_owned(),
                bytes: b"b".to_vec(),
            }])
        );
    }

    #[test]
    fn target_matching_requires_the_same_process_instance_and_focus() {
        let target = PasteTarget {
            window_id: 9,
            window_class: "Edit".to_owned(),
            process_id: 3,
            process_started_at: 7,
            focused_control: Some(crate::PasteControlIdentity::NativeWindow {
                handle: 10,
                class_name: "Edit".to_owned(),
            }),
            app_name: None,
            selected_text: None,
            is_single_line: Some(true),
            geometry: InputTargetGeometry {
                target: PhysicalRect {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                work_area: PhysicalRect {
                    x: 0,
                    y: 0,
                    width: 1,
                    height: 1,
                },
                dpi: 96,
            },
        };
        let focus = crate::FocusedTarget {
            native_window_id: 9,
            process_id: 3,
            process_instance_id: Some(7),
            identity: target.focused_control.clone(),
            safety: FocusSafety::Safe,
            is_text_input: true,
        };
        assert!(target.matches_focus(&focus));
        assert!(!target.matches_window(&crate::FocusedTarget {
            process_instance_id: Some(8),
            ..focus
        }));
    }
}
