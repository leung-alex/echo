//! Native desktop services without a UI framework or browser runtime.
//! All HWND and HANDLE access is kept inside this adapter.
use std::{path::Path, sync::Arc, thread::JoinHandle};
use windows_sys::Win32::{Foundation::*, System::Threading::*, UI::WindowsAndMessaging::*};
mod card_window;
mod common;
mod hotkey;
mod pipe;
mod tray;
pub use hotkey::{HotkeyController, HotkeyReservation};
mod window;
pub use card_window::{
    apply_card_chrome, expand_card_region, finish_card_frame, set_card_region, CardShape,
};
mod environment;
use common::{wide, Handle, Security};
pub use environment::{fit_window, ui_environment, UiEnvironment};
pub use window::{
    apply_theme, attach_window, center_composition, focus_window, reposition_favorites, set_owner,
    start_drag, system_dark, WindowHook,
};

#[derive(Debug, Clone)]
pub enum ShellEvent {
    Open,
    QuickInsert(crate::focus::FocusSnapshot),
    HotkeyStatus(String),
    Favorites,
    Settings,
    Quit,
    Activation(Vec<String>),
    FocusLost,
    ThemeChanged,
    GeometryChanged,
    Error(String),
}
pub type EventHandler = Arc<dyn Fn(ShellEvent) + Send + Sync>;
pub enum Instance {
    Forwarded,
    Primary(NativeShell),
}

pub struct NativeShell {
    _mutex: Handle,
    stop: Arc<Handle>,
    hwnd: isize,
    hotkeys: HotkeyController,
    pipe: Option<JoinHandle<()>>,
    tray: Option<JoinHandle<()>>,
}
impl NativeShell {
    pub fn hotkeys(&self) -> HotkeyController {
        self.hotkeys.clone()
    }
    /// Starts one resident host per user, logon session and canonical data directory.
    /// A secondary process forwards bounded arguments over a user-only local pipe.
    pub fn start(
        data_dir: &Path,
        args: &[String],
        handler: EventHandler,
    ) -> Result<Instance, String> {
        let sid = common::current_sid()?;
        let namespace = common::namespace(data_dir, &sid)?;
        let security = Security::for_user(&sid)?;
        let mutex_name = wide(&format!("Local\\Echo.Native.{namespace}"));
        let mutex =
            unsafe { Handle::new(CreateMutexW(&security.attributes(), 0, mutex_name.as_ptr()))? };
        let exists = unsafe { GetLastError() == ERROR_ALREADY_EXISTS };
        let pipe_name = format!("\\\\.\\pipe\\Echo.Native.{namespace}");
        if exists {
            pipe::forward(&pipe_name, args, &sid)?;
            return Ok(Instance::Forwarded);
        }
        let stop = Arc::new(unsafe {
            Handle::new(CreateEventW(std::ptr::null(), 1, 0, std::ptr::null()))?
        });
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        let tray_callback = handler.clone();
        let (hotkey_tx, hotkey_rx) = std::sync::mpsc::sync_channel(8);
        let tray = std::thread::Builder::new()
            .name("echo-native-tray".into())
            .spawn(move || tray::run(tray_callback, ready_tx, hotkey_rx))
            .map_err(|e| e.to_string())?;
        let hwnd = match ready_rx.recv_timeout(std::time::Duration::from_secs(5)) {
            Ok(Ok(hwnd)) => hwnd,
            Ok(Err(e)) => {
                let _ = tray.join();
                return Err(e);
            }
            Err(e) => return Err(e.to_string()),
        };
        let pipe_stop = stop.clone();
        let pipe = match std::thread::Builder::new()
            .name("echo-native-activation".into())
            .spawn(move || pipe::serve(pipe_name, sid, pipe_stop, handler))
        {
            Ok(p) => p,
            Err(e) => {
                unsafe {
                    PostMessageW(hwnd as HWND, WM_CLOSE, 0, 0);
                }
                let _ = tray.join();
                return Err(e.to_string());
            }
        };
        Ok(Instance::Primary(Self {
            _mutex: mutex,
            stop,
            hwnd,
            hotkeys: HotkeyController::new(hwnd, hotkey_tx),
            pipe: Some(pipe),
            tray: Some(tray),
        }))
    }
}
impl Drop for NativeShell {
    fn drop(&mut self) {
        unsafe {
            SetEvent(self.stop.0);
            PostMessageW(self.hwnd as HWND, WM_CLOSE, 0, 0);
        }
        if let Some(t) = self.pipe.take() {
            let _ = t.join();
        }
        if let Some(t) = self.tray.take() {
            let _ = t.join();
        }
    }
}
