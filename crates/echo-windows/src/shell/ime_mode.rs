//! Preserve the user's mode when Slint detaches/re-attaches this HWND's IME.
//! Never reads or writes another application's input context.
use std::cell::Cell;
use windows_sys::Win32::{
    Foundation::HWND,
    UI::Input::{Ime::*, KeyboardAndMouse::GetKeyboardLayout},
};

#[derive(Clone, Copy)]
struct Mode {
    layout: isize,
    open: bool,
    conversion: u32,
    sentence: u32,
}

#[derive(Default)]
pub(super) struct ImeMode {
    saved: Cell<Option<Mode>>,
    restoring: Cell<bool>,
}

impl ImeMode {
    pub fn clear(&self) {
        self.saved.set(None);
    }

    pub unsafe fn context_changed(
        &self,
        hwnd: HWND,
        activate: bool,
        forward: impl FnOnce() -> isize,
    ) -> isize {
        if !activate {
            self.remember(hwnd);
        }
        // Default-context activation may itself emit mode notifications. Those
        // defaults must not overwrite the user's mode before we restore it.
        let nested = self.restoring.replace(true);
        let result = forward();
        self.restoring.set(nested);
        if activate && !nested {
            self.restore(hwnd);
        }
        result
    }

    pub unsafe fn remember(&self, hwnd: HWND) {
        if self.restoring.get() {
            return;
        }
        let context = ImmGetContext(hwnd);
        if context.is_null() {
            return;
        }
        let mut conversion = 0;
        let mut sentence = 0;
        if ImmGetConversionStatus(context, &mut conversion, &mut sentence) != 0 {
            self.saved.set(Some(Mode {
                layout: GetKeyboardLayout(0) as isize,
                open: ImmGetOpenStatus(context) != 0,
                conversion,
                sentence,
            }));
        }
        ImmReleaseContext(hwnd, context);
    }

    pub unsafe fn restore(&self, hwnd: HWND) {
        if self.restoring.replace(true) {
            return;
        }
        if let Some(mode) = self.saved.get() {
            if mode.layout == GetKeyboardLayout(0) as isize {
                let context = ImmGetContext(hwnd);
                if !context.is_null() {
                    let converted = ImmSetConversionStatus(context, mode.conversion, mode.sentence);
                    let opened = ImmSetOpenStatus(context, mode.open as i32);
                    ImmReleaseContext(hwnd, context);
                    if converted == 0 || opened == 0 {
                        self.clear();
                    }
                }
            } else {
                self.clear();
            }
        }
        self.restoring.set(false);
    }
}
