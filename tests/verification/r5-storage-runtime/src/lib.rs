#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;
    use std::time::{Duration, Instant};

    use echo_engine::{
        CapturePolicy, ClipboardPlatform, ClipboardRepresentation, ClipboardService,
        ClipboardSnapshot, PasteDelivery, PasteDeliveryFailure, PasteTarget,
        PlatformChangePublisher, PlatformChangeSubscription, PlatformError,
    };
    use echo_storage::SharedClipboardStore;
    use tempfile::TempDir;

    #[derive(Default)]
    struct IdlePlatform {
        changes: PlatformChangePublisher,
    }

    impl ClipboardPlatform for IdlePlatform {
        fn subscribe_changes(&self) -> PlatformChangeSubscription {
            self.changes.subscribe()
        }

        fn clipboard_sequence(&self) -> u64 {
            0
        }

        fn read_clipboard(
            &self,
            _policy: &CapturePolicy,
        ) -> Result<Option<ClipboardSnapshot>, PlatformError> {
            Ok(None)
        }

        fn write_clipboard(
            &self,
            _representations: &[ClipboardRepresentation],
        ) -> Result<u64, PlatformError> {
            Ok(0)
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

    #[test]
    fn desktop_startup_does_not_schedule_storage_maintenance_twice() {
        let root = TempDir::new().unwrap();
        let store = Arc::new(SharedClipboardStore::open(root.path()).unwrap());
        let platform = Arc::new(IdlePlatform::default());
        let clipboard = ClipboardService::new(platform, store.clone());

        clipboard.start_maintenance();

        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            let samples = store
                .metrics_snapshot()
                .into_iter()
                .find(|metric| metric.operation == "maintenance_reconcile")
                .map(|metric| metric.samples)
                .unwrap_or(0);
            if samples >= 2 {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }

        let samples = store
            .metrics_snapshot()
            .into_iter()
            .find(|metric| metric.operation == "maintenance_reconcile")
            .map(|metric| metric.samples)
            .unwrap_or(0);
        clipboard.shutdown();
        assert_eq!(
            samples, 1,
            "desktop startup scheduled {samples} maintenance passes"
        );
    }
}
