//! Bounded production-adapter probe, only for an owned synthetic test input.
use std::sync::Arc;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::var("ECHO_WINDOWS_ACCEPTANCE").as_deref() != Ok("1") {
        return Err("Authorization required".into());
    }
    let mut args = std::env::args().skip(1);
    let root = std::path::PathBuf::from(args.next().ok_or("Missing evidence root")?);
    let pid: u32 = args.next().ok_or("Missing owned pid")?.parse()?;
    let marker: serde_json::Value =
        serde_json::from_slice(&std::fs::read(root.join("data/synthetic-fixture.json"))?)?;
    if marker["synthetic"] != true || marker["capture_enabled"] != false {
        return Err("Synthetic input required".into());
    }
    let records: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(root.join("owned-processes.json"))?)?;
    let snapshot = echo_windows::focus::FocusSnapshot::capture();
    if snapshot.process_id != pid
        || !records.iter().any(|r| {
            r["pid"].as_u64() == Some(pid as u64)
                && r["started_filetime"].as_u64() == Some(snapshot.process_started_at)
        })
    {
        return Err("Owned input identity mismatch".into());
    }
    let control = echo_windows::inline::InlineController::start(Arc::new(|event| {
        use echo_windows::inline::InlineEvent::*;
        match event {
            Started(s) => println!("START {:?}", s.query),
            Changed { query, .. } => println!("QUERY {:?}", query),
            Cancelled { reason, .. } => println!("CANCEL {reason}"),
            _ => println!("STATE"),
        }
    }))?;
    control.begin(1, snapshot)?;
    std::fs::write(root.join("inline-probe.ready"), "ready")?;
    let start = std::time::Instant::now();
    while !root.join("inline-probe.stop").exists()
        && start.elapsed() < std::time::Duration::from_secs(20)
    {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    control.cancel(1);
    Ok(())
}
