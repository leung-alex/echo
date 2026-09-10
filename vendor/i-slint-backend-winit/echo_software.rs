// Copyright © SixtyFPS GmbH <info@slint.dev>
// SPDX-License-Identifier: GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0
//! Echo's safe native presentation seam. Win32 ownership/allocation is supplied
//! by echo-windows. Slint rasterizes directly into the native window framebuffer.
use std::{cell::RefCell, rc::Rc};

/// Draw into premultiplied BGRA pixels. The bool signals a newly allocated buffer;
/// the return value says whether the renderer changed any pixels.
pub type Draw<'a> = dyn FnMut(&mut [u32], bool) -> bool + 'a;
/// UI-thread-only native window presenter; the draw callback must not escape.
/// False means a hidden surface skipped drawing and needs another redraw on show.
pub type Presenter = dyn Fn(isize, u32, u32, &mut Draw<'_>) -> Result<bool, String>;
thread_local! { static PRESENTER: RefCell<Option<Rc<Presenter>>> = RefCell::new(None); }
thread_local! { static BEFORE_FRAME: RefCell<Option<Rc<dyn Fn()>>> = RefCell::new(None); }
/// Called on the UI thread before rasterization, with no renderer buffer borrowed.
pub fn install_before_frame(callback: Rc<dyn Fn()>) { BEFORE_FRAME.with(|slot| *slot.borrow_mut() = Some(callback)); }
pub(crate) fn before_frame() { BEFORE_FRAME.with(|slot| { if let Some(callback) = slot.borrow().as_ref() { callback(); } }); }
/// Install before the first software window. Other renderers do not use it.
pub fn install(presenter: Rc<Presenter>) { PRESENTER.with(|slot| *slot.borrow_mut() = Some(presenter)); }
pub(crate) fn presenter() -> Option<Rc<Presenter>> { PRESENTER.with(|slot| slot.borrow().clone()) }
