//! Same-bitness, session-owned IMM/TSF observation on the verified editor thread.
//! The embedded module has no keyboard hook and no edit/IME mutation operation.
mod protocol;
use protocol::*;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::windows::fs::OpenOptionsExt,
    ptr::{null, null_mut},
    sync::{atomic::Ordering, OnceLock},
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::*,
    System::{DataExchange::*, LibraryLoader::*, Memory::*, Threading::*},
    UI::WindowsAndMessaging::*,
};

const DLL: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/echo_ime_observer.dll"));
// Keep the verified artifact open without write/delete sharing and its module
// loaded for the process lifetime, including any retiring hook callbacks.
static MODULE: OnceLock<Result<(isize, File), String>> = OnceLock::new();
fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
fn module() -> Result<isize, String> {
    MODULE
        .get_or_init(|| {
            let hash = format!("{:x}", Sha256::digest(DLL));
            let directory = std::path::PathBuf::from(
                std::env::var_os("LOCALAPPDATA").ok_or("Local app cache unavailable")?,
            )
            .join("Echo")
            .join("ime-observer")
            .join(hash);
            std::fs::create_dir_all(&directory).map_err(|_| "IME observer cache unavailable")?;
            let path = directory.join("echo_ime_observer.dll");
            match OpenOptions::new()
                .write(true)
                .create_new(true)
                .share_mode(0)
                .open(&path)
            {
                Ok(mut file) => file
                    .write_all(DLL)
                    .map_err(|_| "IME observer cache write failed")?,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(_) => return Err("IME observer cache create failed".into()),
            }
            let mut file = OpenOptions::new()
                .read(true)
                .share_mode(1)
                .open(&path)
                .map_err(|_| "IME observer cache lock failed")?;
            let mut bytes = Vec::new();
            (&mut file)
                .take(DLL.len() as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| "IME observer verification failed")?;
            if bytes != DLL {
                return Err("IME observer artifact identity differs".into());
            }
            let path = wide(path.to_str().ok_or("IME observer path unavailable")?);
            let handle = unsafe {
                LoadLibraryExW(
                    path.as_ptr(),
                    null_mut(),
                    LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32,
                )
            };
            if handle.is_null() {
                return Err("Windows refused the IME observer module".into());
            }
            Ok((handle as isize, file))
        })
        .as_ref()
        .map(|(handle, _)| *handle)
        .map_err(Clone::clone)
}

