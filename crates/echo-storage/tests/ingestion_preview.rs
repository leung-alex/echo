use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use echo_engine::{
    CaptureOutcome, CapturePolicy, ClipboardPlatform, ClipboardRepresentation, ClipboardService,
    ClipboardSnapshot, OperationMetric, PasteDelivery, PasteDeliveryFailure, PasteTarget,
    PlatformChangePublisher, PlatformChangeSubscription, PlatformError, SourceContext,
};
use echo_storage::SharedClipboardStore;
use tempfile::TempDir;

struct SnapshotPlatform {
    sequence: AtomicU64,
    snapshot: Mutex<ClipboardSnapshot>,
    changes: PlatformChangePublisher,
}

impl SnapshotPlatform {
    fn new(snapshot: ClipboardSnapshot) -> Self {
        Self {
            sequence: AtomicU64::new(snapshot.sequence),
            snapshot: Mutex::new(snapshot),
            changes: PlatformChangePublisher::default(),
        }
    }

    fn set_snapshot(&self, snapshot: ClipboardSnapshot) {
        self.sequence.store(snapshot.sequence, Ordering::Release);
        *self.snapshot.lock().unwrap() = snapshot;
    }
}

impl ClipboardPlatform for SnapshotPlatform {
    fn subscribe_changes(&self) -> PlatformChangeSubscription {
        self.changes.subscribe()
    }

    fn clipboard_sequence(&self) -> u64 {
        self.sequence.load(Ordering::Acquire)
    }

    fn read_clipboard(
        &self,
        _policy: &CapturePolicy,
    ) -> Result<Option<ClipboardSnapshot>, PlatformError> {
        Ok(Some(self.snapshot.lock().unwrap().clone()))
    }

    fn write_clipboard(
        &self,
        _representations: &[ClipboardRepresentation],
    ) -> Result<u64, PlatformError> {
        Ok(self.sequence.fetch_add(1, Ordering::AcqRel) + 1)
    }

    fn capture_target(&self) -> Result<Option<PasteTarget>, PlatformError> {
        Ok(None)
    }

    fn paste_to_target(&self, _target: &PasteTarget) -> Result<PasteDelivery, PlatformError> {
        Ok(PasteDelivery::Failed(
            PasteDeliveryFailure::InputUnavailable,
        ))
    }
}

fn image_snapshot(sequence: u64, bytes: &[u8]) -> ClipboardSnapshot {
    ClipboardSnapshot {
        sequence,
        source: SourceContext {
            app_name: Some("Image Editor".to_owned()),
            is_source_verified: true,
            is_sensitivity_verified: true,
            ..SourceContext::default()
        },
        representations: vec![ClipboardRepresentation {
            format: "image".to_owned(),
            mime_type: "image/bmp".to_owned(),
            bytes: bytes.to_vec(),
        }],
    }
}

fn large_bmp(width: u32, height: u32) -> Vec<u8> {
    let row_size = (width * 3).div_ceil(4) * 4;
    let pixel_bytes = row_size * height;
    let file_size = 54 + pixel_bytes;
    let mut bytes = vec![0_u8; file_size as usize];
    bytes[0..2].copy_from_slice(b"BM");
    bytes[2..6].copy_from_slice(&file_size.to_le_bytes());
    bytes[10..14].copy_from_slice(&54_u32.to_le_bytes());
    bytes[14..18].copy_from_slice(&40_u32.to_le_bytes());
    bytes[18..22].copy_from_slice(&(width as i32).to_le_bytes());
    bytes[22..26].copy_from_slice(&(height as i32).to_le_bytes());
    bytes[26..28].copy_from_slice(&1_u16.to_le_bytes());
    bytes[28..30].copy_from_slice(&24_u16.to_le_bytes());
    bytes[34..38].copy_from_slice(&pixel_bytes.to_le_bytes());
    for y in 0..height {
        for x in 0..width {
            let offset = 54 + (y * row_size + x * 3) as usize;
            bytes[offset] = (x % 251) as u8;
            bytes[offset + 1] = (y % 241) as u8;
            bytes[offset + 2] = ((x + y) % 239) as u8;
        }
    }
    bytes
}

