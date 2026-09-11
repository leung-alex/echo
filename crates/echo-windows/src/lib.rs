#![cfg_attr(windows, allow(unsafe_op_in_unsafe_fn))]

#[cfg(windows)]
pub mod allocation;

#[cfg(windows)]
pub mod focus;
#[cfg(windows)]
pub mod inline;

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
        PasteControlIdentity, PasteDelivery, PasteDeliveryFailure, PasteTarget, PhysicalRect,
        PlatformChange, PlatformChangePublisher, PlatformChangeSubscription, PlatformError,
        SourceContext,
    };
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, HGLOBAL, HWND, LPARAM, RECT, WPARAM};
    use windows::Win32::Security::{
        GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel,
        TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
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
        GetCurrentProcess, GetProcessTimes, OpenProcess, OpenProcessToken,
        QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows::Win32::UI::Accessibility::{
        IUIAutomation, IUIAutomationElement, UIA_ComboBoxControlTypeId, UIA_DocumentControlTypeId,
        UIA_EditControlTypeId,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        IsWindowEnabled, SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
        VIRTUAL_KEY, VK_CONTROL, VK_V,
    };
    use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, GetAncestor, GetClassNameW, GetForegroundWindow,
        GetGUIThreadInfo, GetWindowLongW, GetWindowRect, GetWindowTextLengthW, GetWindowTextW,
        GetWindowThreadProcessId, IsWindow, PostMessageW, RegisterClassW, SetForegroundWindow,
        CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, ES_PASSWORD, ES_READONLY, GA_ROOT, GWL_STYLE,
        HWND_MESSAGE, WM_CLIPBOARDUPDATE, WM_CLOSE, WM_PASTE, WNDCLASSW, WS_OVERLAPPED,
    };

    const OPEN_ATTEMPTS: usize = 5;

    mod clipboard;
    use clipboard::ClipboardGuard;

    pub struct WindowsPlatform {
        changes: PlatformChangePublisher,
        clipboard_window: Arc<AtomicIsize>,
        clipboard_worker: Mutex<Option<JoinHandle<()>>>,
        source: Arc<Mutex<SourceContext>>,
        captured_paste_target: Mutex<Option<PasteTarget>>,
        inline: Mutex<Option<crate::inline::InlineController>>,
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
                captured_paste_target: Mutex::new(None),
                inline: Mutex::new(None),
            }
        }

        /// Adopt only a current epoch on the domain worker, before any paste.
        pub fn set_inline_controller(&self, controller: crate::inline::InlineController) {
            *self.inline.lock().unwrap_or_else(|e| e.into_inner()) = Some(controller);
        }

        pub fn adopt_captured_target(&self, target: Option<PasteTarget>) {
            *self
                .captured_paste_target
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = target;
        }

        pub fn capture_activation(
            &self,
            snapshot: &crate::focus::FocusSnapshot,
        ) -> crate::focus::CapturedActivation {
            let result = snapshot.capture_target();
            *self
                .captured_paste_target
                .lock()
                .unwrap_or_else(|e| e.into_inner()) = result.target.clone();
            result
        }

        fn validate_captured_target_before_clipboard(&self) -> Result<(), PlatformError> {
            let target = self
                .captured_paste_target
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .clone();
            let Some(target) = target else {
                return Ok(());
            };
            validate_target_identity(&target).map_err(target_delivery_error)?;
            validate_target_integrity(target.process_id).map_err(target_delivery_error)
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
            if !ClipboardGuard::capture_allowed()? {
                return Ok(None);
            }
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
            self.validate_captured_target_before_clipboard()?;
            let prepared = clipboard::PreparedClipboard::new(representations)?;
            let owner = HWND(self.clipboard_window.load(Ordering::Acquire) as *mut c_void);
            prepared.publish(owner)?;
            // Close/publish before recording the token; Windows may synthesize formats.
            Ok(self.clipboard_sequence())
        }

        fn capture_target(&self) -> Result<Option<PasteTarget>, PlatformError> {
            let target = capture_native_target();
            *self
                .captured_paste_target
                .lock()
                .unwrap_or_else(|error| error.into_inner()) =
                target.as_ref().ok().cloned().flatten();
            target
        }

        fn inline_preflight(
            &self,
            ticket: echo_engine::InlineTicket,
            representations: &[ClipboardRepresentation],
        ) -> Result<(), PlatformError> {
            let controller = self
                .inline
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
                .ok_or_else(|| PlatformError("Inline completion is not running".into()))?;
            controller
                .preflight(ticket, representations)
                .map_err(PlatformError)
        }

        fn replace_inline(
            &self,
            ticket: echo_engine::InlineTicket,
            clipboard_sequence: u64,
        ) -> Result<PasteDelivery, PlatformError> {
            let controller = self
                .inline
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
                .ok_or_else(|| PlatformError("Inline completion is not running".into()))?;
            controller
                .paste(ticket, clipboard_sequence)
                .map_err(PlatformError)
        }

        fn reset_paste_window_session(&self) {
            *self
                .captured_paste_target
                .lock()
                .unwrap_or_else(|error| error.into_inner()) = None;
        }

        fn paste_preflight(
            &self,
            target: &PasteTarget,
        ) -> Result<Option<PasteDeliveryFailure>, PlatformError> {
            Ok(validate_target_identity(target)
                .and_then(|hwnd| foreground_allows_restore(hwnd))
                .and_then(|_| validate_target_integrity(target.process_id))
                .and_then(|_| modifiers_released())
                .err())
        }

        fn paste_to_target(&self, target: &PasteTarget) -> Result<PasteDelivery, PlatformError> {
            let hwnd = match validate_target_identity(target) {
                Ok(hwnd) => hwnd,
                Err(reason) => return Ok(PasteDelivery::Failed(reason)),
            };
            if let Err(reason) = validate_target_integrity(target.process_id) {
                return Ok(PasteDelivery::Failed(reason));
            }
            if let Err(reason) = foreground_allows_restore(hwnd) {
                return Ok(PasteDelivery::Failed(reason));
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
            let current_control = match &target.focused_control {
                Some(PasteControlIdentity::NativeWindow { .. }) => {
                    focused_native_control_from_gui(hwnd, target.process_id)
                }
                _ => focused_input_identity(hwnd, target.process_id),
            };
            let Some(current_control) = current_control else {
                return Ok(PasteDelivery::Failed(
                    PasteDeliveryFailure::InputUnavailable,
                ));
            };
            if target.focused_control.as_ref() != Some(&current_control) {
                return Ok(PasteDelivery::Failed(
                    PasteDeliveryFailure::InputUnavailable,
                ));
            }
            if let Err(reason) = modifiers_released() {
                return Ok(PasteDelivery::Failed(reason));
            }
            if unsafe { GetForegroundWindow() } != hwnd {
                return Ok(PasteDelivery::Failed(
                    PasteDeliveryFailure::OriginalWindowUnavailable,
                ));
            }
            match current_control {
                PasteControlIdentity::NativeWindow { handle, .. } => {
                    let control = HWND(handle as *mut c_void);
                    let mut result = 0;
                    let ok = unsafe {
                        windows_sys::Win32::UI::WindowsAndMessaging::SendMessageTimeoutW(
                            control.0,
                            WM_PASTE,
                            0,
                            0,
                            windows_sys::Win32::UI::WindowsAndMessaging::SMTO_ABORTIFHUNG,
                            250,
                            &mut result,
                        )
                    };
                    Ok(if ok != 0 {
                        PasteDelivery::Pasted
                    } else {
                        PasteDelivery::Failed(PasteDeliveryFailure::NativePasteFailed)
                    })
                }
                PasteControlIdentity::AutomationRuntimeId(_) => Ok(match send_paste_shortcut() {
                    Ok(()) => PasteDelivery::Pasted,
                    Err(reason) => PasteDelivery::Failed(reason),
                }),
            }
        }
    }

    fn foreground_allows_restore(target: HWND) -> Result<(), PasteDeliveryFailure> {
        let foreground = unsafe { GetForegroundWindow() };
        if foreground.0.is_null() || foreground == target {
            return Ok(());
        }
        let mut pid = 0;
        unsafe {
            GetWindowThreadProcessId(foreground, Some(&mut pid));
        }
        if pid == std::process::id() {
            Ok(())
        } else {
            Err(PasteDeliveryFailure::OriginalWindowUnavailable)
        }
    }

    pub(crate) fn modifiers_released() -> Result<(), PasteDeliveryFailure> {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
            GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
        };
        let busy = [VK_MENU, VK_CONTROL, VK_SHIFT, VK_LWIN, VK_RWIN]
            .into_iter()
            .any(|key| unsafe { GetAsyncKeyState(i32::from(key)) } < 0);
        if busy {
            Err(PasteDeliveryFailure::ModifierKeysBusy)
        } else {
            Ok(())
        }
    }

    /// Verify the retained plain-text representation under the clipboard lock as
    /// well as its generation. This closes the writer-close/token-read race; a
    /// generation from an unrelated writer must never authorize its payload.
    pub(crate) fn validate_inline_clipboard(sequence: u64, expected: &str) -> bool {
        let Ok(_guard) = ClipboardGuard::open() else {
            return false;
        };
        if u64::from(unsafe { GetClipboardSequenceNumber() }) != sequence {
            return false;
        }
        let maximum = (echo_engine::MAX_COMPOSER_UNITS + 1) * 2;
        let Ok(size) = ClipboardGuard::global_size(13) else {
            return false;
        };
        if size == 0 || size > maximum {
            return false;
        }
        let Ok(bytes) = ClipboardGuard::read_global(13) else {
            return false;
        };
        utf16_clipboard_text(&bytes) == expected
            && u64::from(unsafe { GetClipboardSequenceNumber() }) == sequence
    }

    pub(crate) fn send_paste_shortcut() -> Result<(), PasteDeliveryFailure> {
        modifiers_released()?;
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

    /// One non-text selection step. The caller verifies target identity before
    /// dispatch and observes the exact range after each step; key counts never
    /// authorize a paste.
    pub(crate) fn extend_selection_left() -> Result<(), PasteDeliveryFailure> {
        modifiers_released()?;
        let shift = VIRTUAL_KEY(0x10);
        let left = VIRTUAL_KEY(0x25);
        let inputs = [
            key_input(shift, false),
            key_input(left, false),
            key_input(left, true),
            key_input(shift, true),
        ];
        if unsafe { SendInput(&inputs, size_of::<INPUT>() as i32) } == inputs.len() as u32 {
            Ok(())
        } else {
            let release = [key_input(left, true), key_input(shift, true)];
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
                    dwExtraInfo: crate::inline::INJECTED_TAG,
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

    fn capture_native_target() -> Result<Option<PasteTarget>, PlatformError> {
        Ok(crate::focus::FocusSnapshot::capture()
            .capture_target()
            .target)
    }

    fn focused_input_identity(window: HWND, process_id: u32) -> Option<PasteControlIdentity> {
        crate::focus::focused_identity(window, process_id)
    }

    pub(crate) fn focused_native_control_from_gui(
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

    pub(crate) fn native_control_belongs_to(control: HWND, window: HWND, process_id: u32) -> bool {
        if control.0.is_null() || window.0.is_null() {
            return false;
        }
        let mut owner_process_id = 0;
        (unsafe { GetWindowThreadProcessId(control, Some(&mut owner_process_id)) }) != 0
            && owner_process_id == process_id
            && unsafe { GetAncestor(control, GA_ROOT) } == window
    }

    pub(crate) fn automation_element_is_editable(element: &IUIAutomationElement) -> bool {
        automation_element_has_input_focus(element)
            && unsafe {
                element.CurrentControlType().is_ok_and(|control_type| {
                    control_type == UIA_EditControlTypeId
                        || control_type == UIA_DocumentControlTypeId
                        || control_type == UIA_ComboBoxControlTypeId
                })
            }
    }

    pub(crate) fn automation_element_has_input_focus(element: &IUIAutomationElement) -> bool {
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
        }
    }

    pub(crate) fn automation_element_belongs_to(
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

    pub(crate) fn automation_runtime_id(element: &IUIAutomationElement) -> Option<Vec<i32>> {
        let array = unsafe { element.GetRuntimeId() }.ok()?;
        if array.is_null() {
            return None;
        }
        let result = (|| {
            let lower = unsafe { SafeArrayGetLBound(array, 1) }.ok()?;
            let upper = unsafe { SafeArrayGetUBound(array, 1) }.ok()?;
            let length = usize::try_from(upper.checked_sub(lower)?.checked_add(1)?).ok()?;
            if length == 0 || length > 128 {
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

    pub(crate) fn process_started_at(process_id: u32) -> Option<u64> {
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

    pub(crate) fn validate_target_identity(
        target: &PasteTarget,
    ) -> Result<HWND, PasteDeliveryFailure> {
        let hwnd = HWND(target.window_id as *mut c_void);
        if hwnd.0.is_null() || !unsafe { IsWindow(Some(hwnd)) }.as_bool() {
            return Err(PasteDeliveryFailure::OriginalWindowUnavailable);
        }
        let mut process_id = 0;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut process_id)) };
        if process_id != target.process_id
            || process_started_at(process_id).unwrap_or_default() != target.process_started_at
            || window_class_name(hwnd).as_deref() != Some(target.window_class.as_str())
        {
            return Err(PasteDeliveryFailure::OriginalWindowUnavailable);
        }
        Ok(hwnd)
    }

    fn target_delivery_error(reason: PasteDeliveryFailure) -> PlatformError {
        PlatformError(format!(
            "paste target was rejected before clipboard staging: {reason:?}"
        ))
    }

    pub(crate) fn validate_target_integrity(process_id: u32) -> Result<(), PasteDeliveryFailure> {
        validate_integrity_levels(
            current_process_integrity_level(),
            process_integrity_level(process_id),
        )
    }

    fn validate_integrity_levels(
        current: Option<u32>,
        target: Option<u32>,
    ) -> Result<(), PasteDeliveryFailure> {
        match (current, target) {
            (Some(current), Some(target)) if target <= current => Ok(()),
            _ => Err(PasteDeliveryFailure::ElevatedTarget),
        }
    }

    fn current_process_integrity_level() -> Option<u32> {
        token_integrity_level(unsafe { GetCurrentProcess() })
    }

    fn process_integrity_level(process_id: u32) -> Option<u32> {
        let process =
            unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, process_id) }.ok()?;
        let result = token_integrity_level(process);
        unsafe {
            let _ = CloseHandle(process);
        }
        result
    }

    fn token_integrity_level(process: HANDLE) -> Option<u32> {
        let mut token = HANDLE::default();
        unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()? };
        let result = query_token_integrity_level(token);
        unsafe {
            let _ = CloseHandle(token);
        }
        result
    }

    fn query_token_integrity_level(token: HANDLE) -> Option<u32> {
        let mut required = 0_u32;
        let _ = unsafe { GetTokenInformation(token, TokenIntegrityLevel, None, 0, &mut required) };
        if required < size_of::<TOKEN_MANDATORY_LABEL>() as u32 {
            return None;
        }
        let word_count = (required as usize + size_of::<usize>() - 1) / size_of::<usize>();
        let mut buffer = vec![0_usize; word_count];
        unsafe {
            GetTokenInformation(
                token,
                TokenIntegrityLevel,
                Some(buffer.as_mut_ptr().cast()),
                required,
                &mut required,
            )
            .ok()?;
        }
        let label = unsafe { &*buffer.as_ptr().cast::<TOKEN_MANDATORY_LABEL>() };
        let sid = label.Label.Sid;
        if sid.0.is_null() {
            return None;
        }
        let count = unsafe { GetSidSubAuthorityCount(sid).as_ref() }.copied()? as u32;
        if count == 0 {
            return None;
        }
        unsafe { GetSidSubAuthority(sid, count - 1).as_ref() }.copied()
    }

    pub(crate) fn window_class_name(window: HWND) -> Option<String> {
        let mut buffer = [0_u16; 256];
        let length = unsafe { GetClassNameW(window, &mut buffer) };
        (length > 0).then(|| String::from_utf16_lossy(&buffer[..length as usize]))
    }

    pub(crate) fn window_rect(window: HWND) -> Option<PhysicalRect> {
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

    pub(crate) fn is_native_input_class(class_name: &str) -> bool {
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

    pub(crate) fn inline_image_fingerprint(bmp: &[u8]) -> Option<(u32, usize, [u8; 32])> {
        use sha2::{Digest, Sha256};
        if bmp.len() >= 54 && &bmp[..2] == b"BM" {
            Some((8, bmp.len() - 14, Sha256::digest(&bmp[14..]).into()))
        } else if bmp.starts_with(b"\x89PNG\r\n\x1a\n") {
            // Validate the retained original PNG, which PreparedClipboard also
            // publishes. Avoid decoding a second full frame just to hash it.
            Some((
                register_format("PNG").ok()?,
                bmp.len(),
                Sha256::digest(bmp).into(),
            ))
        } else {
            None
        }
    }

    pub(crate) fn validate_inline_image(sequence: u64, expected: &(u32, usize, [u8; 32])) -> bool {
        use sha2::{Digest, Sha256};
        let Ok(_guard) = ClipboardGuard::open() else {
            return false;
        };
        if u64::from(unsafe { GetClipboardSequenceNumber() }) != sequence {
            return false;
        }
        let Ok(size) = ClipboardGuard::global_size(expected.0) else {
            return false;
        };
        if size < expected.1 || size > expected.1.saturating_add(16) {
            return false;
        }
        let Ok(bytes) = ClipboardGuard::read_global(expected.0) else {
            return false;
        };
        let actual: [u8; 32] = Sha256::digest(&bytes[..expected.1]).into();
        actual == expected.2 && u64::from(unsafe { GetClipboardSequenceNumber() }) == sequence
    }

    fn bmp_to_dib(bmp: &[u8]) -> Option<Vec<u8>> {
        (bmp.len() >= 54 && &bmp[..2] == b"BM").then(|| bmp[14..].to_vec())
    }

    fn image_to_dib(bytes: &[u8]) -> Option<Vec<u8>> {
        if let Some(dib) = bmp_to_dib(bytes) {
            return Some(dib);
        }
        if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
            return None;
        }
        let decoded = image::load_from_memory_with_format(bytes, image::ImageFormat::Png).ok()?;
        let mut bmp = std::io::Cursor::new(Vec::new());
        decoded
            .write_to(&mut bmp, image::ImageOutputFormat::Bmp)
            .ok()?;
        bmp_to_dib(bmp.get_ref())
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

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn higher_integrity_target_is_rejected_before_delivery() {
            assert_eq!(
                validate_integrity_levels(Some(0x2000), Some(0x3000)),
                Err(PasteDeliveryFailure::ElevatedTarget)
            );
            assert_eq!(
                validate_integrity_levels(Some(0x3000), Some(0x2000)),
                Ok(())
            );
            assert_eq!(
                validate_integrity_levels(Some(0x3000), Some(0x3000)),
                Ok(())
            );
        }

        #[test]
        fn inability_to_prove_integrity_fails_closed() {
            assert_eq!(
                validate_integrity_levels(None, Some(0x3000)),
                Err(PasteDeliveryFailure::ElevatedTarget)
            );
            assert_eq!(
                validate_integrity_levels(Some(0x3000), None),
                Err(PasteDeliveryFailure::ElevatedTarget)
            );
        }
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

#[cfg(windows)]
pub mod shell;
