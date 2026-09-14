//! Reject unisolated fixtures; capture disabling exists only in native acceptance builds.
#[cfg(feature = "native-test")]
use echo_engine::{CaptureSettings, ClipboardSink, NormalizedCapture, RecordResult};
use std::path::{Path, PathBuf};

pub(crate) fn validated_root(data: &Path) -> Result<Option<PathBuf>, String> {
    #[cfg(feature = "native-test")]
    if let Some(root) = std::env::var_os("ECHO_NATIVE_TEST_ROOT") {
        return validate(
            data,
            Path::new(&root),
            std::env::var("ECHO_WINDOWS_ACCEPTANCE").as_deref() == Ok("1"),
        )
        .map(Some);
    }
    unisolated_root(data, std::env::var_os("ECHO_NATIVE_TEST_ROOT").is_some())
}

fn unisolated_root(data: &Path, test_requested: bool) -> Result<Option<PathBuf>, String> {
    // Old fixture runners must fail closed rather than start production capture.
    if data.join("synthetic-fixture.json").exists() || test_requested {
        return Err(
            "Synthetic fixtures require an authorized native-test build and isolated test root"
                .into(),
        );
    }
    Ok(None)
}

#[cfg(feature = "native-test")]
fn validate(data: &Path, root: &Path, authorized: bool) -> Result<PathBuf, String> {
    if !authorized {
        return Err("Native test bridge requires explicit acceptance authorization".into());
    }
    let root = root.canonicalize().map_err(|e| e.to_string())?;
    let data = data.canonicalize().map_err(|e| e.to_string())?;
    if data
        != root
            .join("data")
            .canonicalize()
            .map_err(|e| e.to_string())?
        || !data.starts_with(&root)
    {
        return Err("Native test data must be inside the evidence root".into());
    }
    let marker: serde_json::Value = serde_json::from_slice(
        &std::fs::read(data.join("synthetic-fixture.json")).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    if marker["synthetic"] != true || marker["capture_enabled"] != false {
        return Err("Native test bridge only accepts capture-disabled synthetic fixtures".into());
    }
    Ok(root)
}

#[cfg(feature = "native-test")]
pub(crate) struct DisabledCaptureSink;
#[cfg(feature = "native-test")]
impl ClipboardSink for DisabledCaptureSink {
    fn settings(&self) -> Result<CaptureSettings, String> {
        Ok(CaptureSettings {
            history_enabled: false,
            ..CaptureSettings::default()
        })
    }
    fn record(&self, _: NormalizedCapture) -> Result<RecordResult, String> {
        Err("Capture is disabled for isolated native acceptance".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unisolated_fixtures_are_rejected_before_capture() {
        let root = tempfile::tempdir().unwrap();
        assert!(unisolated_root(root.path(), false).unwrap().is_none());
        assert!(unisolated_root(root.path(), true).is_err());
        std::fs::write(root.path().join("synthetic-fixture.json"), "{}").unwrap();
        assert!(unisolated_root(root.path(), false).is_err());
    }

    #[cfg(feature = "native-test")]
    #[test]
    fn isolation_requires_authorization_containment_and_disabled_synthetic_marker() {
        let root = tempfile::tempdir().unwrap();
        let data = root.path().join("data");
        std::fs::create_dir(&data).unwrap();
        let marker = data.join("synthetic-fixture.json");
        assert!(validate(&data, root.path(), true).is_err());
        for invalid in [
            r#"{"synthetic":false,"capture_enabled":false}"#,
            r#"{"synthetic":true,"capture_enabled":true}"#,
        ] {
            std::fs::write(&marker, invalid).unwrap();
            assert!(validate(&data, root.path(), true).is_err());
        }
        std::fs::write(&marker, r#"{"synthetic":true,"capture_enabled":false}"#).unwrap();
        assert!(validate(&data, root.path(), false).is_err());
        let outside = tempfile::tempdir().unwrap();
        assert!(validate(outside.path(), root.path(), true).is_err());
        assert!(validate(&data, root.path(), true).is_ok());
        assert!(!DisabledCaptureSink.settings().unwrap().history_enabled);
    }
}
