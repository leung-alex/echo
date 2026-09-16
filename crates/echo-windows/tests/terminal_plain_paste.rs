#![cfg(windows)]

use echo_engine::{
    ClipboardPlatform, ClipboardRepresentation, PasteControlIdentity, PasteDelivery,
};
use echo_windows::{
    focus::FocusSnapshot,
    inline::{InlineController, InlineEvent},
    WindowsPlatform,
};
use std::{
    os::windows::process::CommandExt,
    path::PathBuf,
    process::{Child, Command},
    sync::{mpsc, Arc},
    thread,
    time::{Duration, Instant},
};

struct OwnedConsole(Child);
fn activate_owned_console(title: &str) -> isize {
    use windows::{
        core::PCWSTR,
        Win32::{
            Foundation::{POINT, RECT},
            UI::{
                Input::KeyboardAndMouse::{
                    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN,
                    MOUSEEVENTF_LEFTUP, MOUSEINPUT,
                },
                WindowsAndMessaging::{
                    FindWindowW, GetAncestor, GetCursorPos, GetForegroundWindow, GetWindowRect,
                    SetCursorPos, SetForegroundWindow, SetWindowPos, ShowWindow, WindowFromPoint,
                    GA_ROOT, HWND_TOP, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_RESTORE,
                },
            },
        },
    };
    let title: Vec<_> = title.encode_utf16().chain(Some(0)).collect();
    let class: Vec<_> = "ConsoleWindowClass".encode_utf16().chain(Some(0)).collect();
    unsafe {
        if let Ok(hwnd) = FindWindowW(PCWSTR(class.as_ptr()), PCWSTR(title.as_ptr())) {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
            if GetForegroundWindow() != hwnd {
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOP),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
                let mut rect = RECT::default();
                if GetWindowRect(hwnd, &mut rect).is_ok() {
                    let point = POINT {
                        x: rect.left + 100,
                        y: rect.top + 12,
                    };
                    // Click only this uniquely titled owned console's unobscured title bar.
                    if GetAncestor(WindowFromPoint(point), GA_ROOT) == hwnd {
                        let mut previous = POINT::default();
                        let _ = GetCursorPos(&mut previous);
                        let _ = SetCursorPos(point.x, point.y);
                        let events =
                            [MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP].map(|flags| INPUT {
                                r#type: INPUT_MOUSE,
                                Anonymous: INPUT_0 {
                                    mi: MOUSEINPUT {
                                        dwFlags: flags,
                                        ..Default::default()
                                    },
                                },
                            });
                        SendInput(&events, std::mem::size_of::<INPUT>() as i32);
                        let _ = SetCursorPos(previous.x, previous.y);
                    }
                }
            }
            return hwnd.0 as isize;
        }
    }
    0
}
impl Drop for OwnedConsole {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

#[test]
#[ignore = "owned visible consoles and preserved clipboard; explicit native acceptance only"]
fn cmd_and_powershell_receive_plain_paste_without_executing_the_payload() {
    assert_eq!(std::env::var("ECHO_WINDOWS_ACCEPTANCE").as_deref(), Ok("1"));
    assert_eq!(
        std::env::var("ECHO_CLIPBOARD_BACKUP_READY").as_deref(),
        Ok("1")
    );
    let root = PathBuf::from(std::env::var("ECHO_DATA_DIR").expect("isolated evidence path"));
    std::fs::create_dir_all(&root).unwrap();
    let platform = WindowsPlatform::new();
    for shell in ["cmd", "pwsh"] {
        let title = format!(
            "Echo isolated terminal fixture {} {shell}",
            root.file_name().unwrap().to_string_lossy()
        );
        let output = root.join(format!("{shell}-received.txt"));
        assert!(!output.exists(), "use a fresh evidence directory");
        let mut command = if shell == "cmd" {
            let script = root.join("read-console.cmd");
            std::fs::write(&script, format!("@echo off\r\ntitle {title}\r\nset /p echo_fixture_line=<CON\r\n>\"{}\" echo %echo_fixture_line%\r\n", output.display())).unwrap();
            let mut c = Command::new("cmd.exe");
            c.args(["/d", "/q", "/c"]).arg(script);
            c
        } else {
            let mut c = Command::new("pwsh.exe");
            // ReadLine consumes the synthetic paste as data, never as a command.
            c.args(["-NoProfile", "-Command", &format!("[Console]::Title='{title}'; $inputStream=[IO.File]::Open('CONIN$',[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::ReadWrite); $reader=[IO.StreamReader]::new($inputStream); $line=$reader.ReadLine(); [IO.File]::WriteAllText('{}',$line)", output.display().to_string().replace('\'', "''"))]);
            c
        };
        let fixture = OwnedConsole(command.creation_flags(0x10).spawn().unwrap());
        println!("owned console pid={} title={title}", fixture.0.id());
        let deadline = Instant::now() + Duration::from_secs(45);
        let snapshot = loop {
            let owned_window = activate_owned_console(&title);
            let snapshot = FocusSnapshot::capture();
            if owned_window != 0 && snapshot.window_id == owned_window {
                break snapshot;
            }
            assert!(
                Instant::now() < deadline,
                "owned {shell} console pid={} did not gain foreground: {snapshot:?}",
                fixture.0.id()
            );
            thread::sleep(Duration::from_millis(50));
        };
        let (send, events) = mpsc::channel();
        let inline = InlineController::start(Arc::new(move |event| {
            let _ = send.send(event);
        }))
        .unwrap();
        inline.begin(1, snapshot).unwrap();
        let deadline = Instant::now() + Duration::from_secs(4);
        let target = loop {
            match events.recv_timeout(Duration::from_millis(100)) {
                Ok(InlineEvent::Unavailable {
                    captured_target: Some(target),
                    ..
                }) => break target,
                Ok(
                    InlineEvent::Unavailable { reason, .. }
                    | InlineEvent::Compatibility { reason, .. },
                ) => panic!("terminal rejected: {reason}"),
                _ => assert!(
                    Instant::now() < deadline,
                    "terminal compatibility target missing"
                ),
            }
        };
        assert!(matches!(
            target.focused_control,
            Some(PasteControlIdentity::PlainPasteWindow { .. })
        ));
        inline.cancel(1);
        drop(inline);
        let mut wrong = target.clone();
        wrong.process_started_at += 1;
        assert!(matches!(
            platform.paste_to_target(&wrong).unwrap(),
            PasteDelivery::Failed(_)
        ));
        platform.adopt_captured_target(Some(target.clone()));
        assert!(platform.paste_preflight(&target).unwrap().is_none());
        platform
            .write_clipboard(&[ClipboardRepresentation {
                format: "text".into(),
                mime_type: "text/plain".into(),
                bytes: b"ECHO_TERMINAL_FIXTURE\r\n".to_vec(),
            }])
            .unwrap();
        assert_eq!(
            platform.paste_to_target(&target).unwrap(),
            PasteDelivery::Pasted
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        while !output.exists() {
            assert!(
                Instant::now() < deadline,
                "{shell} did not receive the pasted line"
            );
            thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(
            std::fs::read_to_string(output).unwrap().trim(),
            "ECHO_TERMINAL_FIXTURE"
        );
        println!("PASS {shell}: original console received exactly the synthetic payload");
    }
}
