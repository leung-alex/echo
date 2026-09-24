//! Explicit Alt+V compatibility for terminal hosts lacking editable UIA ranges.
//! Plain-paste identities never authorize inline replacement. Warp has a separate,
//! explicit pointer-status fallback when precise input geometry is unavailable.
use super::*;

fn supported_host(class: &str, executable: &str) -> bool {
    match class {
        "ConsoleWindowClass" => true,
        "CASCADIA_HOSTING_WINDOW_CLASS" => executable.eq_ignore_ascii_case("WindowsTerminal.exe"),
        "Window Class" => executable.eq_ignore_ascii_case("warp.exe"),
        _ => false,
    }
}

impl FocusSnapshot {
    pub(crate) fn plain_paste_window_identity(&self) -> Option<PasteControlIdentity> {
        if self.indicator_only || self.native_blocked || !self.current() {
            return None;
        }
        let root = win(self.window_id as HWND);
        let class_name = native::window_class_name(root)?;
        let path = native::process_path(self.process_id)?;
        let executable = std::path::Path::new(&path).file_name()?.to_str()?;
        if !supported_host(&class_name, executable) {
            return None;
        }
        // An ordinary native child (including protected/read-only edits) must use
        // the normal control contract. Do not broaden authorization to sibling UI.
        if self.focused_handle != 0 && self.focused_handle != self.window_id {
            return None;
        }
        Some(PasteControlIdentity::PlainPasteWindow {
            focused_handle: self.focused_handle,
            class_name,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compatibility_is_limited_to_terminal_host_contracts() {
        assert!(supported_host("ConsoleWindowClass", "pwsh.exe"));
        assert!(supported_host("ConsoleWindowClass", "cmd.exe"));
        assert!(supported_host(
            "CASCADIA_HOSTING_WINDOW_CLASS",
            "WindowsTerminal.exe"
        ));
        assert!(supported_host("Window Class", "warp.exe"));
        for (class, exe) in [
            ("Window Class", "other.exe"),
            ("Chrome_WidgetWin_1", "warp.exe"),
            ("Edit", "cmd.exe"),
            ("CASCADIA_HOSTING_WINDOW_CLASS", "other.exe"),
        ] {
            assert!(!supported_host(class, exe));
        }
    }
}

// ConsoleWindowClass reports the client shell PID for its root HWND, but the
// caret and IME are owned by conhost. This endpoint is observation-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct InputStatusEndpoint {
    pub window: isize,
    pub input: isize,
    pub process: u32,
    pub started: u64,
    pub thread: u32,
}
impl FocusSnapshot {
    pub(crate) fn console_indicator(&self) -> Option<(InputStatusEndpoint, PopupAnchor)> {
        use windows_sys::Win32::UI::Input::Ime::ImmGetDefaultIMEWnd;
        if !self.current()
            || native::window_class_name(win(self.window_id as HWND))?.as_str()
                != "ConsoleWindowClass"
        {
            return None;
        }
        unsafe {
            let root = self.window_id as HWND;
            let ime = ImmGetDefaultIMEWnd(root);
            let mut process = 0;
            if ime.is_null() || GetWindowThreadProcessId(ime, &mut process) == 0 {
                return None;
            }
            let path = native::process_path(process)?;
            if !std::path::Path::new(&path)
                .file_name()?
                .to_str()?
                .eq_ignore_ascii_case("conhost.exe")
            {
                return None;
            }
            let mut info: GUITHREADINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
            if GetGUIThreadInfo(0, &mut info) == 0 || info.hwndFocus != root {
                return None;
            }
            let mut caret_process = 0;
            let native_caret = (!info.hwndCaret.is_null()
                && GetWindowThreadProcessId(info.hwndCaret, &mut caret_process) != 0
                && caret_process == process)
                .then(|| caret_rectangle(&info))
                .flatten();
            let (caret, source) = native_caret
                .map(|r| (r, AnchorSource::NativeCaret))
                .or_else(|| automation::query(self.clone(), true)?.anchor)?;
            if !super::anchor::caret_in_control(caret, native::window_rect(win(root)))
                || !self.current()
            {
                return None;
            }
            Some((
                InputStatusEndpoint {
                    window: ime as isize,
                    input: self.window_id,
                    process,
                    started: native::process_started_at(process)?,
                    thread: GetWindowThreadProcessId(self.window_id as HWND, std::ptr::null_mut()),
                },
                PopupAnchor {
                    geometry: geometry(caret),
                    source,
                },
            ))
        }
    }
}

impl FocusSnapshot {
    pub(crate) fn terminal_tsf_indicator(&self) -> Option<(InputStatusEndpoint, PopupAnchor)> {
        if !self.current() || self.native_blocked || self.focused_handle != self.window_id {
            return None;
        }
        let class = native::window_class_name(win(self.window_id as HWND))?;
        let path = native::process_path(self.process_id)?;
        if class == "ConsoleWindowClass"
            || !supported_host(&class, std::path::Path::new(&path).file_name()?.to_str()?)
        {
            return None;
        }
        let result = crate::ime_observer::Observer::geometry_only(
            self.focused_handle,
            self.process_id,
            self.process_started_at,
        );
        let observer = result.ok()?;
        let (bounds, source) = observer
            .input_caret()
            .map(|r| (r, AnchorSource::InputMethodCaret))
            .or_else(|| {
                observer
                    .input_bounds()
                    .map(|r| (r, AnchorSource::InputControl))
            })?;
        let host = native::window_rect(win(self.window_id as HWND))?;
        if !valid_rect(bounds)
            || bounds.x < host.x
            || bounds.y < host.y
            || bounds.x + bounds.width > host.x + host.width
            || bounds.y + bounds.height > host.y + host.height
            || (bounds.width >= host.width - 32 && bounds.height >= host.height - 64)
            || !self.current()
        {
            return None;
        }
        Some((
            InputStatusEndpoint {
                window: self.focused_handle,
                input: self.focused_handle,
                process: self.process_id,
                started: self.process_started_at,
                thread: unsafe {
                    GetWindowThreadProcessId(self.focused_handle as HWND, std::ptr::null_mut())
                },
            },
            PopupAnchor {
                geometry: geometry(bounds),
                source,
            },
        ))
    }
}

impl FocusSnapshot {
    fn warp_root(&self) -> bool {
        if !self.current() || self.native_blocked || self.focused_handle != self.window_id {
            return false;
        }
        let Some(path) = native::process_path(self.process_id) else {
            return false;
        };
        native::window_class_name(win(self.window_id as HWND)).as_deref() == Some("Window Class")
            && std::path::Path::new(&path)
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.eq_ignore_ascii_case("warp.exe"))
    }
    pub(crate) fn warp_pointer_anchor(&self) -> Option<PopupAnchor> {
        if !self.warp_root() || self.anchor.source != AnchorSource::Window {
            return None;
        }
        Some(PopupAnchor {
            geometry: geometry(self.pointer?),
            source: AnchorSource::Pointer,
        })
    }
    pub(crate) fn warp_pointer_indicator(&self) -> Option<(InputStatusEndpoint, PopupAnchor)> {
        if !self.warp_root() {
            return None;
        }
        let bounds = self.pointer?;
        Some((
            InputStatusEndpoint {
                window: self.focused_handle,
                input: self.focused_handle,
                process: self.process_id,
                started: self.process_started_at,
                thread: unsafe {
                    GetWindowThreadProcessId(self.focused_handle as HWND, std::ptr::null_mut())
                },
            },
            PopupAnchor {
                geometry: geometry(bounds),
                source: AnchorSource::Pointer,
            },
        ))
    }
}
