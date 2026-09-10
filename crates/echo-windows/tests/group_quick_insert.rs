#![cfg(windows)]

use echo_engine::{ClipboardPlatform, ClipboardRepresentation, PasteDelivery};
use echo_windows::inline::{InlineController, InlineEvent};
use echo_windows::{focus::FocusSnapshot, WindowsPlatform};
use std::sync::{mpsc, Arc};

fn query_event(events: &mpsc::Receiver<InlineEvent>, expected: &str) -> echo_engine::InlineTicket {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(5) {
        match events.recv_timeout(Duration::from_millis(100)) {
            Ok(InlineEvent::Started(value)) if value.query == expected => return value.ticket,
            Ok(InlineEvent::Changed { ticket, query, .. }) if query == expected => return ticket,
            Ok(
                InlineEvent::Unavailable { reason, .. } | InlineEvent::Compatibility { reason, .. },
            ) => panic!("inline input rejected: {reason}"),
            _ => {}
        }
    }
    panic!("inline query did not become {expected:?}");
}

fn expect_event(events: &mpsc::Receiver<InlineEvent>, accept: impl Fn(&InlineEvent) -> bool) {
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(2) {
        if let Ok(event) = events.recv_timeout(Duration::from_millis(100)) {
            if accept(&event) {
                return;
            }
            assert!(
                matches!(
                    event,
                    InlineEvent::Notice { .. } | InlineEvent::Changed { .. }
                ),
                "unexpected inline transition"
            );
        }
    }
    panic!("expected inline event did not arrive");
}
use std::{
    path::PathBuf,
    process::Command,
    thread,
    time::{Duration, Instant},
};

