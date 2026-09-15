//! Screenshot preview performance fixture. Never overwrites an existing store.
use echo_engine::{
    fingerprint, ClipboardRepresentation, ContentType, NormalizedCapture, SourceContext,
};
use echo_storage::ClipboardStore;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let root = std::path::Path::new(args.get(1).ok_or("output path required")?);
    if root.exists() {
        return Err("output already exists".into());
    }
    let sources = std::path::Path::new(args.get(2).ok_or("source directory required")?);
    let mut paths = std::fs::read_dir(sources)?
        .map(|e| e.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()?;
    paths.sort();
    let mut store = ClipboardStore::open(root)?;
    for (i, path) in paths.iter().enumerate() {
        let bytes = std::fs::read(path)?;
        let rep = ClipboardRepresentation {
            format: "image".into(),
            mime_type: "image/png".into(),
            bytes,
        };
        store.record_capture(NormalizedCapture {
            sequence: i as u64 + 1,
            source: SourceContext::default(),
            content_type: ContentType::Image,
            preview_text: Some(format!("Preview fixture {i:03}")),
            fingerprint: fingerprint(std::slice::from_ref(&rep)),
            representations: vec![rep],
            searchable_text: None,
            sanitized_html: None,
        })?;
    }
    std::fs::write(
        root.join("synthetic-fixture.json"),
        r#"{"synthetic":true,"capture_enabled":false}"#,
    )?;
    Ok(())
}
