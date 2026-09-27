//! Explicit Alt+V compatibility for terminal hosts lacking editable UIA ranges.
//! Plain-paste identities never authorize inline replacement. Warp may use a
//! captured pointer for that explicit paste placement and, for Warp's
//! renderer-only passive status, as the last-resort position.
use super::*;

fn is_warp_host(class: &str, executable: &str) -> bool {
    class == "Window Class" && executable.eq_ignore_ascii_case("warp.exe")
}

fn supported_host(class: &str, executable: &str) -> bool {
    match class {
        "ConsoleWindowClass" => true,
        "CASCADIA_HOSTING_WINDOW_CLASS" => executable.eq_ignore_ascii_case("WindowsTerminal.exe"),
        "Window Class" => is_warp_host(class, executable),
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
        assert!(is_warp_host("Window Class", "WARP.EXE"));
        for (class, exe) in [
            ("Window Class", "other.exe"),
            ("Chrome_WidgetWin_1", "warp.exe"),
            ("Edit", "cmd.exe"),
            ("CASCADIA_HOSTING_WINDOW_CLASS", "other.exe"),
        ] {
            assert!(!supported_host(class, exe));
        }
        assert!(!is_warp_host("Chrome_WidgetWin_1", "warp.exe"));
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
        let executable = std::path::Path::new(&path).file_name()?.to_str()?;
        // Warp's TSF GetTextExt has been observed returning a zero-height,
        // off-screen rectangle. Keep that generic observer from overwriting
        // Warp's validated UIA/MSAA or stable input-lane result later.
        if is_warp_host(&class, executable)
            || class == "ConsoleWindowClass"
            || !supported_host(&class, executable)
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
        native::window_class_name(win(self.window_id as HWND))
            .as_deref()
            .zip(
                std::path::Path::new(&path)
                    .file_name()
                    .and_then(|s| s.to_str()),
            )
            .is_some_and(|(class, executable)| is_warp_host(class, executable))
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

    /// Warp's root HWND can expose a valid focused caret through UIA/MSAA even
    /// when its TSF document only reports whole-window bounds. Keep the
    /// observation-only path separate from the explicit pointer anchor used by
    /// Quick Insert. Exact candidates are bound to the current Warp
    /// process/window and must look like a real caret inside the host; when
    /// Warp exposes no caret, use its bounded command-input lane estimate.
    pub(crate) fn warp_indicator(&self) -> Option<(InputStatusEndpoint, PopupAnchor)> {
        if !self.warp_root() {
            return None;
        }
        let host = native::window_rect(win(self.window_id as HWND))?;
        let endpoint = InputStatusEndpoint {
            window: self.window_id,
            input: self.window_id,
            process: self.process_id,
            started: self.process_started_at,
            thread: unsafe {
                GetWindowThreadProcessId(self.window_id as HWND, std::ptr::null_mut())
            },
        };
        if endpoint.thread == 0 {
            return None;
        }

        let accept = |anchor: PopupAnchor| {
            warp_geometry_allowed(anchor.geometry.target, anchor.source, host)
                .then_some((endpoint, anchor))
        };

        // Capture can observe a native caret before Echo is shown.  Reuse it
        // only when it is still a precise source; capture_for_indicator's
        // editor-leading-edge estimate is intentionally rejected here.
        if let Some(value) = accept(self.anchor) {
            return Some(value);
        }

        // The bounded MTA query validates the focused element's process,
        // foreground identity, and UIA/MSAA caret before returning geometry.
        // Warp's read-only document therefore does not need a paste identity.
        if let Some((rect, source)) =
            automation::query(self.clone(), true).and_then(|probe| probe.anchor)
        {
            if let Some(value) = accept(PopupAnchor {
                geometry: geometry(rect),
                source,
            }) {
                return Some(value);
            }
        }

        // Warp's current renderer exposes neither a native hwndCaret nor a
        // UIA/MSAA text range. Keep the estimate bounded to the lower command
        // input lane. Exact providers above always win; the captured pointer
        // is retained only as a last resort if the host is too small to model.
        warp_input_lane(host)
            .map(|rect| {
                (
                    endpoint,
                    PopupAnchor {
                        geometry: geometry(rect),
                        source: AnchorSource::EditorLeadingEdge,
                    },
                )
            })
            .or_else(|| self.warp_pointer_anchor().map(|anchor| (endpoint, anchor)))
    }
}

fn warp_input_lane(host: PhysicalRect) -> Option<PhysicalRect> {
    if host.width < 320 || host.height < 200 {
        return None;
    }
    let lane_height = (host.height / 12).clamp(44, 96);
    let line_height = (lane_height * 7 / 16).clamp(18, 28);
    // Warp's root window includes its left session rail. Keep this estimate
    // tied to the root dimensions instead of inventing a text-width model.
    let content_origin = if host.width >= 960 {
        (host.width / 4).clamp(240, 360)
    } else {
        0
    };
    let inset = (host.width / 64).clamp(16, 32);
    Some(PhysicalRect {
        x: host.x.saturating_add(content_origin).saturating_add(inset),
        y: host
            .y
            .saturating_add(host.height)
            .saturating_sub(lane_height)
            .saturating_add((lane_height - line_height) / 2),
        width: 1,
        height: line_height,
    })
}

fn warp_precise_source(source: AnchorSource) -> bool {
    matches!(
        source,
        AnchorSource::NativeCaret
            | AnchorSource::AutomationCaret
            | AnchorSource::AccessibleCaret
            | AnchorSource::InputMethodCaret
    )
}

fn warp_geometry_allowed(rect: PhysicalRect, source: AnchorSource, host: PhysicalRect) -> bool {
    if !warp_precise_source(source)
        || !super::anchor::caret_in_control(rect, Some(host))
        || rect.x < host.x
        || rect.y < host.y
        || rect.x.saturating_add(rect.width) > host.x.saturating_add(host.width)
        || rect.y.saturating_add(rect.height) > host.y.saturating_add(host.height)
    {
        return false;
    }
    !(rect.width >= host.width.saturating_sub(32) && rect.height >= host.height.saturating_sub(64))
}

#[cfg(test)]
mod warp_geometry_tests {
    use super::*;

    fn host() -> PhysicalRect {
        PhysicalRect {
            x: 100,
            y: 200,
            width: 1200,
            height: 800,
        }
    }

    #[test]
    fn warp_accepts_precise_caret_sources_inside_host() {
        let h = host();
        let caret = PhysicalRect {
            x: 240,
            y: 420,
            width: 2,
            height: 24,
        };
        for source in [
            AnchorSource::NativeCaret,
            AnchorSource::AutomationCaret,
            AnchorSource::AccessibleCaret,
            AnchorSource::InputMethodCaret,
        ] {
            assert!(warp_geometry_allowed(caret, source, h));
        }
    }

    #[test]
    fn warp_rejects_pointer_control_adjacent_and_whole_window_bounds() {
        let h = host();
        let caret = PhysicalRect {
            x: 240,
            y: 420,
            width: 2,
            height: 24,
        };
        assert!(!warp_geometry_allowed(caret, AnchorSource::Pointer, h));
        assert!(!warp_geometry_allowed(caret, AnchorSource::InputControl, h));
        assert!(!warp_geometry_allowed(
            caret,
            AnchorSource::AdjacentCharacter,
            h
        ));
        let whole = PhysicalRect {
            x: h.x,
            y: h.y,
            width: h.width,
            height: h.height,
        };
        assert!(!warp_geometry_allowed(whole, AnchorSource::NativeCaret, h));
    }

    #[test]
    fn warp_input_lane_is_bounded_and_stable() {
        let h = host();
        let lane = warp_input_lane(h).expect("normal Warp host has an input lane");
        assert_eq!(lane.x, h.x + h.width / 4 + h.width / 64);
        assert!(lane.x >= h.x && lane.x < h.x + h.width);
        assert!(lane.y >= h.y && lane.y + lane.height <= h.y + h.height);
        assert_eq!(lane.width, 1);
        assert!(lane.height >= 18 && lane.height <= 28);
    }
}
