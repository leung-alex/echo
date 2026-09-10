//! One premultiplied BGRA DIB is the software window's presentation buffer.
//! The renderer writes into it directly; there is no screenshot/copy/compositor.
use super::{common::error, window::owned};
use std::{ffi::c_void, marker::PhantomData, rc::Rc};
use windows_sys::Win32::{Foundation::*, Graphics::Gdi::*, UI::WindowsAndMessaging::*};

/// A high-resolution timer lease only while finite card motion is running.
/// Dropping it restores the application's ordinary idle timer resolution.
pub struct AnimationClock;
impl AnimationClock {
    pub fn start() -> Option<Self> {
        (unsafe { windows_sys::Win32::Media::timeBeginPeriod(1) } == 0).then_some(Self)
    }
}
impl Drop for AnimationClock {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Media::timeEndPeriod(1);
        }
    }
}

pub struct SoftwareFrame {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    pixels: *mut c_void,
    size: (u32, u32),
    hwnd: HWND,
    invalidated: bool,
    // GDI presentation belongs to the same UI thread as its owned HWND.
    _ui_thread: PhantomData<Rc<()>>,
}
impl Default for SoftwareFrame {
    fn default() -> Self {
        Self {
            dc: std::ptr::null_mut(),
            bitmap: std::ptr::null_mut(),
            previous: std::ptr::null_mut(),
            pixels: std::ptr::null_mut(),
            size: (0, 0),
            hwnd: std::ptr::null_mut(),
            invalidated: true,
            _ui_thread: PhantomData,
        }
    }
}
impl SoftwareFrame {
    pub fn invalidate(&mut self) {
        self.invalidated = true;
    }
    pub fn release(&mut self) {
        unsafe {
            if !self.dc.is_null() && !self.previous.is_null() {
                SelectObject(self.dc, self.previous);
            }
            if !self.bitmap.is_null() {
                DeleteObject(self.bitmap);
            }
            if !self.dc.is_null() {
                DeleteDC(self.dc);
            }
        }
        self.dc = std::ptr::null_mut();
        self.bitmap = std::ptr::null_mut();
        self.previous = std::ptr::null_mut();
        self.pixels = std::ptr::null_mut();
        self.size = (0, 0);
    }
    pub fn bytes(&self) -> usize {
        self.size.0 as usize * self.size.1 as usize * 4
    }
    pub fn render(
        &mut self,
        handle: isize,
        width: u32,
        height: u32,
        draw: &mut dyn FnMut(&mut [u32], bool) -> bool,
    ) -> Result<FrameOutcome, String> {
        let hwnd = owned(handle)?;
        if unsafe { GetWindowThreadProcessId(hwnd, std::ptr::null_mut()) }
            != unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() }
        {
            return Err("Software presentation must run on its window thread".into());
        }
        let len = width
            .checked_mul(height)
            .filter(|_| width > 0 && height > 0 && width <= 16384 && height <= 16384)
            .ok_or("Invalid software window dimensions")? as usize;
        // A late redraw cannot recreate a hidden window's reclaimed buffer.
        if unsafe { IsWindowVisible(hwnd) } == 0 {
            return Ok(FrameOutcome::Hidden);
        }
        let fresh = self.size != (width, height) || self.hwnd != hwnd;
        if fresh {
            self.release();
            let mut info: BITMAPINFO = unsafe { std::mem::zeroed() };
            info.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
            info.bmiHeader.biWidth = width as i32;
            info.bmiHeader.biHeight = -(height as i32);
            info.bmiHeader.biPlanes = 1;
            info.bmiHeader.biBitCount = 32;
            info.bmiHeader.biCompression = BI_RGB;
            unsafe {
                self.dc = CreateCompatibleDC(std::ptr::null_mut());
                if self.dc.is_null() {
                    return Err(format!("Create software presentation DC: {}", error()));
                }
                self.bitmap = CreateDIBSection(
                    self.dc,
                    &info,
                    DIB_RGB_COLORS,
                    &mut self.pixels,
                    std::ptr::null_mut(),
                    0,
                );
                if self.bitmap.is_null() || self.pixels.is_null() {
                    let detail = error();
                    self.release();
                    return Err(format!("Create software presentation buffer: {detail}"));
                }
                self.previous = SelectObject(self.dc, self.bitmap);
                if self.previous.is_null() || self.previous as isize == -1 {
                    let detail = error();
                    self.previous = std::ptr::null_mut();
                    self.release();
                    return Err(format!("Select software presentation buffer: {detail}"));
                }
            }
            self.hwnd = hwnd;
            self.size = (width, height);
        }
        // Winit reapplies its window flags on show; that can remove WS_EX_LAYERED
        // even when this warm DIB is retained. Reassert it before every present.
        let style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) };
        let renewed = style & WS_EX_LAYERED as isize == 0;
        if renewed {
            unsafe {
                SetLastError(0);
                if SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_LAYERED as isize) == 0
                    && GetLastError() != 0
                {
                    return Err(format!("Enable software window alpha: {}", error()));
                }
                if SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                ) == 0
                {
                    return Err(format!("Refresh software window alpha: {}", error()));
                }
            }
        }
        // CreateDIBSection owns exactly len contiguous u32 pixels until release.
        // The safe, scoped callback cannot retain a reference to this allocation.
        let pixels = unsafe { std::slice::from_raw_parts_mut(self.pixels.cast::<u32>(), len) };
        if !draw(pixels, fresh || renewed || self.invalidated) {
            return Ok(FrameOutcome::Unchanged);
        }
        let size = SIZE {
            cx: width as i32,
            cy: height as i32,
        };
        let origin = POINT { x: 0, y: 0 };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        // No destination position or focus change: Quick Insert's anchor is intact.
        if unsafe {
            UpdateLayeredWindow(
                hwnd,
                std::ptr::null_mut(),
                std::ptr::null(),
                &size,
                self.dc,
                &origin,
                0,
                &blend,
                ULW_ALPHA,
            )
        } == 0
        {
            return Err(format!("Present software window: {}", error()));
        }
        unsafe {
            ValidateRect(hwnd, std::ptr::null());
        }
        self.invalidated = false;
        Ok(FrameOutcome::Presented)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameOutcome {
    Hidden,
    Unchanged,
    Presented,
}
impl Drop for SoftwareFrame {
    fn drop(&mut self) {
        self.release();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_owner_never_allocates_or_invokes_renderer() {
        let mut frame = SoftwareFrame::default();
        let mut called = false;
        assert!(frame
            .render(0, 520, 560, &mut |_, _| {
                called = true;
                true
            })
            .is_err());
        assert!(!called);
        assert_eq!(frame.bytes(), 0);
        frame.release();
        frame.release();
        assert_eq!(frame.bytes(), 0);
    }

    #[test]
    fn warm_buffer_survives_window_style_reset_without_reallocation() {
        unsafe {
            // An owned off-screen tool window; no input or desktop capture.
            let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
            let hwnd = CreateWindowExW(
                WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
                class.as_ptr(),
                std::ptr::null(),
                WS_POPUP | WS_VISIBLE,
                -10000,
                -10000,
                32,
                32,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            );
            assert!(!hwnd.is_null());
            let mut frame = SoftwareFrame::default();
            let mut fresh_frames = 0;
            let mut draw = |pixels: &mut [u32], fresh: bool| {
                fresh_frames += usize::from(fresh);
                pixels.fill(0xff406080);
                true
            };
            let first = frame.render(hwnd as isize, 32, 32, &mut draw);
            let allocation = frame.pixels;
            let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style & !(WS_EX_LAYERED as isize));
            let second = frame.render(hwnd as isize, 32, 32, &mut draw);
            let reused = allocation == frame.pixels;
            let unchanged = frame.render(hwnd as isize, 32, 32, &mut |_, _| false);
            frame.invalidate();
            let forced = frame.render(hwnd as isize, 32, 32, &mut |_, fresh| fresh);
            ShowWindow(hwnd, SW_HIDE);
            let hidden = frame.render(hwnd as isize, 32, 32, &mut |_, _| panic!("hidden draw"));
            frame.release();
            DestroyWindow(hwnd);
            assert_eq!(first, Ok(FrameOutcome::Presented));
            assert_eq!(second, Ok(FrameOutcome::Presented));
            assert_eq!(fresh_frames, 2);
            assert!(reused);
            assert_eq!(unchanged, Ok(FrameOutcome::Unchanged));
            assert_eq!(forced, Ok(FrameOutcome::Presented));
            assert_eq!(hidden, Ok(FrameOutcome::Hidden));
        }
    }
}
