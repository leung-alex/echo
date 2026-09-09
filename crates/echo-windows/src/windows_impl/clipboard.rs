//! Clipboard format I/O and native resource ownership.
use super::*;

pub(super) struct ClipboardGuard;

impl ClipboardGuard {
    pub(super) fn open() -> Result<Self, PlatformError> {
        Self::open_with_owner(None)
    }
    pub(super) fn open_for_write(owner: HWND) -> Result<Self, PlatformError> {
        if owner.0.is_null() || !unsafe { IsWindow(Some(owner)) }.as_bool() {
            return Err(PlatformError(
                "Clipboard owner window is unavailable".into(),
            ));
        }
        Self::open_with_owner(Some(owner))
    }
    fn open_with_owner(owner: Option<HWND>) -> Result<Self, PlatformError> {
        for attempt in 0..OPEN_ATTEMPTS {
            if unsafe { OpenClipboard(owner) }.is_ok() {
                return Ok(Self);
            }
            thread::sleep(Duration::from_millis(5 * (attempt + 1) as u64));
        }
        Err(PlatformError("Windows clipboard is busy".to_owned()))
    }

    pub(super) fn capture_allowed() -> Result<bool, PlatformError> {
        let exclude = register_format("ExcludeClipboardContentFromMonitorProcessing")?;
        if format_available(exclude) {
            return Ok(false);
        }
        let history = register_format("CanIncludeInClipboardHistory")?;
        if !format_available(history) {
            return Ok(true);
        }
        let handle = unsafe { GetClipboardData(history) }.map_err(platform_error)?;
        let global = HGLOBAL(handle.0);
        if unsafe { GlobalSize(global) } < size_of::<u32>() {
            return Ok(false);
        }
        let pointer = unsafe { GlobalLock(global) };
        if pointer.is_null() {
            return Err(PlatformError(
                "Unable to read clipboard history policy".into(),
            ));
        }
        let value = unsafe { pointer.cast::<u32>().read_unaligned() };
        let _ = unsafe { GlobalUnlock(global) };
        // Read only the policy DWORD. Unknown values fail closed; cloud policy is separate.
        Ok(value == 1)
    }
    pub(super) fn read_global(format: u32) -> Result<Vec<u8>, PlatformError> {
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

    pub(super) fn global_size(format: u32) -> Result<usize, PlatformError> {
        let handle = unsafe { GetClipboardData(format) }.map_err(platform_error)?;
        Ok(unsafe { GlobalSize(HGLOBAL(handle.0)) })
    }

    pub(super) fn read_global_if_allowed(
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

    pub(super) fn read_file_paths() -> Result<Vec<String>, PlatformError> {
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

    pub(super) fn read_file_paths_if_allowed(
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

    fn unicode_bytes(text: &str) -> Vec<u8> {
        let mut utf16 = text.encode_utf16().collect::<Vec<_>>();
        utf16.push(0);
        let bytes = unsafe {
            std::slice::from_raw_parts(utf16.as_ptr().cast::<u8>(), utf16.len() * size_of::<u16>())
        };
        bytes.to_vec()
    }

    fn file_bytes(bytes: &[u8]) -> Result<Vec<u8>, PlatformError> {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| PlatformError("File paths are not valid UTF-8".into()))?;
        if text.is_empty() || text.contains('\0') || text.lines().any(str::is_empty) {
            return Err(PlatformError(
                "File paths contain an empty path or NUL".into(),
            ));
        }
        let mut utf16 = text
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
        Ok(payload)
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

/// Owns an allocation until Windows accepts it through SetClipboardData.
struct OwnedGlobal(HGLOBAL);
impl OwnedGlobal {
    fn new(bytes: &[u8]) -> Result<Self, PlatformError> {
        let memory = Self(
            unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) }.map_err(platform_error)?,
        );
        memory.fill_with(bytes, |global| unsafe { GlobalLock(global) })
    }
    fn fill_with(
        self,
        bytes: &[u8],
        lock: impl FnOnce(HGLOBAL) -> *mut c_void,
    ) -> Result<Self, PlatformError> {
        let memory = self;
        let pointer = lock(memory.0);
        if pointer.is_null() {
            return Err(PlatformError("Unable to lock clipboard memory".into()));
        }
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.cast::<u8>(), bytes.len());
            let _ = GlobalUnlock(memory.0);
        }
        Ok(memory)
    }
    fn publish(self, format: u32) -> Result<(), PlatformError> {
        self.transfer_with(|global| {
            unsafe { SetClipboardData(format, Some(HANDLE(global.0))) }
                .map(|_| ())
                .map_err(platform_error)
        })
    }
    fn transfer_with(
        mut self,
        publish: impl FnOnce(HGLOBAL) -> Result<(), PlatformError>,
    ) -> Result<(), PlatformError> {
        publish(self.0)?;
        self.0 = HGLOBAL::default();
        Ok(())
    }
}
impl Drop for OwnedGlobal {
    fn drop(&mut self) {
        if !self.0 .0.is_null() {
            let _ = unsafe { windows::Win32::Foundation::GlobalFree(Some(self.0)) };
        }
    }
}

/// Validation and allocation happen before the existing clipboard is cleared.
pub(super) struct PreparedClipboard(Vec<(u32, OwnedGlobal)>);
impl PreparedClipboard {
    pub(super) fn new(items: &[ClipboardRepresentation]) -> Result<Self, PlatformError> {
        let mut prepared = Vec::new();
        for item in items {
            let (format, bytes) = match item.format.as_str() {
                "text" => {
                    let text = std::str::from_utf8(&item.bytes)
                        .map_err(|_| PlatformError("Text representation is not UTF-8".into()))?;
                    if text.contains('\0') {
                        return Err(PlatformError("Text representation contains NUL".into()));
                    }
                    (13, ClipboardGuard::unicode_bytes(text))
                }
                "html" => (register_format("HTML Format")?, nul_terminated(&item.bytes)),
                "rtf" => (
                    register_format("Rich Text Format")?,
                    nul_terminated(&item.bytes),
                ),
                "image" => (
                    8,
                    bmp_to_dib(&item.bytes)
                        .ok_or_else(|| PlatformError("Image representation is not a BMP".into()))?,
                ),
                "files" => (15, ClipboardGuard::file_bytes(&item.bytes)?),
                _ => continue,
            };
            prepared.push((format, OwnedGlobal::new(&bytes)?));
        }
        if prepared.is_empty() {
            return Err(PlatformError(
                "No supported clipboard representations".into(),
            ));
        }
        Ok(Self(prepared))
    }
    pub(super) fn publish(self, owner: HWND) -> Result<(), PlatformError> {
        let _clipboard = ClipboardGuard::open_for_write(owner)?;
        unsafe { EmptyClipboard() }.map_err(platform_error)?;
        for (format, memory) in self.0 {
            memory.publish(format)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
