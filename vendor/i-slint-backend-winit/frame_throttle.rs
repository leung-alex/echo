// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0

use std::rc::{Rc, Weak};

use i_slint_core::timers::{Timer, TimerMode};

#[cfg(target_vendor = "apple")]
mod apple_display_link;

use crate::winitwindowadapter::WinitWindowAdapter;

pub fn create_frame_throttle(
    window_adapter: Weak<WinitWindowAdapter>,
    _winit_window: &winit::window::Window,
    _is_wayland: bool,
) -> Box<dyn FrameThrottle> {
    #[cfg(feature = "echo-software-present")]
    if crate::echo_software::presenter().is_some() {
        return Box::new(EchoFrameThrottle {
            window_adapter,
            timer: Timer::default(),
            last: Rc::new(std::cell::Cell::new(None)),
        });
    }
    if _is_wayland {
        WinitBasedFrameThrottle::create()
    } else {
        #[cfg(target_vendor = "apple")]
        if let Some(throttle) =
            apple_display_link::try_create(window_adapter.clone(), _winit_window)
        {
            return throttle;
        }
        TimerBasedFrameThrottle::create(window_adapter)
    }
}

pub trait FrameThrottle {
    fn request_throttled_redraw(&self, winit_window: &winit::window::Window);
    fn frame_started(&self) {}
}

/// Immediate first frame, then an absolute 60 Hz deadline. A fresh timer delay
/// on every dirty notification would add another frame to Echo's motion clock.
#[cfg(feature = "echo-software-present")]
struct EchoFrameThrottle {
    window_adapter: Weak<WinitWindowAdapter>,
    timer: Timer,
    last: Rc<std::cell::Cell<Option<std::time::Instant>>>,
}
#[cfg(feature = "echo-software-present")]
impl FrameThrottle for EchoFrameThrottle {
    fn frame_started(&self) { self.last.set(Some(std::time::Instant::now())); }
    fn request_throttled_redraw(&self, window: &winit::window::Window) {
        if self.timer.running() { return; }
        let interval = std::time::Duration::from_nanos(16_666_667);
        let remaining = self.last.get().map_or(std::time::Duration::ZERO,
            |last| interval.saturating_sub(last.elapsed()));
        if remaining.is_zero() {
            window.request_redraw();
        } else {
            let window = self.window_adapter.clone();
            // Slint timers have millisecond resolution: round upward, never
            // schedule above the ceiling. A stopped scene owns no active timer.
            let delay = std::time::Duration::from_millis(remaining.as_micros().div_ceil(1000) as u64);
            self.timer.start(TimerMode::SingleShot, delay, move || {
                redraw_now(&window);
            });
        }
    }
}

struct TimerBasedFrameThrottle {
    window_adapter: Weak<WinitWindowAdapter>,
    timer: Rc<Timer>,
}

impl TimerBasedFrameThrottle {
    fn create(window_adapter: Weak<WinitWindowAdapter>) -> Box<dyn FrameThrottle> {
        Box::new(Self { window_adapter, timer: Rc::new(Timer::default()) })
    }
}

impl FrameThrottle for TimerBasedFrameThrottle {
    fn request_throttled_redraw(&self, winit_window: &winit::window::Window) {
        if self.timer.running() {
            return;
        }
        let refresh_interval_millihertz = winit_window
            .current_monitor()
            .and_then(|monitor| monitor.refresh_rate_millihertz())
            .unwrap_or(60000) as u64;
        let window_adapter = self.window_adapter.clone();
        let timer = Rc::downgrade(&self.timer);
        let interval =
            std::time::Duration::from_millis((1000 * 1000) / refresh_interval_millihertz);
        self.timer.start(TimerMode::Repeated, interval, move || {
            redraw_now(&window_adapter);

            let Some(timer) = timer.upgrade() else { return };
            let Some(window_adapter) = window_adapter.upgrade() else { return };

            let keep_running = window_adapter.pending_redraw();

            if timer.running() {
                if !keep_running {
                    timer.stop();
                }
            } else if keep_running {
                timer.restart();
            }
        });
    }
}

fn redraw_now(window_adapter: &Weak<WinitWindowAdapter>) {
    let Some(winit_window) = window_adapter.upgrade().and_then(|adapter| adapter.winit_window())
    else {
        return;
    };
    winit_window.request_redraw();
}

struct WinitBasedFrameThrottle;

impl WinitBasedFrameThrottle {
    fn create() -> Box<dyn FrameThrottle> {
        Box::new(Self)
    }
}

impl FrameThrottle for WinitBasedFrameThrottle {
    fn request_throttled_redraw(&self, winit_window: &winit::window::Window) {
        winit_window.request_redraw();
    }
}