struct Fixture {
    root: PathBuf,
    child: std::process::Child,
}
impl Fixture {
    fn wait(&self, name: &str) -> String {
        let start = Instant::now();
        loop {
            if let Ok(value) = std::fs::read_to_string(self.root.join(name)) {
                if !value.is_empty() {
                    return value;
                }
            }
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "fixture timeout: {name}"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
    fn command(&self, value: &str) -> String {
        let _ = std::fs::remove_file(self.root.join("response"));
        std::fs::write(self.root.join("command.tmp"), value).unwrap();
        std::fs::rename(self.root.join("command.tmp"), self.root.join("command")).unwrap();
        self.wait("response")
    }
    fn capture(&self) -> Option<echo_engine::PasteTarget> {
        let snapshot = FocusSnapshot::capture();
        assert_eq!(
            snapshot.process_id,
            self.child.id(),
            "fixture must own foreground"
        );
        snapshot.capture_target().target
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::write(self.root.join("command"), "stop");
        let start = Instant::now();
        while self.child.try_wait().ok().flatten().is_none()
            && start.elapsed() < Duration::from_secs(5)
        {
            thread::sleep(Duration::from_millis(20));
        }
    }
}

// Run under Invoke-WithClipboardBackup.ps1 with a freshly compiled owned fixture.
#[test]
#[ignore = "requires authorized isolated Windows foreground and clipboard acceptance"]
fn authorized_group_quick_insert() {
    assert_eq!(std::env::var("ECHO_WINDOWS_ACCEPTANCE").as_deref(), Ok("1"));
    assert_eq!(
        std::env::var("ECHO_CLIPBOARD_BACKUP_READY").as_deref(),
        Ok("1")
    );
    let exe = std::env::var("ECHO_GROUP_FIXTURE_EXE").expect("compiled synthetic Group fixture");
    let root = PathBuf::from(std::env::var("ECHO_DATA_DIR").expect("isolated synthetic directory"));
    std::fs::create_dir_all(&root).unwrap();
    assert!(
        !root.join("ready").exists(),
        "use a fresh evidence directory"
    );
    let child = Command::new(exe).arg(&root).spawn().unwrap();
    let fixture = Fixture { root, child };
    fixture.wait("ready");
    // Windows may deny background activation. Allow the owned fixture to be
    // brought forward before testing; never inject into the previous foreground.
    let foreground_deadline = Instant::now() + Duration::from_secs(15);
    while FocusSnapshot::capture().process_id != fixture.child.id() {
        assert!(
            Instant::now() < foreground_deadline,
            "activate the synthetic fixture before acceptance"
        );
        thread::sleep(Duration::from_millis(50));
    }
    echo_windows::focus::warm_accessibility();
    thread::sleep(Duration::from_millis(200));
    let platform = WindowsPlatform::new();
    for role in ["group", "edit", "document"] {
        fixture.command(role);
        let target = fixture
            .capture()
            .expect("writable TextPattern input must be captured");
        assert!(platform.paste_preflight(&target).unwrap().is_none());
        platform
            .write_clipboard(&[ClipboardRepresentation {
                format: "text".into(),
                mime_type: "text/plain".into(),
                bytes: b"echo-group-synthetic".to_vec(),
            }])
            .unwrap();
        assert_eq!(
            platform.paste_to_target(&target).unwrap(),
            PasteDelivery::Pasted
        );
        thread::sleep(Duration::from_millis(100));
        assert_eq!(
            fixture.command("read"),
            "prefix echo-group-synthetic",
            "exactly one insertion"
        );
    }
    for role in ["readonly", "no-text", "blur"] {
        fixture.command("group");
        fixture.command(role);
        assert!(fixture.capture().is_none(), "unsafe input accepted: {role}");
        assert_eq!(fixture.command("read"), "prefix ");
    }
    fixture.command("inline");
    let (send, events) = mpsc::channel();
    let inline = InlineController::start(Arc::new(move |event| {
        let _ = send.send(event);
    }))
    .unwrap();
    inline.begin(71, FocusSnapshot::capture()).unwrap();
    query_event(&events, "");
    assert_eq!(fixture.command("focused"), "True");
    fixture.command("type:ec");
    query_event(&events, "ec");
    fixture.command("type:ho");
    query_event(&events, "echo");
    fixture.command("key:8");
    let ticket = query_event(&events, "ech");
    inline.results_ready(ticket, true);
    let start = Instant::now();
    while !inline.can_confirm(ticket) {
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "inline confirmation never became ready"
        );
        thread::sleep(Duration::from_millis(20));
    }
    fixture.command("key:40");
    expect_event(&events, |event| {
        matches!(event, InlineEvent::Navigate { delta: 1, .. })
    });
    fixture.command("key:13");
    expect_event(&events, |event| matches!(event, InlineEvent::Confirm(_)));
    let payload = [ClipboardRepresentation {
        format: "text".into(),
        mime_type: "text/plain".into(),
        bytes: b"replacement".to_vec(),
    }];
    inline.preflight(ticket, &payload).unwrap();
    let sequence = platform.write_clipboard(&payload).unwrap();
    assert_eq!(
        inline.paste(ticket, sequence).unwrap(),
        PasteDelivery::Pasted
    );
    assert_eq!(fixture.command("read"), "prefix replacement suffix");
    inline.cancel(71);

    fixture.command("inline");
    inline.begin(72, FocusSnapshot::capture()).unwrap();
    query_event(&events, "");
    fixture.command("type:keep");
    let no_results = query_event(&events, "keep");
    inline.results_ready(no_results, false);
    fixture.command("key:13");
    thread::sleep(Duration::from_millis(100));
    assert_eq!(
        fixture.command("read"),
        "prefix keep suffix",
        "no-result Enter must not reach the host"
    );
    fixture.command("key:27");
    expect_event(&events, |event| {
        matches!(event, InlineEvent::Cancelled { .. })
    });
    inline.cancel(72);
    assert_eq!(fixture.command("read"), "prefix keep suffix");
    fixture.command("inline");
    inline.begin(73, FocusSnapshot::capture()).unwrap();
    query_event(&events, "");
    fixture.command("type:query");
    let previous = query_event(&events, "query");
    fixture.command("outside");
    thread::sleep(Duration::from_millis(150));
    assert!(
        !inline.can_confirm(previous),
        "selection outside the query must revoke insertion"
    );
    assert!(inline.preflight(previous, &payload).is_err());
    inline.cancel(73);
    assert_eq!(fixture.command("read"), "prefix query suffix");
    drop(inline);
    fixture.command("group");
    let target = fixture.capture().unwrap();
    fixture.command("group"); // Same HWND, new UIA runtime identity.
    assert!(matches!(
        platform.paste_to_target(&target).unwrap(),
        PasteDelivery::Failed(_)
    ));
    assert_eq!(fixture.command("read"), "prefix ");
    fixture.command("group");
    let target = fixture.capture().unwrap();
    std::fs::write(fixture.root.join("command"), "stop").unwrap();
    thread::sleep(Duration::from_millis(200));
    assert!(matches!(
        platform.paste_to_target(&target).unwrap(),
        PasteDelivery::Failed(_)
    ));
}
