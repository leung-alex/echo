//! Passive caret geometry observer host and its content-free transport.
//!
//! The protocol module is also included by the standalone observer DLL using
//! an explicit `#[path]`; keep it std-only and independent from this host.

pub(crate) mod host_ffi;
pub(crate) mod protocol;
pub(crate) mod scheduler;

use crate::ime_observer::{observer_module_digest, observer_module_handle};
use protocol::{
    Header, Mailbox, REQUEST_CLOSE, REQUEST_PROBE, RESPONSE_CLOSED, RESPONSE_PENDING,
    RESPONSE_READY, RESPONSE_UNAVAILABLE,
};
use scheduler::{RequestDecision, SessionState};
use std::{ptr::null_mut, sync::atomic::Ordering};
use windows_sys::Win32::{
    Foundation::{GetLastError, HWND, LPARAM, LRESULT, WPARAM},
    System::LibraryLoader::GetProcAddress,
    UI::WindowsAndMessaging::{RegisterWindowMessageW, HHOOK, WM_APP},
};

pub const MESSAGE: &str = "Echo.CaretObservation.v1";
pub const PREFIX: &str = "Local\\Echo.CaretObservation.";
pub const REQUEST_MESSAGE: u32 = WM_APP + 0x5a1;
pub const CLOSE_MESSAGE: u32 = WM_APP + 0x5a2;
pub const BOOTSTRAP_TIMEOUT_MS: u32 = 50;
pub const MIN_REQUEST_INTERVAL_MS: u64 = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CaretEndpoint {
    pub root_window: isize,
    pub root_process: u32,
    pub root_started: u64,
    pub input_window: isize,
    pub input_process: u32,
    pub input_started: u64,
    pub input_thread: u32,
    pub focus_generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReplyStatus {
    Pending,
    Ready,
    Unavailable,
    Closed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeometryReply {
    pub status: ReplyStatus,
    pub request_sequence: u32,
    pub response_words: [u32; 64],
    /// Whether the response completed the host's current request identity.
    /// Unavailable responses remain observable for diagnostics even when an
    /// epoch change intentionally invalidated the request.
    pub accepted: bool,
}

pub struct CaretObserver {
    endpoint: CaretEndpoint,
    header: Header,
    mailbox: *mut Mailbox,
    mapping: host_ffi::Mapping,
    atom: u16,
    bootstrap_message: u32,
    scheduler_window: HWND,
    hook: HHOOK,
    session: SessionState,
    last_request_tick: Option<u64>,
    retired: bool,
}

unsafe impl Send for CaretObserver {}

fn response_matches_session(status: ReplyStatus, sequence: u32, pending: Option<u32>) -> bool {
    status == ReplyStatus::Closed || pending == Some(sequence)
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn nonce() -> ([u32; 4], String) {
    unsafe {
        let guid = windows::Win32::System::Com::CoCreateGuid().unwrap_or_default();
        (
            [
                guid.data1,
                u32::from(guid.data2) << 16 | u32::from(guid.data3),
                u32::from_be_bytes(guid.data4[..4].try_into().unwrap_or([0; 4])),
                u32::from_be_bytes(guid.data4[4..].try_into().unwrap_or([0; 4])),
            ],
            format!("{guid:?}"),
        )
    }
}

fn callback() -> Result<unsafe extern "system" fn(i32, WPARAM, LPARAM) -> LRESULT, String> {
    let module = observer_module_handle()?;
    let address =
        unsafe { GetProcAddress(module as _, c"EchoCompositionObserver".as_ptr().cast()) }
            .ok_or("caret observer entry point unavailable")?;
    Ok(unsafe { std::mem::transmute(address) })
}

fn blocked_cross_process_target(endpoint: CaretEndpoint) -> Option<String> {
    let path = crate::windows_impl::process_path(endpoint.input_process)?;
    // ChatGPT.exe is the Windows Codex desktop host. Its WebView2 and TSF
    // threads are not a safe target for a third-party WH_CALLWNDPROC module:
    // a settings navigation can tear down the input stack while the hook is
    // still in flight. Keep the passive indicator on its legacy path for this
    // host instead of injecting the observer DLL into it.
    crate::windows_impl::blocked_cross_process_process(&path)
}

impl CaretObserver {
    pub fn start(endpoint: CaretEndpoint) -> Result<Self, String> {
        if endpoint.root_window == 0
            || endpoint.input_window == 0
            || endpoint.root_process == 0
            || endpoint.input_process == 0
            || endpoint.input_thread == 0
            || endpoint.focus_generation == 0
        {
            return Err("caret endpoint identity unavailable".into());
        }
        if let Some(process_name) = blocked_cross_process_target(endpoint) {
            return Err(format!(
                "target identity is not approved for caret observer: {process_name}"
            ));
        }
        let (nonce_words, nonce_text) = nonce();
        let mapping_name = wide(&format!("{PREFIX}{nonce_text}"));
        let mapping = unsafe { host_ffi::Mapping::create(mapping_name.as_ptr()) }?;
        let mut header = Header {
            magic: protocol::MAGIC,
            version: protocol::VERSION,
            byte_size: 0,
            nonce: nonce_words,
            echo_pid: std::process::id(),
            target_pid: endpoint.input_process,
            input_thread: endpoint.input_thread,
            root_pid: endpoint.root_process,
            echo_started: crate::windows_impl::process_started_at(std::process::id()).unwrap_or(0),
            target_started: endpoint.input_started,
            root_started: endpoint.root_started,
            root_hwnd: endpoint.root_window as usize as u64,
            input_hwnd: endpoint.input_window as usize as u64,
            focus_generation: endpoint.focus_generation,
            dll_digest: observer_module_digest(),
        };
        if header.echo_started == 0 {
            header.echo_started = 1;
        }
        let mailbox = mapping.view.Value.cast::<Mailbox>();
        unsafe {
            std::ptr::write(mailbox, Mailbox::new(header));
        }
        let expected_header = header.for_mailbox();
        let atom_name = wide(&format!("{PREFIX}{nonce_text}"));
        let atom = unsafe { host_ffi::add_atom(atom_name.as_ptr()) };
        if atom == 0 {
            return Err(format!("caret atom registration failed={}", unsafe {
                GetLastError()
            }));
        }
        let bootstrap_message = unsafe { RegisterWindowMessageW(wide(MESSAGE).as_ptr()) };
        if bootstrap_message == 0 {
            unsafe { host_ffi::delete_atom(atom) };
            return Err("caret bootstrap message registration failed".into());
        }
        let hook = match callback().and_then(|callback| unsafe {
            host_ffi::install_hook(callback, endpoint.input_thread, observer_module_handle()?)
        }) {
            Ok(hook) => hook,
            Err(error) => {
                unsafe { host_ffi::delete_atom(atom) };
                return Err(error);
            }
        };
        let mut observer = Self {
            endpoint,
            header: expected_header,
            mailbox,
            mapping,
            atom,
            bootstrap_message,
            scheduler_window: null_mut(),
            hook,
            session: SessionState::new(endpoint.focus_generation),
            last_request_tick: None,
            retired: false,
        };
        let bootstrap = unsafe {
            host_ffi::bootstrap(
                endpoint.input_window as HWND,
                bootstrap_message,
                atom,
                BOOTSTRAP_TIMEOUT_MS,
            )
        };
        if let Err(error) = bootstrap {
            observer.close();
            return Err(error);
        }
        observer.scheduler_window = unsafe {
            host_ffi::u64_as_hwnd((*observer.mailbox).scheduler_hwnd.load(Ordering::Acquire))
        };
        if observer.scheduler_window.is_null() {
            let detail = unsafe {
                (*observer.mailbox)
                    .response
                    .snapshot()
                    .map(|words| format!("status={} reason={}", words[0], words[4]))
                    .unwrap_or_else(|| "response=unavailable".into())
            };
            observer.close();
            return Err(format!("caret scheduler was not acknowledged ({detail})"));
        }
        Ok(observer)
    }

    pub fn try_request(&mut self, now_tick_ms: u64, reason: u32) -> RequestDecision {
        if self
            .last_request_tick
            .is_some_and(|last| now_tick_ms.saturating_sub(last) < MIN_REQUEST_INTERVAL_MS)
        {
            return RequestDecision::Backoff;
        }
        let decision = self.session.request(now_tick_ms);
        let RequestDecision::Sent(sequence) = decision else {
            if matches!(
                decision,
                RequestDecision::Closed | RequestDecision::Unavailable
            ) {
                self.close();
            }
            return decision;
        };
        let mut words = [0_u32; 16];
        words[0] = REQUEST_PROBE;
        words[1] = sequence;
        words[2] = now_tick_ms.saturating_add(SessionState::DEADLINE_MS) as u32;
        words[3] = (now_tick_ms.saturating_add(SessionState::DEADLINE_MS) >> 32) as u32;
        words[4] = reason;
        words[5] = 1; // sensitivity is admitted by the host before scheduling.
        words[12] = self.session.context_epoch as u32;
        words[13] = (self.session.context_epoch >> 32) as u32;
        if unsafe { (*self.mailbox).request.publish(&words) }.is_err() {
            self.close();
            return RequestDecision::Unavailable;
        }
        let posted = unsafe {
            host_ffi::post_scheduler(self.scheduler_window, REQUEST_MESSAGE, sequence as usize)
        };
        if !posted {
            self.close();
            return RequestDecision::Unavailable;
        }
        self.last_request_tick = Some(now_tick_ms);
        decision
    }

    pub fn try_read(&mut self, now_tick_ms: u64) -> Option<GeometryReply> {
        self.session.expire(now_tick_ms);
        let header = unsafe { (*self.mailbox).header };
        if !protocol::header_shape_valid(&header)
            || !protocol::header_identity_matches(&self.header, &header)
        {
            return None;
        }
        let words = unsafe { (*self.mailbox).response.snapshot() }?;
        let status = match words[0] {
            RESPONSE_PENDING => ReplyStatus::Pending,
            RESPONSE_READY => ReplyStatus::Ready,
            RESPONSE_UNAVAILABLE => ReplyStatus::Unavailable,
            RESPONSE_CLOSED => ReplyStatus::Closed,
            _ => return None,
        };
        let sequence = words[1];
        if !response_matches_session(status, sequence, self.session.pending_sequence) {
            return None;
        }
        if status == ReplyStatus::Pending {
            return None;
        }
        if status == ReplyStatus::Closed {
            // CLOSED is a terminal session state, but the target may still
            // own asynchronous COM callback references.  The target updates
            // this same CLOSED snapshot after each final Release; synchronize
            // the host-side count before retiring the observer so diagnostics
            // can prove the real drain.
            self.session.sync_outstanding_callbacks(words[30]);
            self.session.mark_target_closed();
            return Some(GeometryReply {
                status,
                request_sequence: sequence,
                response_words: words,
                accepted: false,
            });
        }
        let epoch = u64::from(words[10]) | (u64::from(words[11]) << 32);
        let epoch_changed = epoch != 0 && epoch != self.session.context_epoch;
        if epoch_changed {
            let _ = self.session.observe_context_epoch(epoch);
        }
        let accepted =
            self.session
                .complete(sequence, self.endpoint.focus_generation, epoch, now_tick_ms);
        // word 30 is written by the target's actual EditSession Release path;
        // the reader must not infer a COM Release from response arrival.
        self.session.sync_outstanding_callbacks(words[30]);
        (accepted || status == ReplyStatus::Unavailable).then_some(GeometryReply {
            status,
            request_sequence: sequence,
            response_words: words,
            accepted: accepted && !epoch_changed,
        })
    }

    pub fn heartbeat(&self, now_tick_ms: u64) {
        unsafe {
            (*self.mailbox)
                .heartbeat_tick
                .store(now_tick_ms, Ordering::Release);
        }
    }

    pub fn close(&mut self) {
        if self.retired {
            return;
        }
        self.retired = true;
        self.session.close();
        unsafe {
            (*self.mailbox).closed.store(1, Ordering::Release);
            let mut words = [0_u32; 16];
            words[0] = REQUEST_CLOSE;
            let _ = (*self.mailbox).request.publish(&words);
            if !self.scheduler_window.is_null() {
                let _ = host_ffi::post_scheduler(self.scheduler_window, CLOSE_MESSAGE, 0);
            }
        }
    }

    pub fn endpoint(&self) -> CaretEndpoint {
        self.endpoint
    }

    pub fn rebind_focus_generation(&mut self, generation: u64) {
        self.endpoint.focus_generation = generation;
        self.session.rebind_generation(generation);
    }
}

impl Drop for CaretObserver {
    fn drop(&mut self) {
        self.close();
        unsafe {
            host_ffi::remove_hook(self.hook);
            host_ffi::delete_atom(self.atom);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_response_is_terminal_without_matching_pending_sequence() {
        assert!(response_matches_session(ReplyStatus::Closed, 0, Some(7)));
        assert!(response_matches_session(ReplyStatus::Closed, 0, None));
        assert!(!response_matches_session(ReplyStatus::Ready, 0, Some(7)));
    }
}
