#![cfg_attr(windows, allow(unsafe_op_in_unsafe_fn))]

#[cfg(windows)]
mod windows_impl {
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::ptr;
    use std::sync::atomic::{AtomicIsize, Ordering};
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    use echo_platform::{
        ClipboardPlatform, ClipboardRepresentation, ClipboardSnapshot,
        InputTargetGeometry, PasteControlIdentity, PasteDelivery,
        PasteDeliveryFailure, PasteTarget, PhysicalRect, PlatformChange, PlatformChangePublisher,
        PlatformChangeSubscription, PlatformError, SourceContext,
    };
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HGLOBAL, HWND, LPARAM, RECT, WPARAM};
    use windows::Win32::System::DataExchange::{
        AddClipboardFormatListener, CloseClipboard, EmptyClipboard, GetClipboardData,
        GetClipboardOwner, GetClipboardSequenceNumber, IsClipboardFormatAvailable,
        OpenClipboard, RegisterClipboardFormatW, RemoveClipboardFormatListener, SetClipboardData,
    };
    use windows::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
    };
    use windows::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, GetClassNameW, GetForegroundWindow,
        GetGUIThreadInfo, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId, IsWindow,
        IsWindowVisible, PostMessageW, RegisterClassW,
        SendMessageW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT,
        ES_PASSWORD, ES_READONLY, GWL_STYLE, WNDCLASSW,
        WM_CLIPBOARDUPDATE, WM_CLOSE, WM_PASTE, HWND_MESSAGE, WS_OVERLAPPED,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;

    const OPEN_ATTEMPTS: usize = 5;

    pub struct WindowsPlatform {
        changes: PlatformChangePublisher,
        clipboard_window: Arc<AtomicIsize>,
        clipboard_worker: Mutex<Option<JoinHandle<()>>>,
        source: Arc<Mutex<SourceContext>>,
    }

    impl WindowsPlatform {
        pub fn new() -> Self {
            let changes = PlatformChangePublisher::default();
            let window = Arc::new(AtomicIsize::new(0));
            let source = Arc::new(Mutex::new(SourceContext::default()));
            let worker_window = Arc::clone(&window);
            let worker_source = Arc::clone(&source);
            let worker_changes = changes.clone();
            let (ready_tx, ready_rx) = mpsc::sync_channel(1);
            let worker = thread::Builder::new()
                .name("echo-clipboard-source".to_owned())
                .spawn(move || clipboard_source_worker(worker_window, worker_source, worker_changes, ready_tx))
                .ok();
            if worker.is_some() {
                if let Ok(handle) = ready_rx.recv_timeout(Duration::from_secs(2)) {
                    window.store(handle, Ordering::Release);
                }
            }
            Self {
                changes,
                clipboard_window: window,
                clipboard_worker: Mutex::new(worker),
                source,
            }
        }
    }

    impl Default for WindowsPlatform {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Drop for WindowsPlatform {
        fn drop(&mut self) {
            self.changes.close();
            let handle = self.clipboard_window.load(Ordering::Acquire);
            if handle != 0 {
                let _ = unsafe {
                    PostMessageW(
                        Some(HWND(handle as *mut c_void)),
                        WM_CLOSE,
                        WPARAM(0),
                        LPARAM(0),
                    )
                };
            }
            if let Some(worker) = self
                .clipboard_worker
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .take()
            {
                let _ = worker.join();
            }
        }
    }

    impl ClipboardPlatform for WindowsPlatform {
        fn subscribe_changes(&self) -> PlatformChangeSubscription {
            self.changes.subscribe()
        }

        fn clipboard_sequence(&self) -> u64 {
            unsafe { u64::from(GetClipboardSequenceNumber()) }
        }

        fn read_clipboard(&self) -> Result<Option<ClipboardSnapshot>, PlatformError> {
            let _clipboard = ClipboardGuard::open()?;
            let sequence = self.clipboard_sequence();
            let source = self
                .source
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone();
            let mut representations = Vec::new();
            if format_available(13) {
                let bytes = ClipboardGuard::read_global(13)?;
                let text = utf16_clipboard_text(&bytes);
                if !text.is_empty() {
                    representations.push(ClipboardRepresentation {
                        format: "text".to_owned(),
                        mime_type: "text/plain;charset=utf-8".to_owned(),
                        bytes: text.into_bytes(),
                    });
                }
            }
            let html = register_format("HTML Format")?;
            if format_available(html) {
                let bytes = trim_trailing_nuls(ClipboardGuard::read_global(html)?);
                if !bytes.is_empty() {
                    representations.push(ClipboardRepresentation {
                        format: "html".to_owned(),
                        mime_type: "text/html".to_owned(),
                        bytes,
                    });
                }
            }
            let rtf = register_format("Rich Text Format")?;
            if format_available(rtf) {
                let bytes = trim_trailing_nuls(ClipboardGuard::read_global(rtf)?);
                if !bytes.is_empty() {
                    representations.push(ClipboardRepresentation {
                        format: "rtf".to_owned(),
                        mime_type: "text/rtf".to_owned(),
                        bytes,
                    });
                }
            }
            let image_format = if format_available(17) { 17 } else { 8 };
            if format_available(image_format) {
                let dib = ClipboardGuard::read_global(image_format)?;
                if let Some(bytes) = dib_to_bmp(&dib) {
                    representations.push(ClipboardRepresentation {
                        format: "image".to_owned(),
                        mime_type: "image/bmp".to_owned(),
                        bytes,
                    });
                }
            }
            if format_available(15) {
                let paths = ClipboardGuard::read_file_paths()?;
                if !paths.is_empty() {
                    representations.push(ClipboardRepresentation {
                        format: "files".to_owned(),
                        mime_type: "text/uri-list".to_owned(),
                        bytes: paths.join("\n").into_bytes(),
                    });
                }
            }
            if self.clipboard_sequence() != sequence {
                return Err(PlatformError("clipboard changed while it was read".to_owned()));
            }
            Ok((!representations.is_empty()).then_some(ClipboardSnapshot {
                sequence,
                source,
                representations,
            }))
        }

        fn write_clipboard(
            &self,
            representations: &[ClipboardRepresentation],
        ) -> Result<u64, PlatformError> {
            let _clipboard = ClipboardGuard::open()?;
            unsafe { EmptyClipboard() }.map_err(platform_error)?;
            for representation in representations {
                match representation.format.as_str() {
                    "text" => ClipboardGuard::write_unicode(&String::from_utf8_lossy(&representation.bytes))?,
                    "html" => ClipboardGuard::write_global(register_format("HTML Format")?, nul_terminated(&representation.bytes))?,
                    "rtf" => ClipboardGuard::write_global(register_format("Rich Text Format")?, nul_terminated(&representation.bytes))?,
                    "image" => {
                        let dib = bmp_to_dib(&representation.bytes)
                            .ok_or_else(|| PlatformError("image representation is not a BMP".to_owned()))?;
                        ClipboardGuard::write_global(8, dib)?;
                    }
                    "files" => ClipboardGuard::write_file_paths(&representation.bytes)?,
                    _ => {}
                }
            }
            Ok(self.clipboard_sequence())
        }

        fn capture_target(&self) -> Result<Option<PasteTarget>, PlatformError> {
            capture_native_target()
        }

        fn paste_to_target(&self, target: &PasteTarget) -> Result<PasteDelivery, PlatformError> {
            let hwnd = HWND(target.window_id as *mut c_void);
            if hwnd.0.is_null()
                || !unsafe { IsWindow(Some(hwnd)) }.as_bool()
                || !unsafe { IsWindowVisible(hwnd) }.as_bool()
                || unsafe { GetForegroundWindow() } != hwnd
            {
                return Ok(PasteDelivery::Failed(
                    PasteDeliveryFailure::OriginalWindowUnavailable,
                ));
            }
            let mut process_id = 0;
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
            if process_id != target.process_id {
                return Ok(PasteDelivery::Failed(PasteDeliveryFailure::OriginalWindowUnavailable));
            }
            let thread_id = unsafe { GetWindowThreadProcessId(hwnd, None) };
            let mut info = windows::Win32::UI::WindowsAndMessaging::GUITHREADINFO {
                cbSize: size_of::<windows::Win32::UI::WindowsAndMessaging::GUITHREADINFO>() as u32,
                ..Default::default()
            };
            if unsafe { GetGUIThreadInfo(thread_id, &mut info) }.is_err() || info.hwndFocus.0.is_null() {
                return Ok(PasteDelivery::Failed(PasteDeliveryFailure::InputUnavailable));
            }
            if let Some(PasteControlIdentity::NativeWindow { handle, .. }) = &target.focused_control {
                if info.hwndFocus.0 as isize != *handle {
                    return Ok(PasteDelivery::Failed(PasteDeliveryFailure::InputUnavailable));
                }
            }
            unsafe { SendMessageW(info.hwndFocus, WM_PASTE, Some(WPARAM(0)), Some(LPARAM(0))) };
            Ok(PasteDelivery::Pasted)
        }
    }

    fn clipboard_source_worker(
        window_slot: Arc<AtomicIsize>,
        source: Arc<Mutex<SourceContext>>,
        changes: PlatformChangePublisher,
        ready: mpsc::SyncSender<isize>,
    ) {
        let class_name = wide("EchoClipboardSourceWindow");
        let title = wide("Echo clipboard source");
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(echo_window_proc),
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        let _ = unsafe { RegisterClassW(&class) };
        let Ok(window) = (unsafe {
            CreateWindowExW(
                Default::default(),
                PCWSTR(class_name.as_ptr()),
                PCWSTR(title.as_ptr()),
                WS_OVERLAPPED,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                None,
                None,
            )
        }) else {
            let _ = ready.send(0);
            return;
        };
        if unsafe { AddClipboardFormatListener(window) }.is_err() {
            let _ = ready.send(0);
            let _ = unsafe { DestroyWindow(window) };
            return;
        }
        window_slot.store(window.0 as isize, Ordering::Release);
        if ready.send(window.0 as isize).is_err() {
            let _ = unsafe { RemoveClipboardFormatListener(window) };
            let _ = unsafe { DestroyWindow(window) };
            return;
        }
        let mut message = windows::Win32::UI::WindowsAndMessaging::MSG::default();
        loop {
            let status = unsafe { windows::Win32::UI::WindowsAndMessaging::GetMessageW(&mut message, None, 0, 0) };
            if status.0 <= 0 || message.message == WM_CLOSE {
                break;
            }
            if message.message == WM_CLIPBOARDUPDATE {
                let owner = unsafe { GetClipboardOwner() }.ok().unwrap_or_default();
                let foreground = unsafe { GetForegroundWindow() };
                let verified = owner == foreground && !owner.0.is_null();
                *source.lock().unwrap_or_else(|error| error.into_inner()) = SourceContext {
                    is_source_verified: verified,
                    is_sensitivity_verified: false,
                    ..SourceContext::default()
                };
                changes.publish(PlatformChange::Clipboard {
                    sequence: unsafe { u64::from(GetClipboardSequenceNumber()) },
                });
            }
        }
        let _ = unsafe { RemoveClipboardFormatListener(window) };
        let _ = unsafe { DestroyWindow(window) };
        window_slot.store(0, Ordering::Release);
    }

    struct ClipboardGuard;

    impl ClipboardGuard {
        fn open() -> Result<Self, PlatformError> {
            for attempt in 0..OPEN_ATTEMPTS {
                if unsafe { OpenClipboard(None) }.is_ok() {
                    return Ok(Self);
                }
                thread::sleep(Duration::from_millis(5 * (attempt + 1) as u64));
            }
            Err(PlatformError("Windows clipboard is busy".to_owned()))
        }

        fn read_global(format: u32) -> Result<Vec<u8>, PlatformError> {
            let handle = unsafe { GetClipboardData(format) }.map_err(platform_error)?;
            let global = HGLOBAL(handle.0);
            let size = unsafe { GlobalSize(global) };
            if size == 0 {
                return Ok(Vec::new());
            }
            let pointer = unsafe { GlobalLock(global) };
            if pointer.is_null() {
                return Err(PlatformError("unable to lock clipboard memory".to_owned()));
            }
            let bytes = unsafe { std::slice::from_raw_parts(pointer.cast::<u8>(), size) }.to_vec();
            unsafe { let _ = GlobalUnlock(global); }
            Ok(bytes)
        }

        fn read_file_paths() -> Result<Vec<String>, PlatformError> {
            let handle = unsafe { GetClipboardData(15) }.map_err(platform_error)?;
            let drop = HDROP(handle.0);
            let count = unsafe { DragQueryFileW(drop, u32::MAX, None) };
            let mut paths = Vec::with_capacity(count as usize);
            for index in 0..count {
                let length = unsafe { DragQueryFileW(drop, index, None) };
                let mut buffer = vec![0_u16; length as usize + 1];
                unsafe { DragQueryFileW(drop, index, Some(&mut buffer)); }
                paths.push(String::from_utf16_lossy(&buffer[..length as usize]));
            }
            Ok(paths)
        }

        fn write_unicode(text: &str) -> Result<(), PlatformError> {
            let mut utf16 = text.encode_utf16().collect::<Vec<_>>();
            utf16.push(0);
            let bytes = unsafe {
                std::slice::from_raw_parts(utf16.as_ptr().cast::<u8>(), utf16.len() * size_of::<u16>())
            };
            Self::write_global(13, bytes.to_vec())
        }

        fn write_file_paths(bytes: &[u8]) -> Result<(), PlatformError> {
            let mut utf16 = String::from_utf8_lossy(bytes)
                .lines()
                .flat_map(|path| path.encode_utf16().chain(std::iter::once(0)))
                .collect::<Vec<_>>();
            utf16.push(0);
            #[repr(C)]
            struct DropFiles { files_offset: u32, point_x: i32, point_y: i32, non_client: i32, wide: i32 }
            let header = DropFiles { files_offset: size_of::<DropFiles>() as u32, point_x: 0, point_y: 0, non_client: 0, wide: 1 };
            let mut payload = vec![0_u8; size_of::<DropFiles>() + utf16.len() * size_of::<u16>()];
            unsafe {
                ptr::copy_nonoverlapping((&header as *const DropFiles).cast::<u8>(), payload.as_mut_ptr(), size_of::<DropFiles>());
                ptr::copy_nonoverlapping(utf16.as_ptr().cast::<u8>(), payload.as_mut_ptr().add(size_of::<DropFiles>()), utf16.len() * size_of::<u16>());
            }
            Self::write_global(15, payload)
        }

        fn write_global(format: u32, bytes: Vec<u8>) -> Result<(), PlatformError> {
            let global = unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) }.map_err(platform_error)?;
            let pointer = unsafe { GlobalLock(global) };
            if pointer.is_null() {
                return Err(PlatformError("unable to allocate clipboard memory".to_owned()));
            }
            unsafe {
                ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast::<u8>(), bytes.len());
                let _ = GlobalUnlock(global);
            }
            let handle = HANDLE(global.0);
            unsafe { SetClipboardData(format, Some(handle)) }.map_err(platform_error)?;
            Ok(())
        }
    }

    impl Drop for ClipboardGuard {
        fn drop(&mut self) {
            unsafe { let _ = CloseClipboard(); }
        }
    }

    fn capture_native_target() -> Result<Option<PasteTarget>, PlatformError> {
        let window = unsafe { GetForegroundWindow() };
        if window.0.is_null() || !unsafe { IsWindowVisible(window) }.as_bool() {
            return Ok(None);
        }
        let mut process_id = 0;
        let thread_id = unsafe { GetWindowThreadProcessId(window, Some(&mut process_id)) };
        if process_id == 0 || process_id == std::process::id() {
            return Ok(None);
        }
        let mut info = windows::Win32::UI::WindowsAndMessaging::GUITHREADINFO {
            cbSize: size_of::<windows::Win32::UI::WindowsAndMessaging::GUITHREADINFO>() as u32,
            ..Default::default()
        };
        if unsafe { GetGUIThreadInfo(thread_id, &mut info) }.is_err() || info.hwndFocus.0.is_null() {
            return Ok(None);
        }
        let focused = info.hwndFocus;
        let class_name = window_class_name(focused).unwrap_or_default();
        let style = unsafe { GetWindowLongW(focused, GWL_STYLE) } as u32;
        let is_input = is_native_input_class(&class_name);
        if !is_input || !unsafe { IsWindowEnabled(focused) }.as_bool() || style & ES_READONLY as u32 != 0 || style & ES_PASSWORD as u32 != 0 {
            return Ok(None);
        }
        let process_started_at = process_started_at(process_id).unwrap_or(0);
        if process_started_at == 0 {
            return Ok(None);
        }
        let rect = window_rect(focused).unwrap_or(PhysicalRect { x: 0, y: 0, width: 1, height: 1 });
        Ok(Some(PasteTarget {
            window_id: window.0 as isize,
            window_class: window_class_name(window).unwrap_or_default(),
            process_id,
            process_started_at,
            focused_control: Some(PasteControlIdentity::NativeWindow { handle: focused.0 as isize, class_name }),
            app_name: None,
            selected_text: None,
            is_single_line: None,
            geometry: InputTargetGeometry { target: rect, work_area: rect, dpi: 96 },
        }))
    }

    fn process_started_at(process_id: u32) -> Option<u64> {
        let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }.ok()?;
        let mut creation = windows::Win32::Foundation::FILETIME::default();
        let mut exit = windows::Win32::Foundation::FILETIME::default();
        let mut kernel = windows::Win32::Foundation::FILETIME::default();
        let mut user = windows::Win32::Foundation::FILETIME::default();
        let result = unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) }
            .ok()
            .map(|_| (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime));
        unsafe { let _ = CloseHandle(process); }
        result
    }

    fn window_class_name(window: HWND) -> Option<String> {
        let mut buffer = [0_u16; 256];
        let length = unsafe { GetClassNameW(window, &mut buffer) };
        (length > 0).then(|| String::from_utf16_lossy(&buffer[..length as usize]))
    }

    fn window_rect(window: HWND) -> Option<PhysicalRect> {
        let mut rect = RECT::default();
        unsafe { GetWindowRect(window, &mut rect) }.ok()?;
        let width = rect.right.checked_sub(rect.left)?;
        let height = rect.bottom.checked_sub(rect.top)?;
        (width > 0 && height > 0).then_some(PhysicalRect { x: rect.left, y: rect.top, width, height })
    }

    fn is_native_input_class(class_name: &str) -> bool {
        let class_name = class_name.to_ascii_lowercase();
        class_name == "edit" || class_name.starts_with("richedit") || class_name.contains(".edit.")
    }

    fn register_format(name: &str) -> Result<u32, PlatformError> {
        let wide = wide(name);
        let format = unsafe { RegisterClipboardFormatW(PCWSTR(wide.as_ptr())) };
        (format != 0).then_some(format).ok_or_else(|| PlatformError(format!("unable to register clipboard format {name}")))
    }

    fn format_available(format: u32) -> bool {
        unsafe { IsClipboardFormatAvailable(format) }.is_ok()
    }

    fn utf16_clipboard_text(bytes: &[u8]) -> String {
        let words = bytes.chunks_exact(2).map(|pair| u16::from_le_bytes([pair[0], pair[1]])).take_while(|word| *word != 0).collect::<Vec<_>>();
        String::from_utf16_lossy(&words)
    }

    fn trim_trailing_nuls(mut bytes: Vec<u8>) -> Vec<u8> {
        while bytes.last() == Some(&0) { bytes.pop(); }
        bytes
    }

    fn nul_terminated(bytes: &[u8]) -> Vec<u8> {
        let mut result = bytes.to_vec();
        if result.last() != Some(&0) { result.push(0); }
        result
    }

    fn dib_to_bmp(dib: &[u8]) -> Option<Vec<u8>> {
        if dib.len() < 40 { return None; }
        let header_size = u32::from_le_bytes(dib.get(0..4)?.try_into().ok()?) as usize;
        if header_size < 40 || header_size > dib.len() { return None; }
        let bit_count = u16::from_le_bytes(dib.get(14..16)?.try_into().ok()?);
        let colors_used = u32::from_le_bytes(dib.get(32..36)?.try_into().ok()?) as usize;
        let palette_entries = if colors_used > 0 { colors_used } else if bit_count <= 8 { 1_usize << bit_count } else { 0 };
        let compression = u32::from_le_bytes(dib.get(16..20)?.try_into().ok()?);
        let external_masks = if header_size == 40 { match compression { 3 => 12, 6 => 16, _ => 0 } } else { 0 };
        let pixel_offset = 14 + header_size + external_masks + palette_entries * 4;
        if pixel_offset > 14 + dib.len() { return None; }
        let file_size = 14 + dib.len();
        let mut bmp = Vec::with_capacity(file_size);
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&(file_size as u32).to_le_bytes());
        bmp.extend_from_slice(&[0; 4]);
        bmp.extend_from_slice(&(pixel_offset as u32).to_le_bytes());
        bmp.extend_from_slice(dib);
        Some(bmp)
    }

    fn bmp_to_dib(bmp: &[u8]) -> Option<Vec<u8>> {
        (bmp.len() >= 54 && &bmp[..2] == b"BM").then(|| bmp[14..].to_vec())
    }

    fn platform_error(error: windows::core::Error) -> PlatformError {
        PlatformError(error.to_string())
    }

    unsafe extern "system" fn echo_window_proc(
        window: HWND,
        message: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> windows::Win32::Foundation::LRESULT {
        unsafe { windows::Win32::UI::WindowsAndMessaging::DefWindowProcW(window, message, wparam, lparam) }
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }
}

#[cfg(windows)]
pub use windows_impl::WindowsPlatform;

#[cfg(not(windows))]
pub struct WindowsPlatform;

#[cfg(not(windows))]
impl WindowsPlatform {
    pub fn new() -> Self {
        Self
    }
}

#[cfg(not(windows))]
impl Default for WindowsPlatform {
    fn default() -> Self {
        Self::new()
    }
}