fn samples(metrics: &[OperationMetric], operation: &str) -> u64 {
    metrics
        .iter()
        .find(|metric| metric.operation == operation)
        .map(|metric| metric.samples)
        .unwrap_or(0)
}

fn capture_fixture() -> (
    TempDir,
    Arc<SnapshotPlatform>,
    Arc<SharedClipboardStore>,
    ClipboardService,
    Vec<u8>,
) {
    let root = TempDir::new().unwrap();
    let image = large_bmp(512, 512);
    let platform = Arc::new(SnapshotPlatform::new(image_snapshot(1, &image)));
    let store = Arc::new(SharedClipboardStore::open(root.path()).unwrap());
    let service = ClipboardService::new(platform.clone(), store.clone());
    (root, platform, store, service, image)
}

#[test]
fn duplicate_large_image_reuses_the_existing_thumbnail_without_new_writes() {
    let (_root, platform, store, service, image) = capture_fixture();
    platform.set_snapshot(image_snapshot(2, &image));
    let first = service.capture_now().unwrap();
    let first_id = match first {
        CaptureOutcome::Recorded(id) => id,
        outcome => panic!("unexpected first capture outcome: {outcome:?}"),
    };
    assert_eq!(
        samples(&service.metrics_snapshot(), "thumbnail_generation"),
        1
    );
    assert_eq!(samples(&store.metrics_snapshot(), "blob_write"), 1);
    assert_eq!(samples(&store.metrics_snapshot(), "thumbnail_write"), 1);

    platform.set_snapshot(image_snapshot(3, &image));
    assert_eq!(
        service.capture_now().unwrap(),
        CaptureOutcome::Duplicate(first_id)
    );
    assert_eq!(
        samples(&service.metrics_snapshot(), "thumbnail_generation"),
        1
    );
    assert_eq!(samples(&store.metrics_snapshot(), "blob_write"), 1);
    assert_eq!(samples(&store.metrics_snapshot(), "thumbnail_write"), 1);
    assert_eq!(store.entry_payload(first_id).unwrap()[0].bytes, image);
    service.shutdown();
}

#[test]
fn corrupt_thumbnail_is_regenerated_once_and_repaired() {
    let (root, platform, store, service, image) = capture_fixture();
    platform.set_snapshot(image_snapshot(2, &image));
    let first_id = match service.capture_now().unwrap() {
        CaptureOutcome::Recorded(id) => id,
        outcome => panic!("unexpected first capture outcome: {outcome:?}"),
    };
    let thumbnail = store
        .entry(first_id)
        .unwrap()
        .unwrap()
        .entry
        .thumbnail
        .unwrap();
    let thumbnail_path = root.path().join("thumbnails").join(&thumbnail.content_hash);
    std::fs::write(&thumbnail_path, b"corrupt").unwrap();
    assert!(store
        .read_thumbnail(&thumbnail.content_hash)
        .unwrap()
        .is_none());

    platform.set_snapshot(image_snapshot(3, &image));
    assert_eq!(
        service.capture_now().unwrap(),
        CaptureOutcome::Duplicate(first_id)
    );
    assert_eq!(
        samples(&service.metrics_snapshot(), "thumbnail_generation"),
        2
    );
    assert_eq!(samples(&store.metrics_snapshot(), "blob_write"), 1);
    assert_eq!(samples(&store.metrics_snapshot(), "thumbnail_write"), 2);
    assert!(store
        .read_thumbnail(&thumbnail.content_hash)
        .unwrap()
        .is_some());
    assert_eq!(store.entry_payload(first_id).unwrap()[0].bytes, image);
    service.shutdown();
}
