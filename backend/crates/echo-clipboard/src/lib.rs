use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use ammonia::Builder;
use echo_platform::{
    ClipboardPlatform, ClipboardRepresentation, ClipboardSnapshot, PlatformChange,
    PlatformChangeSubscription, PlatformError, SourceContext,
};
use sha2::{Digest, Sha256};
use thiserror::Error;

pub use echo_platform::{ClipboardRepresentation as Representation, PasteDelivery, PasteTarget};

const PREVIEW_CHARACTERS: usize = 600;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordResult {
    pub id: i64,
    pub duplicate: bool,
}

pub trait ClipboardSink: Send + Sync {
    fn settings(&self) -> std::result::Result<CaptureSettings, String>;
    fn record(&self, capture: NormalizedCapture) -> std::result::Result<RecordResult, String>;
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
    write_capture: Mutex<()>,
    last_sequence: AtomicU64,
    ignored_sequence: AtomicU64,
    last_error: Mutex<Option<String>>,
}

struct ClipboardListener {
    stop: Arc<std::sync::atomic::AtomicBool>,
    handle: JoinHandle<()>,
}

pub struct ClipboardService {
    shared: Arc<Shared>,
    listener: Mutex<Option<ClipboardListener>>,
    maintenance: Mutex<Option<JoinHandle<()>>>,
}

