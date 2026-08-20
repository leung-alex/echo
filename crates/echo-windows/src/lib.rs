#![cfg_attr(windows, allow(unsafe_op_in_unsafe_fn))]

#[cfg(windows)]
mod windows_impl {
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::path::Path;
    use std::ptr::{self, null_mut};
    use std::sync::atomic::{AtomicIsize, Ordering};
    use std::sync::{mpsc, Arc, Mutex};
    use std::thread::{self, JoinHandle};
    use std::time::Duration;

    use echo_engine::{
        CapturePolicy, ClipboardPlatform, ClipboardRepresentation, ClipboardSnapshot,
        InputTargetGeometry, PasteControlIdentity, PasteDelivery, PasteDeliveryFailure,
        PasteTarget, PhysicalRect, PlatformChange, PlatformChangePublisher,
        PlatformChangeSubscription, PlatformError, SourceContext,
    };
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HGLOBAL, HWND, LPARAM, RECT, WPARAM};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER,
        COINIT_APARTMENTTHREADED,
    };
    use windows::Win32::System::DataExchange::{
        AddClipboardFormatListener, CloseClipboard, EmptyClipboard, GetClipboardData,
        GetClipboardOwner, GetClipboardSequenceNumber, IsClipboardFormatAvailable, OpenClipboard,
        RegisterClipboardFormatW, RemoveClipboardFormatListener, SetClipboardData,
    };
    use windows::Win32::System::Memory::{
        GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
    };
    use windows::Win32::System::Ole::{
        SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetLBound, SafeArrayGetUBound,
        SafeArrayUnaccessData,
    };
    use windows::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
        PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationElement, UIA_ComboBoxControlTypeId,
        UIA_DocumentControlTypeId, UIA_EditControlTypeId,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        IsWindowEnabled, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
        VIRTUAL_KEY, VK_CONTROL, VK_V,
    };
    use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, GetAncestor, GetClassNameW, GetForegroundWindow,
        GetGUIThreadInfo, GetWindowLongW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
        GetWindowThreadProcessId, IsWindow, PostMessageW, RegisterClassW, SendMessageW,
        SetForegroundWindow, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, ES_PASSWORD, ES_READONLY,
        GA_ROOT, GWL_STYLE, HWND_MESSAGE, WM_CLIPBOARDUPDATE, WM_CLOSE, WM_PASTE, WNDCLASSW,
        WS_OVERLAPPED,
    };

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
                .spawn(move || {
                    clipboard_source_worker(worker_window, worker_source, worker_changes, ready_tx)
                })
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

        fn read_clipboard(
            &self,
            policy: &CapturePolicy,
        ) -> Result<Option<ClipboardSnapshot>, PlatformError> {
            let _clipboard = ClipboardGuard::open()?;
            let sequence = self.clipboard_sequence();
            let source = self
                .source
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone();
            let mut representations = Vec::new();
            let mut total_bytes = 0_u64;
            if policy.supports_format("text") && format_available(13) {
                if let Some(bytes) =
                    ClipboardGuard::read_global_if_allowed(13, "text", policy, &mut total_bytes)?
                {
                    let text = utf16_clipboard_text(&bytes);
                    if !text.is_empty() {
                        representations.push(ClipboardRepresentation {
                            format: "text".to_owned(),
                            mime_type: "text/plain;charset=utf-8".to_owned(),
                            bytes: text.into_bytes(),
                        });
                    }
                }
            }
            let html = policy
                .supports_format("html")
                .then(|| register_format("HTML Format"))
                .transpose()?;
            if let Some(html) = html.filter(|format| format_available(*format)) {
                if let Some(bytes) =
                    ClipboardGuard::read_global_if_allowed(html, "html", policy, &mut total_bytes)?
                {
                    let bytes = trim_trailing_nuls(bytes);
                    if !bytes.is_empty() {
                        representations.push(ClipboardRepresentation {
                            format: "html".to_owned(),
                            mime_type: "text/html".to_owned(),
                            bytes,
                        });
                    }
                }
            }
            let rtf = policy
                .supports_format("rtf")
                .then(|| register_format("Rich Text Format"))
                .transpose()?;
            if let Some(rtf) = rtf.filter(|format| format_available(*format)) {
                if let Some(bytes) =
                    ClipboardGuard::read_global_if_allowed(rtf, "rtf", policy, &mut total_bytes)?
                {
                    let bytes = trim_trailing_nuls(bytes);
                    if !bytes.is_empty() {
                        representations.push(ClipboardRepresentation {
                            format: "rtf".to_owned(),
                            mime_type: "text/rtf".to_owned(),
                            bytes,
                        });
                    }
                }
            }
            let image_format = if format_available(17) { 17 } else { 8 };
            if policy.supports_format("image") && format_available(image_format) {
                if let Some(dib) = ClipboardGuard::read_global_if_allowed(
                    image_format,
                    "image",
                    policy,
                    &mut total_bytes,
                )? {
                    if let Some(bytes) = dib_to_bmp(&dib) {
                        representations.push(ClipboardRepresentation {
                            format: "image".to_owned(),
                            mime_type: "image/bmp".to_owned(),
                            bytes,
                        });
                    }
                }
            }
            if policy.supports_format("files") && format_available(15) {
                if let Some(paths) =
                    ClipboardGuard::read_file_paths_if_allowed(policy, &mut total_bytes)?
                {
                    if !paths.is_empty() {
                        representations.push(ClipboardRepresentation {
                            format: "files".to_owned(),
                            mime_type: "text/uri-list".to_owned(),
                            bytes: paths.join("\n").into_bytes(),
                        });
                    }
                }
            }
            if self.clipboard_sequence() != sequence {
                return Err(PlatformError(
                    "clipboard changed while it was read".to_owned(),
                ));
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
                    "text" => ClipboardGuard::write_unicode(&String::from_utf8_lossy(
                        &representation.bytes,
                    ))?,
                    "html" => ClipboardGuard::write_global(
                        register_format("HTML Format")?,
                        nul_terminated(&representation.bytes),
                    )?,
                    "rtf" => ClipboardGuard::write_global(
                        register_format("Rich Text Format")?,
                        nul_terminated(&representation.bytes),
                    )?,
                    "image" => {
                        let dib = bmp_to_dib(&representation.bytes).ok_or_else(|| {
                            PlatformError("image representation is not a BMP".to_owned())
                        })?;
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
            if hwnd.0.is_null() || !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
                return Ok(PasteDelivery::Failed(
                    PasteDeliveryFailure::OriginalWindowUnavailable,
                ));
            }
            let mut process_id = 0;
            unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
            if process_id != target.process_id
                || process_started_at(process_id).unwrap_or_default() != target.process_started_at
                || window_class_name(hwnd).as_deref() != Some(target.window_class.as_str())
            {
                return Ok(PasteDelivery::Failed(
                    PasteDeliveryFailure::OriginalWindowUnavailable,
                ));
            }
            if unsafe { GetForegroundWindow() } != hwnd {
                let _ = unsafe { SetForegroundWindow(hwnd) };
                thread::sleep(Duration::from_millis(20));
            }
            if unsafe { GetForegroundWindow() } != hwnd {
                return Ok(PasteDelivery::Failed(
                    PasteDeliveryFailure::OriginalWindowUnavailable,
                ));
            }
            let Some(current_control) = focused_input_identity(hwnd, target.process_id)
                .or_else(|| focused_native_control_from_gui(hwnd, target.process_id))
            else {
                return Ok(PasteDelivery::Failed(
                    PasteDeliveryFailure::InputUnavailable,
                ));
            };
            if target.focused_control.as_ref() != Some(&current_control) {
                return Ok(PasteDelivery::Failed(
                    PasteDeliveryFailure::InputUnavailable,
                ));
            }
            match current_control {
                PasteControlIdentity::NativeWindow { handle, .. } => {
                    let control = HWND(handle as *mut c_void);
                    unsafe {
                        SendMessageW(control, WM_PASTE, Some(WPARAM(0)), Some(LPARAM(0)));
                    }
                    Ok(PasteDelivery::Pasted)
                }
                PasteControlIdentity::AutomationRuntimeId(_) => send_paste_shortcut()
                    .map(|_| PasteDelivery::Pasted)
                    .map_err(|_| {
                        PlatformError("Windows did not accept the paste shortcut".to_owned())
                    }),
            }
        }
    }

    fn send_paste_shortcut() -> Result<(), PasteDeliveryFailure> {
        let inputs = [
            key_input(VK_CONTROL, false),
            key_input(VK_V, false),
            key_input(VK_V, true),
            key_input(VK_CONTROL, true),
        ];
        let sent = unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) };
        if sent == inputs.len() as u32 {
            Ok(())
        } else {
            let release = [key_input(VK_CONTROL, true), key_input(VK_V, true)];
            let _ = unsafe { SendInput(&release, size_of::<INPUT>() as i32) };
            Err(PasteDeliveryFailure::KeyInjectionFailed)
        }
    }

    fn key_input(key: VIRTUAL_KEY, released: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: key,
                    wScan: 0,
                    dwFlags: if released {
                        KEYEVENTF_KEYUP
                    } else {
                        Default::default()
                    },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
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
            let status = unsafe {
                windows::Win32::UI::WindowsAndMessaging::GetMessageW(&mut message, None, 0, 0)
            };
            if status.0 <= 0 || message.message == WM_CLOSE {
                break;
            }
            if message.message == WM_CLIPBOARDUPDATE {
                let owner = unsafe { GetClipboardOwner() }.ok().unwrap_or_default();
                let foreground = unsafe { GetForegroundWindow() };
                *source.lock().unwrap_or_else(|error| error.into_inner()) =
                    source_context(owner, foreground);
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
            unsafe {
                let _ = GlobalUnlock(global);
            }
            Ok(bytes)
        }

        fn global_size(format: u32) -> Result<usize, PlatformError> {
            let handle = unsafe { GetClipboardData(format) }.map_err(platform_error)?;
            Ok(unsafe { GlobalSize(HGLOBAL(handle.0)) })
        }

        fn read_global_if_allowed(
            format: u32,
            semantic_format: &str,
            policy: &CapturePolicy,
            total_bytes: &mut u64,
        ) -> Result<Option<Vec<u8>>, PlatformError> {
            let available_size = Self::global_size(format)? as u64;
            if available_size != 0
                && !policy.accepts_size(semantic_format, available_size, *total_bytes)
            {
                return Ok(None);
            }
            let bytes = Self::read_global(format)?;
            let byte_size = bytes.len() as u64;
            if !policy.accepts_size(semantic_format, byte_size, *total_bytes) {
                return Ok(None);
            }
            *total_bytes = total_bytes.saturating_add(byte_size);
            Ok(Some(bytes))
        }

        fn read_file_paths() -> Result<Vec<String>, PlatformError> {
            let handle = unsafe { GetClipboardData(15) }.map_err(platform_error)?;
            let drop = HDROP(handle.0);
            let count = unsafe { DragQueryFileW(drop, u32::MAX, None) };
            let mut paths = Vec::with_capacity(count as usize);
            for index in 0..count {
                let length = unsafe { DragQueryFileW(drop, index, None) };
                let mut buffer = vec![0_u16; length as usize + 1];
                unsafe {
                    DragQueryFileW(drop, index, Some(&mut buffer));
                }
                paths.push(String::from_utf16_lossy(&buffer[..length as usize]));
            }
            Ok(paths)
        }

        fn read_file_paths_if_allowed(
            policy: &CapturePolicy,
            total_bytes: &mut u64,
        ) -> Result<Option<Vec<String>>, PlatformError> {
            let available_size = Self::global_size(15)? as u64;
            if available_size != 0 && !policy.accepts_size("files", available_size, *total_bytes) {
                return Ok(None);
            }
            let paths = Self::read_file_paths()?;
            let byte_size = paths.join("\n").len() as u64;
            if !policy.accepts_size("files", byte_size, *total_bytes) {
                return Ok(None);
            }
            *total_bytes = total_bytes.saturating_add(byte_size);
            Ok(Some(paths))
        }

        fn write_unicode(text: &str) -> Result<(), PlatformError> {
            let mut utf16 = text.encode_utf16().collect::<Vec<_>>();
            utf16.push(0);
            let bytes = unsafe {
                std::slice::from_raw_parts(
                    utf16.as_ptr().cast::<u8>(),
                    utf16.len() * size_of::<u16>(),
                )
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
            struct DropFiles {
                files_offset: u32,
                point_x: i32,
                point_y: i32,
                non_client: i32,
                wide: i32,
            }
            let header = DropFiles {
                files_offset: size_of::<DropFiles>() as u32,
                point_x: 0,
                point_y: 0,
                non_client: 0,
                wide: 1,
            };
            let mut payload = vec![0_u8; size_of::<DropFiles>() + utf16.len() * size_of::<u16>()];
            unsafe {
                ptr::copy_nonoverlapping(
                    (&header as *const DropFiles).cast::<u8>(),
                    payload.as_mut_ptr(),
                    size_of::<DropFiles>(),
                );
                ptr::copy_nonoverlapping(
                    utf16.as_ptr().cast::<u8>(),
                    payload.as_mut_ptr().add(size_of::<DropFiles>()),
                    utf16.len() * size_of::<u16>(),
                );
            }
            Self::write_global(15, payload)
        }

        fn write_global(format: u32, bytes: Vec<u8>) -> Result<(), PlatformError> {
            let global =
                unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) }.map_err(platform_error)?;
            let pointer = unsafe { GlobalLock(global) };
            if pointer.is_null() {
                return Err(PlatformError(
                    "unable to allocate clipboard memory".to_owned(),
                ));
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
            unsafe {
                let _ = CloseClipboard();
            }
        }
    }

    fn capture_native_target() -> Result<Option<PasteTarget>, PlatformError> {
        let window = unsafe { GetForegroundWindow() };
        if window.0.is_null() || !unsafe { IsWindow(Some(window)) }.as_bool() {
            return Ok(None);
        }
        let mut process_id = 0;
        unsafe { GetWindowThreadProcessId(window, Some(&mut process_id)) };
        if process_id == 0 || process_id == std::process::id() {
            return Ok(None);
        }
        let focused_control = focused_input_identity(window, process_id)
            .or_else(|| focused_native_control_from_gui(window, process_id));
        let Some(focused_control) = focused_control else {
            return Ok(None);
        };
        let focused_rect = match &focused_control {
            PasteControlIdentity::NativeWindow { handle, class_name } => {
                let focused = HWND(*handle as *mut c_void);
                let style = unsafe { GetWindowLongW(focused, GWL_STYLE) } as u32;
                if !is_native_input_class(class_name)
                    || !unsafe { IsWindowEnabled(focused) }.as_bool()
                    || style & ES_READONLY as u32 != 0
                    || style & ES_PASSWORD as u32 != 0
                {
                    return Ok(None);
                }
                window_rect(focused)
            }
            PasteControlIdentity::AutomationRuntimeId(_) => None,
        };
        let process_started_at = process_started_at(process_id).unwrap_or(0);
        if process_started_at == 0 {
            return Ok(None);
        }
        let rect = focused_rect
            .or_else(|| window_rect(window))
            .unwrap_or(PhysicalRect {
                x: 0,
                y: 0,
                width: 1,
                height: 1,
            });
        Ok(Some(PasteTarget {
            window_id: window.0 as isize,
            window_class: window_class_name(window).unwrap_or_default(),
            process_id,
            process_started_at,
            focused_control: Some(focused_control),
            app_name: None,
            selected_text: None,
            is_single_line: None,
            geometry: InputTargetGeometry {
                target: rect,
                work_area: rect,
                dpi: 96,
            },
        }))
    }

    fn focused_input_identity(window: HWND, process_id: u32) -> Option<PasteControlIdentity> {
        unsafe {
            let initialization = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let initialized = initialization.is_ok();
            let result = (|| {
                let automation: IUIAutomation =
                    match CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok() {
                        Some(value) => value,
                        None => return None,
                    };
                let focused = match automation.GetFocusedElement().ok() {
                    Some(value) => value,
                    None => return None,
                };
                let focused_process = focused.CurrentProcessId().ok().map(|value| value as u32);
                let belongs = automation_element_belongs_to(&automation, &focused, window);
                let editable = automation_element_is_editable(&focused);
                if focused_process != Some(process_id) || !belongs || !editable {
                    return None;
                }
                focused
                    .CurrentNativeWindowHandle()
                    .ok()
                    .filter(|control| native_control_belongs_to(*control, window, process_id))
                    .and_then(|control| {
                        let class_name = window_class_name(control)?;
                        is_native_input_class(&class_name).then_some(
                            PasteControlIdentity::NativeWindow {
                                handle: control.0 as isize,
                                class_name,
                            },
                        )
                    })
                    .or_else(|| {
                        automation_runtime_id(&focused)
                            .map(PasteControlIdentity::AutomationRuntimeId)
                    })
            })();
            if initialized {
                CoUninitialize();
            }
            result
        }
    }

    fn focused_native_control_from_gui(
        window: HWND,
        process_id: u32,
    ) -> Option<PasteControlIdentity> {
        let thread_id = unsafe { GetWindowThreadProcessId(window, None) };
        let mut info = windows::Win32::UI::WindowsAndMessaging::GUITHREADINFO {
            cbSize: size_of::<windows::Win32::UI::WindowsAndMessaging::GUITHREADINFO>() as u32,
            ..Default::default()
        };
        unsafe { GetGUIThreadInfo(thread_id, &mut info).ok()? };
        let control = info.hwndFocus;
        if control.0.is_null() || !native_control_belongs_to(control, window, process_id) {
            return None;
        }
        let class_name = window_class_name(control)?;
        let style = unsafe { GetWindowLongW(control, GWL_STYLE) } as u32;
        if !is_native_input_class(&class_name)
            || !unsafe { IsWindowEnabled(control) }.as_bool()
            || style & ES_READONLY as u32 != 0
            || style & ES_PASSWORD as u32 != 0
        {
            return None;
        }
        Some(PasteControlIdentity::NativeWindow {
            handle: control.0 as isize,
            class_name,
        })
    }

    fn native_control_belongs_to(control: HWND, window: HWND, process_id: u32) -> bool {
        if control.0.is_null() || window.0.is_null() {
            return false;
        }
        let mut owner_process_id = 0;
        (unsafe { GetWindowThreadProcessId(control, Some(&mut owner_process_id)) }) != 0
            && owner_process_id == process_id
            && unsafe { GetAncestor(control, GA_ROOT) } == window
    }

    fn automation_element_is_editable(element: &IUIAutomationElement) -> bool {
        unsafe {
            element
                .CurrentIsEnabled()
                .is_ok_and(|value| value.as_bool())
                && element
                    .CurrentIsKeyboardFocusable()
                    .is_ok_and(|value| value.as_bool())
                && element
                    .CurrentHasKeyboardFocus()
                    .is_ok_and(|value| value.as_bool())
                && element
                    .CurrentIsPassword()
                    .is_ok_and(|value| !value.as_bool())
                && element.CurrentControlType().is_ok_and(|control_type| {
                    control_type == UIA_EditControlTypeId
                        || control_type == UIA_DocumentControlTypeId
                        || control_type == UIA_ComboBoxControlTypeId
                })
        }
    }

    fn automation_element_belongs_to(
        automation: &IUIAutomation,
        element: &IUIAutomationElement,
        window: HWND,
    ) -> bool {
        unsafe {
            let Ok(root) = automation.ElementFromHandle(window) else {
                return false;
            };
            let Ok(walker) = automation.ControlViewWalker() else {
                return false;
            };
            let mut current = element.clone();
            for _ in 0..256 {
                if automation
                    .CompareElements(&current, &root)
                    .is_ok_and(|same| same.as_bool())
                {
                    return true;
                }
                let Ok(parent) = walker.GetParentElement(&current) else {
                    return false;
                };
                current = parent;
            }
            false
        }
    }

    fn automation_runtime_id(element: &IUIAutomationElement) -> Option<Vec<i32>> {
        let array = unsafe { element.GetRuntimeId() }.ok()?;
        if array.is_null() {
            return None;
        }
        let result = (|| {
            let lower = unsafe { SafeArrayGetLBound(array, 1) }.ok()?;
            let upper = unsafe { SafeArrayGetUBound(array, 1) }.ok()?;
            let length = usize::try_from(upper.checked_sub(lower)?.checked_add(1)?).ok()?;
            if length == 0 {
                return None;
            }
            let mut data = null_mut();
            unsafe { SafeArrayAccessData(array, &mut data) }.ok()?;
            let runtime_id = (!data.is_null()).then(|| unsafe {
                std::slice::from_raw_parts(data.cast::<i32>(), length).to_vec()
            });
            let _ = unsafe { SafeArrayUnaccessData(array) };
            runtime_id
        })();
        let _ = unsafe { SafeArrayDestroy(array) };
        result
    }

    fn source_context(owner: HWND, foreground: HWND) -> SourceContext {
        if owner.0.is_null() || foreground.0.is_null() {
            return SourceContext::default();
        }
        let owner_root = unsafe { GetAncestor(owner, GA_ROOT) };
        let mut owner_process = 0;
        let mut foreground_process = 0;
        unsafe {
            GetWindowThreadProcessId(owner_root, Some(&mut owner_process));
            GetWindowThreadProcessId(foreground, Some(&mut foreground_process));
        }
        let verified =
            owner_root == foreground && owner_process != 0 && owner_process == foreground_process;
        if !verified {
            return SourceContext::default();
        }
        let (executable, app_name) = process_path(foreground_process)
            .map(|path| {
                let app_name = Path::new(&path)
                    .file_stem()
                    .and_then(|name| name.to_str())
                    .map(str::to_owned);
                (Some(path), app_name)
            })
            .unwrap_or((None, None));
        let window_title = window_title(foreground);
        let thread_id = unsafe { GetWindowThreadProcessId(foreground, None) };
        let mut info = windows::Win32::UI::WindowsAndMessaging::GUITHREADINFO {
            cbSize: size_of::<windows::Win32::UI::WindowsAndMessaging::GUITHREADINFO>() as u32,
            ..Default::default()
        };
        let sensitivity_verified = unsafe { GetGUIThreadInfo(thread_id, &mut info) }.is_ok()
            && !info.hwndFocus.0.is_null();
        let is_password_input = sensitivity_verified
            && is_native_input_class(&window_class_name(info.hwndFocus).unwrap_or_default())
            && (unsafe { GetWindowLongW(info.hwndFocus, GWL_STYLE) } as u32 & ES_PASSWORD as u32
                != 0);
        SourceContext {
            app_name,
            executable,
            window_title,
            is_source_verified: true,
            is_sensitivity_verified: sensitivity_verified,
            is_password_input,
            is_private_window: false,
        }
    }

    fn process_path(process_id: u32) -> Option<String> {
        let process =
            unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }.ok()?;
        let mut buffer = vec![0_u16; 32_768];
        let mut length = buffer.len() as u32;
        let result = unsafe {
            QueryFullProcessImageNameW(
                process,
                PROCESS_NAME_WIN32,
                windows::core::PWSTR(buffer.as_mut_ptr()),
                &mut length,
            )
        }
        .ok()
        .map(|_| String::from_utf16_lossy(&buffer[..length as usize]));
        unsafe {
            let _ = CloseHandle(process);
        }
        result
    }

    fn window_title(window: HWND) -> Option<String> {
        let length = unsafe { GetWindowTextLengthW(window) };
        if length <= 0 {
            return None;
        }
        let mut buffer = vec![0_u16; length as usize + 1];
        let copied = unsafe { GetWindowTextW(window, &mut buffer) };
        (copied > 0).then(|| String::from_utf16_lossy(&buffer[..copied as usize]))
    }

    fn process_started_at(process_id: u32) -> Option<u64> {
        let process =
            unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }.ok()?;
        let mut creation = windows::Win32::Foundation::FILETIME::default();
        let mut exit = windows::Win32::Foundation::FILETIME::default();
        let mut kernel = windows::Win32::Foundation::FILETIME::default();
        let mut user = windows::Win32::Foundation::FILETIME::default();
        let result =
            unsafe { GetProcessTimes(process, &mut creation, &mut exit, &mut kernel, &mut user) }
                .ok()
                .map(|_| {
                    (u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime)
                });
        unsafe {
            let _ = CloseHandle(process);
        }
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
        (width > 0 && height > 0).then_some(PhysicalRect {
            x: rect.left,
            y: rect.top,
            width,
            height,
        })
    }

    fn is_native_input_class(class_name: &str) -> bool {
        let class_name = class_name.to_ascii_lowercase();
        class_name == "edit" || class_name.starts_with("richedit") || class_name.contains(".edit.")
    }

    fn register_format(name: &str) -> Result<u32, PlatformError> {
        let wide = wide(name);
        let format = unsafe { RegisterClipboardFormatW(PCWSTR(wide.as_ptr())) };
        (format != 0)
            .then_some(format)
            .ok_or_else(|| PlatformError(format!("unable to register clipboard format {name}")))
    }

    fn format_available(format: u32) -> bool {
        unsafe { IsClipboardFormatAvailable(format) }.is_ok()
    }

    fn utf16_clipboard_text(bytes: &[u8]) -> String {
        let words = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .take_while(|word| *word != 0)
            .collect::<Vec<_>>();
        String::from_utf16_lossy(&words)
    }

    fn trim_trailing_nuls(mut bytes: Vec<u8>) -> Vec<u8> {
        while bytes.last() == Some(&0) {
            bytes.pop();
        }
        bytes
    }

    fn nul_terminated(bytes: &[u8]) -> Vec<u8> {
        let mut result = bytes.to_vec();
        if result.last() != Some(&0) {
            result.push(0);
        }
        result
    }

    fn dib_to_bmp(dib: &[u8]) -> Option<Vec<u8>> {
        if dib.len() < 40 {
            return None;
        }
        let header_size = u32::from_le_bytes(dib.get(0..4)?.try_into().ok()?) as usize;
        if header_size < 40 || header_size > dib.len() {
            return None;
        }
        let bit_count = u16::from_le_bytes(dib.get(14..16)?.try_into().ok()?);
        let colors_used = u32::from_le_bytes(dib.get(32..36)?.try_into().ok()?) as usize;
        let palette_entries = if colors_used > 0 {
            colors_used
        } else if bit_count <= 8 {
            1_usize << bit_count
        } else {
            0
        };
        let compression = u32::from_le_bytes(dib.get(16..20)?.try_into().ok()?);
        let external_masks = if header_size == 40 {
            match compression {
                3 => 12,
                6 => 16,
                _ => 0,
            }
        } else {
            0
        };
        let pixel_offset = 14 + header_size + external_masks + palette_entries * 4;
        if pixel_offset > 14 + dib.len() {
            return None;
        }
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
        unsafe {
            windows::Win32::UI::WindowsAndMessaging::DefWindowProcW(window, message, wparam, lparam)
        }
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
