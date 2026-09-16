//! Explicit Alt+V compatibility for terminal hosts lacking editable UIA ranges.
//! Never use these window identities for passive badges or inline replacement.
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