impl ClipboardService {
    pub fn new(platform: Arc<dyn ClipboardPlatform>, sink: Arc<dyn ClipboardSink>) -> Self {
        let baseline_sequence = platform.clipboard_sequence();
        let changes = platform.subscribe_changes();
        let post_subscribe_sequence = platform.clipboard_sequence();
        let shared = Arc::new(Shared {
            platform,
            sink,
            write_capture: Mutex::new(()),
            last_sequence: AtomicU64::new(baseline_sequence),
            ignored_sequence: AtomicU64::new(0),
            last_error: Mutex::new(None),
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
        Self {
            shared,
            listener: Mutex::new(Some(ClipboardListener { stop, handle })),
            maintenance: Mutex::new(None),
        }
    }

    pub fn capture_now(&self) -> Result<CaptureOutcome> {
        let sequence = self.shared.platform.clipboard_sequence();
        capture_sequence(&self.shared, sequence)
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

    pub fn start_maintenance(&self) {
        let mut maintenance = self
            .maintenance
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if maintenance.is_some() {
            return;
        }
        let shared = Arc::clone(&self.shared);
        *maintenance = thread::Builder::new()
            .name("echo-clipboard-maintenance".to_owned())
            .spawn(move || {
                if let Err(error) = shared.sink.reconcile() {
                    if let Ok(mut last_error) = shared.last_error.lock() {
                        *last_error = Some(error);
                    }
                }
            })
            .ok();
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
        if let Some(maintenance) = self
            .maintenance
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            let _ = maintenance.join();
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
        let _ = capture_sequence(&shared, sequence);
    }
    loop {
        match changes.recv_timeout(Duration::from_millis(100)) {
            Ok(PlatformChange::Clipboard { sequence }) => {
                let _ = capture_sequence(&shared, sequence);
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) if !stop.load(Ordering::Acquire) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn capture_sequence(shared: &Arc<Shared>, sequence: u64) -> Result<CaptureOutcome> {
    let _gate = lock(&shared.write_capture)?;
    if shared.ignored_sequence.load(Ordering::Acquire) == sequence {
        shared.last_sequence.store(sequence, Ordering::Release);
        return Ok(CaptureOutcome::SelfWriteIgnored);
    }
    if shared.last_sequence.load(Ordering::Acquire) == sequence {
        return Ok(CaptureOutcome::Unsupported);
    }
    let settings = shared
        .sink
        .settings()
        .map_err(|error| ClipboardError::Sink(error))?;
    if !settings.history_enabled {
        shared.last_sequence.store(sequence, Ordering::Release);
        return Ok(CaptureOutcome::Disabled);
    }
    let Some(snapshot) = shared.platform.read_clipboard()? else {
        shared.last_sequence.store(sequence, Ordering::Release);
        return Ok(CaptureOutcome::Unsupported);
    };
    if snapshot.sequence != sequence || shared.platform.clipboard_sequence() != sequence {
        return Err(ClipboardError::ChangedDuringRead);
    }
    let normalized = normalize(snapshot, &settings)?;
    if normalized.representations.is_empty() {
        shared.last_sequence.store(sequence, Ordering::Release);
        return Ok(CaptureOutcome::Unsupported);
    }
    let result = shared
        .sink
        .record(normalized)
        .map_err(|error| ClipboardError::Sink(error))?;
    shared.last_sequence.store(sequence, Ordering::Release);
    Ok(if result.duplicate {
        CaptureOutcome::Duplicate(result.id)
    } else {
        CaptureOutcome::Recorded(result.id)
    })
}

fn normalize(snapshot: ClipboardSnapshot, settings: &CaptureSettings) -> Result<NormalizedCapture> {
    let sensitive = snapshot.source.is_password_input || snapshot.source.is_private_window;
    if sensitive && !settings.record_sensitive {
        return Ok(NormalizedCapture {
            sequence: snapshot.sequence,
            source: SourceContext::default(),
            content_type: ContentType::Text,
            preview_text: None,
            searchable_text: None,
            sanitized_html: None,
            fingerprint: fingerprint(&snapshot.representations),
            representations: Vec::new(),
        });
    }
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
    for representation in &snapshot.representations {
        match representation.format.as_str() {
            "text" | "files" => {
                if plain_text.is_none() {
                    plain_text = String::from_utf8(representation.bytes.clone()).ok();
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
    }
    let content_type = if snapshot
        .representations
        .iter()
        .any(|item| item.format == "image")
    {
        ContentType::Image
    } else if snapshot
        .representations
        .iter()
        .any(|item| item.format == "files")
    {
        ContentType::Files
    } else if sanitized_html.is_some() {
        ContentType::Html
    } else if snapshot
        .representations
        .iter()
        .any(|item| item.format == "rtf")
    {
        ContentType::Rtf
    } else {
        ContentType::Text
    };
    let searchable_text = plain_text.clone().or_else(|| preview.clone());
    let preview_text = plain_text
        .or(preview)
        .map(|text| text.chars().take(PREVIEW_CHARACTERS).collect());
    Ok(NormalizedCapture {
        sequence: snapshot.sequence,
        source,
        content_type,
        preview_text,
        searchable_text,
        sanitized_html,
        fingerprint: fingerprint(&snapshot.representations),
        representations: snapshot.representations,
    })
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
    let mut hasher = Sha256::new();
    for representation in representations {
        hasher.update((representation.format.len() as u64).to_le_bytes());
        hasher.update(representation.format.as_bytes());
        hasher.update((representation.bytes.len() as u64).to_le_bytes());
        hasher.update(&representation.bytes);
    }
    format!("{:x}", hasher.finalize())
}

fn lock<T>(mutex: &Mutex<T>) -> Result<MutexGuard<'_, T>> {
    mutex.lock().map_err(|_| ClipboardError::StateUnavailable)
}

#[derive(Default)]
pub struct MemorySink {
    settings: Mutex<CaptureSettings>,
    records: Mutex<Vec<NormalizedCapture>>,
}

impl MemorySink {
    pub fn records(&self) -> Vec<NormalizedCapture> {
        self.records
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    }

    pub fn set_settings(&self, settings: CaptureSettings) {
        *self
            .settings
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = settings;
    }
}

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
    use echo_platform::{
        FocusSafety, InputTargetGeometry, PasteDelivery, PasteTarget, PhysicalRect,
        PlatformChangePublisher,
    };
    use std::collections::VecDeque;

    struct TestPlatform {
        publisher: PlatformChangePublisher,
        sequence: AtomicU64,
        snapshots: Mutex<VecDeque<ClipboardSnapshot>>,
        writes: Mutex<Vec<Vec<ClipboardRepresentation>>>,
    }

    impl TestPlatform {
        fn new(snapshot: ClipboardSnapshot) -> Self {
            Self {
                publisher: PlatformChangePublisher::default(),
                sequence: AtomicU64::new(snapshot.sequence),
                snapshots: Mutex::new(VecDeque::from([snapshot])),
                writes: Mutex::new(Vec::new()),
            }
        }

        fn publish(&self, sequence: u64) {
            self.sequence.store(sequence, Ordering::Release);
            self.publisher
                .publish(PlatformChange::Clipboard { sequence });
        }
    }

    impl ClipboardPlatform for TestPlatform {
        fn subscribe_changes(&self) -> PlatformChangeSubscription {
            self.publisher.subscribe()
        }

        fn clipboard_sequence(&self) -> u64 {
            self.sequence.load(Ordering::Acquire)
        }

        fn read_clipboard(&self) -> std::result::Result<Option<ClipboardSnapshot>, PlatformError> {
            Ok(self
                .snapshots
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .pop_front())
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
            focused_control: Some(echo_platform::PasteControlIdentity::NativeWindow {
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
        let focus = echo_platform::FocusedTarget {
            native_window_id: 9,
            process_id: 3,
            process_instance_id: Some(7),
            identity: target.focused_control.clone(),
            safety: FocusSafety::Safe,
            is_text_input: true,
        };
        assert!(target.matches_focus(&focus));
        assert!(!target.matches_window(&echo_platform::FocusedTarget {
            process_instance_id: Some(8),
            ..focus
        }));
    }
}
