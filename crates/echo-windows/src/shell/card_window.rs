//! Native ownership-safe clipping and chrome for floating content cards.
use super::{common::error, window::owned};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::*, Gdi::*},
    UI::{Controls::MARGINS, WindowsAndMessaging::*},
};
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CardShape {
    Rounded { bounds: [i32; 4], radius: i32 },
    Polygon(Vec<[i32; 2]>),
}
fn valid_coordinate(value: i32) -> bool {
    (-1_000_000..=1_000_000).contains(&value)
}
impl CardShape {
    fn validate(&self) -> Result<(), String> {
        let valid = match self {
            Self::Rounded {
                bounds: [l, t, r, b],
                radius,
            } => {
                [*l, *t, *r, *b].into_iter().all(valid_coordinate)
                    && r > l
                    && b > t
                    && (0..=10_000).contains(radius)
            }
            Self::Polygon(points) => {
                (3..=32).contains(&points.len())
                    && points.iter().flatten().copied().all(valid_coordinate)
            }
        };
        if valid {
            Ok(())
        } else {
            Err("Invalid card window shape".into())
        }
    }
}
struct Region(HRGN);
impl Drop for Region {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                DeleteObject(self.0);
            }
        }
    }
}
/// Shapes are physical client coordinates. `None` clears the clip for a moving deck.
/// SetWindowRgn transfers ownership only on success; temporary regions always use RAII.
pub fn set_card_region(handle: isize, shapes: Option<&[CardShape]>) -> Result<(), String> {
    let hwnd = owned(handle)?;
    if let Some(shapes) = shapes {
        if shapes.is_empty() || shapes.len() > 8 {
            return Err("Invalid card shape count".into());
        }
        for shape in shapes {
            shape.validate()?;
        }
    }
    unsafe {
        let Some(shapes) = shapes else {
            return if SetWindowRgn(hwnd, std::ptr::null_mut(), 1) != 0 {
                Ok(())
            } else {
                Err(error())
            };
        };
        let mut combined = Region(CreateRectRgn(0, 0, 0, 0));
        if combined.0.is_null() {
            return Err(error());
        }
        for shape in shapes {
            let part = Region(match shape {
                CardShape::Rounded {
                    bounds: [l, t, r, b],
                    radius,
                } => CreateRoundRectRgn(*l, *t, *r, *b, radius * 2, radius * 2),
                CardShape::Polygon(points) => {
                    let points = points
                        .iter()
                        .map(|p| POINT { x: p[0], y: p[1] })
                        .collect::<Vec<_>>();
                    CreatePolygonRgn(points.as_ptr(), points.len() as i32, ALTERNATE)
                }
            });
            if part.0.is_null() || CombineRgn(combined.0, combined.0, part.0, RGN_OR) == RGN_ERROR {
                return Err(error());
            }
        }
        let mut rect: RECT = std::mem::zeroed();
        let mut origin = POINT { x: 0, y: 0 };
        if GetWindowRect(hwnd, &mut rect) == 0 || ClientToScreen(hwnd, &mut origin) == 0 {
            return Err(error());
        }
        if OffsetRgn(combined.0, origin.x - rect.left, origin.y - rect.top) == RGN_ERROR {
            return Err(error());
        }
        if SetWindowRgn(hwnd, combined.0, 1) == 0 {
            return Err(error());
        }
        combined.0 = std::ptr::null_mut();
    }
    Ok(())
}
/// The application paints card shadows; DWM must not paint an outer rectangle.
pub fn apply_card_chrome(handle: isize) -> Result<(), String> {
    let hwnd = owned(handle)?;
    unsafe {
        let policy = DWMNCRP_DISABLED;
        let corners = DWMWCP_DONOTROUND;
        let border = DWMWA_COLOR_NONE;
        let backdrop = DWMSBT_NONE;
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY as u32,
            (&policy as *const i32).cast(),
            4,
        );
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE as u32,
            (&corners as *const i32).cast(),
            4,
        );
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_BORDER_COLOR as u32,
            (&border as *const u32).cast(),
            4,
        );
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_SYSTEMBACKDROP_TYPE as u32,
            (&backdrop as *const i32).cast(),
            4,
        );
        let margins = MARGINS {
            cxLeftWidth: 0,
            cxRightWidth: 0,
            cyTopHeight: 0,
            cyBottomHeight: 0,
        };
        DwmExtendFrameIntoClientArea(hwnd, &margins);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_foreign_or_invalid_window() {
        assert!(apply_card_chrome(0).is_err());
        assert!(set_card_region(0, None).is_err());
    }
    #[test]
    fn region_input_is_bounded_before_native_allocation() {
        assert!(CardShape::Polygon(vec![[0, 0]; 33]).validate().is_err());
        assert!(CardShape::Polygon(vec![[0, 0]; 2]).validate().is_err());
        assert!(CardShape::Rounded {
            bounds: [0, 0, 0, 100],
            radius: 20
        }
        .validate()
        .is_err());
        assert!(CardShape::Rounded {
            bounds: [0, 0, 100, 100],
            radius: -1
        }
        .validate()
        .is_err());
        assert!(CardShape::Rounded {
            bounds: [0, 0, 100, 100],
            radius: 20
        }
        .validate()
        .is_ok());
        assert!(CardShape::Polygon(vec![[-100, 0], [0, 100], [100, 0]])
            .validate()
            .is_ok());
    }
}

pub(super) fn resize_hit([l, t, r, b]: [i32; 4], [x, y]: [i32; 2], edge: i32) -> i32 {
    if r <= l || b <= t || edge <= 0 {
        return 0;
    }
    let [l, t, r, b, x, y, e] = [l, t, r, b, x, y, edge].map(i64::from);
    let vertical = y >= t - e && y <= b + e;
    let horizontal = x >= l - e && x <= r + e;
    let left = vertical && (x - l).abs() <= e;
    let right = vertical && (x - r).abs() <= e;
    let top = horizontal && (y - t).abs() <= e;
    let bottom = horizontal && (y - b).abs() <= e;
    (match (left, right, top, bottom) {
        (true, _, true, _) => HTTOPLEFT,
        (_, true, true, _) => HTTOPRIGHT,
        (true, _, _, true) => HTBOTTOMLEFT,
        (_, true, _, true) => HTBOTTOMRIGHT,
        (true, _, _, _) => HTLEFT,
        (_, true, _, _) => HTRIGHT,
        (_, _, true, _) => HTTOP,
        (_, _, _, true) => HTBOTTOM,
        _ => 0,
    }) as i32
}
#[cfg(test)]
mod hit_tests {
    use super::*;
    #[test]
    fn side_cards_and_shadows_are_not_resize_handles() {
        let bounds = [190, 24, 930, 776];
        assert_eq!(resize_hit(bounds, [100, 400], 6), 0);
        assert_eq!(resize_hit(bounds, [190, 850], 6), 0);
        assert_eq!(resize_hit(bounds, [1100, 776], 6), 0);
        assert_eq!(resize_hit(bounds, [190, 400], 6), HTLEFT as i32);
        assert_eq!(resize_hit(bounds, [928, 774], 6), HTBOTTOMRIGHT as i32);
    }
    #[test]
    fn negative_monitor_coordinates_are_supported() {
        assert_eq!(
            resize_hit([-1730, 24, -990, 776], [-1730, 400], 6),
            HTLEFT as i32
        );
        assert_eq!(resize_hit([0, 0, 0, 0], [0, 0], 6), 0);
    }
}
