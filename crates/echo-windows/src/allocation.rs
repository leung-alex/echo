//! Scoped accounting of Rust-owned display data, including opaque UI strings.
//! Allocation still uses System. This does not measure GDI, GPU or process peaks.
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

thread_local! {
    static ACTIVE: Cell<bool> = const { Cell::new(false) };
    static NET: Cell<isize> = const { Cell::new(0) };
}
fn account(bytes: isize) {
    let _ = ACTIVE.try_with(|active| {
        if active.get() {
            let _ = NET.try_with(|net| net.set(net.get().saturating_add(bytes)));
        }
    });
}
pub struct AccountedSystem;
unsafe impl GlobalAlloc for AccountedSystem {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc(layout);
        if !p.is_null() {
            account(layout.size() as isize);
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(layout);
        if !p.is_null() {
            account(layout.size() as isize);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        account(-(layout.size() as isize));
        System.dealloc(p, layout);
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let next = System.realloc(p, layout, size);
        if !next.is_null() {
            account(size as isize - layout.size() as isize);
        }
        next
    }
}
struct Scope;
impl Drop for Scope {
    fn drop(&mut self) {
        ACTIVE.with(|active| active.set(false));
    }
}
/// Measure a *fresh owned result*: inputs must be borrowed, and temporary
/// allocations must be destroyed inside the closure. Existing objects must not
/// be freed in it. Clone/shared references are not charged twice.
pub fn measure_owned<T>(build: impl FnOnce() -> T) -> (T, usize) {
    ACTIVE.with(|active| assert!(!active.replace(true), "Nested display allocation scope"));
    NET.with(|net| net.set(0));
    let scope = Scope;
    let result = build();
    let bytes = NET.with(|net| net.get());
    drop(scope);
    assert!(
        bytes >= 0,
        "Display allocation scope freed preexisting ownership"
    );
    (result, bytes as usize)
}
