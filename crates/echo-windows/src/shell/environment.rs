//! Read-only Windows display/accessibility policy and owned-window placement.
use super::{common::error, window::owned};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::Power::*,
    UI::{Accessibility::*, HiDpi::GetDpiForWindow, WindowsAndMessaging::*},
};
#[derive(Debug, Clone, Copy)]
pub struct UiEnvironment {
    pub animations: bool,
    pub high_contrast: bool,
    pub on_battery: bool,
    pub refresh_hz: u32,
    pub background: u32,
    pub text: u32,
    pub highlight: u32,
    pub highlight_text: u32,
    pub muted: u32,
}
pub fn ui_environment(handle: Option<isize>) -> UiEnvironment {
    unsafe {
        let mut animations = 1i32;
        if SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            (&mut animations as *mut i32).cast(),
            0,
        ) == 0
        {
            animations = 0;
        }
        let mut contrast: HIGHCONTRASTW = std::mem::zeroed();
        contrast.cbSize = std::mem::size_of::<HIGHCONTRASTW>() as u32;
        let high_contrast = SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            contrast.cbSize,
            (&mut contrast as *mut HIGHCONTRASTW).cast(),
            0,
        ) != 0
            && contrast.dwFlags & HCF_HIGHCONTRASTON != 0;
        let mut power: SYSTEM_POWER_STATUS = std::mem::zeroed();
        let on_battery = GetSystemPowerStatus(&mut power) != 0
            && power.ACLineStatus == 0
            && power.BatteryFlag != 128;
        let hwnd = handle
            .and_then(|h| owned(h).ok())
            .unwrap_or(std::ptr::null_mut());
        let dc = GetDC(hwnd);
        let hz = if dc.is_null() {
            60
        } else {
            let value = GetDeviceCaps(dc, VREFRESH as i32);
            ReleaseDC(hwnd, dc);
            value
        };
        UiEnvironment {
            animations: animations != 0,
            high_contrast,
            on_battery,
            refresh_hz: if (40..=240).contains(&hz) {
                hz as u32
            } else {
                60
            },
            background: GetSysColor(COLOR_WINDOW),
            text: GetSysColor(COLOR_WINDOWTEXT),
            highlight: GetSysColor(COLOR_HIGHLIGHT),
            highlight_text: GetSysColor(COLOR_HIGHLIGHTTEXT),
            muted: GetSysColor(COLOR_GRAYTEXT),
        }
    }
}
fn fit_bounds(rect: RECT, work: RECT, margin: i32, center: bool) -> RECT {
    let margin = margin
        .max(0)
        .min(((work.right - work.left).min(work.bottom - work.top) / 8).max(0));
    let width = (rect.right - rect.left).clamp(1, (work.right - work.left - 2 * margin).max(1));
    let height = (rect.bottom - rect.top).clamp(1, (work.bottom - work.top - 2 * margin).max(1));
    let left = if center {
        work.left + (work.right - work.left - width) / 2
    } else {
        rect.left.clamp(
            work.left + margin,
            (work.right - margin - width).max(work.left + margin),
        )
    };
    let top = if center {
        work.top + (work.bottom - work.top - height) / 2
    } else {
        rect.top.clamp(
            work.top + margin,
            (work.bottom - margin - height).max(work.top + margin),
        )
    };
    RECT {
        left,
        top,
        right: left + width,
        bottom: top + height,
    }
}
pub fn fit_window(handle: isize, center: bool, margin_dip: f32) -> Result<(), String> {
    let hwnd = owned(handle)?;
    unsafe {
        let mut rect: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut rect) == 0 {
            return Err(error());
        }
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return Err(error());
        }
        let dpi = GetDpiForWindow(hwnd).max(96) as f32 / 96.0;
        let fit = fit_bounds(rect, info.rcWork, (margin_dip * dpi).round() as i32, center);
        if (rect.left, rect.top, rect.right, rect.bottom)
            != (fit.left, fit.top, fit.right, fit.bottom)
            && SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                fit.left,
                fit.top,
                fit.right - fit.left,
                fit.bottom - fit.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            ) == 0
        {
            return Err(error());
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oversized_high_dpi_window_stays_on_negative_coordinate_monitor() {
        let work = RECT {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1040,
        };
        let fit = fit_bounds(
            RECT {
                left: -1800,
                top: 50,
                right: 440,
                bottom: 1650,
            },
            work,
            32,
            true,
        );
        assert_eq!(
            (fit.left, fit.top, fit.right, fit.bottom),
            (-1888, 32, -32, 1008)
        );
    }
    #[test]
    fn small_workarea_is_not_forced_to_a_large_ui_minimum() {
        let fit = fit_bounds(
            RECT {
                left: 0,
                top: 0,
                right: 1120,
                bottom: 800,
            },
            RECT {
                left: 0,
                top: 0,
                right: 320,
                bottom: 240,
            },
            16,
            false,
        );
        assert!(fit.right <= 304 && fit.bottom <= 224);
        assert!(fit.left >= 16 && fit.top >= 16);
    }
    #[test]
    fn foreign_or_invalid_window_is_not_moved() {
        assert!(fit_window(0, true, 16.0).is_err());
    }
}