pub(crate) struct Observer {
    hook: HHOOK,
    mapping: HANDLE,
    view: MEMORY_MAPPED_VIEW_ADDRESS,
    atom: u16,
    window: HWND,
    input: HWND,
    pid: u32,
    thread: u32,
    message: u32,
}
pub(crate) struct Observation {
    pub active: bool,
    pub preedit: String,
}
fn decode(sample: Sample, pid: u32, thread: u32, window: u64) -> Option<Observation> {
    if sample.pid != pid
        || sample.thread != thread
        || sample.window != window
        || sample.units as usize > MAX_UNITS
    {
        return None;
    }
    match sample.status {
        1 if sample.units == 0 => Some(Observation {
            active: false,
            preedit: String::new(),
        }),
        2 if sample.units > 0 => Some(Observation {
            active: true,
            preedit: String::from_utf16(&sample.text[..sample.units as usize]).ok()?,
        }),
        3 | 4 if sample.units == 0 => Some(Observation {
            active: sample.status == 4,
            preedit: String::new(),
        }),
        _ => None,
    }
}
impl Observer {
    pub fn new(window: isize, pid: u32, started: u64, tsf_only: bool) -> Result<Self, String> {
        Self::create(window, window, pid, started, u32::from(tsf_only))
    }
    pub fn status_only(window: isize, pid: u32, started: u64) -> Result<Self, String> {
        Self::create(window, window, pid, started, STATE_ONLY)
    }
    pub fn geometry_only(window: isize, pid: u32, started: u64) -> Result<Self, String> {
        Self::create(window, window, pid, started, STATE_GEOMETRY)
    }
    pub fn status_at(target: crate::focus::InputStatusEndpoint) -> Result<Self, String> {
        Self::create(
            target.window,
            target.input,
            target.process,
            target.started,
            STATE_ONLY,
        )
    }
    fn create(
        window: isize,
        input: isize,
        pid: u32,
        started: u64,
        kind: u32,
    ) -> Result<Self, String> {
        unsafe {
            let window = window as HWND;
            let mut owner = 0;
            let thread = GetWindowThreadProcessId(window, &mut owner);
            if thread == 0 || owner != pid || started == 0 {
                return Err("IME observer target identity unavailable".into());
            }
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return Err("IME observer process unavailable".into());
            }
            let (mut target_wow64, mut own_wow64) = (0, 0);
            let compatible = IsWow64Process(process, &mut target_wow64) != 0
                && IsWow64Process(GetCurrentProcess(), &mut own_wow64) != 0
                && target_wow64 == own_wow64;
            CloseHandle(process);
            if !compatible {
                return Err("IME observer requires a matching process architecture".into());
            }
            let module = module()? as HMODULE;
            let callback = GetProcAddress(module, c"EchoCompositionObserver".as_ptr().cast())
                .ok_or("IME observer entry point unavailable")?;
            let guid = windows::Win32::System::Com::CoCreateGuid()
                .map_err(|_| "IME observer session identity unavailable")?;
            let name = wide(&format!("{PREFIX}{guid:?}"));
            let mut observer = Self {
                hook: null_mut(),
                mapping: null_mut(),
                view: MEMORY_MAPPED_VIEW_ADDRESS { Value: null_mut() },
                atom: 0,
                window,
                input: input as HWND,
                pid,
                thread,
                message: RegisterWindowMessageW(wide(MESSAGE).as_ptr()),
            };
            observer.mapping = CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                null(),
                PAGE_READWRITE,
                0,
                std::mem::size_of::<Channel>() as u32,
                name.as_ptr(),
            );
            if observer.mapping.is_null()
                || GetLastError() == ERROR_ALREADY_EXISTS
                || observer.message == 0
            {
                return Err("IME observer channel unavailable".into());
            }
            observer.view = MapViewOfFile(
                observer.mapping,
                FILE_MAP_READ | FILE_MAP_WRITE,
                0,
                0,
                std::mem::size_of::<Channel>(),
            );
            if observer.view.Value.is_null() {
                return Err("IME observer channel mapping failed".into());
            }
            let mut channel = Channel::new(pid, thread, window as u64, started, kind == 1);
            channel.tsf_only = kind;
            channel.input_window = input as u64;
            std::ptr::write(observer.view.Value.cast::<Channel>(), channel);
            observer.atom = GlobalAddAtomW(name.as_ptr());
            if observer.atom == 0 {
                return Err("IME observer channel registration failed".into());
            }
            observer.hook = SetWindowsHookExW(
                WH_CALLWNDPROC,
                Some(std::mem::transmute::<
                    unsafe extern "system" fn() -> isize,
                    unsafe extern "system" fn(i32, usize, isize) -> isize,
                >(callback)),
                module,
                thread,
            );
            if observer.hook.is_null() {
                return Err("Windows refused target-thread composition observation".into());
            }
            // Installation alone is not capability evidence. Require the target's
            // synchronous identity-bound response before exposing this observer.
            let sample = observer
                .sample()
                .ok_or("IME observer target did not acknowledge a read")?;
            if (matches!(kind, STATE_ONLY | STATE_GEOMETRY) && sample.status != STATE_REPLY)
                || (!matches!(kind, STATE_ONLY | STATE_GEOMETRY)
                    && decode(sample, pid, thread, window as u64).is_none())
            {
                return Err("IME observer returned an unavailable state".into());
            }
            Ok(observer)
        }
    }
    pub fn read(&self) -> Option<Observation> {
        decode(self.sample()?, self.pid, self.thread, self.window as u64)
    }
    pub fn input_caret(&self) -> Option<echo_engine::PhysicalRect> {
        let sample = self.sample()?;
        let [left, top, right, bottom] = sample.caret;
        (sample.status == STATE_REPLY && right > left && bottom > top).then_some(
            echo_engine::PhysicalRect {
                x: left,
                y: top,
                width: right - left,
                height: bottom - top,
            },
        )
    }
    pub fn input_bounds(&self) -> Option<echo_engine::PhysicalRect> {
        let sample = self.sample()?;
        let [left, top, right, bottom] = sample.bounds;
        (sample.status == STATE_REPLY && right > left && bottom > top).then_some(
            echo_engine::PhysicalRect {
                x: left,
                y: top,
                width: right - left,
                height: bottom - top,
            },
        )
    }
    pub fn input_state(&self) -> Option<(echo_engine::InputMode, echo_engine::CompositionState)> {
        use echo_engine::{CompositionState, InputMode};
        let sample = self.sample()?;
        if sample.status != STATE_REPLY || sample.units != 0 {
            return None;
        }
        Some((
            match sample.mode {
                1 => InputMode::Chinese,
                2 => InputMode::English,
                _ => InputMode::Unknown,
            },
            match sample.composition {
                1 => CompositionState::Idle,
                2 => CompositionState::Composing,
                _ => CompositionState::Unknown,
            },
        ))
    }
    fn sample(&self) -> Option<Sample> {
        unsafe {
            let mut pid = 0;
            if GetWindowThreadProcessId(self.window, &mut pid) != self.thread
                || pid != self.pid
                || GetAncestor(self.input, GA_ROOT) != GetForegroundWindow()
            {
                return None;
            }
            let channel = &*self.view.Value.cast::<Channel>();
            let request = channel.next_request();
            let started = Instant::now();
            let mut result = 0;
            if SendMessageTimeoutW(
                self.window,
                self.message,
                self.atom as usize,
                0,
                SMTO_ABORTIFHUNG | SMTO_BLOCK | SMTO_ERRORONEXIT,
                40,
                &mut result,
            ) == 0
                || started.elapsed().as_millis() > 40
                || channel.response.load(Ordering::Acquire) != request
            {
                return None;
            }
            let sample = std::ptr::read_volatile(&channel.sample);
            if channel.response.load(Ordering::Acquire) != request {
                return None;
            }
            (sample.pid == self.pid
                && sample.thread == self.thread
                && sample.window == self.window as u64)
                .then_some(sample)
        }
    }
}
impl Drop for Observer {
    fn drop(&mut self) {
        unsafe {
            if !self.hook.is_null() && UnhookWindowsHookEx(self.hook) == 0 {
                eprintln!(
                    "Echo IME observer retirement failed: pid={} thread={} error={}",
                    self.pid,
                    self.thread,
                    GetLastError()
                );
            }
            // Each remote callback opens its own mapping handle/view. Retirement
            // cannot invalidate a callback that was already executing there.
            if self.atom != 0 {
                GlobalDeleteAtom(self.atom);
            }
            if !self.view.Value.is_null() {
                UnmapViewOfFile(self.view);
            }
            if !self.mapping.is_null() {
                CloseHandle(self.mapping);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tsf_lifecycle_samples_do_not_require_or_fabricate_preedit_text() {
        let mut sample = Sample::unknown();
        sample.pid = 1;
        sample.thread = 2;
        sample.window = 3;
        for (status, active) in [(3, false), (4, true), (3, false)] {
            sample.status = status;
            let result = decode(sample, 1, 2, 3).unwrap();
            assert_eq!(result.active, active);
            assert!(result.preedit.is_empty());
            assert!(decode(sample, 1, 2, 4).is_none());
        }
        sample.units = 1;
        assert!(decode(sample, 1, 2, 3).is_none());
        sample.units = 0;
        sample.status = 0;
        assert!(decode(sample, 1, 2, 3).is_none());
    }
    #[test]
    fn target_thread_samples_require_identity_valid_utf16_and_explicit_state() {
        let mut sample = Sample::unknown();
        sample.pid = 1;
        sample.thread = 2;
        sample.window = 3;
        assert!(decode(sample, 1, 2, 3).is_none());
        sample.status = 1;
        assert!(!decode(sample, 1, 2, 3).unwrap().active);
        sample.status = 2;
        sample.units = 5;
        sample.text[..5].copy_from_slice(&"nihao".encode_utf16().collect::<Vec<_>>());
        assert_eq!(decode(sample, 1, 2, 3).unwrap().preedit, "nihao");
        assert!(decode(sample, 9, 2, 3).is_none());
        assert!(decode(sample, 1, 9, 3).is_none());
        assert!(decode(sample, 1, 2, 9).is_none());
        sample.units = MAX_UNITS as u32 + 1;
        assert!(decode(sample, 1, 2, 3).is_none());
        sample.units = 1;
        sample.text[0] = 0xd800;
        assert!(decode(sample, 1, 2, 3).is_none());
        sample.status = 1;
        assert!(decode(sample, 1, 2, 3).is_none());
    }
}
