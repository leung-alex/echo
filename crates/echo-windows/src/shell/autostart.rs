//! Per-user Windows logon registration for the resident Echo process.
use super::common::wide;
use std::{path::Path, ptr::null_mut};
use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW, RegSetValueExW,
    HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
};

const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const VALUE_NAME: &str = "Echo";
pub const STARTUP_ARGUMENT: &str = "--startup";

trait RunValueStore {
    fn set(&mut self, command: &str) -> Result<(), String>;
    fn delete(&mut self) -> Result<(), String>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct AutoStartController;

impl AutoStartController {
    pub fn new() -> Self {
        Self
    }

    pub fn apply(&self, enabled: bool) -> Result<(), String> {
        let command = if enabled {
            let executable = std::env::current_exe().map_err(|e| e.to_string())?;
            Some(startup_command(&executable)?)
        } else {
            None
        };
        let mut store = WindowsRunValueStore;
        sync_run_value(&mut store, enabled, command.as_deref())
    }
}

fn sync_run_value(
    store: &mut impl RunValueStore,
    enabled: bool,
    command: Option<&str>,
) -> Result<(), String> {
    if enabled {
        store.set(command.ok_or_else(|| "missing startup command".to_string())?)
    } else {
        store.delete()
    }
}

struct WindowsRunValueStore;

impl RunValueStore for WindowsRunValueStore {
    fn set(&mut self, command: &str) -> Result<(), String> {
        let key_path = wide(RUN_KEY);
        let value_name = wide(VALUE_NAME);
        let command = wide(command);
        unsafe {
            let mut key = null_mut();
            let mut disposition = 0;
            let result = RegCreateKeyExW(
                HKEY_CURRENT_USER,
                key_path.as_ptr(),
                0,
                null_mut(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                null_mut(),
                &mut key,
                &mut disposition,
            );
            if result != ERROR_SUCCESS {
                return Err(registry_error("create startup key", result));
            }
            let result = RegSetValueExW(
                key,
                value_name.as_ptr(),
                0,
                REG_SZ,
                command.as_ptr().cast(),
                (command.len() * std::mem::size_of::<u16>()) as u32,
            );
            let close_result = RegCloseKey(key);
            if result != ERROR_SUCCESS {
                return Err(registry_error("write startup value", result));
            }
            if close_result != ERROR_SUCCESS {
                return Err(registry_error("close startup key", close_result));
            }
            Ok(())
        }
    }

    fn delete(&mut self) -> Result<(), String> {
        let key_path = wide(RUN_KEY);
        let value_name = wide(VALUE_NAME);
        unsafe {
            let mut key = null_mut();
            let result = RegOpenKeyExW(
                HKEY_CURRENT_USER,
                key_path.as_ptr(),
                0,
                KEY_SET_VALUE,
                &mut key,
            );
            if result == ERROR_FILE_NOT_FOUND {
                return Ok(());
            }
            if result != ERROR_SUCCESS {
                return Err(registry_error("open startup key", result));
            }
            let result = RegDeleteValueW(key, value_name.as_ptr());
            let close_result = RegCloseKey(key);
            if result != ERROR_SUCCESS && result != ERROR_FILE_NOT_FOUND {
                return Err(registry_error("delete startup value", result));
            }
            if close_result != ERROR_SUCCESS {
                return Err(registry_error("close startup key", close_result));
            }
            Ok(())
        }
    }
}

fn startup_command(executable: &Path) -> Result<String, String> {
    let executable = executable
        .to_str()
        .ok_or_else(|| "Echo executable path is not valid Unicode".to_string())?;
    Ok(format!(
        "{} {STARTUP_ARGUMENT}",
        quote_windows_argument(executable)
    ))
}

fn quote_windows_argument(value: &str) -> String {
    let mut result = String::with_capacity(value.len() + 2);
    result.push('"');
    let mut backslashes = 0;
    for character in value.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                result.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
                result.push('"');
                backslashes = 0;
            }
            _ => {
                result.extend(std::iter::repeat_n('\\', backslashes));
                result.push(character);
                backslashes = 0;
            }
        }
    }
    result.extend(std::iter::repeat_n('\\', backslashes * 2));
    result.push('"');
    result
}

fn registry_error(operation: &str, code: u32) -> String {
    let detail = std::io::Error::from_raw_os_error(code as i32);
    format!("Could not {operation}: {detail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeRunValueStore {
        value: Option<String>,
        set_calls: usize,
        delete_calls: usize,
        fail_set: bool,
        fail_delete: bool,
    }

    impl RunValueStore for FakeRunValueStore {
        fn set(&mut self, command: &str) -> Result<(), String> {
            self.set_calls += 1;
            if self.fail_set {
                return Err("set failed".into());
            }
            self.value = Some(command.into());
            Ok(())
        }

        fn delete(&mut self) -> Result<(), String> {
            self.delete_calls += 1;
            if self.fail_delete {
                return Err("delete failed".into());
            }
            self.value = None;
            Ok(())
        }
    }

    #[test]
    fn startup_command_quotes_paths_with_spaces() {
        assert_eq!(
            startup_command(Path::new(r"C:\Program Files\Echo\Echo.exe")).unwrap(),
            r#""C:\Program Files\Echo\Echo.exe" --startup"#
        );
    }

    #[test]
    fn startup_command_escapes_embedded_quotes_and_trailing_slashes() {
        assert_eq!(
            quote_windows_argument(r#"C:\Echo\"preview\"\"#),
            r#""C:\Echo\\\"preview\\\"\\""#
        );
    }

    #[test]
    fn startup_registration_is_idempotent_and_only_owns_echo_value() {
        let mut store = FakeRunValueStore::default();
        sync_run_value(&mut store, true, Some(r#""C:\Echo\Echo.exe" --startup"#)).unwrap();
        sync_run_value(&mut store, true, Some(r#""C:\Echo\Echo.exe" --startup"#)).unwrap();
        assert_eq!(store.set_calls, 2);
        assert_eq!(
            store.value.as_deref(),
            Some(r#""C:\Echo\Echo.exe" --startup"#)
        );
        sync_run_value(&mut store, false, None).unwrap();
        assert_eq!(store.delete_calls, 1);
        assert!(store.value.is_none());
    }

    #[test]
    fn startup_registration_propagates_backend_failures() {
        let mut store = FakeRunValueStore {
            fail_set: true,
            ..Default::default()
        };
        let error = sync_run_value(&mut store, true, Some("echo --startup")).unwrap_err();
        assert_eq!(error, "set failed");

        let mut store = FakeRunValueStore {
            fail_delete: true,
            ..Default::default()
        };
        let error = sync_run_value(&mut store, false, None).unwrap_err();
        assert_eq!(error, "delete failed");
    }
}
