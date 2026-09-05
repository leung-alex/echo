//! Local, user-authenticated activation transport. No TCP listener or polling loop.
use super::{
    common::{error, process_sid, wide, Handle, Security},
    EventHandler, ShellEvent,
};
use std::{
    ptr::{null, null_mut},
    sync::Arc,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Storage::FileSystem::*,
    System::{Pipes::*, Threading::*, IO::*},
    UI::WindowsAndMessaging::AllowSetForegroundWindow,
};
const MAX_MESSAGE: usize = 64 * 1024;
const DEADLINE_MS: u32 = 3000;

fn validate_args(args: &[String]) -> bool {
    if args.len() > 2 || args.iter().any(|s| s.len() > MAX_MESSAGE) {
        return false;
    }
    match args {
        [] => true,
        [flag] => matches!(
            flag.as_str(),
            "--background" | "--quit" | "--favorites" | "--settings" | "--history"
        ),
        [flag, value] => flag == "--echo-activate" && !value.is_empty(),
        _ => false,
    }
}
fn decode(bytes: &[u8]) -> Result<Vec<String>, String> {
    if bytes.is_empty() || bytes.len() > MAX_MESSAGE {
        return Err("invalid activation size".into());
    }
    let args: Vec<String> =
        serde_json::from_slice(bytes).map_err(|_| "invalid activation message")?;
    if !validate_args(&args) {
        return Err("invalid activation arguments".into());
    }
    Ok(args)
}
unsafe fn pending(
    handle: HANDLE,
    op: &mut OVERLAPPED,
    stop: HANDLE,
    timeout: u32,
) -> Result<u32, String> {
    let handles = [op.hEvent, stop];
    let wait = WaitForMultipleObjects(2, handles.as_ptr(), 0, timeout);
    let mut count = 0;
    if wait == WAIT_OBJECT_0 {
        if GetOverlappedResult(handle, op, &mut count, 0) != 0 {
            return Ok(count);
        }
        return Err(error());
    }
    // The buffer and OVERLAPPED must outlive cancellation completion.
    CancelIoEx(handle, op);
    GetOverlappedResult(handle, op, &mut count, 1);
    if wait == WAIT_TIMEOUT {
        Err("activation timed out".into())
    } else {
        Err("activation cancelled".into())
    }
}
unsafe fn connect(pipe: HANDLE, stop: HANDLE) -> Result<(), String> {
    let event = Handle::new(CreateEventW(null(), 1, 0, null()))?;
    let mut op: OVERLAPPED = std::mem::zeroed();
    op.hEvent = event.0;
    if ConnectNamedPipe(pipe, &mut op) != 0 {
        return Ok(());
    }
    match GetLastError() {
        ERROR_PIPE_CONNECTED => Ok(()),
        ERROR_IO_PENDING => pending(pipe, &mut op, stop, INFINITE).map(|_| ()),
        _ => Err(error()),
    }
}
unsafe fn transfer(
    pipe: HANDLE,
    stop: HANDLE,
    buffer: &mut [u8],
    write: bool,
) -> Result<usize, String> {
    let event = Handle::new(CreateEventW(null(), 1, 0, null()))?;
    let mut op: OVERLAPPED = std::mem::zeroed();
    op.hEvent = event.0;
    let mut n = 0;
    let result = if write {
        WriteFile(pipe, buffer.as_ptr(), buffer.len() as u32, &mut n, &mut op)
    } else {
        ReadFile(
            pipe,
            buffer.as_mut_ptr(),
            buffer.len() as u32,
            &mut n,
            &mut op,
        )
    };
    if result != 0 {
        return Ok(n as usize);
    }
    if GetLastError() != ERROR_IO_PENDING {
        return Err(error());
    }
    pending(pipe, &mut op, stop, DEADLINE_MS).map(|n| n as usize)
}
pub(super) fn serve(name: String, sid: String, stop: Arc<Handle>, handler: EventHandler) {
    let security = match Security::for_user(&sid) {
        Ok(x) => x,
        Err(e) => {
            handler(ShellEvent::Error(e));
            return;
        }
    };
    let name = wide(&name);
    loop {
        unsafe {
            if WaitForSingleObject(stop.0, 0) == WAIT_OBJECT_0 {
                return;
            }
            let pipe = match Handle::new(CreateNamedPipeW(
                name.as_ptr(),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                MAX_MESSAGE as u32,
                MAX_MESSAGE as u32,
                DEADLINE_MS,
                &security.attributes(),
            )) {
                Ok(p) => p,
                Err(e) => {
                    handler(ShellEvent::Error(format!("Activation pipe: {e}")));
                    return;
                }
            };
            if connect(pipe.0, stop.0).is_err() {
                return;
            }
            let mut client = 0;
            if GetNamedPipeClientProcessId(pipe.0, &mut client) == 0
                || process_sid(client).ok().as_deref() != Some(sid.as_str())
            {
                DisconnectNamedPipe(pipe.0);
                continue;
            }
            let mut data = vec![0_u8; MAX_MESSAGE];
            if let Ok(n) = transfer(pipe.0, stop.0, &mut data, false) {
                if let Ok(args) = decode(&data[..n]) {
                    handler(ShellEvent::Activation(args));
                    let mut ack = [1_u8];
                    if transfer(pipe.0, stop.0, &mut ack, true).is_ok() {
                        let _ = transfer(pipe.0, stop.0, &mut ack, false);
                    }
                }
            }
            DisconnectNamedPipe(pipe.0);
        }
    }
}
pub(super) fn forward(name: &str, args: &[String], sid: &str) -> Result<(), String> {
    if !validate_args(args) {
        return Err("invalid activation arguments".into());
    }
    let mut data = serde_json::to_vec(args).map_err(|e| e.to_string())?;
    if data.len() > MAX_MESSAGE {
        return Err("activation too large".into());
    }
    let name = wide(name);
    let until = Instant::now() + Duration::from_millis(u64::from(DEADLINE_MS));
    unsafe {
        let pipe = loop {
            let raw = CreateFileW(
                name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                null(),
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                null_mut(),
            );
            if raw != INVALID_HANDLE_VALUE {
                break Handle::new(raw)?;
            }
            if Instant::now() >= until {
                return Err("resident Echo did not accept activation within 3 seconds".into());
            }
            // Bounded startup handoff retry, not a resident background timer.
            WaitNamedPipeW(name.as_ptr(), 50);
            std::thread::sleep(Duration::from_millis(10));
        };
        let mut server = 0;
        if GetNamedPipeServerProcessId(pipe.0, &mut server) == 0
            || process_sid(server).ok().as_deref() != Some(sid)
        {
            return Err("activation server has a different user identity".into());
        }
        AllowSetForegroundWindow(server);
        let mode = PIPE_READMODE_MESSAGE;
        if SetNamedPipeHandleState(pipe.0, &mode, null(), null()) == 0 {
            return Err(error());
        }
        let stop = Handle::new(CreateEventW(null(), 1, 0, null()))?;
        if transfer(pipe.0, stop.0, &mut data, true)? != data.len() {
            return Err("activation write was incomplete".into());
        }
        let mut ack = [0_u8];
        if transfer(pipe.0, stop.0, &mut ack, false)? != 1 || ack[0] != 1 {
            return Err("activation was not acknowledged".into());
        }
        let _ = transfer(pipe.0, stop.0, &mut ack, true);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn activation_protocol_is_bounded_and_strict() {
        assert!(decode(b"[]").is_ok());
        assert!(decode(br#"["--favorites"]"#).is_ok());
        for data in [
            b"null".as_slice(),
            b"{}",
            br#"["--shell","rm"]"#,
            br#"["--echo-activate",""]"#,
        ] {
            assert!(decode(data).is_err());
        }
        assert!(decode(&vec![b' '; MAX_MESSAGE + 1]).is_err());
    }
}
